use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Log {
        line: String,
        is_stderr: bool,
    },
    Progress {
        time_s: f64,
        percent: Option<f64>,
        speed: Option<f64>,
        fps: Option<f64>,
    },
    Finished {
        exit_code: i32,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone)]
pub struct ProcessController {
    pub(crate) stdin: Arc<Mutex<Option<ChildStdin>>>,
    pub(crate) child: Arc<Mutex<Option<std::process::Child>>>,
    pub(crate) is_cancelled: Arc<AtomicBool>,
    pub(crate) delegate: Arc<Mutex<Option<ProcessController>>>,
}

impl ProcessController {
    pub fn new(stdin: Option<ChildStdin>, child: Option<std::process::Child>) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            delegate: Arc::new(Mutex::new(None)),
        }
    }

    pub fn new_empty() -> Self {
        Self::new(None, None)
    }

    pub fn with_child_arc(child: Arc<Mutex<Option<std::process::Child>>>, is_cancelled: Arc<AtomicBool>) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(None)),
            child,
            is_cancelled,
            delegate: Arc::new(Mutex::new(None)),
        }
    }

    pub fn attach_from(&self, other: &ProcessController) {
        if self.is_cancelled() {
            other.cancel();
        }
        if let Ok(mut lock) = self.delegate.lock() {
            *lock = Some(other.clone());
        }
    }

    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);

        // Forward cancellation to delegate if any
        if let Ok(lock) = self.delegate.lock() {
            if let Some(ref d) = *lock {
                d.cancel();
            }
        }

        // Attempt clean quit by writing 'q' to stdin
        if let Ok(mut lock) = self.stdin.lock() {
            if let Some(mut stdin) = lock.take() {
                let _ = stdin.write_all(b"q\n");
                let _ = stdin.flush();
            }
        }

        // Spawn a timeout thread to kill if still running after 2.5 seconds
        let child_lock = Arc::clone(&self.child);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(2500));
            if let Ok(mut lock) = child_lock.lock() {
                if let Some(ref mut child) = *lock {
                    let _ = child.kill();
                }
            }
        });
    }

    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressParser {
    None,
    Ffmpeg,
    WhisperJson,
}

pub struct Runner;

impl Runner {
    pub fn decorate_args(mut args: Vec<String>) -> Vec<String> {
        // Insert -progress pipe:1 to drive the UI progress bar
        if !args.iter().any(|a| a == "-progress") {
            args.insert(0, "pipe:1".to_string());
            args.insert(0, "-progress".to_string());
        }
        // Ensure stats (frame=, fps=, etc.) are emitted to stderr even when redirected to a pipe
        if !args.iter().any(|a| a == "-stats" || a == "-nostats") {
            args.insert(0, "-stats".to_string());
        }
        // Default loglevel to info if not already specified by caller/settings
        if !args.iter().any(|a| a == "-loglevel") {
            args.insert(0, "info".to_string());
            args.insert(0, "-loglevel".to_string());
        }
        args
    }

    pub fn start_ffmpeg_with_program(
        program: &std::path::Path,
        args: Vec<String>,
        total_duration_s: Option<f64>,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let args = Self::decorate_args(args);
        let prog_str = program.to_string_lossy();
        Self::start_process(&prog_str, &args, total_duration_s, ProgressParser::Ffmpeg)
    }

    pub fn start_ffmpeg(
        args: Vec<String>,
        total_duration_s: Option<f64>,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        Self::start_ffmpeg_with_program(std::path::Path::new("ffmpeg"), args, total_duration_s)
    }

