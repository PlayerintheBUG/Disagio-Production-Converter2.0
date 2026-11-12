use crate::catalog::Catalog;
use crate::model::{AudioRule, CodecConfig, Job, JobTemplate, SubtitleRule, VideoRule};
use crate::validate::paths_refer_to_same_file;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
    Other,
}

impl MediaKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MediaKind::Video => "Video",
            MediaKind::Audio => "Audio",
            MediaKind::Image => "Immagine",
            MediaKind::Other => "Altro",
        }
    }
}

pub fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "bmp" | "tiff" | "tif" | "gif" | "tga" | "dng" | "raw" | "cr2" | "nef" | "arw"
    )
}

pub fn detect_media_kind(path: &Path) -> MediaKind {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if is_video_ext(ext) {
        MediaKind::Video
    } else if is_audio_ext(ext) {
        MediaKind::Audio
    } else if is_image_ext(ext) {
        MediaKind::Image
    } else {
        MediaKind::Other
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum QualityTier {
    High,
    #[default]
    Medium,
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SizeCap {
    Megabytes(f64),
    PercentReduction(f64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AutoOptions {
    pub size_cap: Option<SizeCap>,
    pub use_bpp_probe: bool,
}

/// Modalità di determinazione della qualità video.
/// `Manual` = usa i parametri scelti dall'utente nel pannello del codec (comportamento originale).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum QualityMode {
    #[default]
    Manual,
    Preset(QualityTier),
    Fixed { param_id: String, value: f64 },
    Auto(AutoOptions),
}

/// Modalità di determinazione del bitrate audio.
/// `Manual` = usa il bitrate scelto dall'utente; `Auto` = calcolato dai canali sorgente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AudioQualityMode {
    #[default]
    Manual,
    Auto,
}

impl QualityMode {
    pub fn label(&self) -> String {
        match self {
            QualityMode::Manual => "Manuale (parametri codec)".to_string(),
            QualityMode::Preset(QualityTier::High) => "Preset: Alta".to_string(),
            QualityMode::Preset(QualityTier::Medium) => "Preset: Media".to_string(),
            QualityMode::Preset(QualityTier::Low) => "Preset: Bassa".to_string(),
            QualityMode::Fixed { value, .. } => format!("Fissa: {}", value),
            QualityMode::Auto(opts) => match &opts.size_cap {
                Some(SizeCap::Megabytes(mb)) => format!("Auto: max {} MB", mb),
                Some(SizeCap::PercentReduction(pct)) => format!("Auto: -{}%", pct),
                None => "Auto (qualità adattiva)".to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedQuality {
    pub cq: Option<u32>,
    pub bitrate_kbps: Option<u32>,
    pub extra_params: HashMap<String, serde_json::Value>,
    pub summary: String,
}

/// Id del parametro di controllo qualità (rate control) per famiglia di encoder.
pub fn rate_control_param(codec_id: &str) -> &'static str {
    if codec_id.ends_with("_nvenc") {
        "cq"
    } else if codec_id.ends_with("_vaapi") {
        "qp"
    } else if codec_id.ends_with("_qsv") {
        "global_quality"
    } else if codec_id.ends_with("_amf") {
        "qp_i"
    } else {
        "crf"
    }
}

fn crf_max_for(codec_id: &str) -> i32 {
    if codec_id.contains("svtav1") {
        63
    } else {
        51
    }
}

/// Chiavi dei parametri di rate control da neutralizzare in modalità bitrate.
const RATE_CONTROL_KEYS: &[&str] = &["crf", "cq", "qp", "global_quality", "qp_i", "qp_p"];

pub fn resolve_quality(
    mode: &QualityMode,
    probe: Option<&crate::probe::ProbeResult>,
    dst_width: Option<u32>,
    dst_height: Option<u32>,
    codec_id: &str,
    src_file_size: Option<u64>,
    audio_bitrate_kbps: Option<u32>,
) -> ResolvedQuality {
    let mut extra_params = HashMap::new();

    match mode {
        QualityMode::Manual => ResolvedQuality {
            cq: None,
            bitrate_kbps: None,
            extra_params,
            summary: "Manuale".to_string(),
        },
        QualityMode::Fixed { param_id, value } => {
            let param_id = if param_id.is_empty() {
                rate_control_param(codec_id).to_string()
            } else {
                param_id.clone()
            };
            let clamped = (*value as i32).clamp(0, crf_max_for(codec_id)) as u32;
            if param_id == "qp_i" {
                extra_params.insert("rc".to_string(), serde_json::json!("cqp"));
                extra_params.insert("qp_p".to_string(), serde_json::json!(clamped));
            }
            extra_params.insert(param_id.clone(), serde_json::json!(clamped));
            ResolvedQuality {
                cq: Some(clamped),
                bitrate_kbps: None,
                extra_params,
                summary: format!("Fissa ({} {})", param_id.to_uppercase(), clamped),
            }
        }
        QualityMode::Preset(tier) => {
            let (param_key, base_val) = if codec_id.ends_with("_nvenc") {
                ("cq", 24)
            } else if codec_id.ends_with("_vaapi") {
                ("qp", 24)
            } else if codec_id.ends_with("_qsv") {
                ("global_quality", 24)
            } else if codec_id == "libsvtav1" {
                ("crf", 28)
            } else if codec_id == "libx265" {
                ("crf", 25)
            } else {
                ("crf", 22)
            };

            let delta = match tier {
                QualityTier::High => -4,
                QualityTier::Medium => 0,
                QualityTier::Low => 4,
            };
            let cq_val = (base_val + delta).clamp(8, 51) as u32;
            extra_params.insert(param_key.to_string(), serde_json::json!(cq_val));

            let tier_label = match tier {
                QualityTier::High => "Alta",
                QualityTier::Medium => "Media",
                QualityTier::Low => "Bassa",
            };
            ResolvedQuality {
                cq: Some(cq_val),
                bitrate_kbps: None,
                extra_params,
                summary: format!("{} ({} {})", tier_label, param_key.to_uppercase(), cq_val),
            }
        }
        QualityMode::Auto(opts) => {
            let duration_s = probe.and_then(|p| p.duration_s).unwrap_or(0.0);
            let audio_kbps_est = audio_bitrate_kbps.unwrap_or(0) as f64;

            // 1. Check size cap
            if let Some(ref cap) = opts.size_cap {
                let cap_applies = match cap {
                    SizeCap::Megabytes(mb) => *mb > 0.0 && duration_s > 0.0,
                    SizeCap::PercentReduction(_) => src_file_size.map(|s| s > 0).unwrap_or(false) && duration_s > 0.0,
                };

                if cap_applies {
                    let target_kbps = match cap {
                        SizeCap::Megabytes(mb) => {
                            let target_bits = mb * 8.0 * 1024.0 * 1024.0;
                            let audio_bits = audio_kbps_est * 1000.0 * duration_s;
                            let video_bits = (target_bits - audio_bits).max(0.0);
                            Some((video_bits / duration_s / 1000.0) as u32)
                        }
                        SizeCap::PercentReduction(pct) => {
                            let src_bytes = src_file_size.unwrap_or(0) as f64;
                            let target_bytes = src_bytes * (1.0 - pct.clamp(0.0, 99.0) / 100.0);
                            let target_bits = target_bytes * 8.0;
                            let audio_bits = audio_kbps_est * 1000.0 * duration_s;
                            let video_bits = (target_bits - audio_bits).max(0.0);
                            Some((video_bits / duration_s / 1000.0) as u32)
                        }
                    };

                    if let Some(target_kbps) = target_kbps {
                        let min_kbps = if codec_id.contains("av1") {
                            100
                        } else if codec_id.contains("hevc") || codec_id.contains("x265") {
                            150
                        } else {
                            200
                        };
                        let final_kbps = target_kbps.max(min_kbps);
                        let cap_desc = match cap {
                            SizeCap::Megabytes(mb) => format!("max {:.0} MB", mb),
                            SizeCap::PercentReduction(pct) => format!("-{:.0}%", pct),
                        };

                        // In modalità bitrate il CRF/CQ va neutralizzato (Value::Null
                        // sopprime l'emissione del default del TOML nel builder).
                        for key in RATE_CONTROL_KEYS {
                            extra_params.insert(key.to_string(), serde_json::Value::Null);
                        }
                        extra_params.insert("b_v".to_string(), serde_json::json!(format!("{}k", final_kbps)));
                        if codec_id.ends_with("_nvenc") || codec_id.ends_with("_amf") {
                            extra_params.insert("rc".to_string(), serde_json::json!("cbr"));
                        }

                        return ResolvedQuality {
                            cq: None,
                            bitrate_kbps: Some(final_kbps),
                            extra_params,
                            summary: format!("AUTO Bitrate {} kbps (target: {})", final_kbps, cap_desc),
                        };
                    }
                }
            }

            // 2. Compute CQ from base + deltas (matching Disagio algorithm)
            let base_cq: i32 = if codec_id.contains("av1") {
                28
            } else if codec_id.contains("hevc") || codec_id.contains("x265") {
                24
            } else if codec_id.contains("264") {
                20
            } else {
                24
            };

            let src_video = probe.and_then(|p| p.video_streams().into_iter().next());
            let src_w = src_video.and_then(|v| v.width).unwrap_or(1920);
            let src_h = src_video.and_then(|v| v.height).unwrap_or(1080);
            let dst_h = dst_height.unwrap_or(src_h);
            let dst_w = dst_width.unwrap_or(src_w);

            let delta_res: i32 = if dst_h >= 4320 {
                -6
            } else if dst_h >= 2160 {
                -4
            } else if dst_h >= 1440 {
                -2
            } else if dst_h >= 1080 {
                0
            } else if dst_h >= 720 {
                2
            } else {
                4
            };

            let fps = src_video.and_then(|v| v.fps).unwrap_or(25.0);
            let delta_fps: i32 = if fps <= 24.0 {
                1
            } else if fps <= 30.0 {
                0
            } else if fps <= 60.0 {
                -2
            } else {
                -4
            };

            let depth = src_video
                .and_then(|v| v.pix_fmt.as_deref())
                .map(|f| if f.contains("10") { 10 } else if f.contains("12") { 12 } else { 8 })
                .unwrap_or(8);
            let delta_depth: i32 = if depth <= 8 {
                0
            } else if depth == 10 {
                -2
            } else {
                -3
            };

            let cq_max = crf_max_for(codec_id);
            let (delta_bpp, bpp_dst) = if !opts.use_bpp_probe {
                (0, 0.0)
            } else {
                let src_pixels = (src_w * src_h).max(1) as f64;
                let dst_pixels = (dst_w * dst_h).max(1) as f64;
                // Il bit_rate del contenitore include l'audio: sottrai una stima
                // del contributo audio per avvicinarti al bitrate video reale.
                let audio_stream_count = probe
                    .map(|p| p.streams.iter().filter(|s| s.kind == crate::model::StreamKind::Audio).count())
                    .unwrap_or(0) as f64;
                let container_bitrate = probe.and_then(|p| p.bit_rate).unwrap_or(5_000_000) as f64;
                let video_bitrate = (container_bitrate - audio_stream_count * 128_000.0).max(container_bitrate * 0.25);
                let bpp = (video_bitrate / fps.max(1.0)) / src_pixels;
                let bpp_dst = bpp * (src_pixels / dst_pixels);

                let delta: i32 = if bpp_dst < 0.05 {
                    6
                } else if bpp_dst < 0.15 {
                    3
                } else if bpp_dst < 0.40 {
                    0
                } else if bpp_dst < 1.0 {
                    -3
                } else if bpp_dst < 3.0 {
                    -5
                } else {
                    -7
                };
                (delta, bpp_dst)
            };

            let final_cq = (base_cq + delta_res + delta_fps + delta_depth + delta_bpp).clamp(8, cq_max) as u32;

            match rate_control_param(codec_id) {
                "cq" => {
                    extra_params.insert("cq".to_string(), serde_json::json!(final_cq));
                }
                "qp" => {
                    extra_params.insert("qp".to_string(), serde_json::json!(final_cq));
                }
                "global_quality" => {
                    extra_params.insert("global_quality".to_string(), serde_json::json!(final_cq));
                }
                "qp_i" => {
                    extra_params.insert("rc".to_string(), serde_json::json!("cqp"));
                    extra_params.insert("qp_i".to_string(), serde_json::json!(final_cq));
                    extra_params.insert("qp_p".to_string(), serde_json::json!(final_cq));
                }
                _ => {
                    extra_params.insert("crf".to_string(), serde_json::json!(final_cq));
                }
            }

            let cap_note = if opts.size_cap.is_some() {
                "; cap non applicabile (durata/dimensione sorgente sconosciuta)"
            } else {
                ""
            };

            let bpp_note = if opts.use_bpp_probe {
                format!("; bpp sorgente {:.2}", bpp_dst)
            } else {
                String::new()
            };

            ResolvedQuality {
                cq: Some(final_cq),
                bitrate_kbps: None,
                extra_params,
                summary: format!(
                    "AUTO (CQ {}; base {}, ris {:+}, fps {:+}, profondità {:+}, complessità {:+}{}{})",
                    final_cq, base_cq, delta_res, delta_fps, delta_depth, delta_bpp, bpp_note, cap_note
                ),
            }
        }
    }
}

/// Bitrate audio consigliato (kbps) in base al codec di destinazione e ai canali sorgente.
/// `None` per i codec dove non c'è nulla da calcolare (es. FLAC lossless o codec sconosciuti).
pub fn resolve_audio_bitrate(codec_id: &str, channels: u32) -> Option<u32> {
    let ch = channels.max(1);
    match codec_id {
        "libopus" => Some((ch * 64).clamp(64, 320)),
        "aac" => Some((ch * 64).clamp(64, 384)),
        "libmp3lame" => Some(if ch <= 1 { 96 } else { 192 }),
        _ => None,
    }
}

/// Estrae le dimensioni di destinazione dai filtri `scale` presenti sul flusso.
/// Ritorna `(larghezza, altezza)` dove un lato proporzionale (`-2` o `min(N,ih)`) è `None`.
pub fn dst_dims_from_filters(filters: &[String]) -> (Option<u32>, Option<u32>) {
    let scale = filters.iter().find(|f| f.starts_with("scale="));
    let Some(scale) = scale else {
        return (None, None);
    };

    let parse_side = |s: &str| -> Option<u32> {
        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        digits.parse::<u32>().ok()
    };

    let parts: Vec<&str> = scale["scale=".len()..].split(':').collect();
    let w = parts.first().and_then(|s| {
        if s.trim() == "-2" || s.trim() == "-1" {
            None
        } else {
            parse_side(s)
        }
    });
    let h = parts.get(1).and_then(|s| {
        if s.trim() == "-2" || s.trim() == "-1" {
            None
        } else {
            parse_side(s)
        }
    });
    (w, h)
}

/// Applica le modalità di qualità (video + audio) a un job già istanziato.
/// Usato sia dal pannello del file singolo che dal motore batch.
/// Ritorna le righe di riepilogo da mostrare nei log.
pub fn apply_quality_to_job(
    job: &mut Job,
    video_mode: &QualityMode,
    audio_mode: &AudioQualityMode,
    probe: Option<&crate::probe::ProbeResult>,
    src_file_size: Option<u64>,
) -> Vec<String> {
    let mut summaries = Vec::new();

    // --- Audio: bitrate automatico ---
    let mut audio_kbps: Option<u32> = None;
    let mut audio_copy = false;
    for stream in job.output.streams.iter_mut() {
        if stream.kind != crate::model::StreamKind::Audio {
            continue;
        }
        match &mut stream.codec {
            CodecConfig::Encode { id, params } => {
                let src_audio = probe.and_then(|p| {
                    p.streams
                        .iter()
                        .find(|s| s.kind == crate::model::StreamKind::Audio && s.index == stream.source_index)
                });
                let channels = src_audio.and_then(|s| s.channels).unwrap_or(2);

                match audio_mode {
                    AudioQualityMode::Auto => {
                        if let Some(kbps) = resolve_audio_bitrate(id, channels) {
                            params.insert("bitrate".to_string(), serde_json::json!(format!("{}k", kbps)));
                            summaries.push(format!(
                                "Audio → bitrate automatico {}k ({} canali, {})",
                                kbps, channels, id
                            ));
                            if audio_kbps.is_none() {
                                audio_kbps = Some(kbps);
                            }
                        }
                    }
                    AudioQualityMode::Manual => {
                        if audio_kbps.is_none() {
                            audio_kbps = params
                                .get("bitrate")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.trim_end_matches('k').parse::<u32>().ok());
                        }
                    }
                }
            }
            CodecConfig::Copy => audio_copy = true,
        }
    }
    if audio_kbps.is_none() && audio_copy {
        audio_kbps = Some(192);
    }

    // --- Video: qualità automatica / preset / fissa ---
    for stream in job.output.streams.iter_mut() {
        if stream.kind != crate::model::StreamKind::Video {
            continue;
        }
        let CodecConfig::Encode { id, params } = &mut stream.codec else {
            continue;
        };
        if *video_mode == QualityMode::Manual {
            continue;
        }

        let (dst_w, dst_h) = dst_dims_from_filters(&stream.filters);
        let resolved = resolve_quality(
            video_mode,
            probe,
            dst_w,
            dst_h,
            id,
            src_file_size,
            audio_kbps,
        );
        for (k, v) in resolved.extra_params {
            params.insert(k, v);
        }
        summaries.push(format!("Video ({}) → {}", id, resolved.summary));
    }

    summaries
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlanState {
    New,
    ExistsSkip,
    ExistsOverwrite,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CollisionPolicy {
    #[default]
    Skip,
    Rename,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub input: PathBuf,
    pub output: PathBuf,
    pub state: PlanState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub lines: Vec<String>,
    pub warnings: Vec<String>,
}

/// Standalone batch profile defining video, audio, and subtitle conversion rules
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchProfile {
    pub name: String,
    pub target_container: String,
    pub video_rule: Option<VideoRule>,
    pub audio_rule: AudioRule,
    pub subtitle_rule: SubtitleRule,
    pub copy_cover_art: bool,
    #[serde(default)]
    pub video_quality: QualityMode,
    #[serde(default)]
    pub audio_quality: AudioQualityMode,
}

impl BatchProfile {
    /// Compatibilità: `true` = prova NVENC. Preferisci `default_video_with`, che usa l'encoder
    /// effettivamente rilevato.
    pub fn default_video(gpu: bool) -> Self {
        Self::default_video_with(if gpu { Some("hevc_nvenc") } else { None })
    }

    /// Valori di partenza sensati per un encoder GPU (gli id esistono nei TOML dei codec).
    fn gpu_default_params(codec_id: &str) -> std::collections::HashMap<String, serde_json::Value> {
        let mut p = std::collections::HashMap::new();
        if codec_id.ends_with("_nvenc") {
            p.insert("rc".to_string(), serde_json::json!("vbr"));
            p.insert("cq".to_string(), serde_json::json!(24));
            p.insert("preset".to_string(), serde_json::json!("p5"));
        } else if codec_id.ends_with("_vaapi") {
            p.insert("qp".to_string(), serde_json::json!(24));
        } else if codec_id.ends_with("_qsv") {
            p.insert("global_quality".to_string(), serde_json::json!(24));
            p.insert("preset".to_string(), serde_json::json!("medium"));
        }
        // AMF e altri: valori predefiniti del TOML
        p
    }

    /// Profilo video standard. `gpu_codec` = encoder GPU REALMENTE disponibile (dal rilevamento), se c'è.
    pub fn default_video_with(gpu_codec: Option<&str>) -> Self {
        let (codec, params) = match gpu_codec {
            Some(id) => (id.to_string(), Self::gpu_default_params(id)),
            None => {
                let mut p = std::collections::HashMap::new();
                p.insert("crf".to_string(), serde_json::json!(23));
                p.insert("preset".to_string(), serde_json::json!("medium"));
                ("libx264".to_string(), p)
            }
        };

        Self {
            name: "Video Standard".to_string(),
            target_container: "mp4".to_string(),
            video_rule: Some(VideoRule {
                codec_id: codec,
                params,
                scale_preset: None,
                custom_width: None,
                custom_height: None,
            }),
            audio_rule: AudioRule::EncodeAll {
                codec_id: "aac".to_string(),
                params: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("bitrate".to_string(), serde_json::json!("128k"));
                    p
                },
            },
            subtitle_rule: SubtitleRule::CopyCompatible,
            copy_cover_art: true,
            video_quality: QualityMode::Manual,
            audio_quality: AudioQualityMode::Manual,
        }
    }

    pub fn default_audio() -> Self {
        let mut p = std::collections::HashMap::new();
        p.insert("bitrate".to_string(), serde_json::json!("128k"));
        Self {
            name: "Audio Opus".to_string(),
            target_container: "opus".to_string(),
            video_rule: None,
            audio_rule: AudioRule::EncodeAll {
                codec_id: "libopus".to_string(),
                params: p,
            },
            subtitle_rule: SubtitleRule::Ignore,
            copy_cover_art: false, // Opus muxer rejects cover art streams
            video_quality: QualityMode::Manual,
            audio_quality: AudioQualityMode::Manual,
        }
    }

    pub fn archive_video(gpu_codec: Option<&str>) -> Self {
        let (codec, params) = match gpu_codec {
            Some(id) => (id.to_string(), Self::gpu_default_params(id)),
            None => {
                let mut p = std::collections::HashMap::new();
                p.insert("crf".to_string(), serde_json::json!(24));
                p.insert("preset".to_string(), serde_json::json!("medium"));
                ("libx265".to_string(), p)
            }
        };

        Self {
            name: "Video Archivio (HEVC/MKV)".to_string(),
            target_container: "mkv".to_string(),
            video_rule: Some(VideoRule {
                codec_id: codec,
                params,
                scale_preset: None,
                custom_width: None,
                custom_height: None,
            }),
            audio_rule: AudioRule::EncodeAll {
                codec_id: "libopus".to_string(),
                params: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("bitrate".to_string(), serde_json::json!("128k"));
                    p
                },
            },
            subtitle_rule: SubtitleRule::CopyCompatible,
            copy_cover_art: true,
            video_quality: QualityMode::Manual,
            audio_quality: AudioQualityMode::Manual,
        }
    }

    pub fn compatible_video(gpu_codec: Option<&str>) -> Self {
        let (codec, params) = match gpu_codec {
            Some(id) => (id.to_string(), Self::gpu_default_params(id)),
            None => {
                let mut p = std::collections::HashMap::new();
                p.insert("crf".to_string(), serde_json::json!(21));
                p.insert("preset".to_string(), serde_json::json!("medium"));
                ("libx264".to_string(), p)
            }
        };

        Self {
            name: "Video Compatibile (H.264/MP4)".to_string(),
            target_container: "mp4".to_string(),
            video_rule: Some(VideoRule {
                codec_id: codec,
                params,
                scale_preset: None,
                custom_width: None,
                custom_height: None,
            }),
            audio_rule: AudioRule::EncodeAll {
                codec_id: "aac".to_string(),
                params: {
                    let mut p = std::collections::HashMap::new();
                    p.insert("bitrate".to_string(), serde_json::json!("128k"));
                    p
                },
            },
            subtitle_rule: SubtitleRule::CopyCompatible,
            copy_cover_art: true,
            video_quality: QualityMode::Manual,
            audio_quality: AudioQualityMode::Manual,
        }
    }

    pub fn default_image() -> Self {
        Self {
            name: "Immagine JPEG".to_string(),
            target_container: "jpg".to_string(),
            video_rule: None,
            audio_rule: AudioRule::CopyAll,
            subtitle_rule: SubtitleRule::Ignore,
            copy_cover_art: false,
            video_quality: QualityMode::Manual,
            audio_quality: AudioQualityMode::Manual,
        }
    }

    pub fn describe(&self, catalog: &Catalog) -> ProfileSummary {
        let mut lines = Vec::new();
        let mut warnings = Vec::new();

        let caps = crate::containers::container_caps(&self.target_container);

        if let Some(ref v) = self.video_rule {
            let codec_label = catalog.get_codec(&v.codec_id).map(|c| c.label.as_str()).unwrap_or(&v.codec_id);
            let hw_badge = if catalog.get_codec(&v.codec_id).map(|c| c.hw.is_some()).unwrap_or(false) {
                "⚡ GPU"
            } else {
                "🖥 CPU"
            };
            let scale = v.scale_preset.as_deref().unwrap_or("originale");
            lines.push(format!(
                "Video: {} ({}) · Risoluzione: {} · Contenitore: .{}",
                codec_label, hw_badge, scale, self.target_container
            ));

            if !caps.video {
                warnings.push(format!(
                    "Il contenitore .{} non supporta flussi video!",
                    self.target_container
                ));
            }
        } else {
            lines.push(format!(
                "Video: Nessuno (solo audio/altri flussi) · Contenitore: .{}",
                self.target_container
            ));
        }

        match &self.audio_rule {
            AudioRule::CopyAll => {
                lines.push("Audio: Copia traccia originale".to_string());
            }
            AudioRule::EncodeAll { codec_id, params } => {
                let codec_label = catalog.get_codec(codec_id).map(|c| c.label.as_str()).unwrap_or(codec_id);
                let br = params.get("bitrate").and_then(|v| v.as_str()).unwrap_or("predefinito");
                lines.push(format!("Audio: {} (bitrate {})", codec_label, br));
            }
            AudioRule::EncodeFirstCopyRest { codec_id, params } => {
                let codec_label = catalog.get_codec(codec_id).map(|c| c.label.as_str()).unwrap_or(codec_id);
                let br = params.get("bitrate").and_then(|v| v.as_str()).unwrap_or("predefinito");
                lines.push(format!("Audio: Primo flusso {} ({}), altri in copia", codec_label, br));
            }
        }

        match self.subtitle_rule {
            SubtitleRule::Ignore => {
                lines.push("Sottotitoli: Ignora".to_string());
            }
            SubtitleRule::CopyCompatible => {
                lines.push("Sottotitoli: Copia compatibili".to_string());
                if !caps.subtitles {
                    warnings.push(format!(
                        "Il contenitore .{} non supporta tracce di sottotitoli!",
                        self.target_container
                    ));
                }
            }
        }

        if self.video_quality != QualityMode::Manual {
            lines.push(format!("Qualità video: {}", self.video_quality.label()));
        }
        if self.audio_quality == AudioQualityMode::Auto {
            lines.push("Bitrate audio: Automatico (dai canali sorgente)".to_string());
        }

        ProfileSummary { lines, warnings }
    }

    pub fn to_job_template(&self) -> JobTemplate {
        JobTemplate {
            video_rule: self.video_rule.clone(),
            audio_rule: self.audio_rule.clone(),
            subtitle_rule: self.subtitle_rule.clone(),
            copy_cover_art: self.copy_cover_art,
            video_quality: self.video_quality.clone(),
            audio_quality: self.audio_quality,
        }
    }
}

pub fn format_batch_output_name(
    template: &str,
    stem: &str,
    codec_id: &str,
    quality_label: &str,
    res_label: &str,
    index: usize,
) -> String {
    let mut out = if template.is_empty() {
        "{nome}".to_string()
    } else {
        template.to_string()
    };

    out = out.replace("{nome}", stem);
    out = out.replace("{codec}", codec_id);
    out = out.replace("{qualita}", quality_label);
    out = out.replace("{risoluzione}", res_label);
    out = out.replace("{n}", &index.to_string());
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OnErrorPolicy {
    #[default]
    Continue,
    Stop,
    RetryOnce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSettings {
    pub folder: PathBuf,
    pub recursive: bool,
    pub active_exts: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestSettings {
    pub folder: Option<PathBuf>,
    pub subfolder_name: String,
    pub keep_tree: bool,
    pub naming_template: String,
    pub collision_policy: CollisionPolicy,
    pub copy_other_files: bool,
}

impl Default for DestSettings {
    fn default() -> Self {
        Self {
            folder: None,
            subfolder_name: "Convertiti".to_string(),
            keep_tree: true,
            naming_template: "{nome}".to_string(),
            collision_policy: CollisionPolicy::Rename,
            copy_other_files: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunOptions {
    pub parallel_jobs: usize,
    pub max_gpu_sessions: usize,
    pub on_error: OnErrorPolicy,
    pub generate_report: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            parallel_jobs: 2,
            max_gpu_sessions: 2,
            on_error: OnErrorPolicy::Continue,
            generate_report: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchProfileOverride {
    pub exclude: bool,
    pub profile: Option<BatchProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSettings {
    pub source: SourceSettings,
    pub dest: DestSettings,
    pub profiles: BTreeMap<MediaKind, BatchProfile>,
    pub overrides: HashMap<PathBuf, BatchProfileOverride>,
    pub run: RunOptions,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchPlanItem {
    pub input: PathBuf,
    pub output: PathBuf,
    pub media_kind: MediaKind,
    pub assigned_profile_name: String,
    pub state: PlanState,
    pub input_size_bytes: u64,
    pub estimated_output_bytes: u64,
    pub is_copy_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchPlan {
    pub items: Vec<BatchPlanItem>,
    pub total_input_bytes: u64,
    pub total_estimated_output_bytes: u64,
    pub available_dest_space_bytes: Option<u64>,
    pub video_count: usize,
    pub audio_count: usize,
    pub image_count: usize,
    pub other_count: usize,
    pub skipped_count: usize,
}

pub fn plan_batch(
    settings: &BatchSettings,
    _probe_cache: Option<&HashMap<PathBuf, crate::probe::ProbeResult>>,
) -> BatchPlan {
    let src = &settings.source.folder;
    let out_dest = settings.dest.folder.clone().unwrap_or_else(|| {
        src.join(&settings.dest.subfolder_name)
    });
    let out_dest_abs = out_dest.canonicalize().unwrap_or_else(|_| out_dest.clone());

    let mut used_output_paths = HashSet::new();
    let mut plan_items = Vec::new();

    let mut total_input_bytes: u64 = 0;
    let mut total_estimated_output_bytes: u64 = 0;
    let mut video_count = 0;
    let mut audio_count = 0;
    let mut image_count = 0;
    let mut other_count = 0;
    let mut skipped_count = 0;

    let walker = WalkDir::new(src)
        .max_depth(if settings.source.recursive { usize::MAX } else { 1 })
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if let Ok(canon) = e.path().canonicalize() {
                if canon == out_dest_abs || canon.starts_with(&out_dest_abs) {
                    return false;
                }
            } else if e.path() == out_dest {
                return false;
            }
            let file_name = e.file_name().to_string_lossy();
            if file_name.starts_with('.') && file_name.len() > 1 {
                return false;
            }
            true
        });

    let mut matching_paths = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let p = entry.path().to_path_buf();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();

        // Check active extension filter
        if !settings.source.active_exts.is_empty()
            && !settings.source.active_exts.iter().any(|allowed| allowed.eq_ignore_ascii_case(&ext))
        {
            continue;
        }

        matching_paths.push(p);
    }
    matching_paths.sort();

    for (idx, p) in matching_paths.into_iter().enumerate() {
        let rel = match p.strip_prefix(src) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let media_kind = detect_media_kind(&p);
        let input_size = p.metadata().map(|m| m.len()).unwrap_or(0);
        total_input_bytes += input_size;

        match media_kind {
            MediaKind::Video => video_count += 1,
            MediaKind::Audio => audio_count += 1,
            MediaKind::Image => image_count += 1,
            MediaKind::Other => other_count += 1,
        }

        let is_excluded = settings.overrides.get(&p).map(|o| o.exclude).unwrap_or(false);
        if is_excluded {
            skipped_count += 1;
            continue;
        }

        if media_kind == MediaKind::Other && !settings.dest.copy_other_files {
            skipped_count += 1;
            continue;
        }

        let assigned_profile = settings
            .overrides
            .get(&p)
            .and_then(|o| o.profile.clone())
            .or_else(|| settings.profiles.get(&media_kind).cloned());

        let target_ext = if let Some(ref prof) = assigned_profile {
            prof.target_container.clone()
        } else {
            p.extension().and_then(|e| e.to_str()).unwrap_or("").to_string()
        };

        let is_copy_only = media_kind == MediaKind::Other
            || assigned_profile
                .as_ref()
                .map(|pr| pr.video_rule.is_none() && matches!(pr.audio_rule, AudioRule::CopyAll))
                .unwrap_or(false);

        let parent_rel = if settings.dest.keep_tree {
            rel.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
        } else {
            PathBuf::new()
        };

        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
        let codec_id = assigned_profile
            .as_ref()
            .and_then(|pr| pr.video_rule.as_ref().map(|v| v.codec_id.clone()))
            .unwrap_or_else(|| "copy".to_string());

        let formatted_stem = format_batch_output_name(
            &settings.dest.naming_template,
            stem,
            &codec_id,
            "std",
            "orig",
            idx + 1,
        );

        let mut target_path = out_dest.join(parent_rel).join(format!("{}.{}", formatted_stem, target_ext));

        let is_same_as_input = paths_refer_to_same_file(&target_path, &p);
        let exists_on_disk = target_path.exists();
        let exists_in_plan = used_output_paths.contains(&target_path);

        let mut state = PlanState::New;
        let parent = target_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();

        if is_same_as_input {
            let mut counter = 2;
            let mut candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
            while candidate.exists() || used_output_paths.contains(&candidate) {
                counter += 1;
                candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
            }
            target_path = candidate;
            state = PlanState::Renamed;
        } else if exists_in_plan || exists_on_disk {
            match settings.dest.collision_policy {
                CollisionPolicy::Skip => {
                    state = PlanState::ExistsSkip;
                    skipped_count += 1;
                }
                CollisionPolicy::Overwrite => {
                    if exists_in_plan {
                        let mut counter = 2;
                        let mut candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
                        while candidate.exists() || used_output_paths.contains(&candidate) {
                            counter += 1;
                            candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
                        }
                        target_path = candidate;
                        state = PlanState::Renamed;
                    } else {
                        state = PlanState::ExistsOverwrite;
                    }
                }
                CollisionPolicy::Rename => {
                    let mut counter = 2;
                    let mut candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
                    while candidate.exists() || used_output_paths.contains(&candidate) {
                        counter += 1;
                        candidate = parent.join(format!("{} ({}).{}", formatted_stem, counter, target_ext));
                    }
                    target_path = candidate;
                    state = PlanState::Renamed;
                }
            }
        }

        used_output_paths.insert(target_path.clone());

        let video_quality = assigned_profile.as_ref().map(|p| &p.video_quality);
        let estimated_output = if state == PlanState::ExistsSkip {
            0
        } else if is_copy_only {
            input_size
        } else if let Some(QualityMode::Auto(opts)) = video_quality {
            match opts.size_cap {
                Some(SizeCap::Megabytes(mb)) => (mb * 1024.0 * 1024.0) as u64,
                Some(SizeCap::PercentReduction(pct)) => {
                    (input_size as f64 * (1.0 - pct.clamp(0.0, 99.0) / 100.0)) as u64
                }
                None => (input_size as f64 * 0.60) as u64,
            }
        } else {
            (input_size as f64 * 0.60) as u64
        };
        total_estimated_output_bytes += estimated_output;

        let profile_name = assigned_profile.map(|p| p.name).unwrap_or_else(|| "Copia diretta".to_string());

        plan_items.push(BatchPlanItem {
            input: p,
            output: target_path,
            media_kind,
            assigned_profile_name: profile_name,
            state,
            input_size_bytes: input_size,
            estimated_output_bytes: estimated_output,
            is_copy_only,
        });
    }

    let available_dest_space = get_available_space_bytes(&out_dest);

    BatchPlan {
        items: plan_items,
        total_input_bytes,
        total_estimated_output_bytes,
        available_dest_space_bytes: available_dest_space,
        video_count,
        audio_count,
        image_count,
        other_count,
        skipped_count,
    }
}

pub fn discover_extensions(src: &Path, recursive: bool) -> Vec<(String, usize)> {
    if !src.exists() || !src.is_dir() {
        return Vec::new();
    }
    let mut ext_counts: HashMap<String, usize> = HashMap::new();
    let walker = WalkDir::new(src)
        .max_depth(if recursive { usize::MAX } else { 1 })
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') || name.len() <= 1
        });

    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                let ext_lower = ext.to_lowercase();
                *ext_counts.entry(ext_lower).or_insert(0) += 1;
            }
        }
    }

    let mut list: Vec<(String, usize)> = ext_counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list
}

pub fn scan_and_plan_batch(settings: &BatchSettings) -> (Vec<(String, usize)>, BatchPlan) {
    let exts = discover_extensions(&settings.source.folder, settings.source.recursive);
    let plan = plan_batch(settings, None);
    (exts, plan)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReportItem {
    pub input: PathBuf,
    pub output: PathBuf,
    pub profile: String,
    pub duration_s: Option<f64>,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReport {
    pub timestamp: String,
    pub total_files: usize,
    pub succeeded_files: usize,
    pub failed_files: usize,
    pub items: Vec<BatchReportItem>,
}

impl BatchReport {
    pub fn write_json(&self, path: &Path) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self).map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, content)
    }

    pub fn write_csv(&self, path: &Path) -> Result<(), std::io::Error> {
        use std::io::Write;
        let mut file = std::fs::File::create(path)?;
        writeln!(file, "input,output,profile,duration_s,input_bytes,output_bytes,exit_code,error")?;
        for item in &self.items {
            writeln!(
                file,
                "\"{}\",\"{}\",\"{}\",{},{},{},{},\"{}\"",
                item.input.display(),
                item.output.display(),
                item.profile,
                item.duration_s.unwrap_or(0.0),
                item.input_bytes,
                item.output_bytes,
                item.exit_code.unwrap_or(-1),
                item.error.as_deref().unwrap_or(""),
            )?;
        }
        Ok(())
    }
}

pub fn is_audio_ext(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "mp3" | "flac" | "wav" | "opus" | "m4a" | "ogg" | "aac" | "wma" | "aiff" | "alac"
    )
}

pub fn is_video_ext(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "mkv" | "mp4" | "mov" | "avi" | "webm" | "m4v" | "ts" | "m2ts" | "wmv" | "flv" | "vob" | "ogv"
    )
}

/// Plan folder conversion from source directory to target destination.
/// Respects subdirectories, extension filtering, deterministic file sorting, and collision resolution.
pub fn plan_folder(
    src: &Path,
    out_dest: &Path,
    exts: &[&str],
    new_ext: &str,
    keep_tree: bool,
    collision_policy: CollisionPolicy,
) -> Vec<PlanItem> {
    plan_folder_smart(src, out_dest, exts, new_ext, None, keep_tree, collision_policy)
}

/// Plan folder conversion with optional distinct extensions for video and audio files (Smart mode)
pub fn plan_folder_smart(
    src: &Path,
    out_dest: &Path,
    exts: &[&str],
    video_ext: &str,
    audio_ext: Option<&str>,
    keep_tree: bool,
    collision_policy: CollisionPolicy,
) -> Vec<PlanItem> {
    let mut items = Vec::new();
    let mut used_output_paths: HashSet<PathBuf> = HashSet::new();

    // Canonicalize out_dest if possible to reliably filter it out
    let out_dest_abs = out_dest.canonicalize().unwrap_or_else(|_| out_dest.to_path_buf());

    let walker = WalkDir::new(src)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }

            // Exclude output folder if located inside source
            if let Ok(canon) = e.path().canonicalize() {
                if canon == out_dest_abs || canon.starts_with(&out_dest_abs) {
                    return false;
                }
            } else if e.path() == out_dest {
                return false;
            }

            // Exclude hidden files / directories
            let file_name = e.file_name().to_string_lossy();
            if file_name.starts_with('.') && file_name.len() > 1 {
                return false;
            }

            true
        });

    // Collect and sort matching files deterministically by path (B5)
    let mut matching_paths: Vec<PathBuf> = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }

        let p = entry.path();
        let matches_ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| exts.iter().any(|allowed| allowed.eq_ignore_ascii_case(ext)))
            .unwrap_or(false);

        if matches_ext {
            matching_paths.push(p.to_path_buf());
        }
    }
    matching_paths.sort();

    for p in matching_paths {
        let rel = match p.strip_prefix(src) {
            Ok(r) => r,
            Err(_) => continue,
        };

        let target_rel = if keep_tree {
            rel.to_path_buf()
        } else {
            PathBuf::from(rel.file_name().unwrap_or(rel.as_os_str()))
        };

        let file_ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        let target_ext = if let Some(a_ext) = audio_ext {
            if is_audio_ext(file_ext) {
                a_ext
            } else {
                video_ext
            }
        } else {
            video_ext
        };

        let mut target_path = out_dest.join(target_rel);
        target_path.set_extension(target_ext);

        let parent = target_path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        let stem = target_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();

        let mut state = PlanState::New;

        // Check if destination is the same file as input, or exists on disk, or was planned
        let is_same_as_input = paths_refer_to_same_file(&target_path, &p);
        let exists_on_disk = target_path.exists();
        let exists_in_plan = used_output_paths.contains(&target_path);

        if is_same_as_input {
            // NEVER overwrite the input file itself, even under Overwrite policy!
            let mut counter = 2;
            let mut candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
            while candidate.exists() || used_output_paths.contains(&candidate) {
                counter += 1;
                candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
            }
            target_path = candidate;
            state = PlanState::Renamed;
        } else if exists_in_plan || exists_on_disk {
            match collision_policy {
                CollisionPolicy::Skip => {
                    state = PlanState::ExistsSkip;
                }
                CollisionPolicy::Overwrite => {
                    if exists_in_plan {
                        // Internal plan collision: two inputs targeting same output file! (B6)
                        // Even with Overwrite, two files in the same batch must never target the same file.
                        let mut counter = 2;
                        let mut candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
                        while candidate.exists() || used_output_paths.contains(&candidate) {
                            counter += 1;
                            candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
                        }
                        target_path = candidate;
                        state = PlanState::Renamed;
                    } else {
                        state = PlanState::ExistsOverwrite;
                    }
                }
                CollisionPolicy::Rename => {
                    let mut counter = 2;
                    let mut candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
                    while candidate.exists() || used_output_paths.contains(&candidate) {
                        counter += 1;
                        candidate = parent.join(format!("{} ({}).{}", stem, counter, target_ext));
                    }
                    target_path = candidate;
                    state = PlanState::Renamed;
                }
            }
        }

        used_output_paths.insert(target_path.clone());
        items.push(PlanItem {
            input: p,
            output: target_path,
            state,
        });
    }

    items
}

