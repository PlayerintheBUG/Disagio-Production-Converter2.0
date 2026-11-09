use crate::app::FfGuiApp;
use eframe::egui::{self, Color32, RichText};
use ffgui_core::storage::Storage;

pub fn render(ctx: &egui::Context, app: &mut FfGuiApp) {
    if !app.show_settings_window {
        return;
    }

    let mut is_open = app.show_settings_window;
    let mut settings_modified = false;

    egui::Window::new("⚙ Impostazioni Programma")
        .open(&mut is_open)
        .resizable(true)
        .default_size([580.0, 520.0])
        .max_height(ctx.screen_rect().height() * 0.85)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading("Generali & Percorsi");
                    ui.add_space(4.0);

                    // Default Output Directory
                    ui.horizontal(|ui| {
                        ui.label("Cartella di output predefinita:");
                        if ui.button("📂 Sfoglia...").clicked() {
                            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                                app.settings.default_output_dir = Some(dir);
                                settings_modified = true;
                            }
                        }
                        if let Some(ref dir) = app.settings.default_output_dir {
                            ui.label(RichText::new(dir.to_string_lossy().to_string()).monospace());
                            if ui.small_button("✕").clicked() {
                                app.settings.default_output_dir = None;
                                settings_modified = true;
                            }
                        } else {
                            ui.small("(predefinita: stessa cartella dell'input)");
                        }
                    });

                    // Custom ffmpeg binary
                    ui.horizontal(|ui| {
                        ui.label("Percorso binario ffmpeg:");
                        if ui.button("📂 Sfoglia...").clicked() {
                            if let Some(file) = rfd::FileDialog::new().pick_file() {
                                app.settings.ffmpeg_path = Some(file);
                                settings_modified = true;
                            }
                        }
                        if let Some(ref f) = app.settings.ffmpeg_path {
                            ui.label(RichText::new(f.to_string_lossy().to_string()).monospace());
                            if ui.small_button("✕").clicked() {
                                app.settings.ffmpeg_path = None;
                                settings_modified = true;
                            }
                        } else {
                            ui.small("(predefinito: ffmpeg di sistema)");
                        }
                    });

                    // Custom ffprobe binary
                    ui.horizontal(|ui| {
                        ui.label("Percorso binario ffprobe:");
                        if ui.button("📂 Sfoglia...").clicked() {
                            if let Some(file) = rfd::FileDialog::new().pick_file() {
                                app.settings.ffprobe_path = Some(file);
                                settings_modified = true;
                            }
                        }
                        if let Some(ref f) = app.settings.ffprobe_path {
                            ui.label(RichText::new(f.to_string_lossy().to_string()).monospace());
                            if ui.small_button("✕").clicked() {
                                app.settings.ffprobe_path = None;
                                settings_modified = true;
                            }
                        } else {
                            ui.small("(predefinito: ffprobe di sistema)");
                        }
                    });

                    // Log level control
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Livello log di FFmpeg (-loglevel):");
                        egui::ComboBox::from_id_salt("settings_log_level_select")
                            .selected_text(&app.settings.log_level)
                            .show_ui(ui, |ui| {
                                if ui.selectable_value(&mut app.settings.log_level, "info".to_string(), "Info (consigliato)").clicked() {
                                    settings_modified = true;
                                }
                                if ui.selectable_value(&mut app.settings.log_level, "warning".to_string(), "Warning").clicked() {
                                    settings_modified = true;
                                }
                                if ui.selectable_value(&mut app.settings.log_level, "error".to_string(), "Error").clicked() {
                                    settings_modified = true;
                                }
                                if ui.selectable_value(&mut app.settings.log_level, "debug".to_string(), "Debug").clicked() {
                                    settings_modified = true;
                                }
                            });
                    });

                    // Whisper preferred model
                    ui.horizontal(|ui| {
                        ui.label("Modello Whisper predefinito:");
                        egui::ComboBox::from_id_salt("settings_whisper_model_select")
                            .selected_text(&app.settings.preferred_whisper_model)
                            .show_ui(ui, |ui| {
                                for m in &["tiny", "base", "small", "medium", "large-v3"] {
                                    if ui.selectable_value(&mut app.settings.preferred_whisper_model, m.to_string(), *m).clicked() {
                                        settings_modified = true;
                                    }
                                }
                            });
                    });

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Accelerazione Hardware / GPU");
                    ui.add_space(4.0);

                    // GPU toggle
                    if ui.checkbox(&mut app.settings.gpu_enabled, RichText::new("Abilita codifica con GPU hardware (NVENC / VAAPI / QSV / AMF)").strong()).changed() {
                        settings_modified = true;
                    }

                    if app.settings.gpu_enabled {
                        ui.horizontal(|ui| {
                            ui.label("Priorità encoder:");
                            if ui.selectable_value(&mut app.settings.gpu_mode, "auto".to_string(), "Automatica").clicked() { settings_modified = true; }
                            if ui.selectable_value(&mut app.settings.gpu_mode, "cuda".to_string(), "CUDA / NVENC (NVIDIA)").clicked() { settings_modified = true; }
                            if ui.selectable_value(&mut app.settings.gpu_mode, "qsv".to_string(), "QuickSync (Intel)").clicked() { settings_modified = true; }
                            if ui.selectable_value(&mut app.settings.gpu_mode, "amf".to_string(), "AMF (AMD)").clicked() { settings_modified = true; }
                            if ui.selectable_value(&mut app.settings.gpu_mode, "vaapi".to_string(), "VAAPI (Linux)").clicked() { settings_modified = true; }
                        });

                        // VAAPI custom device
                        ui.horizontal(|ui| {
                            ui.label("Dispositivo VAAPI:");
                            let mut dev_str = app.settings.vaapi_device.clone().unwrap_or_else(|| "/dev/dri/renderD128".to_string());
                            if ui.text_edit_singleline(&mut dev_str).changed() {
                                app.settings.vaapi_device = Some(dev_str);
                                settings_modified = true;
                            }
                        });

                        // Hardware Decoding (-hwaccel)
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Decodifica Hardware (-hwaccel):").strong());
                            egui::ComboBox::from_id_salt("hwaccel_decode_select")
                                .selected_text(match app.settings.hwaccel_decode.as_str() {
                                    "auto" => "Automatica (-hwaccel auto, consigliato)",
                                    "cuda" => "CUDA / NVDEC (NVIDIA)",
                                    "vaapi" => "VAAPI (Linux Intel/AMD)",
                                    "qsv" => "Intel QuickSync (QSV)",
                                    "disabled" | "none" => "Disabilitata (solo CPU)",
                                    other => other,
                                })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_value(&mut app.settings.hwaccel_decode, "auto".to_string(), "Automatica (-hwaccel auto, consigliato)").clicked() { settings_modified = true; }
                                    if ui.selectable_value(&mut app.settings.hwaccel_decode, "cuda".to_string(), "CUDA / NVDEC (NVIDIA)").clicked() { settings_modified = true; }
                                    if ui.selectable_value(&mut app.settings.hwaccel_decode, "vaapi".to_string(), "VAAPI (Linux Intel/AMD)").clicked() { settings_modified = true; }
                                    if ui.selectable_value(&mut app.settings.hwaccel_decode, "qsv".to_string(), "Intel QuickSync (QSV)").clicked() { settings_modified = true; }
                                    if ui.selectable_value(&mut app.settings.hwaccel_decode, "disabled".to_string(), "Disabilitata (solo CPU)").clicked() { settings_modified = true; }
                                });
                        });
                        ui.small(RichText::new("La decodifica automatica sgombera la CPU usando la GPU per leggere i file, con fallback software trasparente.").color(Color32::from_rgb(170, 170, 170)));

                        ui.add_space(4.0);

                        // Status of detection
                        if app.is_detecting_gpu {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.colored_label(Color32::from_rgb(100, 200, 255), "Verifica encoder GPU in corso...");
                            });
                        } else if let Some(ref det) = app.gpu_detection {
                            ui.label(RichText::new("Disponibilità hardware verificata su questo sistema (test sintetico):").strong());
                            for enc in &det.encoders {
                                let (badge, color) = if enc.is_available {
                                    ("✔ FUNZIONANTE", Color32::GREEN)
                                } else {
                                    ("✖ NON DISPONIBILE", Color32::GRAY)
                                };
                                ui.horizontal(|ui| {
                                    ui.colored_label(color, badge);
                                    ui.label(&enc.label);
                                });
                            }
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.heading("Coda & Concorrenza (Scheduler Parallelo)");
                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label("Attività di codifica simultanee in coda:");
                        if ui.add(egui::Slider::new(&mut app.settings.parallel_jobs, 1..=8).text("processi")).changed() {
                            settings_modified = true;
                        }
                    });
                    ui.small(RichText::new("Numero massimo di conversioni parallele per sfruttare tutti i core della CPU.").color(Color32::from_rgb(170, 170, 170)));

                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Sessioni GPU simultanee massime:");
                        if ui.add(egui::Slider::new(&mut app.settings.max_gpu_sessions, 1..=4).text("sessioni GPU")).changed() {
                            settings_modified = true;
                        }
                    });
                    ui.small(RichText::new("Limita le sessioni GPU simultanee per non saturare la memoria VRAM della scheda video.").color(Color32::from_rgb(170, 170, 170)));

                    ui.add_space(4.0);
                    if ui.checkbox(&mut app.settings.auto_cpu_fallback, RichText::new("Fallback automatico su CPU se la codifica GPU fallisce").strong()).changed() {
                        settings_modified = true;
                    }
                    ui.small(RichText::new("Se un'attività con encoder GPU fallisce per memoria o driver, viene automaticamente rieseguita con l'equivalente CPU senza bloccare la coda.").color(Color32::from_rgb(170, 170, 170)));

                    ui.add_space(12.0);
                    ui.separator();

                    // Save / Close buttons
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("💾 Salva Impostazioni").color(Color32::GREEN).strong()).clicked() {
                            if let Err(e) = Storage::save_settings(&app.settings) {
                                app.set_toast(format!("Errore salvataggio impostazioni: {}", e));
                            } else {
                                app.set_toast("Impostazioni salvate con successo!");
                            }
                            app.show_settings_window = false;
                        }

                        if ui.button("Chiudi").clicked() {
                            let _ = Storage::save_settings(&app.settings);
                            app.show_settings_window = false;
                        }
                    });
                });
            });
        });

    if settings_modified {
        let _ = Storage::save_settings(&app.settings);
    }

    if app.show_settings_window && !is_open {
        let _ = Storage::save_settings(&app.settings);
    }
    app.show_settings_window = is_open;
}
