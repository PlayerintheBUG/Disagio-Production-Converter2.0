use crate::app::{AppMode, FfGuiApp, StreamControlState, SubtitleMode};
use eframe::egui::{self, Color32, RichText, Slider};
use ffgui_core::backends::whisper::WhisperBackend;
use ffgui_core::catalog::{CodecDef, ParamDef, ParamType};
use ffgui_core::model::StreamKind;
use std::sync::atomic::Ordering;

pub fn render(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.vertical(|ui| {
        // Mode selector
        ui.horizontal(|ui| {
            if ui.selectable_label(app.mode == AppMode::Video, RichText::new("🎬 Video").strong()).clicked() {
                app.set_mode(AppMode::Video);
            }
            if ui.selectable_label(app.mode == AppMode::Audio, RichText::new("🎵 Audio").strong()).clicked() {
                app.set_mode(AppMode::Audio);
            }
            if ui.selectable_label(app.mode == AppMode::Image, RichText::new("🖼 Immagine").strong()).clicked() {
                app.set_mode(AppMode::Image);
            }
            if ui.selectable_label(app.mode == AppMode::Subtitles, RichText::new("💬 Sottotitoli").strong()).clicked() {
                app.set_mode(AppMode::Subtitles);
            }
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        match app.mode {
            AppMode::Video => render_video_controls(ui, app),
            AppMode::Audio => render_audio_controls(ui, app),
            AppMode::Image => render_image_controls(ui, app),
            AppMode::Subtitles => render_subtitles_controls(ui, app),
        }
    });
}

fn render_param(
    ui: &mut egui::Ui,
    ctrl: &mut StreamControlState,
    param: &ParamDef,
    codec_def: &CodecDef,
) {
    if !param.is_active(&ctrl.params, &codec_def.params) {
        return;
    }

    ui.group(|ui| {
        match param.param_type {
            ParamType::Int => {
                let min = param.min.unwrap_or(0.0) as i64;
                let max = param.max.unwrap_or(100.0) as i64;
                let mut current = ctrl
                    .params
                    .get(&param.id)
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| {
                        param.default.as_ref().and_then(|d| d.as_i64()).unwrap_or(min)
                    });

                ui.horizontal(|ui| {
                    ui.label(&param.label);
                    if ui.add(Slider::new(&mut current, min..=max)).changed() {
                        ctrl.params.insert(param.id.clone(), serde_json::json!(current));
                        ctrl.selected_preset = None;
                    }
                });
            }
            ParamType::Float => {
                let min = param.min.unwrap_or(0.0);
                let max = param.max.unwrap_or(100.0);
                let mut current = ctrl
                    .params
                    .get(&param.id)
                    .and_then(|v| v.as_f64())
                    .unwrap_or_else(|| {
                        param.default.as_ref().and_then(|d| d.as_f64()).unwrap_or(min)
                    });

                ui.horizontal(|ui| {
                    ui.label(&param.label);
                    if ui.add(Slider::new(&mut current, min..=max).step_by(0.1)).changed() {
                        ctrl.params.insert(param.id.clone(), serde_json::json!(current));
                        ctrl.selected_preset = None;
                    }
                });
            }
            ParamType::Choice => {
                let choices = param.choices.clone().unwrap_or_default();
                let mut current = ctrl
                    .params
                    .get(&param.id)
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| {
                        param.default.as_ref().and_then(|d| d.as_str()).unwrap_or("")
                    })
                    .to_string();

                ui.horizontal(|ui| {
                    ui.label(&param.label);
                    egui::ComboBox::from_id_salt(format!("choice_{}_{}", param.id, ctrl.source_index))
                        .selected_text(&current)
                        .show_ui(ui, |ui| {
                            for ch in &choices {
                                if ui.selectable_value(&mut current, ch.clone(), ch).clicked() {
                                    ctrl.params.insert(param.id.clone(), serde_json::json!(current));
                                    ctrl.selected_preset = None;
                                }
                            }
                        });
                });
            }
            ParamType::Bool => {
                let mut current = ctrl
                    .params
                    .get(&param.id)
                    .and_then(|v| v.as_bool())
                    .unwrap_or_else(|| {
                        param.default.as_ref().and_then(|d| d.as_bool()).unwrap_or(false)
                    });

                if ui.checkbox(&mut current, &param.label).changed() {
                    ctrl.params.insert(param.id.clone(), serde_json::json!(current));
                    ctrl.selected_preset = None;
                }
            }
        }

        if let Some(ref hint) = param.hint {
            ui.small(RichText::new(hint).color(Color32::from_rgb(180, 180, 180)));
        }
    });
}

