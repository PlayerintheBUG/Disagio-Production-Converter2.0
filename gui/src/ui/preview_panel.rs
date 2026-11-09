use crate::app::FfGuiApp;
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, RichText};
use ffgui_core::builder::CommandBuilder;
use ffgui_core::cmdview::{build_compact, part_path, CompactCommand, InputSummary, TokenKind};
use ffgui_core::runner::Runner;
use ffgui_core::validate::validate_job;
use std::time::{Duration, Instant};

/// Come mostrare il comando ffmpeg.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CmdViewMode {
    /// Una riga leggibile con {INPUT} e {OUTPUT}, senza specificatori inutili
    #[default]
    Compact,
    /// Esattamente ciò che esegue ffgui (file .part, -progress, -loglevel)
    Exact,
}

/// Tutto ciò che serve per mostrare e copiare il comando, calcolato una volta per frame.
struct Shown {
    input: String,
    output: String,
    compact: CompactCommand,
    exact: String,
}

fn compute(app: &FfGuiApp) -> Option<Shown> {
    let job = app.build_current_job()?;
    let args = app.build_job_args(&job);
    let summary = app
        .probe_result
        .as_ref()
        .map(|p| InputSummary::from_streams(&p.streams));
    let compact = build_compact(&job, &app.catalog, &args, summary.as_ref());

    // Comando eseguito davvero: scrive su .part e il runner aggiunge log e avanzamento
    let mut run_job = job.clone();
    run_job.output.path = part_path(&job.output.path);
    let exact_args = Runner::decorate_args(app.build_job_args(&run_job));
    let exact = CommandBuilder::to_preview_string(&exact_args);

    Some(Shown {
        input: job.input.path.to_string_lossy().to_string(),
        output: job.output.path.to_string_lossy().to_string(),
        compact,
        exact,
    })
}

fn kind_color(kind: TokenKind) -> Color32 {
    match kind {
        TokenKind::Program => Color32::from_rgb(150, 155, 165),
        TokenKind::Global => Color32::from_rgb(170, 175, 185),
        TokenKind::Hardware => Color32::from_rgb(185, 150, 255),
        TokenKind::Input => Color32::from_rgb(255, 205, 100),
        TokenKind::Map => Color32::from_rgb(150, 155, 165),
        TokenKind::Video => Color32::from_rgb(110, 190, 255),
        TokenKind::Audio => Color32::from_rgb(120, 220, 150),
        TokenKind::Subtitle => Color32::from_rgb(230, 205, 110),
        TokenKind::Filter => Color32::from_rgb(255, 150, 200),
        TokenKind::Output => Color32::from_rgb(255, 170, 90),
    }
}

/// Riga colorata per sezione; i pezzi cambiati da poco hanno lo sfondo evidenziato.
fn compact_layout(compact: &CompactCommand, changed: &[bool], flashing: bool, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let font = egui::FontId::monospace(13.0);

    for (i, token) in compact.tokens.iter().enumerate() {
        let highlight = flashing && changed.get(i).copied().unwrap_or(false);
        let format = TextFormat {
            font_id: font.clone(),
            color: kind_color(token.kind),
            background: if highlight { Color32::from_rgb(95, 82, 20) } else { Color32::TRANSPARENT },
            ..Default::default()
        };
        job.append(&token.display_text(), 0.0, format);
        job.append(
            " ",
            0.0,
            TextFormat { font_id: font.clone(), color: Color32::GRAY, ..Default::default() },
        );
    }
    job
}

