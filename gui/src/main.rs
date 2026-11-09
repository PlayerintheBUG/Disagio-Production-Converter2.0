mod app;
mod ui;

use app::FfGuiApp;
use eframe::egui::{self, Color32, RichText};
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1240.0, 840.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("ffgui - FFmpeg GUI"),
        ..Default::default()
    };

    eframe::run_native(
        "ffgui",
        native_options,
        Box::new(|cc| Ok(Box::new(FfGuiApp::new(cc)))),
    )
}

impl eframe::App for FfGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll runner events & async tasks
        self.poll_runner_events();
        self.poll_probe();
        self.poll_gpu_detection();
        self.poll_batch_scan();

        // Repaint periodically while a job is running or background tasks active (Section 3.2)
        if self.is_running
            || self.is_probing
            || self.is_detecting_gpu
            || self.rx_events.is_some()
            || !self.running_tasks.is_empty()
            || self.is_batch_scanning
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        // Handle drag and drop files or folders
        ctx.input(|i| {
            if let Some(file) = i.raw.dropped_files.first() {
                if let Some(ref path) = file.path {
                    if path.is_dir() {
                        self.batch_settings.source.folder = path.clone();
                        self.active_tab = app::MainTab::Batch;
                        self.trigger_batch_scan();
                    } else {
                        self.open_file(path.clone());
                        self.active_tab = app::MainTab::SingleFile;
                    }
                }
            }
        });

        // Top Header
        egui::TopBottomPanel::top("top_header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("ffgui").strong().color(Color32::from_rgb(0, 180, 255)));
                ui.label(RichText::new("– Interfaccia trasparente per FFmpeg").italics().color(Color32::GRAY));

                ui.add_space(16.0);

                // Tab Switcher buttons
                let single_btn = ui.selectable_value(
                    &mut self.active_tab,
                    app::MainTab::SingleFile,
                    RichText::new("🎬 File Singolo").strong(),
                );
                if single_btn.clicked() {
                    self.active_tab = app::MainTab::SingleFile;
                }

                let batch_btn = ui.selectable_value(
                    &mut self.active_tab,
                    app::MainTab::Batch,
                    RichText::new("📁 Conversione Batch").strong(),
                );
                if batch_btn.clicked() {
                    self.active_tab = app::MainTab::Batch;
                    if self.batch_plan_v2.is_none() && !self.batch_settings.source.folder.as_os_str().is_empty() {
                        self.trigger_batch_scan();
                    }
                }

                ui.add_space(12.0);

                // Settings button
                if ui.button("⚙ Impostazioni").clicked() {
                    self.show_settings_window = true;
                }

                ui.add_space(8.0);

                // Log Panel toggle button
                let log_text = if self.show_log_panel {
                    RichText::new("🖥 Nascondi Terminale").color(Color32::from_rgb(100, 220, 255))
                } else {
                    RichText::new("🖥 Mostra Terminale").strong().color(Color32::from_rgb(255, 200, 80))
                };
                if ui.button(log_text).clicked() {
                    self.show_log_panel = !self.show_log_panel;
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((ref msg, time)) = self.toast_message {
                        if time.elapsed() < Duration::from_secs(3) {
                            ui.colored_label(Color32::from_rgb(100, 255, 150), format!("ℹ {}", msg));
                        }
                    }
                });
            });
        });

        // Dedicated bottom dock: always-visible, resizable Terminal & Execution bar (Section 5)
        if self.show_log_panel {
            egui::TopBottomPanel::bottom("bottom_terminal_panel")
                .resizable(true)
                .min_height(170.0)
                .max_height(ctx.screen_rect().height() * 0.45)
                .default_height(240.0)
                .show(ctx, |ui| {
                    ui::log_panel::render(ui, self);
                });
        }

        // Render modal settings window
        ui::settings_dialog::render(ctx, self);

        // Central main panel
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.active_tab {
                app::MainTab::SingleFile => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // Three columns layout for the upper work area
                            let total_width = ui.available_width();
                            let col1_w = (total_width * 0.30).max(260.0);
                            let col2_w = (total_width * 0.44).max(340.0);
                            let col3_w = (total_width - col1_w - col2_w - 20.0).max(200.0);

                            ui.horizontal_top(|ui| {
                                // Left column: Files & streams
                                ui.allocate_ui_with_layout(
                                    egui::vec2(col1_w, 350.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui::stream_panel::render(ui, self);
                                    },
                                );

                                ui.separator();

                                // Center column: Codec controls & settings
                                ui.allocate_ui_with_layout(
                                    egui::vec2(col2_w, 350.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui::controls_panel::render(ui, self);
                                    },
                                );

                                ui.separator();

                                // Right column: Queue & Presets
                                ui.allocate_ui_with_layout(
                                    egui::vec2(col3_w, 350.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui::queue_panel::render(ui, self);
                                    },
                                );
                            });

                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);

                            // Middle section: Command Preview & Copy
                            ui::preview_panel::render(ui, self);

                            // If bottom dock is hidden, provide a banner to reopen it
                            if !self.show_log_panel {
                                ui.add_space(12.0);
                                ui.horizontal(|ui| {
                                    if ui.button(RichText::new("🖥 Mostra Terminale & Log").strong().color(Color32::from_rgb(0, 180, 255))).clicked() {
                                        self.show_log_panel = true;
                                    }
                                    ui.label(RichText::new("(Il terminale inferiore è temporaneamente nascosto)").italics().color(Color32::GRAY));
                                });
                            }
                        });
                }
                app::MainTab::Batch => {
                    ui::batch_dialog::render_batch_page(ui, self);
                }
            }
        });
    }
}