    /// Generic process runner (used for ffmpeg, whisper, etc.)
    pub fn start_process(
        program: &str,
        args: &[String],
        total_duration_s: Option<f64>,
        progress_parser: ProgressParser,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let mut cmd = Command::new(program);
        cmd.args(args);
        crate::hide_console_window(&mut cmd);
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let controller = ProcessController::new(stdin, Some(child));
        let (tx, rx) = channel::<Event>();

        // Stdout reader thread
        let tx_stdout = tx.clone();
        let handle_stdout = stdout.map(|stdout_stream| {
            thread::spawn(move || match progress_parser {
                ProgressParser::Ffmpeg => {
                    parse_ffmpeg_stdout(stdout_stream, total_duration_s, tx_stdout);
                }
                ProgressParser::WhisperJson => {
                    parse_whisper_stdout(stdout_stream, total_duration_s, tx_stdout);
                }
                ProgressParser::None => {
                    let reader = BufReader::new(stdout_stream);
                    for line in reader.lines().map_while(Result::ok) {
                        let _ = tx_stdout.send(Event::Log {
                            line,
                            is_stderr: false,
                        });
                    }
                }
            })
        });

        // Stderr reader thread (splits on both \n and \r)
        let tx_stderr = tx.clone();
        let handle_stderr = stderr.map(|stderr_stream| {
            thread::spawn(move || {
                parse_stderr_lines(stderr_stream, tx_stderr);
            })
        });

        // Child wait thread
        let controller_clone = controller.clone();
        thread::spawn(move || {
            let wait_res = loop {
                let mut status_opt = None;
                if let Ok(mut lock) = controller_clone.child.lock() {
                    if let Some(ref mut c) = *lock {
                        match c.try_wait() {
                            Ok(Some(status)) => {
                                status_opt = Some(Ok(status));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                status_opt = Some(Err(e));
                            }
                        }
                    } else {
                        status_opt = Some(Err(std::io::Error::other(
                            "Process handle not found",
                        )));
                    }
                    if status_opt.is_some() {
                        *lock = None;
                    }
                }
                if let Some(res) = status_opt {
                    break res;
                }
                thread::sleep(Duration::from_millis(25));
            };

            // CRITICAL: Wait for reader threads to completely drain stdout and stderr pipes
            // before emitting Finished or Failed. This prevents dropping the last error logs!
            if let Some(h) = handle_stdout {
                let _ = h.join();
            }
            if let Some(h) = handle_stderr {
                let _ = h.join();
            }

            match wait_res {
                Ok(status) => {
                    if controller_clone.is_cancelled() {
                        let _ = tx.send(Event::Failed {
                            reason: "Processo annullato dall'utente".to_string(),
                        });
                    } else if let Some(code) = status.code() {
                        let _ = tx.send(Event::Finished { exit_code: code });
                    } else {
                        let _ = tx.send(Event::Failed {
                            reason: "Processo terminato da un segnale di sistema".to_string(),
                        });
                    }
                }
                Err(e) => {
                    let _ = tx.send(Event::Failed {
                        reason: e.to_string(),
                    });
                }
            }
        });

        Ok((controller, rx))
    }
}

fn parse_ffmpeg_stdout(
    stream: impl Read,
    total_duration_s: Option<f64>,
    tx: Sender<Event>,
) {
    let reader = BufReader::new(stream);
    let mut map: HashMap<String, String> = HashMap::new();

    for line_res in reader.lines() {
        let Ok(line) = line_res else { break };
        let trimmed = line.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());

            if k.trim() == "progress" {
                let mut time_s = 0.0;
                if let Some(us_str) = map.get("out_time_us") {
                    if let Ok(us) = us_str.parse::<f64>() {
                        time_s = us / 1_000_000.0;
                    }
                } else if let Some(ms_str) = map.get("out_time_ms") {
                    if let Ok(ms) = ms_str.parse::<f64>() {
                        time_s = ms / 1_000_000.0;
                    }
                }

                let percent = total_duration_s.and_then(|dur| {
                    if dur > 0.0 {
                        Some(((time_s / dur) * 100.0).clamp(0.0, 100.0))
                    } else {
                        None
                    }
                });

                let speed = map.get("speed").and_then(|s| {
                    s.trim_end_matches('x').trim().parse::<f64>().ok()
                });

                let fps = map.get("fps").and_then(|s| s.parse::<f64>().ok());

                let _ = tx.send(Event::Progress {
                    time_s,
                    percent,
                    speed,
                    fps,
                });
            }
        }
    }
}

fn parse_whisper_stdout(
    stream: impl Read,
    total_duration_s: Option<f64>,
    tx: Sender<Event>,
) {
    let reader = BufReader::new(stream);
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let start = val.get("start").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let end = val.get("end").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let text = val.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let total_dur = val
                .get("total_duration")
                .and_then(|v| v.as_f64())
                .or(total_duration_s);

            let percent = total_dur.and_then(|d| {
                if d > 0.0 {
                    Some(((end / d) * 100.0).clamp(0.0, 100.0))
                } else {
                    None
                }
            });

            let _ = tx.send(Event::Progress {
                time_s: end,
                percent,
                speed: None,
                fps: None,
            });

            let _ = tx.send(Event::Log {
                line: format!("[{:.1}s -> {:.1}s] {}", start, end, text),
                is_stderr: false,
            });
        } else {
            let _ = tx.send(Event::Log {
                line: trimmed.to_string(),
                is_stderr: false,
            });
        }
    }
}

