use ffgui_core::builder::CommandBuilder;
use ffgui_core::catalog::Catalog;
use ffgui_core::model::{Job, OutStream, StreamKind};
use ffgui_core::probe::probe_file;
use ffgui_core::runner::{Event, Runner};
use ffgui_core::validate::validate_job;
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Uso: ffcli <input_file> <output_file> [codec: libx265|libx264|copy]");
        std::process::exit(1);
    }

    let input_path = PathBuf::from(&args[1]);
    let output_path = PathBuf::from(&args[2]);
    let codec_choice = args.get(3).map(|s| s.as_str()).unwrap_or("libx265");

    println!("=== ffgui Core CLI Test Utility ===");
    println!("File di input: {:?}", input_path);
    println!("File di output: {:?}", output_path);

    // 1. Probe input file
    println!("\n[1] Esecuzione probe con ffprobe...");
    let probe_info = probe_file(&input_path)?;
    println!(
        "Formato: {}, Durata: {:.2}s, Stream rilevati: {}",
        probe_info.format_name.as_deref().unwrap_or("sconosciuto"),
        probe_info.duration_s.unwrap_or(0.0),
        probe_info.streams.len()
    );

    for s in &probe_info.streams {
        println!(
            "  - Stream #{}: {:?} ({}) - {}x{} @ {:?} fps / {} canali @ {:?} Hz",
            s.index,
            s.kind,
            s.codec_name.as_deref().unwrap_or("?"),
            s.width.unwrap_or(0),
            s.height.unwrap_or(0),
            s.fps,
            s.channels.unwrap_or(0),
            s.sample_rate
        );
    }

    // 2. Load Catalog
    println!("\n[2] Caricamento catalogo codec...");
    let catalog_dir = Path::new("codecs");
    let catalog = Catalog::load_from_dirs(Some(catalog_dir), None, false)?;
    println!("Codec caricati nel catalogo: {}", catalog.codecs.len());

    // 3. Build Job Model
    println!("\n[3] Creazione del modello Job...");
    let mut job = Job::new(&input_path, &output_path);

    // Map each probe stream
    for s in &probe_info.streams {
        match s.kind {
            StreamKind::Video => {
                if codec_choice == "copy" {
                    job.output.streams.push(OutStream::new_copy(StreamKind::Video, s.index));
                } else {
                    let mut params = HashMap::new();
                    if codec_choice == "libx265" {
                        params.insert("crf".to_string(), serde_json::json!(23));
                        params.insert("preset".to_string(), serde_json::json!("medium"));
                        params.insert("pix_fmt".to_string(), serde_json::json!("yuv420p"));
                    }
                    job.output.streams.push(OutStream::new_encode(
                        StreamKind::Video,
                        s.index,
                        codec_choice,
                        params,
                    ));
                }
            }
            StreamKind::Audio => {
                job.output.streams.push(OutStream::new_copy(StreamKind::Audio, s.index));
            }
            _ => {}
        }
    }

    // 4. Validate
    println!("\n[4] Validazione del modello...");
    let issues = validate_job(&job, &catalog);
    let mut has_errors = false;
    for issue in &issues {
        if issue.is_error() {
            eprintln!("  [ERRORE] {}", issue.message);
            has_errors = true;
        } else {
            println!("  [AVVISO] {}", issue.message);
        }
    }
    if has_errors {
        eprintln!("La validazione ha rilevato errori bloccanti. Arresto.");
        std::process::exit(2);
    }
    println!("Validazione superata senza errori.");

    // 5. Build Command
    println!("\n[5] Generazione argomenti ffmpeg...");
    let cmd_args = CommandBuilder::build(&job, &catalog);
    let preview = CommandBuilder::to_preview_string(&cmd_args);
    println!("Comando generato:\n  {}", preview);

    // 6. Run process
    println!("\n[6] Avvio del processo ffmpeg...");
    let (_controller, rx) = Runner::start_ffmpeg(cmd_args, probe_info.duration_s)?;

    while let Ok(event) = rx.recv() {
        match event {
            Event::Log { line, is_stderr } => {
                if is_stderr {
                    eprintln!("  [log] {}", line);
                } else {
                    println!("  [out] {}", line);
                }
            }
            Event::Progress {
                time_s,
                percent,
                speed,
                fps,
            } => {
                let pct_str = percent.map(|p| format!("{:.1}%", p)).unwrap_or_else(|| "N/A".to_string());
                let spd_str = speed.map(|s| format!("{:.1}x", s)).unwrap_or_else(|| "N/A".to_string());
                let fps_str = fps.map(|f| format!("{:.0}fps", f)).unwrap_or_else(|| "N/A".to_string());
                print!("\r  -> Avanzamento: {} ({:.1}s) | vel: {} | {}", pct_str, time_s, spd_str, fps_str);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            Event::Finished { exit_code } => {
                println!("\nProcesso terminato con codice: {}", exit_code);
                if exit_code == 0 {
                    println!("Conversione riuscita!");
                }
            }
            Event::Failed { reason } => {
                eprintln!("\nProcesso fallito: {}", reason);
            }
        }
    }

    Ok(())
}