fn render_video_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    let video_ctrl_idx = app
        .stream_controls
        .iter()
        .position(|c| c.kind == StreamKind::Video && c.enabled);

    let Some(idx) = video_ctrl_idx else {
        ui.label(RichText::new("Nessuno stream video abilitato").italics().color(Color32::GRAY));
        return;
    };

    let catalog = app.catalog.clone();
    let is_gpu = catalog.get_codec(&app.stream_controls[idx].codec_id).map(|c| c.hw.is_some()).unwrap_or(false);

    let mut toast_to_set = None;
    let mut new_codec_to_apply = None;

    ui.horizontal(|ui| {
        ui.label(RichText::new("Codifica:").strong());

        let gpu_btn = ui.selectable_label(
            is_gpu,
            RichText::new("⚡ GPU (Hardware)").strong().color(if is_gpu { Color32::from_rgb(100, 240, 150) } else { Color32::GRAY }),
        );
        if gpu_btn.clicked() && !is_gpu {
            let cur_codec = &app.stream_controls[idx].codec_id;
            // Use pick_gpu_encoder checking GpuDetectionResult, with fallback to get_gpu_counterpart
            let chosen_gpu = if let Some(ref det) = app.gpu_detection {
                ffgui_core::backends::gpu::GpuBackend::pick_gpu_encoder(det, cur_codec, &app.settings.gpu_mode)
            } else {
                None
            }.or_else(|| {
                ffgui_core::backends::gpu::GpuBackend::get_gpu_counterpart(cur_codec, &app.settings.gpu_mode).map(|s| s.to_string())
            });

            if let Some(gpu_codec) = chosen_gpu {
                app.settings.gpu_enabled = true;
                new_codec_to_apply = Some(gpu_codec);
            } else {
                toast_to_set = Some(format!("Nessuna controparte GPU disponibile per {}", cur_codec));
            }
        }

        let cpu_btn = ui.selectable_label(
            !is_gpu,
            RichText::new("🖥 CPU (Software)").strong().color(if !is_gpu { Color32::from_rgb(140, 180, 250) } else { Color32::GRAY }),
        );
        if cpu_btn.clicked() && is_gpu {
            app.settings.gpu_enabled = false;
            let cur_codec = &app.stream_controls[idx].codec_id;
            let cpu_codec = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(cur_codec).unwrap_or("libx264");
            new_codec_to_apply = Some(cpu_codec.to_string());
        }

        // Quick HW decode badge
        let hw_dec = app.settings.hwaccel_decode.as_str();
        if hw_dec != "disabled" && hw_dec != "none" && !hw_dec.is_empty() {
            ui.small(RichText::new(format!("🟢 Decodifica HW: {}", hw_dec)).color(Color32::from_rgb(120, 220, 160)));
        }
    });

    if let Some(toast) = toast_to_set {
        app.set_toast(toast);
    }
    if let Some(codec) = new_codec_to_apply {
        app.stream_controls[idx].change_codec(&codec, &catalog);
    }

    let ctrl = &mut app.stream_controls[idx];

    ui.add_space(3.0);

    // Codec selector
    ui.horizontal(|ui| {
        ui.label(RichText::new("Codec Video:").strong());
        egui::ComboBox::from_id_salt("video_codec_select")
            .selected_text(if ctrl.codec_id == "copy" {
                "Copia flusso (Stream Copy)".to_string()
            } else if let Some(def) = catalog.get_codec(&ctrl.codec_id) {
                if def.hw.is_some() {
                    format!("⚡ {}", def.label)
                } else {
                    def.label.clone()
                }
            } else {
                ctrl.codec_id.clone()
            })
            .show_ui(ui, |ui| {
                if ui.selectable_label(ctrl.codec_id == "copy", "Copia flusso (Stream Copy)").clicked() {
                    ctrl.codec_id = "copy".to_string();
                }

                // Partition codecs into CPU and GPU
                let video_codecs = catalog.by_kind(StreamKind::Video);
                let (gpu_codecs, cpu_codecs): (Vec<_>, Vec<_>) = video_codecs.into_iter().partition(|c| c.hw.is_some());

                // 1. GPU Encoders - grouped by backend and prioritizing CUDA / NVENC
                ui.separator();
                ui.colored_label(Color32::from_rgb(100, 220, 150), RichText::new("⚡ Accelerazione GPU / Hardware").strong());

                let nvenc: Vec<_> = gpu_codecs.iter().filter(|c| c.id.ends_with("_nvenc")).collect();
                let qsv: Vec<_> = gpu_codecs.iter().filter(|c| c.id.ends_with("_qsv")).collect();
                let amf: Vec<_> = gpu_codecs.iter().filter(|c| c.id.ends_with("_amf")).collect();
                let vaapi: Vec<_> = gpu_codecs.iter().filter(|c| c.id.ends_with("_vaapi")).collect();

                let groups = [
                    ("NVIDIA NVENC (CUDA - Priorità)", nvenc),
                    ("Intel QuickSync (QSV)", qsv),
                    ("AMD (AMF)", amf),
                    ("Linux VAAPI", vaapi),
                ];

                for (group_title, group_codecs) in groups {
                    if !group_codecs.is_empty() {
                        ui.label(RichText::new(format!("  ▪ {}", group_title)).small().color(Color32::from_rgb(160, 210, 180)));
                        for codec in group_codecs {
                            let is_avail = app.gpu_detection.as_ref()
                                .map(|d| d.is_encoder_available(&codec.id))
                                .unwrap_or(true);

                            let is_sel = ctrl.codec_id == codec.id;
                            let text = if is_avail {
                                format!("    ⚡ {}", codec.label)
                            } else {
                                format!("    ⚡ {} (non testato / fallback auto)", codec.label)
                            };
                            let color = if is_avail {
                                Color32::from_rgb(100, 220, 150)
                            } else {
                                Color32::from_rgb(160, 200, 180)
                            };
                            if ui.selectable_label(is_sel, RichText::new(text).color(color)).clicked() {
                                app.settings.gpu_enabled = true;
                                ctrl.change_codec(&codec.id, &catalog);
                            }
                        }
                    }
                }

                // 2. CPU Encoders
                ui.separator();
                ui.colored_label(Color32::from_rgb(140, 180, 250), RichText::new("🖥 Codifica CPU (Software)").strong());
                for codec in cpu_codecs {
                    let is_sel = ctrl.codec_id == codec.id;
                    if ui.selectable_label(is_sel, &codec.label).clicked() {
                        ctrl.change_codec(&codec.id, &catalog);
                    }
                }
            });
    });

    // GPU notification & warning badge
    if let Some(codec_def) = catalog.get_codec(&ctrl.codec_id) {
        if codec_def.hw.is_some() {
            ui.horizontal(|ui| {
                ui.colored_label(Color32::from_rgb(100, 220, 150), "⚡ Modalità GPU attiva.");
                ui.small(RichText::new("(più veloce; 'Prova 10s' consigliata per verificare la qualità)").color(Color32::GRAY));
            });
        }
    }

    if ctrl.codec_id != "copy" {
        if let Some(codec_def) = catalog.get_codec(&ctrl.codec_id) {
            // Presets bar
            if !codec_def.presets.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Preset:");
                    for preset in &codec_def.presets {
                        let is_sel = ctrl.selected_preset.as_deref() == Some(&preset.id);
                        if ui.selectable_label(is_sel, &preset.label).clicked() {
                            ctrl.apply_preset(&preset.id, &catalog);
                        }
                    }
                });
            }

            ui.add_space(6.0);
            ui.separator();
            ui.add_space(4.0);

            // Dynamic TOML parameters
            for param in &codec_def.params {
                render_param(ui, ctrl, param, codec_def);
            }
        }
    }

    // Video filters & scaling (proportional scaling)
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filtro Scala:").strong());
        egui::ComboBox::from_id_salt("scale_preset_box")
            .selected_text(&ctrl.scale_preset)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut ctrl.scale_preset, "Originale".to_string(), "Originale (nessun filtro)");
                ui.selectable_value(&mut ctrl.scale_preset, "1080p (altezza 1080)".to_string(), "Full HD 1080p (proporzionale)");
                ui.selectable_value(&mut ctrl.scale_preset, "720p (altezza 720)".to_string(), "HD 720p (proporzionale)");
                ui.selectable_value(&mut ctrl.scale_preset, "480p (altezza 480)".to_string(), "SD 480p (proporzionale)");
                ui.selectable_value(&mut ctrl.scale_preset, "Personalizzato".to_string(), "Personalizzato...");
            });
    });

    if ctrl.scale_preset == "Personalizzato" {
        ui.horizontal(|ui| {
            ui.label("L:");
            ui.add(egui::DragValue::new(&mut ctrl.custom_width).speed(1.0).range(16..=7680));
            ui.label("A:");
            ui.add(egui::DragValue::new(&mut ctrl.custom_height).speed(1.0).range(16..=4320));
        });
    }

    // Trimming / Taglio
    render_trim_controls(ui, app);
}

