use crate::queue::TranscribeJob;
use crate::runner::{Event, ProcessController, ProgressParser, Runner};
use crate::storage::Storage;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct WhisperBackend;

impl WhisperBackend {
    pub fn venv_dir() -> PathBuf {
        Storage::whisper_env_dir().join("venv")
    }

    pub fn python_executable() -> PathBuf {
        #[cfg(windows)]
        {
            Self::venv_dir().join("Scripts").join("python.exe")
        }
        #[cfg(not(windows))]
        {
            Self::venv_dir().join("bin").join("python")
        }
    }

    pub fn is_env_ready() -> bool {
        let python = Self::python_executable();
        if !python.exists() {
            return false;
        }

        Command::new(python)
            .args(["-c", "import faster_whisper"])
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    /// Sets up the python venv and installs faster-whisper.
    /// Reads both stdout and stderr and supports proper cancellation.
    pub fn setup_env() -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let venv_dir = Self::venv_dir();
        let _ = Storage::ensure_dirs();

        let (tx, rx) = channel::<Event>();
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let child_arc: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let controller = ProcessController::with_child_arc(child_arc.clone(), is_cancelled.clone());

        thread::spawn(move || {
            let tx_log = |msg: String| {
                let _ = tx.send(Event::Log {
                    line: msg,
                    is_stderr: false,
                });
            };

            if is_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = tx.send(Event::Failed {
                    reason: "Installazione annullata dall'utente".to_string(),
                });
                return;
            }

            tx_log(format!("Creazione ambiente virtuale Python in {}...", venv_dir.display()));

            let py_cmd = if cfg!(windows) { "python" } else { "python3" };
            let venv_spawn = Command::new(py_cmd)
                .args(["-m", "venv"])
                .arg(&venv_dir)
                .spawn();

            let venv_child = match venv_spawn {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Event::Failed {
                        reason: format!("Impossibile avviare {} per creare l'ambiente virtuale: {}", py_cmd, e),
                    });
                    return;
                }
            };

            if let Ok(mut lock) = child_arc.lock() {
                *lock = Some(venv_child);
            }

            let venv_status = loop {
                let mut status_opt = None;
                if let Ok(mut lock) = child_arc.lock() {
                    if let Some(ref mut c) = *lock {
                        match c.try_wait() {
                            Ok(Some(status)) => status_opt = Some(Ok(status)),
                            Ok(None) => {}
                            Err(e) => status_opt = Some(Err(e)),
                        }
                    } else {
                        status_opt = Some(Err(std::io::Error::other(
                            "Process handle lost",
                        )));
                    }
                    if status_opt.is_some() {
                        *lock = None;
                    }
                }
                if let Some(res) = status_opt {
                    break res;
                }
                thread::sleep(std::time::Duration::from_millis(25));
            };

            if is_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = tx.send(Event::Failed {
                    reason: "Installazione annullata dall'utente".to_string(),
                });
                return;
            }

            match venv_status {
                Ok(s) if s.success() => {
                    tx_log("Ambiente virtuale creato con successo. Installazione di faster-whisper in corso...".to_string());
                }
                Ok(s) => {
                    let _ = tx.send(Event::Failed {
                        reason: format!("Creazione ambiente virtuale fallita con codice {:?}", s.code()),
                    });
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Event::Failed {
                        reason: format!("Errore durante la creazione dell'ambiente virtuale: {}", e),
                    });
                    return;
                }
            }

            let python_bin = Self::python_executable();
            let mut child = match Command::new(&python_bin)
                .args(["-m", "pip", "install", "--upgrade", "pip", "faster-whisper"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Event::Failed {
                        reason: format!("Errore esecuzione pip tramite {}: {}", python_bin.display(), e),
                    });
                    return;
                }
            };

            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            if let Ok(mut lock) = child_arc.lock() {
                *lock = Some(child);
            }

            let tx_out = tx.clone();
            let handle_out = stdout.map(|out_stream| {
                thread::spawn(move || {
                    let reader = BufReader::new(out_stream);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_out.send(Event::Log {
                            line,
                            is_stderr: false,
                        });
                    }
                })
            });

            let tx_err = tx.clone();
            let handle_err = stderr.map(|err_stream| {
                thread::spawn(move || {
                    let reader = BufReader::new(err_stream);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_err.send(Event::Log {
                            line,
                            is_stderr: true,
                        });
                    }
                })
            });

            let wait_res = loop {
                let mut status_opt = None;
                if let Ok(mut lock) = child_arc.lock() {
                    if let Some(ref mut c) = *lock {
                        match c.try_wait() {
                            Ok(Some(status)) => status_opt = Some(Ok(status)),
                            Ok(None) => {}
                            Err(e) => status_opt = Some(Err(e)),
                        }
                    } else {
                        status_opt = Some(Err(std::io::Error::other(
                            "Process handle lost",
                        )));
                    }
                    if status_opt.is_some() {
                        *lock = None;
                    }
                }
                if let Some(res) = status_opt {
                    break res;
                }
                thread::sleep(std::time::Duration::from_millis(25));
            };

            if let Some(h) = handle_out {
                let _ = h.join();
            }
            if let Some(h) = handle_err {
                let _ = h.join();
            }

            if is_cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                let _ = tx.send(Event::Failed {
                    reason: "Installazione annullata dall'utente".to_string(),
                });
                return;
            }

            match wait_res {
                Ok(s) if s.success() => {
                    tx_log("Installazione di faster-whisper completata con successo!".to_string());
                    let _ = tx.send(Event::Finished { exit_code: 0 });
                }
                Ok(s) => {
                    let code = s.code().unwrap_or(-1);
                    let _ = tx.send(Event::Finished { exit_code: code });
                }
                Err(e) => {
                    let _ = tx.send(Event::Failed {
                        reason: format!("Installazione fallita: {}", e),
                    });
                }
            }
        });

        Ok((controller, rx))
    }

    /// Run transcription using the embedded or provided whisper_run.py script.
    pub fn start_transcription(
        job: &TranscribeJob,
        audio_duration_s: Option<f64>,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let script_path = Storage::ensure_whisper_script()?;
        let python = Self::python_executable();

        let mut args = vec![
            script_path.to_string_lossy().to_string(),
            "--audio".to_string(),
            job.audio_file.to_string_lossy().to_string(),
            "--output".to_string(),
            job.output_file.to_string_lossy().to_string(),
            "--model".to_string(),
            job.model.clone(),
            "--device".to_string(),
            job.device.clone(),
            "--compute-type".to_string(),
            job.compute_type.clone(),
        ];

        if let Some(ref lang) = job.language {
            args.push("--language".to_string());
            args.push(lang.clone());
        }

        if job.vad_filter {
            args.push("--vad".to_string());
        }

        Runner::start_process(
            &python.to_string_lossy(),
            &args,
            audio_duration_s,
            ProgressParser::WhisperJson,
        )
    }
}
