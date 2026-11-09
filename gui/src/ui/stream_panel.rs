use crate::app::{AppMode, FfGuiApp};
use eframe::egui::{self, Color32, RichText};
use ffgui_core::model::StreamKind;

pub fn render(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.vertical(|ui| {
        ui.heading("File & Stream");
        ui.add_space(4.0);

        // Open file button & drop zone
        ui.horizontal(|ui| {
            if ui.button("📂 Sfoglia file...").clicked() {
                if let Some(file) = rfd::FileDialog::new().pick_file() {
                    app.open_file(file);
                }
            }

            if let Some(ref path) = app.input_path {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                ui.label(RichText::new(name).strong());
            } else {
                ui.label(RichText::new("Nessun file aperto").italics().color(Color32::GRAY));
            }
        });

        if app.is_probing {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.colored_label(Color32::from_rgb(100, 200, 255), "Analisi file con ffprobe in corso...");
            });
        }

        if let Some(ref err) = app.probe_error {
            ui.label(RichText::new(err).color(Color32::RED));
        }

        // File stats from probe
        if let Some(ref probe) = app.probe_result {
            egui::Frame::canvas(ui.style()).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if let Some(dur) = probe.duration_s {
                        ui.label(format!("⏱ Durata: {:.1}s", dur));
                    }
                    if let Some(ref fmt) = probe.format_name {
                        ui.label(format!("📦 {}", fmt));
                    }
                    if let Some(br) = probe.bit_rate {
                        ui.label(format!("📊 {} kbps", br / 1000));
                    }
                });
            });

            ui.add_space(6.0);
            ui.label(RichText::new("Stream sorgente:").strong());

            let out_ext = app.output_path.as_ref()
                .and_then(|p| p.extension().and_then(|e| e.to_str()))
                .unwrap_or(match app.mode {
                    AppMode::Video | AppMode::Subtitles => "mp4",
                    AppMode::Audio => "mp3",
                    AppMode::Image => "webp",
                });
            let caps = ffgui_core::containers::container_caps(out_ext);

            egui::ScrollArea::vertical()
                .max_height(250.0)
                .show(ui, |ui| {
                    for (i, stream) in probe.streams.iter().enumerate() {
                        let unsupported_warning = if stream.kind == StreamKind::CoverArt && !caps.cover_art {
                            Some(format!("⚠️ Copertina non supportata da .{}", out_ext))
                        } else if stream.kind == StreamKind::Subtitle && !caps.subtitles {
                            Some(format!("⚠️ Sottotitoli non supportati da .{}", out_ext))
                        } else if stream.kind == StreamKind::Video && !caps.video {
                            Some(format!("⚠️ Video non supportato da .{}", out_ext))
                        } else {
                            None
                        };

                        let mut ctrl = app.stream_controls.get_mut(i);
                        if unsupported_warning.is_some() {
                            if let Some(ref mut c) = ctrl {
                                c.enabled = false;
                            }
                        }

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                if let Some(c) = ctrl {
                                    if unsupported_warning.is_some() {
                                        let mut disabled_val = false;
                                        ui.add_enabled(false, egui::Checkbox::new(&mut disabled_val, ""));
                                    } else {
                                        ui.checkbox(&mut c.enabled, "");
                                    }
                                }

                                let (badge_text, badge_color) = match stream.kind {
                                    StreamKind::Video => ("VIDEO", Color32::from_rgb(33, 150, 243)),
                                    StreamKind::Audio => ("AUDIO", Color32::from_rgb(76, 175, 80)),
                                    StreamKind::Subtitle => ("SUB", Color32::from_rgb(255, 152, 0)),
                                    StreamKind::Image => ("IMG", Color32::from_rgb(156, 39, 176)),
                                    StreamKind::Attachment => ("ATTACH", Color32::from_rgb(120, 144, 156)),
                                    StreamKind::CoverArt => ("COPERTINA", Color32::from_rgb(233, 30, 99)),
                                    StreamKind::Data => ("DATA", Color32::from_rgb(158, 158, 158)),
                                };

                                ui.colored_label(badge_color, RichText::new(format!("[{}]", badge_text)).strong());

                                let codec_desc = stream.codec_name.as_deref().unwrap_or("?");
                                ui.label(format!("#{} {}", stream.index, codec_desc));

                                if let Some(ref warn) = unsupported_warning {
                                    ui.colored_label(Color32::from_rgb(255, 189, 46), RichText::new(warn).small().italics());
                                }
                            });

                            // Stream specs
                            ui.horizontal_wrapped(|ui| {
                                if let (Some(w), Some(h)) = (stream.width, stream.height) {
                                    ui.small(format!("{}x{}", w, h));
                                }
                                if let Some(fps) = stream.fps {
                                    ui.small(format!("{:.1} fps", fps));
                                }
                                if let Some(ch) = stream.channels {
                                    ui.small(format!("{} ch", ch));
                                }
                                if let Some(sr) = stream.sample_rate {
                                    ui.small(format!("{} Hz", sr));
                                }
                                if let Some(ref lang) = stream.language {
                                    ui.small(format!("[{}]", lang));
                                }
                            });
                        });
                    }
                });
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Output file configuration
        ui.label(RichText::new("Destinazione di salvataggio:").strong());
        ui.horizontal(|ui| {
            if ui.button("📁 Cambia destinazione...").clicked() {
                let mut dialog = rfd::FileDialog::new();
                if let Some(ref current_out) = app.output_path {
                    if let Some(parent) = current_out.parent() {
                        dialog = dialog.set_directory(parent);
                    }
                    if let Some(name) = current_out.file_name().and_then(|n| n.to_str()) {
                        dialog = dialog.set_file_name(name);
                    }
                }
                if let Some(file) = dialog.save_file() {
                    app.output_path = Some(file);
                }
            }

            if let Some(ref out) = app.output_path {
                let name = out.file_name().and_then(|n| n.to_str()).unwrap_or("out");
                ui.label(RichText::new(name).monospace());
            }
        });

        // Quick container extension changer
        if let Some(ref current_out) = app.output_path {
            let mut selected_ext = current_out
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mkv")
                .to_lowercase();

            let orig_ext = selected_ext.clone();
            egui::ComboBox::from_label("Formato contenitore")
                .selected_text(format!(".{}", selected_ext))
                .show_ui(ui, |ui| {
                    match app.mode {
                        AppMode::Video | AppMode::Subtitles => {
                            ui.selectable_value(&mut selected_ext, "mkv".to_string(), ".mkv (Matroska)");
                            ui.selectable_value(&mut selected_ext, "mp4".to_string(), ".mp4 (MPEG-4)");
                            ui.selectable_value(&mut selected_ext, "mov".to_string(), ".mov (QuickTime)");
                            ui.selectable_value(&mut selected_ext, "webm".to_string(), ".webm (WebM)");
                        }
                        AppMode::Audio => {
                            ui.selectable_value(&mut selected_ext, "mp3".to_string(), ".mp3 (MP3)");
                            ui.selectable_value(&mut selected_ext, "flac".to_string(), ".flac (FLAC Lossless)");
                            ui.selectable_value(&mut selected_ext, "opus".to_string(), ".opus (Opus)");
                            ui.selectable_value(&mut selected_ext, "m4a".to_string(), ".m4a (AAC)");
                            ui.selectable_value(&mut selected_ext, "wav".to_string(), ".wav (PCM Wave)");
                        }
                        AppMode::Image => {
                            ui.selectable_value(&mut selected_ext, "webp".to_string(), ".webp (WebP)");
                            ui.selectable_value(&mut selected_ext, "png".to_string(), ".png (PNG)");
                            ui.selectable_value(&mut selected_ext, "jpg".to_string(), ".jpg (JPEG)");
                        }
                    }
                });

            if selected_ext != orig_ext {
                let mut new_path = current_out.clone();
                new_path.set_extension(&selected_ext);
                app.output_path = Some(new_path);
            }
        }
    });
}