fn render_audio_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    let audio_ctrl_idx = app
        .stream_controls
        .iter()
        .position(|c| c.kind == StreamKind::Audio && c.enabled);

    let Some(idx) = audio_ctrl_idx else {
        ui.label(RichText::new("Nessuno stream audio abilitato").italics().color(Color32::GRAY));
        return;
    };

    let catalog = app.catalog.clone();
    let ctrl = &mut app.stream_controls[idx];

    ui.horizontal(|ui| {
        ui.label(RichText::new("Codec Audio:").strong());
        egui::ComboBox::from_id_salt("audio_codec_select")
            .selected_text(if ctrl.codec_id == "copy" {
                "Copia flusso (Stream Copy)".to_string()
            } else if let Some(def) = catalog.get_codec(&ctrl.codec_id) {
                def.label.clone()
            } else {
                ctrl.codec_id.clone()
            })
            .show_ui(ui, |ui| {
                if ui.selectable_label(ctrl.codec_id == "copy", "Copia flusso (Stream Copy)").clicked() {
                    ctrl.codec_id = "copy".to_string();
                }
                for codec in catalog.by_kind(StreamKind::Audio) {
                    if ui.selectable_label(ctrl.codec_id == codec.id, &codec.label).clicked() {
                        ctrl.change_codec(&codec.id, &catalog);
                    }
                }
            });
    });

    if ctrl.codec_id != "copy" {
        if let Some(codec_def) = catalog.get_codec(&ctrl.codec_id) {
            if !codec_def.presets.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Preset:");
                    for preset in &codec_def.presets {
                        let is_sel = ctrl.selected_preset.as_deref() == Some(&preset.id);
                        if ui.selectable_label(is_sel, &preset.label).clicked() {
                            ctrl.apply_preset(&preset.id, &catalog);
                        }
                    }
                });
            }

            ui.add_space(6.0);
            ui.separator();

            for param in &codec_def.params {
                render_param(ui, ctrl, param, codec_def);
            }
        }
    }

    render_trim_controls(ui, app);
}

