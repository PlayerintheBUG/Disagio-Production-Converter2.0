use crate::catalog::{values_equal, Catalog};
use crate::model::{CodecConfig, Job, StreamKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    pub stream_index: Option<usize>,
    pub param_id: Option<String>,
    pub message: String,
}

impl ValidationIssue {
    pub fn error(stream_index: Option<usize>, param_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Error,
            stream_index,
            param_id,
            message: message.into(),
        }
    }

    pub fn warning(stream_index: Option<usize>, param_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            stream_index,
            param_id,
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == IssueSeverity::Error
    }
}

pub fn paths_refer_to_same_file(p1: &Path, p2: &Path) -> bool {
    if p1 == p2 {
        return true;
    }

    fn normalize_path(p: &Path) -> Option<PathBuf> {
        if let Ok(c) = p.canonicalize() {
            return Some(c);
        }
        if let Some(parent) = p.parent() {
            let parent_to_check = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            if let Ok(parent_canon) = parent_to_check.canonicalize() {
                if let Some(name) = p.file_name() {
                    return Some(parent_canon.join(name));
                }
            }
        }
        None
    }

    let n1 = normalize_path(p1);
    let n2 = normalize_path(p2);

    if let (Some(c1), Some(c2)) = (n1, n2) {
        if c1 == c2 {
            return true;
        }
        if cfg!(windows) || cfg!(target_os = "macos") {
            let s1 = c1.to_string_lossy().to_lowercase();
            let s2 = c2.to_string_lossy().to_lowercase();
            return s1 == s2;
        }
    }

    false
}

