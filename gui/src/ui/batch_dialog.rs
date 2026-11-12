use crate::app::FfGuiApp;
use eframe::egui::{self, Color32, RichText};
use ffgui_core::batch::{AudioQualityMode, BatchProfile, CollisionPolicy, MediaKind, OnErrorPolicy, PlanState};
use ffgui_core::model::{AudioRule, StreamKind};

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Full-page batch interface matching Section 12.4
pub fn render_batch_page(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    let total_width = ui.available_width();
    let col_left_w = (total_width * 0.46).max(340.0);
    let col_right_w = (total_width - col_left_w - 20.0).max(360.0);

    ui.horizontal_top(|ui| {
        // =========================================================
        // LEFT COLUMN: SORGENTE, IMPOSTAZIONI PER TIPO, DESTINAZIONE
        // =========================================================
        ui.allocate_ui_with_layout(
            egui::vec2(col_left_w, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("batch_left_scroll")
                    .show(ui, |ui| {
                        render_source_section(ui, app);
                        ui.add_space(8.0);
                        render_profiles_section(ui, app);
                        ui.add_space(8.0);
                        render_destination_section(ui, app);
                        ui.add_space(12.0);
                    });
            },
        );

        ui.separator();

        // =========================================================
        // RIGHT COLUMN: PIANO LIVE, RIEPILOGO, ANTEPRIMA, ESECUZIONE
        // =========================================================
        ui.allocate_ui_with_layout(
            egui::vec2(col_right_w, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("batch_right_scroll")
                    .show(ui, |ui| {
                        render_plan_section(ui, app);
                        ui.add_space(8.0);
                        render_summary_section(ui, app);
                        ui.add_space(8.0);
                        render_cmd_preview_section(ui, app);
                        ui.add_space(8.0);
                        render_execution_controls(ui, app);
                        ui.add_space(12.0);
                    });
            },
        );
    });
}

// ---------------------------------------------------------------------------
// 1. SORGENTE & TIPI DI FILE
// ---------------------------------------------------------------------------
fn render_source_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("1. SORGENTE").color(Color32::from_rgb(0, 180, 255)));
            if app.is_batch_scanning {
                ui.spinner();
                ui.colored_label(Color32::from_rgb(100, 200, 255), "Scansione...");
            }
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Cartella:").strong());
            let path_str = if app.batch_settings.source.folder.as_os_str().is_empty() {
                "Nessuna cartella selezionata".to_string()
            } else {
                app.batch_settings.source.folder.to_string_lossy().to_string()
            };
            ui.label(RichText::new(path_str).monospace());

            if ui.button("📂 Sfoglia...").clicked() {
                if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                    app.batch_settings.source.folder = folder;
                    app.trigger_batch_scan();
                }
            }
        });

        ui.add_space(2.0);
        if ui.checkbox(&mut app.batch_settings.source.recursive, "Includi sottocartelle (scansione ricorsiva)").changed() {
            app.trigger_batch_scan();
        }

        if let Some(ref plan) = app.batch_plan_v2 {
            ui.add_space(4.0);
            ui.colored_label(
                Color32::from_rgb(160, 215, 255),
                format!(
                    "Rilevati: {} video · {} audio · {} immagini · {} altri",
                    plan.video_count, plan.audio_count, plan.image_count, plan.other_count
                ),
            );
        }

        // Extension chips with counts
        if !app.batch_discovered_exts.is_empty() {
            ui.add_space(4.0);
            let mut re_scan = false;
            let discovered = app.batch_discovered_exts.clone();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Estensioni trovate (clicca per attivare/disattivare):").small().strong());
                if ui.small_button("Tutte").clicked() {
                    for (ext, _) in &discovered {
                        app.batch_settings.source.active_exts.insert(ext.clone());
                    }
                    re_scan = true;
                }
                if ui.small_button("Nessuna").clicked() {
                    app.batch_settings.source.active_exts.clear();
                    re_scan = true;
                }
            });

            ui.horizontal_wrapped(|ui| {
                for (ext, count) in &discovered {
                    let is_active = app.batch_settings.source.active_exts.contains(ext);
                    let label = format!("{}: {}", ext, count);
                    let color = if is_active {
                        Color32::from_rgb(100, 220, 150)
                    } else {
                        Color32::GRAY
                    };
                    if ui.selectable_label(is_active, RichText::new(label).color(color)).clicked() {
                        if is_active {
                            app.batch_settings.source.active_exts.remove(ext);
                        } else {
                            app.batch_settings.source.active_exts.insert(ext.clone());
                        }
                        re_scan = true;
                    }
                }
            });

            if re_scan {
                app.trigger_batch_scan();
            }
        }
    });
}