fn render_image_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    let img_ctrl_idx = app
        .stream_controls
        .iter()
        .position(|c| (c.kind == StreamKind::Image || c.kind == StreamKind::Video) && c.enabled);

    let Some(idx) = img_ctrl_idx else {
        ui.label(RichText::new("Nessuno stream immagine/video abilitato").italics().color(Color32::GRAY));
        return;
    };

    let catalog = app.catalog.clone();
    let ctrl = &mut app.stream_controls[idx];

    ui.horizontal(|ui| {
        ui.label(RichText::new("Formato Immagine:").strong());
        egui::ComboBox::from_id_salt("img_codec_select")
            .selected_text(if let Some(def) = catalog.get_codec(&ctrl.codec_id) {
                def.label.clone()
            } else {
                ctrl.codec_id.clone()
            })
            .show_ui(ui, |ui| {
                for codec in catalog.by_kind(StreamKind::Image) {
                    if ui.selectable_label(ctrl.codec_id == codec.id, &codec.label).clicked() {
                        ctrl.change_codec(&codec.id, &catalog);
                    }
                }
            });
    });

    if let Some(codec_def) = catalog.get_codec(&ctrl.codec_id) {
        if !codec_def.presets.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Preset:");
                for preset in &codec_def.presets {
                    let is_sel = ctrl.selected_preset.as_deref() == Some(&preset.id);
                    if ui.selectable_label(is_sel, &preset.label).clicked() {
                        ctrl.apply_preset(&preset.id, &catalog);
                    }
                }
            });
        }

        ui.add_space(6.0);
        ui.separator();

        for param in &codec_def.params {
            render_param(ui, ctrl, param, codec_def);
        }
    }
}

