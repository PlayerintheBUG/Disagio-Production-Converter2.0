use crate::app::FfGuiApp;
use eframe::egui::{self, Color32, ProgressBar, RichText};
use ffgui_core::queue::{StepKind, StepStatus};
use ffgui_core::storage::{Storage, UserPreset};

pub fn render(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.vertical(|ui| {
        ui.heading("Coda & Preset");
        ui.add_space(4.0);

        // Queue controls
        ui.horizontal(|ui| {
            if ui.button("➕ Accoda").clicked() {
                if let Some(job) = app.build_current_job() {
                    let name = job
                        .output
                        .path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Job")
                        .to_string();
                    app.queue.push(name, StepKind::Ffmpeg(job));
                    app.set_toast("Lavoro aggiunto alla coda");
                }
            }

            if app.is_queue_running {
                if ui.button(RichText::new("⏹ Ferma").color(Color32::RED).strong()).clicked() {
                    app.stop_queue();
                }
            } else {
                let can_start_queue = app.queue.has_pending() && !app.is_running;
                if ui.add_enabled(can_start_queue, egui::Button::new(RichText::new("▶ Avvia").color(Color32::GREEN).strong())).clicked() {
                    app.start_queue();
                }
            }

            if ui.button("🗑 Svuota").clicked() {
                app.queue.clear();
            }
        });

        ui.add_space(4.0);
        ui.separator();
        ui.label(RichText::new("Lavori in coda:").strong());

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                if app.queue.is_empty() {
                    ui.label(RichText::new("Coda vuota").italics().color(Color32::GRAY));
                } else {
                    let mut to_move_up = None;
                    let mut to_move_down = None;
                    let mut to_retry = None;
                    let mut to_remove = None;

                    for item in &app.queue.items {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let (status_str, status_color) = match item.status {
                                    StepStatus::Pending => ("In attesa", Color32::GRAY),
                                    StepStatus::Running => ("In corso", Color32::from_rgb(33, 150, 243)),
                                    StepStatus::Finished => ("Completato", Color32::GREEN),
                                    StepStatus::Failed => ("Fallito", Color32::RED),
                                    StepStatus::Cancelled => ("Annullato", Color32::YELLOW),
                                };

                                ui.colored_label(status_color, format!("[{}]", status_str));
                                let is_selected = app.selected_log_job == Some(item.id);
                                if ui.selectable_label(is_selected, &item.description).clicked() {
                                    app.selected_log_job = Some(item.id);
                                }

                                if ui.small_button("▲").clicked() {
                                    to_move_up = Some(item.id);
                                }
                                if ui.small_button("▼").clicked() {
                                    to_move_down = Some(item.id);
                                }
                                if (item.status == StepStatus::Failed || item.status == StepStatus::Cancelled) && ui.small_button("🔄").clicked() {
                                    to_retry = Some(item.id);
                                }
                                if ui.small_button("✕").clicked() {
                                    to_remove = Some(item.id);
                                }
                            });

                            if item.status == StepStatus::Running {
                                ui.add(ProgressBar::new(item.progress as f32 / 100.0));
                            }
                        });
                    }

                    if let Some(id) = to_move_up {
                        app.queue.move_up(id);
                    }
                    if let Some(id) = to_move_down {
                        app.queue.move_down(id);
                    }
                    if let Some(id) = to_retry {
                        app.queue.retry(id);
                    }
                    if let Some(id) = to_remove {
                        app.queue.remove(id);
                    }
                }
            });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // User Presets
        ui.label(RichText::new("Preset Utente:").strong());

        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut app.new_preset_name);
            if ui.button("💾 Salva").clicked() && !app.new_preset_name.trim().is_empty() {
                let mode_str = match app.mode {
                    crate::app::AppMode::Video => "video",
                    crate::app::AppMode::Audio => "audio",
                    crate::app::AppMode::Image => "image",
                    crate::app::AppMode::Subtitles => "subtitles",
                }
                .to_string();

                let container = app.output_path.as_ref().and_then(|p| {
                    p.extension().and_then(|e| e.to_str()).map(|s| s.to_string())
                });

                let mut video_codec = None;
                let mut video_preset = None;
                let mut video_params = std::collections::HashMap::new();
                let mut video_scale = None;

                let mut audio_codec = None;
                let mut audio_preset = None;
                let mut audio_params = std::collections::HashMap::new();

                for ctrl in &app.stream_controls {
                    if !ctrl.enabled {
                        continue;
                    }
                    match ctrl.kind {
                        ffgui_core::model::StreamKind::Video => {
                            if video_codec.is_none() {
                                video_codec = Some(ctrl.codec_id.clone());
                                video_preset = ctrl.selected_preset.clone();
                                video_params = ctrl.params.clone();
                                if ctrl.scale_preset != "Originale" {
                                    video_scale = Some(ctrl.scale_preset.clone());
                                }
                            }
                        }
                        ffgui_core::model::StreamKind::Audio if audio_codec.is_none() => {
                            audio_codec = Some(ctrl.codec_id.clone());
                            audio_preset = ctrl.selected_preset.clone();
                            audio_params = ctrl.params.clone();
                        }
                        _ => {}
                    }
                }

                let preset = UserPreset {
                    version: 1,
                    name: app.new_preset_name.trim().to_string(),
                    description: None,
                    mode: mode_str,
                    container,
                    video_codec,
                    video_preset,
                    video_params,
                    video_scale,
                    audio_codec,
                    audio_preset,
                    audio_params,
                };

                if Storage::save_preset(&preset).is_ok() {
                    app.user_presets = Storage::list_presets();
                    app.set_toast(format!("Preset '{}' salvato!", app.new_preset_name));
                    app.new_preset_name.clear();
                }
            }
        });

        egui::ScrollArea::vertical()
            .max_height(140.0)
            .show(ui, |ui| {
                if app.user_presets.is_empty() {
                    ui.label(RichText::new("Nessun preset salvato").italics().color(Color32::GRAY));
                } else {
                    let mut delete_name = None;
                    let mut load_idx = None;

                    for (idx, p) in app.user_presets.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.button(&p.name).clicked() {
                                load_idx = Some(idx);
                            }

                            if ui.small_button("🗑").clicked() {
                                delete_name = Some(p.name.clone());
                            }
                        });
                    }

                    if let Some(idx) = load_idx {
                        let p = &app.user_presets[idx];
                        let name = p.name.clone();

                        match p.mode.as_str() {
                            "video" => app.mode = crate::app::AppMode::Video,
                            "audio" => app.mode = crate::app::AppMode::Audio,
                            "image" => app.mode = crate::app::AppMode::Image,
                            "subtitles" => app.mode = crate::app::AppMode::Subtitles,
                            _ => {}
                        }

                        if let Some(ref ext) = p.container {
                            if let Some(ref mut out) = app.output_path {
                                out.set_extension(ext);
                            }
                        }

                        if let Some(ref v_codec) = p.video_codec {
                            for ctrl in app.stream_controls.iter_mut().filter(|c| c.kind == ffgui_core::model::StreamKind::Video) {
                                ctrl.enabled = true;
                                ctrl.change_codec(v_codec, &app.catalog);
                                if let Some(ref pre) = p.video_preset {
                                    ctrl.apply_preset(pre, &app.catalog);
                                }
                                for (k, v) in &p.video_params {
                                    ctrl.params.insert(k.clone(), v.clone());
                                }
                                if let Some(ref scale) = p.video_scale {
                                    ctrl.scale_preset = scale.clone();
                                }
                            }
                        }

                        if let Some(ref a_codec) = p.audio_codec {
                            for ctrl in app.stream_controls.iter_mut().filter(|c| c.kind == ffgui_core::model::StreamKind::Audio) {
                                ctrl.enabled = true;
                                ctrl.change_codec(a_codec, &app.catalog);
                                if let Some(ref pre) = p.audio_preset {
                                    ctrl.apply_preset(pre, &app.catalog);
                                }
                                for (k, v) in &p.audio_params {
                                    ctrl.params.insert(k.clone(), v.clone());
                                }
                            }
                        }

                        app.set_toast(format!("Preset '{}' caricato!", name));
                    }

                    if let Some(name) = delete_name {
                        let _ = Storage::delete_preset(&name);
                        app.user_presets = Storage::list_presets();
                    }
                }
            });
    });
}
