use crate::app::FfGuiApp;
use eframe::egui::{self, Color32, ProgressBar, RichText};
use ffgui_core::model::StreamKind;

pub fn render(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.vertical(|ui| {
        // Upper Control & Progress Bar
        ui.horizontal(|ui| {
            // Action buttons
            if app.is_running {
                if ui.button(RichText::new("⏹ Interrompi").color(Color32::RED).strong()).clicked() {
                    app.cancel_current_job();
                }
            } else {
                let can_start = app.input_path.is_some() && app.output_path.is_some();
                if ui.add_enabled(can_start, egui::Button::new(RichText::new("▶ Avvia Codifica").color(Color32::GREEN).strong())).clicked() {
                    app.start_current_job();
                }

                if ui.add_enabled(can_start, egui::Button::new(RichText::new("⏱ Prova 10s").strong())).clicked() {
                    app.start_sample_test_job(10.0);
                }
            }

            // Progress bar
            if app.is_running && app.current_progress.is_none() {
                let time_val = ui.input(|i| i.time);
                let anim_pct = (time_val * 0.5).fract() as f32;
                let mut label = "In corso...".to_string();
                if let Some(time) = app.current_time_s {
                    label = format!("{:.1}s", time);
                }
                if let Some(spd) = app.current_speed {
                    label.push_str(&format!(" | {:.1}x", spd));
                }
                if let Some(fps) = app.current_fps {
                    label.push_str(&format!(" | {:.0} fps", fps));
                }
                ui.add(ProgressBar::new(anim_pct).text(label).desired_width(280.0));
            } else {
                let pct = app.current_progress.unwrap_or(0.0) as f32 / 100.0;
                let mut label = format!("{:.1}%", pct * 100.0);
                if let Some(time) = app.current_time_s {
                    label.push_str(&format!(" ({:.1}s)", time));
                }
                if let Some(spd) = app.current_speed {
                    label.push_str(&format!(" | {:.1}x", spd));
                }
                if let Some(fps) = app.current_fps {
                    label.push_str(&format!(" | {:.0} fps", fps));
                }
                ui.add(ProgressBar::new(pct).text(label).desired_width(280.0));
            }

            // Status indication
            if app.is_running {
                ui.colored_label(Color32::from_rgb(100, 255, 150), RichText::new("🟢 In esecuzione...").strong());
            } else if let Some(ref status) = app.exit_status {
                let is_failure = match status {
                    Ok(code) => *code != 0,
                    Err(_) => true,
                };

                match status {
                    Ok(code) => {
                        if *code == 0 {
                            ui.colored_label(Color32::GREEN, RichText::new("✔ Completato con successo (0)").strong());
                        } else {
                            ui.colored_label(Color32::RED, RichText::new(format!("✖ Uscita con codice {}", code)).strong());
                        }
                    }
                    Err(err) => {
                        ui.colored_label(Color32::RED, RichText::new(format!("✖ {}", err)).strong());
                    }
                }

                // When GPU encoding fails, offer "Riprova con la CPU"
                if is_failure {
                    let current_gpu_codec = app.stream_controls.iter()
                        .find(|c| c.kind == StreamKind::Video && c.enabled)
                        .and_then(|c| {
                            if let Some(def) = app.catalog.get_codec(&c.codec_id) {
                                if def.hw.is_some() {
                                    return Some(c.codec_id.clone());
                                }
                            }
                            None
                        });

                    if let Some(gpu_id) = current_gpu_codec {
                        if let Some(cpu_counterpart) = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(&gpu_id) {
                            if ui.button(RichText::new(format!("🔄 Riprova con CPU ({})", cpu_counterpart)).color(Color32::from_rgb(255, 180, 50)).strong()).clicked() {
                                let catalog = app.catalog.clone();
                                if let Some(v_ctrl) = app.stream_controls.iter_mut().find(|c| c.kind == StreamKind::Video && c.enabled) {
                                    v_ctrl.change_codec(cpu_counterpart, &catalog);
                                }
                                app.start_current_job();
                            }
                        }
                    }
                }
            } else {
                ui.colored_label(Color32::GRAY, "⚪ In attesa");
            }
        });

        ui.add_space(4.0);

        // Terminal Window Container
        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(14, 15, 18))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(45, 48, 55)))
            .inner_margin(0.0)
            .show(ui, |ui| {
                // Terminal Header Bar
                egui::Frame::NONE
                    .fill(Color32::from_rgb(26, 28, 33))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Terminal dots
                            ui.colored_label(Color32::from_rgb(255, 95, 87), "●");
                            ui.colored_label(Color32::from_rgb(255, 189, 46), "●");
                            ui.colored_label(Color32::from_rgb(39, 201, 63), "●");
                            ui.add_space(4.0);
                            ui.label(RichText::new("FFmpeg Terminal (stdout / stderr)").monospace().strong().color(Color32::from_rgb(220, 220, 225)));

                            ui.add_space(8.0);
                            ui.label(RichText::new("🔍").color(Color32::GRAY));
                            ui.add(
                                egui::TextEdit::singleline(&mut app.log_filter)
                                    .hint_text("Filtra output log...")
                                    .desired_width(140.0),
                            );
                            if !app.log_filter.is_empty() && ui.small_button("✕").clicked() {
                                app.log_filter.clear();
                            }

                            // Total lines / Filtered stats
                            let total_lines = app.logs.len();
                            let filter_lower = app.log_filter.trim().to_lowercase();
                            let is_filtered = !filter_lower.is_empty();
                            if is_filtered {
                                let match_count = app.logs.iter().filter(|(l, _)| l.to_lowercase().contains(&filter_lower)).count();
                                ui.small(RichText::new(format!("({}/{} righe)", match_count, total_lines)).color(Color32::from_rgb(100, 200, 255)));
                            } else {
                                ui.small(RichText::new(format!("({} righe)", total_lines)).color(Color32::GRAY));
                            }

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("🗑 Pulisci").clicked() {
                                    app.logs.clear();
                                    app.exit_status = None;
                                }

                                if ui.button("📋 Copia log").clicked() {
                                    let full_log: String = if is_filtered {
                                        app.logs.iter()
                                            .filter(|(l, _)| l.to_lowercase().contains(&filter_lower))
                                            .map(|(l, _)| l.as_str())
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    } else {
                                        app.logs.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join("\n")
                                    };
                                    ui.ctx().copy_text(full_log);
                                    app.set_toast("Log terminale copiato negli appunti!");
                                }

                                ui.checkbox(&mut app.auto_scroll_log, "Auto-scroll");
                            });
                        });
                    });

                ui.separator();

                // Terminal Body
                let avail_h = (ui.available_height() - 8.0).max(140.0);
                let scroll_area = egui::ScrollArea::vertical()
                    .stick_to_bottom(app.auto_scroll_log)
                    .max_height(avail_h);

                let filter_lower = app.log_filter.trim().to_lowercase();
                let filtered_indices: Vec<usize> = if filter_lower.is_empty() {
                    (0..app.logs.len()).collect()
                } else {
                    app.logs
                        .iter()
                        .enumerate()
                        .filter(|(_, (l, _))| l.to_lowercase().contains(&filter_lower))
                        .map(|(i, _)| i)
                        .collect()
                };

                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(ui, |ui| {
                        if app.logs.is_empty() {
                            scroll_area.show(ui, |ui| {
                                ui.label(
                                    RichText::new("Terminal log pronto. I messaggi e l'output in tempo reale di ffmpeg appariranno qui.")
                                        .monospace()
                                        .color(Color32::from_rgb(110, 115, 125)),
                                );
                            });
                        } else if filtered_indices.is_empty() {
                            scroll_area.show(ui, |ui| {
                                ui.label(
                                    RichText::new("Nessuna riga di log corrisponde al filtro inserito.")
                                        .monospace()
                                        .color(Color32::from_rgb(180, 150, 100)),
                                );
                            });
                        } else {
                            let row_height = 16.0;
                            let num_rows = filtered_indices.len();
                            scroll_area.show_rows(ui, row_height, num_rows, |ui, row_range| {
                                for r in row_range {
                                    if let Some(&orig_idx) = filtered_indices.get(r) {
                                        if let Some((line, is_stderr)) = app.logs.get(orig_idx) {
                                            let line_lower = line.to_lowercase();
                                            let stripped = {
                                                let t = line.trim_start();
                                                if t.starts_with('[') && t.contains("] ") {
                                                    t.split_once("] ").map(|(_, r)| r).unwrap_or(t).trim_start()
                                                } else {
                                                    t
                                                }
                                            };
                                            let stripped_lower = stripped.to_lowercase();

                                            let is_cmd = line.starts_with("$ ") || stripped.starts_with("$ ");
                                            let is_err = line.contains("[error]")
                                                || line.contains("[fatal]")
                                                || line.contains("[panic]")
                                                || line_lower.starts_with("error:")
                                                || line_lower.contains("conversion failed")
                                                || (line_lower.contains("failed") && !line_lower.contains("0 failed"));
                                            let is_warn = line.contains("[warning]")
                                                || line_lower.starts_with("warning:")
                                                || line_lower.contains("deprecated");
                                            let is_stream = stripped.starts_with("Stream #")
                                                || stripped.starts_with("Input #")
                                                || stripped.starts_with("Output #")
                                                || stripped.starts_with("Duration:")
                                                || stripped.starts_with("Metadata:")
                                                || stripped.starts_with("Stream mapping:");
                                            let is_banner = stripped_lower.starts_with("ffmpeg version")
                                                || stripped_lower.starts_with("built with")
                                                || stripped_lower.starts_with("configuration:")
                                                || stripped_lower.starts_with("libav");
                                            let is_progress = stripped.starts_with("frame=")
                                                || stripped.starts_with("size=")
                                                || (stripped.contains("fps=") && stripped.contains("time="));

                                            let color = if is_cmd {
                                                Color32::from_rgb(255, 220, 100) // Golden Yellow for command line
                                            } else if is_err {
                                                Color32::from_rgb(255, 95, 87) // Terminal Red
                                            } else if is_warn {
                                                Color32::from_rgb(255, 189, 46) // Terminal Yellow
                                            } else if is_banner {
                                                Color32::from_rgb(140, 160, 180) // Steel gray for banner
                                            } else if is_stream {
                                                Color32::from_rgb(90, 200, 250) // Sky Blue
                                            } else if is_progress {
                                                Color32::from_rgb(57, 211, 83) // Soft Green
                                            } else if *is_stderr {
                                                Color32::from_rgb(215, 225, 235) // Terminal Light Gray
                                            } else {
                                                Color32::from_rgb(100, 220, 255) // Cyan
                                            };

                                            ui.label(RichText::new(line).monospace().size(12.0).color(color));
                                        }
                                    }
                                }
                            });
                        }
                    });
            });

        // 4.4: Common Error Catalog with 1-Click Remediation
        render_common_error_remediation(ui, app);
    });
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RemediationAction {
    DisableCoverArt,
    SwitchToCpu(String),
    CreateOutputDir(std::path::PathBuf),
    FixFilterStreamcopyConflict,
    SingleWaveAudio,
    DisableHwAccel,
}