// ---------------------------------------------------------------------------
// 2. IMPOSTAZIONI PER TIPO (VIDEO, AUDIO, IMMAGINI, ALTRI)
// ---------------------------------------------------------------------------
fn render_profiles_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    let mut trigger_rescan = false;

    let video_codecs: Vec<(String, String)> = app.catalog.by_kind(StreamKind::Video)
        .into_iter().map(|c| (c.id.clone(), c.label.clone())).collect();

    let cur_video_profile_name = app.batch_settings.profiles.get(&MediaKind::Video)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Nessuno".to_string());

    let has_gpu = app.settings.gpu_enabled;
    let gpu_enc = if has_gpu {
        app.gpu_detection.as_ref().and_then(|d| d.first_available_gpu_encoder()).map(|s| s.to_string())
    } else {
        None
    };

    let video_summary = app.batch_settings.profiles.get(&MediaKind::Video).map(|p| p.describe(&app.catalog));
    let audio_summary = app.batch_settings.profiles.get(&MediaKind::Audio).map(|p| p.describe(&app.catalog));

    ui.group(|ui| {
        ui.heading(RichText::new("2. IMPOSTAZIONI PER TIPO").color(Color32::from_rgb(0, 180, 255)));
        ui.add_space(4.0);

        // --- VIDEO PROFILE ---
        ui.group(|ui| {
            let mut set_profile: Option<BatchProfile> = None;
            let mut do_copy_main = false;

            ui.horizontal(|ui| {
                ui.label(RichText::new("🎬 Video").strong());

                egui::ComboBox::from_id_salt("batch_video_preset_combo")
                    .selected_text(&cur_video_profile_name)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(cur_video_profile_name.contains("Archivio"), "Video Archivio (H.265 / MKV)").clicked() {
                            set_profile = Some(BatchProfile::archive_video(gpu_enc.as_deref()));
                        }
                        if ui.selectable_label(cur_video_profile_name.contains("Compatibile"), "Video Compatibile (H.264 / MP4)").clicked() {
                            set_profile = Some(BatchProfile::compatible_video(gpu_enc.as_deref()));
                        }
                    });

                if ui.button("📥 Copia da Principale").on_hover_text("Copia i codec e i parametri impostati nel pannello singolo").clicked() {
                    do_copy_main = true;
                }
            });

            if let Some(prof) = set_profile {
                app.batch_settings.profiles.insert(MediaKind::Video, prof);
                trigger_rescan = true;
            }
            if do_copy_main {
                app.copy_main_panel_to_video_profile();
                trigger_rescan = true;
            }

            if let Some(ref summary) = video_summary {
                for line in &summary.lines {
                    ui.label(RichText::new(format!("  • {}", line)).small().color(Color32::from_rgb(200, 220, 240)));
                }
                for warn in &summary.warnings {
                    ui.colored_label(Color32::YELLOW, format!("  ⚠ {}", warn));
                }
            }

            if let Some(prof) = app.batch_settings.profiles.get_mut(&MediaKind::Video) {
                ui.collapsing("Qualità video (Auto / Preset / Fissa)...", |ui| {
                    crate::ui::controls_panel::render_quality_mode_ui(ui, &mut prof.video_quality);
                });

                // Quick inline modifiers for Video Profile
                ui.collapsing("Modifica regole video...", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Contenitore:");
                        for c in &["mkv", "mp4", "webm", "mov"] {
                            if ui.selectable_value(&mut prof.target_container, c.to_string(), *c).clicked() {
                                trigger_rescan = true;
                            }
                        }
                    });

                    if let Some(ref mut v) = prof.video_rule {
                        ui.horizontal(|ui| {
                            ui.label("Codec video:");
                            egui::ComboBox::from_id_salt("batch_v_codec_edit")
                                .selected_text(&v.codec_id)
                                .show_ui(ui, |ui| {
                                    for (cid, clabel) in &video_codecs {
                                        if ui.selectable_value(&mut v.codec_id, cid.clone(), clabel).clicked() {
                                            trigger_rescan = true;
                                        }
                                    }
                                });
                        });

                        ui.horizontal(|ui| {
                            ui.label("Filtro scala:");
                            let cur_scale = v.scale_preset.clone().unwrap_or_else(|| "Originale".to_string());
                            egui::ComboBox::from_id_salt("batch_v_scale_edit")
                                .selected_text(&cur_scale)
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(cur_scale == "Originale", "Originale").clicked() {
                                        v.scale_preset = None;
                                        trigger_rescan = true;
                                    }
                                    if ui.selectable_label(cur_scale == "1080p (altezza 1080)", "1080p").clicked() {
                                        v.scale_preset = Some("1080p (altezza 1080)".to_string());
                                        trigger_rescan = true;
                                    }
                                    if ui.selectable_label(cur_scale == "720p (altezza 720)", "720p").clicked() {
                                        v.scale_preset = Some("720p (altezza 720)".to_string());
                                        trigger_rescan = true;
                                    }
                                });
                        });
                    }
                });
            }
        });

        ui.add_space(4.0);

        // --- AUDIO PROFILE ---
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎵 Audio").strong());
                if let Some(ref summary) = audio_summary {
                    for line in &summary.lines {
                        ui.label(RichText::new(line).small().color(Color32::from_rgb(180, 230, 190)));
                    }
                }
            });

            if let Some(prof) = app.batch_settings.profiles.get_mut(&MediaKind::Audio) {
                ui.collapsing("Modifica regole audio...", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Formato / Contenitore:");
                        for c in &["opus", "m4a", "mp3", "flac"] {
                            if ui.selectable_value(&mut prof.target_container, c.to_string(), *c).clicked() {
                                trigger_rescan = true;
                            }
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Trattamento audio:");
                        let is_copy = matches!(prof.audio_rule, AudioRule::CopyAll);
                        if ui.selectable_label(is_copy, "Copia traccia originale").clicked() {
                            prof.audio_rule = AudioRule::CopyAll;
                            trigger_rescan = true;
                        }
                        if ui.selectable_label(!is_copy, "Ricodifica (Opus/AAC)").clicked() {
                            let mut p = std::collections::HashMap::new();
                            p.insert("bitrate".to_string(), serde_json::json!("128k"));
                            prof.audio_rule = AudioRule::EncodeAll {
                                codec_id: "libopus".to_string(),
                                params: p,
                            };
                            trigger_rescan = true;
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Bitrate:");
                        if ui.radio(prof.audio_quality == AudioQualityMode::Manual, "Manuale").clicked() {
                            prof.audio_quality = AudioQualityMode::Manual;
                        }
                        if ui.radio(prof.audio_quality == AudioQualityMode::Auto, "Automatico").clicked() {
                            prof.audio_quality = AudioQualityMode::Auto;
                        }
                    });
                    if prof.audio_quality == AudioQualityMode::Auto {
                        ui.small(RichText::new("Bitrate calcolato dai canali della traccia sorgente").color(Color32::GRAY));
                    }
                });
            }
        });

        ui.add_space(4.0);

        // --- IMAGE PROFILE ---
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🖼 Immagini").strong());
                if let Some(prof) = app.batch_settings.profiles.get_mut(&MediaKind::Image) {
                    ui.label(RichText::new(format!("Destinazione: .{}", prof.target_container)).small());
                    egui::ComboBox::from_id_salt("batch_img_container_combo")
                        .selected_text(&prof.target_container)
                        .show_ui(ui, |ui| {
                            for c in &["jpg", "webp", "png"] {
                                if ui.selectable_value(&mut prof.target_container, c.to_string(), *c).clicked() {
                                    trigger_rescan = true;
                                }
                            }
                        });
                }
            });
        });

        ui.add_space(4.0);

        // --- OTHER FILES ---
        ui.horizontal(|ui| {
            if ui.checkbox(&mut app.batch_settings.dest.copy_other_files, "Copia altri file non multimediali (copertine, .srt, .nfo)").changed() {
                trigger_rescan = true;
            }
        });
    });

    if trigger_rescan {
        app.trigger_batch_scan();
    }
}