pub fn render(ui: &mut egui::Ui, app: &mut FfGuiApp) {
    ui.group(|ui| {
        let shown = compute(app);

        // Quali pezzi sono cambiati rispetto al frame precedente (per il lampeggio)
        if let Some(sh) = &shown {
            let tokens: Vec<String> = sh.compact.tokens.iter().map(|t| t.text.clone()).collect();
            if tokens != app.cmd_prev_tokens {
                if !app.cmd_prev_tokens.is_empty() {
                    let flags: Vec<bool> = tokens
                        .iter()
                        .map(|t| !app.cmd_prev_tokens.contains(t))
                        .collect();
                    app.cmd_changed_flags = flags;
                    app.cmd_changed_at = Some(Instant::now());
                }
                app.cmd_prev_tokens = tokens;
            }
        }
        let flashing = app
            .cmd_changed_at
            .is_some_and(|t| t.elapsed() < Duration::from_millis(900));
        if flashing {
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }

        // Intestazione: stato, badge, scelta della vista, copia
        ui.horizontal(|ui| {
            if app.is_running {
                ui.colored_label(Color32::from_rgb(255, 80, 80), RichText::new("▶ IN ESECUZIONE").strong());
            } else {
                ui.colored_label(Color32::from_rgb(0, 200, 255), RichText::new("⚡ COMANDO LIVE").strong());
            }

            if shown.is_some() {
                if app.settings.gpu_enabled {
                    ui.colored_label(Color32::from_rgb(100, 220, 150), RichText::new("⚡ GPU").small().strong());
                }
                let hw_dec = app.settings.hwaccel_decode.as_str();
                if hw_dec != "disabled" && hw_dec != "none" && !hw_dec.is_empty() {
                    ui.colored_label(
                        Color32::from_rgb(120, 200, 255),
                        RichText::new(format!("-hwaccel {}", hw_dec)).small(),
                    );
                }
            }

            ui.separator();
            ui.selectable_value(&mut app.cmd_view_mode, CmdViewMode::Compact, "Compatto")
                .on_hover_text("Una riga leggibile, con {INPUT} e {OUTPUT}");
            ui.selectable_value(&mut app.cmd_view_mode, CmdViewMode::Exact, "Esatto")
                .on_hover_text("Il comando esattamente come lo esegue ffgui");

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(sh) = &shown {
                    let (text, tip) = match app.cmd_view_mode {
                        CmdViewMode::Compact => (
                            sh.compact.to_shell(&sh.input, &sh.output),
                            "Comando pronto per il terminale, con i percorsi reali",
                        ),
                        CmdViewMode::Exact => (sh.exact.clone(), "Comando esatto, come lo esegue ffgui"),
                    };
                    if ui.button("📋 Copia").on_hover_text(tip).clicked() {
                        ui.ctx().copy_text(text);
                        app.set_toast("Comando FFmpeg copiato negli appunti!");
                    }
                    if app.cmd_view_mode == CmdViewMode::Compact
                        && ui
                            .button("📋 Con segnaposto")
                            .on_hover_text("Copia la riga com'è mostrata, con {INPUT} e {OUTPUT}")
                            .clicked()
                    {
                        ui.ctx().copy_text(sh.compact.to_text());
                        app.set_toast("Comando con segnaposto copiato!");
                    }
                }
            });
        });

        ui.add_space(2.0);

        // Il comando
        egui::Frame::canvas(ui.style())
            .fill(Color32::from_rgb(22, 23, 26))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 53, 60)))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                match &shown {
                    None => {
                        ui.label(
                            RichText::new("ffmpeg (seleziona un file di input per generare il comando in tempo reale)")
                                .monospace()
                                .color(Color32::GRAY),
                        );
                    }
                    Some(sh) => match app.cmd_view_mode {
                        CmdViewMode::Compact => {
                            let layout =
                                compact_layout(&sh.compact, &app.cmd_changed_flags, flashing, ui.available_width());
                            ui.label(layout);
                        }
                        CmdViewMode::Exact => {
                            ui.label(
                                RichText::new(&sh.exact)
                                    .monospace()
                                    .color(Color32::from_rgb(220, 230, 242)),
                            );
                        }
                    },
                }
            });

        // Note sotto il comando
        if let Some(sh) = &shown {
            let mut notes: Vec<&str> = Vec::new();
            match app.cmd_view_mode {
                CmdViewMode::Compact => {
                    notes.push("{INPUT} e {OUTPUT} sono i file scelti. In esecuzione ffgui scrive su un file .part e aggiunge log e avanzamento (vista «Esatto»).");
                    if sh.compact.map_hidden {
                        notes.push("-map omesso: coincide con la scelta automatica di ffmpeg.");
                    }
                    if sh.compact.kept_specifiers {
                        notes.push("Indici degli stream mantenuti: ci sono più stream dello stesso tipo.");
                    }
                }
                CmdViewMode::Exact => {
                    notes.push("Esattamente ciò che viene eseguito; il file .part viene rinominato a fine lavoro.");
                }
            }
            ui.label(RichText::new(notes.join(" · ")).small().color(Color32::GRAY));
        }

        // Problemi di validazione
        if let Some(job) = app.build_current_job() {
            let issues = validate_job(&job, &app.catalog);
            if !issues.is_empty() {
                ui.add_space(3.0);
                for issue in issues {
                    if issue.is_error() {
                        ui.horizontal(|ui| {
                            ui.colored_label(Color32::RED, "✖ ERRORE:");
                            ui.colored_label(Color32::RED, issue.message);
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.colored_label(Color32::YELLOW, "⚠ AVVISO:");
                            ui.colored_label(Color32::YELLOW, issue.message);
                        });
                    }
                }
            }
        }
    });
}