pub fn validate_job(job: &Job, catalog: &Catalog) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // 1. Output file equals input file (using canonical resolution)
    if !job.input.path.as_os_str().is_empty()
        && !job.output.path.as_os_str().is_empty()
        && paths_refer_to_same_file(&job.input.path, &job.output.path)
    {
        issues.push(ValidationIssue::error(
            None,
            None,
            "Il file di output non può coincidere con il file di input",
        ));
    }

    // Input file existence and readability
    if !job.input.path.as_os_str().is_empty() {
        if job.input.path.exists() {
            if std::fs::File::open(&job.input.path).is_err() {
                issues.push(ValidationIssue::error(
                    None,
                    None,
                    format!("Impossibile aprire il file di input in lettura: {:?}", job.input.path),
                ));
            }
        } else {
            issues.push(ValidationIssue::warning(
                None,
                None,
                format!("Il file di input non è stato trovato sul disco: {:?}", job.input.path),
            ));
        }
    }

    // Input trim bounds
    if let Some(ss) = job.input.start_time {
        if ss < 0.0 || ss.is_nan() {
            issues.push(ValidationIssue::error(
                None,
                None,
                "Il tempo di inizio (-ss) non può essere negativo o non valido",
            ));
        }
    }
    if let Some(t) = job.input.duration {
        if t <= 0.0 || t.is_nan() {
            issues.push(ValidationIssue::error(
                None,
                None,
                "La durata (-t) deve essere un valore strettamente positivo",
            ));
        }
    }

    // Free disk space check in output directory
    if let Some(parent) = job.output.path.parent() {
        let dir_to_check = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        if dir_to_check.exists() {
            if let Some(free_bytes) = crate::batch::get_available_space_bytes(dir_to_check) {
                if free_bytes < 50 * 1024 * 1024 {
                    issues.push(ValidationIssue::warning(
                        None,
                        None,
                        format!(
                            "Spazio libero su disco molto ridotto ({:.1} MB) nella cartella di destinazione",
                            free_bytes as f64 / (1024.0 * 1024.0)
                        ),
                    ));
                }
            }
        }
    }

    // 2. Output must have at least one stream
    if job.output.streams.is_empty() {
        issues.push(ValidationIssue::error(
            None,
            None,
            "Nessuno stream selezionato per l'output",
        ));
    }

    // Output container extension
    let out_ext = job
        .output
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
        .or_else(|| job.output.format.clone().map(|s| s.to_lowercase()));

    // Controlli sull'insieme degli stream rispetto al contenitore
    if let Some(ref ext) = out_ext {
        let caps = crate::containers::container_caps(ext);
        let audio_count = job.output.streams.iter().filter(|s| s.kind == StreamKind::Audio).count();
        if audio_count > 1 && !caps.multi_audio {
            issues.push(ValidationIssue::error(
                None,
                None,
                format!(
                    "Il contenitore .{} accetta una sola traccia audio, ma ne hai selezionate {}",
                    ext, audio_count
                ),
            ));
        }
        for (i, s) in job.output.streams.iter().enumerate() {
            if s.kind == StreamKind::Attachment && !caps.attachments {
                issues.push(ValidationIssue::error(
                    Some(i),
                    None,
                    format!("Il contenitore .{} non supporta allegati (per esempio i font dei sottotitoli)", ext),
                ));
            }
        }
    }

    // 3. Validate each output stream
    for (i, stream) in job.output.streams.iter().enumerate() {
        if let Some(ref ext) = out_ext {
            let caps = crate::containers::container_caps(ext);
            if stream.kind == StreamKind::CoverArt && !caps.cover_art {
                issues.push(ValidationIssue::error(
                    Some(i),
                    None,
                    format!("Il contenitore .{} non supporta copertine/cover art (lo stream video verrebbe rifiutato dal muxer)", ext),
                ));
            } else if stream.kind == StreamKind::Subtitle && !caps.subtitles {
                issues.push(ValidationIssue::error(
                    Some(i),
                    None,
                    format!("Il contenitore .{} non supporta tracce di sottotitoli", ext),
                ));
            } else if stream.kind == StreamKind::Video && !caps.video {
                issues.push(ValidationIssue::error(
                    Some(i),
                    None,
                    format!("Il contenitore .{} è un formato solo audio e non supporta flussi video", ext),
                ));
            }
        }

        match &stream.codec {
            CodecConfig::Copy => {
                // Copy together with filters is invalid
                if !stream.filters.is_empty() {
                    issues.push(ValidationIssue::error(
                        Some(i),
                        None,
                        "Impossibile applicare filtri quando lo stream è in modalità Copia (Copy)",
                    ));
                }
            }
            CodecConfig::Encode { id, params } => {
                if let Some(codec_def) = catalog.get_codec(id) {
                    // Check container compatibility
                    if let Some(ref ext) = out_ext {
                        if !codec_def.containers.iter().any(|c| c.eq_ignore_ascii_case(ext)) {
                            issues.push(ValidationIssue::error(
                                Some(i),
                                None,
                                format!(
                                    "Il codec '{}' non è compatibile con il contenitore .{} (supportati: {})",
                                    id,
                                    ext,
                                    codec_def.containers.join(", ")
                                ),
                            ));
                        }
                    }

                    // Check params in deterministic sorted order
                    let mut sorted_params: Vec<_> = params.iter().collect();
                    sorted_params.sort_by_key(|(k, _)| (*k).clone());

                    for (param_name, param_val) in sorted_params {
                        if let Some(param_def) = codec_def.get_param(param_name) {
                            if !param_def.is_active(params, &codec_def.params) {
                                continue;
                            }

                            // Check numerical bounds
                            if let Some(num) = param_val.as_f64() {
                                if let Some(min) = param_def.min {
                                    if num < min {
                                        issues.push(ValidationIssue::error(
                                            Some(i),
                                            Some(param_name.clone()),
                                            format!(
                                                "Il valore {} per '{}' è inferiore al minimo consentito ({})",
                                                num, param_def.label, min
                                            ),
                                        ));
                                    }
                                }
                                if let Some(max) = param_def.max {
                                    if num > max {
                                        issues.push(ValidationIssue::error(
                                            Some(i),
                                            Some(param_name.clone()),
                                            format!(
                                                "Il valore {} per '{}' è superiore al massimo consentito ({})",
                                                num, param_def.label, max
                                            ),
                                        ));
                                    }
                                }
                            }

                            // Check choice
                            if let Some(s) = param_val.as_str() {
                                if let Some(ref choices) = param_def.choices {
                                    if !choices.iter().any(|c| c == s) {
                                        issues.push(ValidationIssue::error(
                                            Some(i),
                                            Some(param_name.clone()),
                                            format!(
                                                "Valore '{}' non valido per '{}'. Scelte possibili: {}",
                                                s,
                                                param_def.label,
                                                choices.join(", ")
                                            ),
                                        ));
                                    }
                                }
                            }
                        } else {
                            issues.push(ValidationIssue::warning(
                                Some(i),
                                Some(param_name.clone()),
                                format!("Parametro '{}' sconosciuto per il codec '{}'", param_name, id),
                            ));
                        }
                    }

                    // Check custom codec rules (evaluates both explicit and default values)
                    for rule in &codec_def.rules {
                        let effective_val = params
                            .get(&rule.when.param)
                            .or_else(|| {
                                codec_def
                                    .get_param(&rule.when.param)
                                    .and_then(|p| p.default.as_ref())
                            });

                        if let Some(val) = effective_val {
                            let matches = rule.when.in_values.iter().any(|inv| values_equal(inv, val));
                            if matches {
                                // Check if 'unless' condition suppresses this rule
                                let is_exempt = if let Some(ref unless_cond) = rule.unless {
                                    let unless_val = params
                                        .get(&unless_cond.param)
                                        .or_else(|| {
                                            codec_def
                                                .get_param(&unless_cond.param)
                                                .and_then(|p| p.default.as_ref())
                                        });
                                    if let Some(u_val) = unless_val {
                                        unless_cond.in_values.iter().any(|inv| values_equal(inv, u_val))
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };

                                if !is_exempt {
                                    if let Some(ref err_msg) = rule.then.error {
                                        issues.push(ValidationIssue::error(
                                            Some(i),
                                            Some(rule.when.param.clone()),
                                            err_msg.clone(),
                                        ));
                                    }
                                    if let Some(ref warn_msg) = rule.then.warn {
                                        issues.push(ValidationIssue::warning(
                                            Some(i),
                                            Some(rule.when.param.clone()),
                                            warn_msg.clone(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                } else {
                    issues.push(ValidationIssue::warning(
                        Some(i),
                        None,
                        format!("Codec '{}' non trovato nel catalogo", id),
                    ));
                }
            }
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CodecDef;
    use crate::model::{OutStream, StreamKind};
    use std::path::PathBuf;

    #[test]
    fn test_validation_input_equals_output() {
        let mut catalog = Catalog::new();
        let toml_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codecs/libx265.toml");
        let codec = CodecDef::from_file(&toml_path).unwrap();
        catalog.add_or_replace(codec);

        let mut job = Job::new("video.mp4", "video.mp4");
        job.output.streams.push(OutStream::new_copy(StreamKind::Video, 0));

        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("non può coincidere")));
    }

    #[test]
    fn test_validation_copy_with_filters() {
        let catalog = Catalog::new();
        let mut job = Job::new("in.mkv", "out.mkv");
        let mut stream = OutStream::new_copy(StreamKind::Video, 0);
        stream.filters.push("scale=1280:720".into());
        job.output.streams.push(stream);

        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("filtri")));
    }

    #[test]
    fn test_validation_incompatible_container() {
        let mut catalog = Catalog::new();
        let toml_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codecs/libx265.toml");
        let codec = CodecDef::from_file(&toml_path).unwrap();
        catalog.add_or_replace(codec);

        let mut job = Job::new("in.mp4", "out.avi"); // libx265 containers: mkv, mp4, mov
        job.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "libx265",
            Default::default(),
        ));
        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("non è compatibile")));
    }

    #[test]
    fn test_rule_validation_prores_4444() {
        let mut catalog = Catalog::new();
        let toml_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codecs/prores_ks.toml");
        let codec = CodecDef::from_file(&toml_path).unwrap();
        catalog.add_or_replace(codec);

        let mut job = Job::new("in.mov", "out.mov");
        let mut params = std::collections::HashMap::new();
        params.insert("profile".to_string(), serde_json::json!("4444"));

        job.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "prores_ks",
            params,
        ));

        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| !i.is_error() && i.message.contains("4:4:4")));

        // Now test with pix_fmt = yuv444p10le: unless condition should suppress the warning!
        let mut job_exempt = Job::new("in.mov", "out.mov");
        let mut params_exempt = std::collections::HashMap::new();
        params_exempt.insert("profile".to_string(), serde_json::json!("4444"));
        params_exempt.insert("pix_fmt".to_string(), serde_json::json!("yuv444p10le"));
        job_exempt.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "prores_ks",
            params_exempt,
        ));
        let issues_exempt = validate_job(&job_exempt, &catalog);
        assert!(!issues_exempt.iter().any(|i| i.message.contains("4:4:4")));
    }

    #[test]
    fn test_opus_with_cover_art_fails_validation() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("track.flac", "track.opus");
        job.output.streams.push(OutStream::new_encode(StreamKind::Audio, 0, "libopus", std::collections::HashMap::new()));
        job.output.streams.push(OutStream::new_copy(StreamKind::CoverArt, 1));

        let issues = validate_job(&job, &catalog);
        let cover_err = issues.iter().find(|i| i.is_error() && i.message.contains("copertin"));
        assert!(cover_err.is_some());
    }

    #[test]
    fn test_same_file_overwrite_fails_validation() {
        let catalog = Catalog::load_default(false);
        let temp = tempfile::NamedTempFile::new().unwrap();
        let path = temp.path().to_str().unwrap();
        let job = Job::new(path, path);
        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("coincidere con il file di input")));
    }

    #[test]
    fn test_invalid_start_time_and_duration() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mp4");
        job.input.start_time = Some(-5.0);
        job.input.duration = Some(-1.0);
        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("inizio (-ss)")));
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("durata (-t)")));
    }

    #[test]
    fn test_unknown_param_warning() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mp4");
        let mut params = std::collections::HashMap::new();
        params.insert("nonexistent_param_xyz".to_string(), serde_json::json!("123"));
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "libx264", params));
        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| !i.is_error() && i.message.contains("sconosciuto")));
    }
}

#[cfg(test)]
mod container_checks {
    use super::*;
    use crate::model::OutStream;

    fn two_audio_job(out: &str) -> Job {
        let mut job = Job::new("in.mka", out);
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 0));
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));
        job
    }

    #[test]
    fn due_tracce_audio_verso_mp3_sono_un_errore() {
        let catalog = Catalog::load_default(false);
        let issues = validate_job(&two_audio_job("out.mp3"), &catalog);
        assert!(
            issues.iter().any(|i| i.is_error() && i.message.contains("una sola traccia audio")),
            "{:?}",
            issues
        );
    }

    #[test]
    fn due_tracce_audio_verso_mka_vanno_bene() {
        let catalog = Catalog::load_default(false);
        let issues = validate_job(&two_audio_job("out.mka"), &catalog);
        assert!(!issues.iter().any(|i| i.message.contains("una sola traccia audio")), "{:?}", issues);
    }

    #[test]
    fn allegati_verso_mp4_sono_un_errore() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mp4");
        job.output.streams.push(OutStream::new_copy(StreamKind::Attachment, 3));
        let issues = validate_job(&job, &catalog);
        assert!(issues.iter().any(|i| i.is_error() && i.message.contains("allegati")), "{:?}", issues);
    }
}