// ---------------------------------------------------------------------------
// 3. DESTINAZIONE & NOMI
// ---------------------------------------------------------------------------
fn render_destination_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.heading(RichText::new("3. DESTINAZIONE").color(Color32::from_rgb(0, 180, 255)));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let is_subfolder = app.batch_settings.dest.folder.is_none();
            if ui.radio(is_subfolder, "Sottocartella dentro l'origine:").clicked() {
                app.batch_settings.dest.folder = None;
                app.trigger_batch_scan();
            }
            if is_subfolder && ui.text_edit_singleline(&mut app.batch_settings.dest.subfolder_name).changed() {
                app.trigger_batch_scan();
            }
        });

        ui.horizontal(|ui| {
            let is_custom = app.batch_settings.dest.folder.is_some();
            if ui.radio(is_custom, "Cartella personalizzata:").clicked() && !is_custom {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    app.batch_settings.dest.folder = Some(dir);
                    app.trigger_batch_scan();
                }
            }
            if let Some(ref dir) = app.batch_settings.dest.folder {
                ui.label(RichText::new(dir.to_string_lossy().to_string()).monospace());
                if ui.button("Cambia...").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        app.batch_settings.dest.folder = Some(d);
                        app.trigger_batch_scan();
                    }
                }
                if ui.small_button("✕").clicked() {
                    app.batch_settings.dest.folder = None;
                    app.trigger_batch_scan();
                }
            }
        });

        ui.add_space(2.0);
        if ui.checkbox(&mut app.batch_settings.dest.keep_tree, "Mantieni la struttura ad albero delle cartelle").changed() {
            app.trigger_batch_scan();
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Modello nome file:");
            if ui.text_edit_singleline(&mut app.batch_settings.dest.naming_template).changed() {
                app.trigger_batch_scan();
            }
        });
        ui.small(RichText::new("Segnaposti: {nome}, {codec}, {qualita}, {risoluzione}, {n}").color(Color32::GRAY));

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Se il file esiste già:");
            egui::ComboBox::from_id_salt("batch_collision_combo")
                .selected_text(match app.batch_settings.dest.collision_policy {
                    CollisionPolicy::Rename => "Rinomina aggiungendo (2)",
                    CollisionPolicy::Skip => "Salta file",
                    CollisionPolicy::Overwrite => "Sovrascrivi",
                })
                .show_ui(ui, |ui| {
                    if ui.selectable_value(&mut app.batch_settings.dest.collision_policy, CollisionPolicy::Rename, "Rinomina aggiungendo (2)").clicked() {
                        app.trigger_batch_scan();
                    }
                    if ui.selectable_value(&mut app.batch_settings.dest.collision_policy, CollisionPolicy::Skip, "Salta file").clicked() {
                        app.trigger_batch_scan();
                    }
                    if ui.selectable_value(&mut app.batch_settings.dest.collision_policy, CollisionPolicy::Overwrite, "Sovrascrivi").clicked() {
                        app.trigger_batch_scan();
                    }
                });
        });

        if app.batch_settings.dest.collision_policy == CollisionPolicy::Overwrite {
            ui.colored_label(Color32::from_rgb(255, 170, 80), "⚠ Attenzione: i file di output esistenti con lo stesso nome verranno sovrascritti!");
        }
    });
}