fn parse_stderr_lines(stream: impl Read, tx: Sender<Event>) {
    let mut buffer = Vec::new();
    let mut byte_buf = [0u8; 1024];
    let mut reader = BufReader::new(stream);

    loop {
        match reader.read(&mut byte_buf) {
            Ok(0) => break,
            Ok(n) => {
                for &b in &byte_buf[..n] {
                    if b == b'\n' || b == b'\r' {
                        if !buffer.is_empty() {
                            let raw_line = String::from_utf8_lossy(&buffer);
                            let line_str = raw_line.trim_end_matches(&['\r', '\n'][..]);
                            if !line_str.trim().is_empty() {
                                let _ = tx.send(Event::Log {
                                    line: line_str.to_string(),
                                    is_stderr: true,
                                });
                            }
                            buffer.clear();
                        }
                    } else {
                        buffer.push(b);
                    }
                }
            }
            Err(_) => break,
        }
    }

    if !buffer.is_empty() {
        let raw_line = String::from_utf8_lossy(&buffer);
        let line_str = raw_line.trim_end_matches(&['\r', '\n'][..]);
        if !line_str.trim().is_empty() {
            let _ = tx.send(Event::Log {
                line: line_str.to_string(),
                is_stderr: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_short_job_success() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let out_file = temp_dir.path().join("out.mp4");

        let args = vec![
            "-hide_banner".to_string(),
            "-y".to_string(),
            "-f".to_string(),
            "lavfi".to_string(),
            "-i".to_string(),
            "testsrc=duration=0.5:size=160x120:rate=10".to_string(),
            "-c:v".to_string(),
            "libx264".to_string(),
            "-preset".to_string(),
            "ultrafast".to_string(),
            out_file.to_string_lossy().to_string(),
        ];

        let (_controller, rx) = Runner::start_ffmpeg(args, Some(0.5)).expect("start_ffmpeg");

        let mut received_progress = false;
        let mut received_finished = false;

        while let Ok(event) = rx.recv() {
            match event {
                Event::Progress { percent, .. } => {
                    if percent.is_some() {
                        received_progress = true;
                    }
                }
                Event::Finished { exit_code } => {
                    assert_eq!(exit_code, 0);
                    received_finished = true;
                }
                Event::Failed { reason } => {
                    panic!("Process unexpectedly failed: {}", reason);
                }
                Event::Log { .. } => {}
            }
        }

        assert!(received_progress, "Should have received progress event");
        assert!(received_finished, "Should have received finished event");
        assert!(out_file.exists());
    }

    #[test]
    fn test_runner_failing_job() {
        let args = vec![
            "-hide_banner".to_string(),
            "-i".to_string(),
            "/path/that/does/not/exist.xyz".to_string(),
            "/tmp/out_fail.mp4".to_string(),
        ];

        let (_controller, rx) = Runner::start_ffmpeg(args, None).expect("start_ffmpeg");

        let mut exit_code_non_zero = false;
        while let Ok(event) = rx.recv() {
            if let Event::Finished { exit_code } = event {
                if exit_code != 0 {
                    exit_code_non_zero = true;
                }
            }
        }
        assert!(exit_code_non_zero, "Expected non-zero exit code for failing command");
    }

    #[test]
    fn test_runner_last_log_arrives_before_finished() {
        let args = vec![
            "-hide_banner".to_string(),
            "-i".to_string(),
            "/nonexistent_file_error.xyz".to_string(),
            "/tmp/fail_test.mp4".to_string(),
        ];

        let (_controller, rx) = Runner::start_ffmpeg(args, None).expect("start_ffmpeg");

        let mut events = Vec::new();
        while let Ok(event) = rx.recv() {
            events.push(event);
        }

        assert!(!events.is_empty());
        let last_event = events.last().unwrap();
        assert!(matches!(last_event, Event::Finished { .. } | Event::Failed { .. }));

        let has_log_before = events[..events.len() - 1]
            .iter()
            .any(|e| matches!(e, Event::Log { .. }));
        assert!(has_log_before, "Logs from stderr must arrive BEFORE Finished/Failed");
    }

    #[test]
    fn test_process_controller_attach_from_delegation() {
        let parent_ctrl = ProcessController::new_empty();
        let child_ctrl = ProcessController::new_empty();

        parent_ctrl.attach_from(&child_ctrl);
        assert!(!child_ctrl.is_cancelled());

        parent_ctrl.cancel();
        assert!(parent_ctrl.is_cancelled());
        assert!(child_ctrl.is_cancelled(), "Delegated child controller must be cancelled");

        // Late attachment after parent is already cancelled:
        let late_child = ProcessController::new_empty();
        parent_ctrl.attach_from(&late_child);
        assert!(late_child.is_cancelled(), "Attaching to an already cancelled controller must immediately cancel child");
    }
}