fn render_subtitles_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.label(RichText::new("Generazione e gestione sottotitoli con Whisper").strong());
    ui.add_space(4.0);

    let is_ready = app.whisper_env_ready.load(Ordering::SeqCst);
    ui.horizontal(|ui| {
        if is_ready {
            ui.label(RichText::new("● faster-whisper pronto").color(Color32::GREEN));
        } else {
            ui.label(RichText::new("○ faster-whisper non installato").color(Color32::YELLOW));
            if ui.button("Installa ambiente Whisper").clicked() {
                if let Ok((ctrl, rx)) = WhisperBackend::setup_env() {
                    app.is_running = true;
                    app.is_whisper_setup_running = true; // Bug 3.5: track setup completion
                    app.runner_controller = Some(ctrl);
                    app.rx_events = Some(rx);
                    app.logs.clear();
                }
            }
        }
    });

    ui.add_space(6.0);
    ui.group(|ui| {
        // Bug 3.1: Audio track selection if multiple audio tracks exist
        let audio_tracks: Vec<(usize, usize, String)> = if let Some(ref probe) = app.probe_result {
            probe.streams
                .iter()
                .filter(|s| s.kind == StreamKind::Audio)
                .enumerate()
                .map(|(track_idx, s)| {
                    let lang = s.language.clone().unwrap_or_else(|| "ind".to_string());
                    let title = s.title.clone().unwrap_or_else(|| format!("Traccia {}", track_idx + 1));
                    let codec = s.codec_name.clone().unwrap_or_else(|| "audio".to_string());
                    (track_idx, s.index, format!("#{} {} ({}, {})", track_idx, title, lang, codec))
                })
                .collect()
        } else {
            Vec::new()
        };

        if audio_tracks.len() > 1 {
            ui.horizontal(|ui| {
                ui.label("Traccia audio sorgente:");
                let current_label = audio_tracks
                    .iter()
                    .find(|(t, _, _)| *t == app.subtitle_options.selected_audio_track)
                    .map(|(_, _, l)| l.as_str())
                    .unwrap_or("Seleziona traccia");

                egui::ComboBox::from_id_salt("sub_audio_track_select")
                    .selected_text(current_label)
                    .show_ui(ui, |ui| {
                        for (track_idx, _stream_idx, label) in &audio_tracks {
                            ui.selectable_value(
                                &mut app.subtitle_options.selected_audio_track,
                                *track_idx,
                                label,
                            );
                        }
                    });
            });
            ui.add_space(4.0);
        }

        ui.horizontal(|ui| {
            ui.label("Modalità sottotitoli:");
            ui.selectable_value(
                &mut app.subtitle_options.mode,
                SubtitleMode::ExtractAndTranscribeTrack,
                "Traccia nel contenitore (Soft)",
            );
            ui.selectable_value(
                &mut app.subtitle_options.mode,
                SubtitleMode::BurnIntoVideo,
                "Incorporati nel video (Burned-in)",
            );
            ui.selectable_value(
                &mut app.subtitle_options.mode,
                SubtitleMode::TranscribeOnly,
                "Solo file SRT/VTT",
            );
        });

        ui.horizontal(|ui| {
            ui.label("Modello:");
            egui::ComboBox::from_id_salt("whisper_model_combo")
                .selected_text(&app.subtitle_options.model)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.subtitle_options.model, "tiny".to_string(), "tiny (rapidissimo, 39M)");
                    ui.selectable_value(&mut app.subtitle_options.model, "base".to_string(), "base (consigliato, 74M)");
                    ui.selectable_value(&mut app.subtitle_options.model, "small".to_string(), "small (buona precisione, 244M)");
                    ui.selectable_value(&mut app.subtitle_options.model, "medium".to_string(), "medium (alta precisione, 769M)");
                    ui.selectable_value(&mut app.subtitle_options.model, "large-v3".to_string(), "large-v3 (massima qualità, 1550M)");
                });
        });

        ui.horizontal(|ui| {
            ui.label("Dispositivo:");
            ui.selectable_value(&mut app.subtitle_options.device, "cpu".to_string(), "CPU");
            ui.selectable_value(&mut app.subtitle_options.device, "cuda".to_string(), "CUDA (GPU)");
            ui.checkbox(&mut app.subtitle_options.vad_filter, "Filtro VAD (salta silenzi)");
        });

        ui.horizontal(|ui| {
            ui.label("Lingua:");
            ui.text_edit_singleline(&mut app.subtitle_options.language);
            ui.small("(es. 'it', 'en', oppure 'auto')");
        });

        ui.horizontal(|ui| {
            ui.label("Sottotitoli esterni (.srt/.vtt):");
            if ui.button("📂 Sfoglia...").clicked() {
                if let Some(sub) = rfd::FileDialog::new()
                    .add_filter("Sottotitoli", &["srt", "vtt", "ass"])
                    .pick_file()
                {
                    app.subtitle_options.custom_sub_file = Some(sub);
                }
            }
            if let Some(ref sub) = app.subtitle_options.custom_sub_file {
                let name = sub.file_name().and_then(|n| n.to_str()).unwrap_or("sub.srt");
                ui.label(RichText::new(name).monospace());
                if ui.small_button("✕").clicked() {
                    app.subtitle_options.custom_sub_file = None;
                }
            } else {
                ui.small("(opzionale: se presente, salta la trascrizione)");
            }
        });

        ui.add_space(4.0);
        ui.separator();
        ui.add_space(2.0);

        let can_start_sub = app.input_path.is_some() && app.output_path.is_some() && !app.is_running;
        if ui.add_enabled(can_start_sub, egui::Button::new(RichText::new("▶ Avvia Pipeline Sottotitoli").color(Color32::GREEN).strong())).clicked() {
            app.start_subtitles_pipeline();
        }
    });
}

fn render_trim_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.add_space(6.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Taglio temporale (Trim):").strong());

        let mut has_start = app.start_time.is_some();
        if ui.checkbox(&mut has_start, "Inizio (-ss)").changed() {
            app.start_time = if has_start { Some(0.0) } else { None };
        }
        if let Some(ref mut st) = app.start_time {
            ui.add(egui::DragValue::new(st).speed(1.0).range(0.0..=86400.0).suffix("s"));
        }

        let mut has_dur = app.duration.is_some();
        if ui.checkbox(&mut has_dur, "Durata (-t)").changed() {
            app.duration = if has_dur { Some(10.0) } else { None };
        }
        if let Some(ref mut d) = app.duration {
            ui.add(egui::DragValue::new(d).speed(1.0).range(0.1..=86400.0).suffix("s"));
        }
    });
}