/// Estimate free space in bytes at the given path, traversing to the first existing ancestor (B4).
#[cfg(unix)]
pub fn get_available_space_bytes(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let mut current = path.to_path_buf();
    while !current.exists() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    let c_path = CString::new(current.to_string_lossy().as_bytes()).ok()?;
    let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();

    unsafe {
        if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
            let stat = stat.assume_init();
            #[allow(clippy::unnecessary_cast)]
            let free_blocks = stat.f_bavail as u64;
            #[allow(clippy::unnecessary_cast)]
            let block_size = stat.f_frsize as u64;
            Some(free_blocks * block_size)
        } else {
            None
        }
    }
}

#[cfg(windows)]
pub fn get_available_space_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    let mut current = path.to_path_buf();
    while !current.exists() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    let mut wide: Vec<u16> = current.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    extern "system" {
        fn GetDiskFreeSpaceExW(
            lpDirectoryName: *const u16,
            lpFreeBytesAvailableToCaller: *mut u64,
            lpTotalNumberOfBytes: *mut u64,
            lpTotalNumberOfFreeBytes: *mut u64,
        ) -> i32;
    }

    let ret = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        )
    };

    if ret != 0 {
        Some(free_bytes_available)
    } else {
        None
    }
}

#[cfg(not(any(unix, windows)))]
pub fn get_available_space_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_plan_folder_tree_and_collisions() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path();

        // Create sample files
        let sub = src.join("subdir");
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(src.join("clip1.mkv"), b"123").unwrap();
        std::fs::write(src.join("clip1.mp4"), b"456").unwrap(); // Name collision for mp4 conversion
        std::fs::write(sub.join("clip2.mov"), b"789").unwrap();
        std::fs::write(src.join(".hidden.mkv"), b"hidden").unwrap(); // Hidden file

        let out_dir = src.join("convertiti");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(out_dir.join("already.mp4"), b"000").unwrap(); // Output dir content

        let exts = ["mkv", "mp4", "mov"];
        let plan = plan_folder(src, &out_dir, &exts, "mp4", true, CollisionPolicy::Rename);

        // Hidden file and out_dir contents must be excluded
        assert_eq!(plan.len(), 3);

        // Check that collision clip1.mkv vs clip1.mp4 was resolved with (2)
        let outputs: Vec<String> = plan
            .iter()
            .map(|p| p.output.file_name().unwrap().to_str().unwrap().to_string())
            .collect();

        assert!(outputs.contains(&"clip1.mp4".to_string()));
        assert!(outputs.contains(&"clip1 (2).mp4".to_string()));
        assert!(outputs.contains(&"clip2.mp4".to_string()));

        // Check relative folder preserved
        let clip2_item = plan.iter().find(|p| p.input.ends_with("clip2.mov")).unwrap();
        assert!(clip2_item.output.ends_with("subdir/clip2.mp4"));
    }

    #[test]
    fn test_plan_folder_smart_audio_and_video() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path();

        std::fs::write(src.join("film.mkv"), b"video").unwrap();
        std::fs::write(src.join("song.flac"), b"audio").unwrap();

        let out_dir = src.join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        let exts = ["mkv", "flac"];
        let plan = plan_folder_smart(
            src,
            &out_dir,
            &exts,
            "mp4",
            Some("opus"),
            false,
            CollisionPolicy::Skip,
        );

        assert_eq!(plan.len(), 2);
        let film = plan.iter().find(|p| p.input.ends_with("film.mkv")).unwrap();
        let song = plan.iter().find(|p| p.input.ends_with("song.flac")).unwrap();

        assert_eq!(film.output.extension().unwrap().to_str().unwrap(), "mp4");
        assert_eq!(song.output.extension().unwrap().to_str().unwrap(), "opus");
    }
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::catalog::Catalog;
    use std::collections::HashMap;

    fn check_profile(profile: &BatchProfile, catalog: &Catalog) {
        let mut rules: Vec<(&str, &HashMap<String, serde_json::Value>)> = Vec::new();
        if let Some(v) = &profile.video_rule {
            rules.push((v.codec_id.as_str(), &v.params));
        }
        if let AudioRule::EncodeAll { codec_id, params } = &profile.audio_rule {
            rules.push((codec_id.as_str(), params));
        }
        for (codec_id, params) in rules {
            let def = catalog
                .get_codec(codec_id)
                .unwrap_or_else(|| panic!("codec '{}' assente dal catalogo", codec_id));
            for (key, value) in params {
                let p = def
                    .params
                    .iter()
                    .find(|p| &p.id == key)
                    .unwrap_or_else(|| panic!("{}: il parametro '{}' non esiste nel TOML", codec_id, key));
                if let (Some(choices), Some(s)) = (&p.choices, value.as_str()) {
                    assert!(
                        choices.iter().any(|c| c == s),
                        "{}: valore '{}' non valido per '{}' (ammessi: {:?})",
                        codec_id, s, key, choices
                    );
                }
            }
        }
    }

    #[test]
    fn i_profili_predefiniti_usano_parametri_che_esistono() {
        let catalog = Catalog::load_default(false);
        check_profile(&BatchProfile::default_audio(), &catalog);
        check_profile(&BatchProfile::default_video_with(None), &catalog);
        for gpu in ["hevc_nvenc", "hevc_vaapi", "hevc_qsv", "hevc_amf"] {
            check_profile(&BatchProfile::default_video_with(Some(gpu)), &catalog);
        }
    }

    #[test]
    fn test_resolve_quality_tiers() {
        let q_med = resolve_quality(&QualityMode::Preset(QualityTier::Medium), None, None, None, "libx264", None, None);
        assert_eq!(q_med.cq, Some(22));

        let q_high = resolve_quality(&QualityMode::Preset(QualityTier::High), None, None, None, "libx264", None, None);
        assert_eq!(q_high.cq, Some(18));

        let q_low = resolve_quality(&QualityMode::Preset(QualityTier::Low), None, None, None, "libx264", None, None);
        assert_eq!(q_low.cq, Some(26));
    }

    #[test]
    fn test_resolve_quality_auto_size_cap_nvenc_uses_b_v_and_cbr() {
        use crate::probe::ProbeResult;
        let probe = ProbeResult {
            file: PathBuf::from("video.mp4"),
            duration_s: Some(60.0),
            format_name: Some("mov,mp4".to_string()),
            bit_rate: Some(10_000_000),
            streams: vec![],
        };
        let auto_opts = AutoOptions {
            size_cap: Some(SizeCap::Megabytes(50.0)),
            use_bpp_probe: false,
        };
        let res = resolve_quality(
            &QualityMode::Auto(auto_opts),
            Some(&probe),
            None,
            None,
            "hevc_nvenc",
            None,
            Some(128),
        );

        assert!(res.bitrate_kbps.is_some());
        assert_eq!(res.extra_params.get("b_v"), Some(&serde_json::json!(format!("{}k", res.bitrate_kbps.unwrap()))));
        assert_eq!(res.extra_params.get("rc"), Some(&serde_json::json!("cbr")));
        assert_eq!(res.extra_params.get("cq"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn test_resolve_quality_auto_amf_uses_cqp() {
        let res = resolve_quality(&QualityMode::Auto(AutoOptions::default()), None, None, None, "hevc_amf", None, None);
        assert_eq!(res.extra_params.get("rc"), Some(&serde_json::json!("cqp")));
        assert_eq!(res.extra_params.get("qp_i"), res.extra_params.get("qp_p"));
        assert_eq!(res.cq, res.extra_params.get("qp_i").and_then(|v| v.as_u64()).map(|v| v as u32));
    }

    #[test]
    fn test_resolve_quality_auto_percent_cap_requires_source_size() {
        use crate::probe::ProbeResult;
        let probe = ProbeResult {
            file: PathBuf::from("video.mp4"),
            duration_s: Some(60.0),
            format_name: None,
            bit_rate: None,
            streams: vec![],
        };
        let auto_opts = AutoOptions {
            size_cap: Some(SizeCap::PercentReduction(30.0)),
            use_bpp_probe: false,
        };

        // Senza dimensione sorgente il cap non è applicabile: fallback su CQ, non su bitrate minimo
        let res = resolve_quality(
            &QualityMode::Auto(auto_opts.clone()),
            Some(&probe),
            None,
            None,
            "libx264",
            None,
            None,
        );
        assert!(res.bitrate_kbps.is_none());
        assert!(res.summary.contains("cap non applicabile"));

        // Con la dimensione sorgente il cap produce un bitrate coerente
        let res = resolve_quality(
            &QualityMode::Auto(auto_opts),
            Some(&probe),
            None,
            None,
            "libx264",
            Some(100 * 1024 * 1024),
            None,
        );
        let kbps = res.bitrate_kbps.expect("cap percentuale applicato");
        // 70 MB * 8 / 60 s ≈ 9787 kbps (senza audio)
        assert!(kbps > 9000 && kbps < 10500, "kbps = {}", kbps);
        assert_eq!(res.extra_params.get("b_v"), Some(&serde_json::json!(format!("{}k", kbps))));
        assert_eq!(res.extra_params.get("crf"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn test_resolve_quality_auto_bpp_probe_respected() {
        use crate::probe::ProbeResult;
        let probe = ProbeResult {
            file: PathBuf::from("video.mp4"),
            duration_s: Some(60.0),
            format_name: None,
            bit_rate: Some(40_000_000),
            streams: vec![],
        };
        let with_bpp = resolve_quality(
            &QualityMode::Auto(AutoOptions { size_cap: None, use_bpp_probe: true }),
            Some(&probe),
            None,
            None,
            "libx264",
            None,
            None,
        );
        let without_bpp = resolve_quality(
            &QualityMode::Auto(AutoOptions { size_cap: None, use_bpp_probe: false }),
            Some(&probe),
            None,
            None,
            "libx264",
            None,
            None,
        );

        // Sorgente molto complessa (40 Mbps @ 1080p): il bpp probe abbassa il CQ
        assert!(with_bpp.cq.unwrap() < without_bpp.cq.unwrap());
        assert_eq!(without_bpp.cq.unwrap(), 20);
    }

    #[test]
    fn test_resolve_quality_manual_is_noop() {
        let res = resolve_quality(&QualityMode::Manual, None, None, None, "libx264", None, None);
        assert!(res.extra_params.is_empty());
    }

    #[test]
    fn test_apply_quality_to_job_end_to_end() {
        use crate::model::{CodecConfig, Job, OutStream, StreamKind};
        use crate::probe::{MediaStream, ProbeResult};
        use std::collections::HashMap;

        let probe = ProbeResult {
            file: PathBuf::from("in.mkv"),
            duration_s: Some(60.0),
            format_name: None,
            bit_rate: Some(8_000_000),
            streams: vec![
                MediaStream {
                    index: 0,
                    kind: StreamKind::Video,
                    width: Some(3840),
                    height: Some(2160),
                    fps: Some(60.0),
                    pix_fmt: Some("yuv420p10le".to_string()),
                    ..Default::default()
                },
                MediaStream {
                    index: 1,
                    kind: StreamKind::Audio,
                    channels: Some(6),
                    ..Default::default()
                },
            ],
        };

        let mut job = Job::new("in.mkv", "out.mkv");
        job.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "libx265",
            HashMap::new(),
        ));
        job.output
            .streams
            .push(OutStream::new_encode(StreamKind::Audio, 1, "aac", HashMap::new()));

        let summaries = apply_quality_to_job(
            &mut job,
            &QualityMode::Auto(AutoOptions { size_cap: None, use_bpp_probe: true }),
            &AudioQualityMode::Auto,
            Some(&probe),
            None,
        );

        assert_eq!(summaries.len(), 2);
        let v = &job.output.streams[0];
        let CodecConfig::Encode { params, .. } = &v.codec else {
            panic!("video encode atteso");
        };
        // 4K 60fps 10bit: base 24, ris -4, fps -2, depth -2, bpp +6 -> CQ 22
        assert_eq!(params.get("crf"), Some(&serde_json::json!(22)));

        let a = &job.output.streams[1];
        let CodecConfig::Encode { id, params } = &a.codec else {
            panic!("audio encode atteso");
        };
        assert_eq!(id, "aac");
        assert_eq!(params.get("bitrate"), Some(&serde_json::json!("384k")));
    }

    #[test]
    fn test_dst_dims_from_filters() {
        assert_eq!(dst_dims_from_filters(&["scale=-2:'min(1080,ih)'".to_string()]), (None, Some(1080)));
        assert_eq!(dst_dims_from_filters(&["scale=-2:720".to_string()]), (None, Some(720)));
        assert_eq!(dst_dims_from_filters(&["scale=1920:1080".to_string()]), (Some(1920), Some(1080)));
        assert_eq!(dst_dims_from_filters(&[]), (None, None));
    }

    #[test]
    fn test_resolve_quality_auto_size_cap() {
        use crate::probe::ProbeResult;
        let probe = ProbeResult {
            file: PathBuf::from("video.mp4"),
            duration_s: Some(60.0), // 60 seconds
            format_name: Some("mov,mp4".to_string()),
            bit_rate: Some(10_000_000),
            streams: vec![],
        };

        // 50 MB target on a 60s file:
        let auto_opts = AutoOptions {
            size_cap: Some(SizeCap::Megabytes(50.0)),
            use_bpp_probe: false,
        };
        let res = resolve_quality(
            &QualityMode::Auto(auto_opts),
            Some(&probe),
            Some(1920),
            Some(1080),
            "libx264",
            Some(100 * 1024 * 1024),
            None,
        );

        assert!(res.bitrate_kbps.is_some());
        let kbps = res.bitrate_kbps.unwrap();
        // 50MB * 8 * 1024 * 1024 = 419,430,400 bits. minus audio (192,000 * 60 = 11,520,000) = 407,910,400 bits.
        // / 60 / 1000 = ~6798 kbps.
        assert!(kbps > 6000 && kbps < 7500);
        assert!(res.summary.contains("AUTO Bitrate"));
    }

    #[test]
    fn test_plan_batch_naming_and_tree() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let sub = src.join("season1");
        std::fs::create_dir_all(&sub).unwrap();

        std::fs::write(sub.join("ep01.mkv"), b"video1").unwrap();
        std::fs::write(sub.join("ep02.mkv"), b"video2").unwrap();
        std::fs::write(src.join("track.flac"), b"audio").unwrap();
        std::fs::write(src.join("cover.jpg"), b"image").unwrap();

        let out_dir = temp.path().join("dest");

        let mut profiles = BTreeMap::new();
        profiles.insert(MediaKind::Video, BatchProfile::archive_video(None));
        profiles.insert(MediaKind::Audio, BatchProfile::default_audio());

        let settings = BatchSettings {
            source: SourceSettings {
                folder: src.clone(),
                recursive: true,
                active_exts: HashSet::new(),
            },
            dest: DestSettings {
                folder: Some(out_dir.clone()),
                subfolder_name: "Convertiti".to_string(),
                keep_tree: true,
                naming_template: "{nome}_{codec}".to_string(),
                collision_policy: CollisionPolicy::Rename,
                copy_other_files: true,
            },
            profiles,
            overrides: HashMap::new(),
            run: RunOptions::default(),
        };

        let plan = plan_batch(&settings, None);
        assert_eq!(plan.video_count, 2);
        assert_eq!(plan.audio_count, 1);
        assert_eq!(plan.image_count, 1);

        let ep01 = plan.items.iter().find(|i| i.input.ends_with("ep01.mkv")).unwrap();
        assert_eq!(ep01.output.parent().unwrap().file_name().unwrap(), "season1");
        assert_eq!(ep01.output.file_name().unwrap(), "ep01_libx265.mkv");
    }

    #[test]
    fn test_batch_report_serialization() {
        let temp = tempfile::tempdir().unwrap();
        let json_path = temp.path().join("report.json");
        let csv_path = temp.path().join("report.csv");

        let report = BatchReport {
            timestamp: "2026-11-09T12:00:00Z".to_string(),
            total_files: 1,
            succeeded_files: 1,
            failed_files: 0,
            items: vec![
                BatchReportItem {
                    input: PathBuf::from("/in/video.mkv"),
                    output: PathBuf::from("/out/video.mp4"),
                    profile: "Video Standard".to_string(),
                    duration_s: Some(12.5),
                    input_bytes: 1000,
                    output_bytes: 600,
                    exit_code: Some(0),
                    error: None,
                }
            ],
        };

        report.write_json(&json_path).expect("write_json");
        report.write_csv(&csv_path).expect("write_csv");

        assert!(json_path.exists());
        assert!(csv_path.exists());
        let csv_content = std::fs::read_to_string(&csv_path).unwrap();
        assert!(csv_content.contains("/in/video.mkv"));
        assert!(csv_content.contains("Video Standard"));
    }
}