// ---------------------------------------------------------------------------
// RIGHT COLUMN: PIANO LIVE
// ---------------------------------------------------------------------------
fn render_plan_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("PIANO DI CONVERSIONE (LIVE)").color(Color32::from_rgb(0, 180, 255)));
            if let Some(ref plan) = app.batch_plan_v2 {
                ui.label(RichText::new(format!("({} elementi)", plan.items.len())).small().color(Color32::GRAY));
            }
        });
        ui.add_space(2.0);

        let Some(plan) = app.batch_plan_v2.clone() else {
            ui.label(RichText::new("Seleziona una cartella per generare il piano in tempo reale.").italics().color(Color32::GRAY));
            return;
        };

        if plan.items.is_empty() {
            ui.colored_label(Color32::YELLOW, "Nessun file trovato corrispondente alle estensioni selezionate.");
            return;
        }

        let mut toggle_exclude_path = None;

        // Table showing plan items
        egui::ScrollArea::vertical()
            .id_salt("batch_plan_items_scroll")
            .max_height(220.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("File sorgente").strong());
                    ui.separator();
                    ui.label(RichText::new("Tipo").strong());
                    ui.separator();
                    ui.label(RichText::new("Stato").strong());
                    ui.separator();
                    ui.label(RichText::new("Dimensione").strong());
                });
                ui.separator();

                for item in &plan.items {
                    let file_name = item.input.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let is_excluded = app.batch_settings.overrides.get(&item.input).map(|o| o.exclude).unwrap_or(false);

                    ui.horizontal(|ui| {
                        let name_label = if is_excluded {
                            RichText::new(&file_name).strikethrough().color(Color32::GRAY)
                        } else {
                            RichText::new(&file_name)
                        };
                        ui.label(name_label);

                        ui.label(RichText::new(item.media_kind.as_str()).small().color(Color32::from_rgb(140, 200, 255)));

                        let (badge_text, badge_color) = if is_excluded {
                            ("ESCLUSO", Color32::GRAY)
                        } else {
                            match item.state {
                                PlanState::New => ("NUOVO", Color32::GREEN),
                                PlanState::ExistsSkip => ("SALTA (Esiste)", Color32::from_rgb(180, 180, 80)),
                                PlanState::ExistsOverwrite => ("SOVRASCRIVI", Color32::from_rgb(255, 120, 80)),
                                PlanState::Renamed => ("RINOMINA", Color32::from_rgb(100, 180, 255)),
                            }
                        };
                        ui.colored_label(badge_color, RichText::new(badge_text).small().strong());

                        ui.label(RichText::new(format_bytes(item.input_size_bytes)).small().color(Color32::GRAY));

                        // Quick exclude / include toggle
                        if ui.small_button(if is_excluded { "Ripristina" } else { "Escludi" }).clicked() {
                            toggle_exclude_path = Some(item.input.clone());
                        }
                    });
                }
            });

        if let Some(path) = toggle_exclude_path {
            let entry = app.batch_settings.overrides.entry(path).or_insert_with(|| {
                ffgui_core::batch::BatchProfileOverride { exclude: false, profile: None }
            });
            entry.exclude = !entry.exclude;
            app.trigger_batch_scan();
        }
    });
}