fn render_common_error_remediation(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    if app.is_running {
        return;
    }

    let is_failure = match app.exit_status {
        Some(Ok(code)) => code != 0,
        Some(Err(_)) => true,
        None => false,
    };

    if !is_failure || app.logs.is_empty() {
        return;
    }

    let log_text = app.logs.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(" ");
    let mut detected_issue: Option<(&str, &str, &str, RemediationAction)> = None;

    if log_text.contains("Unsupported codec id in stream")
        || (log_text.contains("Automatic encoder selection failed") && app.stream_controls.iter().any(|c| c.kind == StreamKind::CoverArt && c.enabled))
    {
        detected_issue = Some((
            "🖼️ Flusso Copertina non supportato dal contenitore",
            "Il formato del file di uscita non supporta l'inclusione di immagini di copertina come traccia video.",
            "🛠️ Disattiva Copertina e Riprova",
            RemediationAction::DisableCoverArt,
        ));
    } else if log_text.contains("libcuda.so.1")
        || log_text.contains("cudaErrorNoDevice")
        || log_text.contains("No NVENC capable")
        || log_text.contains("Device creation failed")
        || log_text.contains("Driver does not support the required nvenc API")
    {
        let current_codec = app.stream_controls.iter()
            .find(|c| c.kind == StreamKind::Video && c.enabled)
            .map(|c| c.codec_id.clone())
            .unwrap_or_else(|| "hevc_nvenc".to_string());

        let cpu_target = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(&current_codec)
            .unwrap_or("libx264")
            .to_string();

        detected_issue = Some((
            "⚡ Accelerazione Hardware GPU (NVIDIA) non disponibile",
            "I driver GPU o la scheda video non supportano o non permettono l'accesso all'encoder hardware NVENC.",
            "🔄 Passa alla codifica CPU e Riprova",
            RemediationAction::SwitchToCpu(cpu_target),
        ));
    } else if log_text.contains("No such file or directory") {
        if let Some(ref out_path) = app.output_path {
            if let Some(parent) = out_path.parent() {
                detected_issue = Some((
                    "📁 Cartella di destinazione mancante o non raggiungibile",
                    "La cartella in cui salvare il file convertito non esiste ancora sul disco.",
                    "📁 Crea Cartella e Riprova",
                    RemediationAction::CreateOutputDir(parent.to_path_buf()),
                ));
            }
        }
    } else if log_text.contains("Filtering and streamcopy cannot be used together") {
        detected_issue = Some((
            "⚙ Conflitto tra Filtri Video e Copia Flusso (Streamcopy)",
            "Non è possibile applicare filtri (come il ridimensionamento) mantenendo la modalità 'Copia Flusso'.",
            "🛠️ Passa a Codifica Attiva (libx264) e Riprova",
            RemediationAction::FixFilterStreamcopyConflict,
        ));
    } else if log_text.contains("WAVE files have exactly one stream") || log_text.contains("too many streams for wave") {
        detected_issue = Some((
            "🎵 Il contenitore WAV supporta esclusivamente un flusso audio",
            "Nel file sono stati selezionati più flussi audio o flussi video non compatibili con WAV.",
            "🛠️ Mantieni solo 1° Traccia Audio e Riprova",
            RemediationAction::SingleWaveAudio,
        ));
    } else if log_text.contains("Device setup failed for pb")
        || log_text.contains("Cannot open /dev/dri")
        || log_text.contains("Failed to initialise VAAPI")
    {
        detected_issue = Some((
            "🔌 Dispositivo di Decodifica Hardware (VAAPI) non accessibile",
            "Il dispositivo grafico /dev/dri non è accessibile o non supporta la decodifica richiesta.",
            "🛠️ Disabilita Decodifica Hardware e Riprova",
            RemediationAction::DisableHwAccel,
        ));
    }

    if let Some((title, explanation, btn_label, action)) = detected_issue {
        ui.add_space(4.0);
        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(32, 24, 20))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(255, 120, 60)))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title).strong().color(Color32::from_rgb(255, 180, 80)));
                        ui.label(RichText::new(explanation).size(11.5).color(Color32::from_rgb(220, 220, 220)));
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new(btn_label).color(Color32::GREEN).strong()).clicked() {
                            match action {
                                RemediationAction::DisableCoverArt => {
                                    for c in &mut app.stream_controls {
                                        if c.kind == StreamKind::CoverArt {
                                            c.enabled = false;
                                        }
                                    }
                                    app.set_toast("Copertina disattivata. Riavvio codifica...");
                                    app.start_current_job();
                                }
                                RemediationAction::SwitchToCpu(cpu_codec) => {
                                    let catalog = app.catalog.clone();
                                    app.settings.gpu_enabled = false;
                                    if let Some(v) = app.stream_controls.iter_mut().find(|c| c.kind == StreamKind::Video && c.enabled) {
                                        v.change_codec(&cpu_codec, &catalog);
                                    }
                                    app.set_toast(format!("Passato alla codifica CPU ({cpu_codec}). Riavvio..."));
                                    app.start_current_job();
                                }
                                RemediationAction::CreateOutputDir(dir) => {
                                    let _ = std::fs::create_dir_all(&dir);
                                    app.set_toast("Cartella creata con successo. Riavvio...");
                                    app.start_current_job();
                                }
                                RemediationAction::FixFilterStreamcopyConflict => {
                                    let catalog = app.catalog.clone();
                                    if let Some(v) = app.stream_controls.iter_mut().find(|c| c.kind == StreamKind::Video && c.enabled) {
                                        v.change_codec("libx264", &catalog);
                                    }
                                    app.set_toast("Codifica video impostata su libx264. Riavvio...");
                                    app.start_current_job();
                                }
                                RemediationAction::SingleWaveAudio => {
                                    let mut first_audio_found = false;
                                    for c in &mut app.stream_controls {
                                        if c.kind == StreamKind::Audio {
                                            if !first_audio_found {
                                                first_audio_found = true;
                                                c.enabled = true;
                                            } else {
                                                c.enabled = false;
                                            }
                                        } else {
                                            c.enabled = false;
                                        }
                                    }
                                    app.set_toast("Configurato flusso singolo per WAV. Riavvio...");
                                    app.start_current_job();
                                }
                                RemediationAction::DisableHwAccel => {
                                    app.settings.hwaccel_decode = "disabled".to_string();
                                    app.set_toast("Decodifica hardware disabilitata. Riavvio...");
                                    app.start_current_job();
                                }
                            }
                        }
                    });
                });
            });
    }
}