// ---------------------------------------------------------------------------
// RIGHT COLUMN: RIEPILOGO & CONTROLLO DISCO
// ---------------------------------------------------------------------------
fn render_summary_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.heading(RichText::new("RIEPILOGO & SPAZIO").color(Color32::from_rgb(0, 180, 255)));
        ui.add_space(2.0);

        let Some(ref plan) = app.batch_plan_v2 else {
            ui.label(RichText::new("Nessun piano attivo").italics().color(Color32::GRAY));
            return;
        };

        let to_process = plan.items.iter().filter(|i| i.state != PlanState::ExistsSkip).count();
        let skipped = plan.items.len() - to_process;

        ui.horizontal(|ui| {
            ui.label(format!(
                "Totale: {} file ({} da convertire, {} saltati)",
                plan.items.len(), to_process, skipped
            ));
        });

        let est_output_str = format_bytes(plan.total_estimated_output_bytes);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Spazio stimato output: ~{}", est_output_str)).strong());

            if let Some(free_bytes) = plan.available_dest_space_bytes {
                let free_str = format_bytes(free_bytes);
                if plan.total_estimated_output_bytes > free_bytes {
                    ui.colored_label(Color32::RED, RichText::new(format!("✖ Spazio insufficiente (libero: {})", free_str)).strong());
                } else {
                    ui.colored_label(Color32::GREEN, RichText::new(format!("✔ Spazio libero: {}", free_str)).strong());
                }
            }
        });
    });
}

// ---------------------------------------------------------------------------
// RIGHT COLUMN: ANTEPRIMA COMANDO
// ---------------------------------------------------------------------------
fn render_cmd_preview_section(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("ANTEPRIMA COMANDO FFMPEG").color(Color32::from_rgb(0, 180, 255)));
            if app.settings.gpu_enabled {
                ui.colored_label(Color32::from_rgb(100, 220, 150), RichText::new("⚡ GPU").small().strong());
            }
        });
        ui.add_space(2.0);

        let cmd_preview = if let Some(prof) = app.batch_settings.profiles.get(&MediaKind::Video) {
            let hw_flag = match app.settings.hwaccel_decode.as_str() {
                "disabled" | "none" => "",
                "cuda" => "-hwaccel cuda ",
                "vaapi" => "-hwaccel vaapi ",
                "qsv" => "-hwaccel qsv ",
                _ => "-hwaccel auto ",
            };

            let v_codec = prof.video_rule.as_ref().map(|v| v.codec_id.as_str()).unwrap_or("copy");
            let a_codec = match &prof.audio_rule {
                AudioRule::CopyAll => "copy",
                AudioRule::EncodeAll { codec_id, .. } => codec_id.as_str(),
                AudioRule::EncodeFirstCopyRest { codec_id, .. } => codec_id.as_str(),
            };

            format!(
                "ffmpeg {}-i {{INPUT}} -c:v {} -c:a {} {{OUTPUT}}.{}",
                hw_flag, v_codec, a_codec, prof.target_container
            )
        } else {
            "ffmpeg -i {INPUT} -c copy {OUTPUT}".to_string()
        };

        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(20, 22, 26))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 54, 62)))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(&cmd_preview).monospace().color(Color32::from_rgb(210, 225, 245)));
            });
    });
}

// ---------------------------------------------------------------------------
// RIGHT COLUMN: CONTROLLI DI ESECUZIONE & AVVIO
// ---------------------------------------------------------------------------
fn render_execution_controls(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        ui.heading(RichText::new("ESECUZIONE").color(Color32::from_rgb(0, 180, 255)));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("File paralleli:");
            ui.add(egui::Slider::new(&mut app.settings.parallel_jobs, 1..=8).text("processi"));

            ui.add_space(8.0);
            ui.label("Sessioni GPU:");
            ui.add(egui::Slider::new(&mut app.settings.max_gpu_sessions, 1..=4).text("GPU max"));
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label("Se un file fallisce:");
            ui.selectable_value(&mut app.batch_settings.run.on_error, OnErrorPolicy::Continue, "Continua");
            ui.selectable_value(&mut app.batch_settings.run.on_error, OnErrorPolicy::Stop, "Interrompi");
        });

        ui.add_space(2.0);
        ui.checkbox(&mut app.settings.auto_cpu_fallback, RichText::new("Fallback automatico su CPU se la codifica GPU fallisce").strong());

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);

        // Big Action Buttons
        let has_plan = app.batch_plan_v2.as_ref().map(|p| !p.items.is_empty()).unwrap_or(false);
        ui.horizontal(|ui| {
            let start_btn = egui::Button::new(RichText::new("▶ Avvia Batch Adesso").color(Color32::GREEN).strong())
                .min_size(egui::vec2(160.0, 32.0));
            if ui.add_enabled(has_plan && !app.is_running, start_btn).clicked() {
                app.enqueue_batch_plan_v2(true);
            }

            let queue_btn = egui::Button::new(RichText::new("➕ Aggiungi a Coda").color(Color32::from_rgb(100, 200, 255)).strong())
                .min_size(egui::vec2(140.0, 32.0));
            if ui.add_enabled(has_plan, queue_btn).clicked() {
                app.enqueue_batch_plan_v2(false);
            }
        });
    });
}
