use crate::catalog::Catalog;
use crate::model::{CodecConfig, Job, StreamKind};

pub struct CommandBuilder;

impl CommandBuilder {
    pub fn build(job: &Job, catalog: &Catalog) -> Vec<String> {
        Self::build_with_options(job, catalog, None, None)
    }

    /// Build ffmpeg args, optionally substituting {vaapi_device} in input_args
    pub fn build_with_vaapi(job: &Job, catalog: &Catalog, vaapi_device: Option<&str>) -> Vec<String> {
        Self::build_with_options(job, catalog, vaapi_device, None)
    }

    /// Build ffmpeg args with optional vaapi device and hardware decoding acceleration
    pub fn build_with_options(
        job: &Job,
        catalog: &Catalog,
        vaapi_device: Option<&str>,
        hwaccel_decode: Option<&str>,
    ) -> Vec<String> {
        let mut args = Vec::new();

        // 1. Global options
        args.push("-y".to_string());

        // 2. Collect hw input_args from any encode stream's codec definition
        // (only the first hw codec's input_args applies to the whole command)
        let mut hw_input_args: Vec<String> = Vec::new();
        for stream in &job.output.streams {
            if let crate::model::CodecConfig::Encode { id, .. } = &stream.codec {
                if let Some(codec_def) = catalog.get_codec(id) {
                    if !codec_def.input_args.is_empty() && hw_input_args.is_empty() {
                        hw_input_args = codec_def.input_args.iter().map(|a| {
                            a.replace("{vaapi_device}", vaapi_device.unwrap_or("/dev/dri/renderD128"))
                        }).collect();
                    }
                }
            }
        }

        // Inject hw input_args before seek options (they must come first)
        args.extend(hw_input_args);

        // Hardware decoding acceleration (before -i)
        if let Some(hwaccel) = hwaccel_decode {
            let hw_lower = hwaccel.to_lowercase();
            if !hw_lower.is_empty() && hw_lower != "disabled" && hw_lower != "none" {
                args.push("-hwaccel".to_string());
                args.push(hw_lower);
            }
        }

        // 3. Input options (before -i)
        if let Some(ss) = job.input.start_time {
            args.push("-ss".to_string());
            args.push(format_seconds(ss));
        }
        if let Some(t) = job.input.duration {
            args.push("-t".to_string());
            args.push(format_seconds(t));
        }

        // 4. -i with input file
        args.push("-i".to_string());
        args.push(job.input.path.to_string_lossy().to_string());

        // 5. -map for each output stream
        for stream in &job.output.streams {
            args.push("-map".to_string());
            args.push(format!("0:{}", stream.source_index));
        }

        // 6. Per-stream codec settings and filters
        let mut video_idx = 0;
        let mut audio_idx = 0;
        let mut sub_idx = 0;
        let mut att_idx = 0;
        let mut data_idx = 0;
        let mut has_image = false;

        for stream in &job.output.streams {
            let spec = match stream.kind {
                StreamKind::Video => {
                    let idx = video_idx;
                    video_idx += 1;
                    format!("v:{}", idx)
                }
                StreamKind::Audio => {
                    let idx = audio_idx;
                    audio_idx += 1;
                    format!("a:{}", idx)
                }
                StreamKind::Subtitle => {
                    let idx = sub_idx;
                    sub_idx += 1;
                    format!("s:{}", idx)
                }
                StreamKind::Image => {
                    has_image = true;
                    let idx = video_idx;
                    video_idx += 1;
                    format!("v:{}", idx)
                }
                StreamKind::CoverArt => {
                    let idx = video_idx;
                    video_idx += 1;
                    format!("v:{}", idx)
                }
                StreamKind::Attachment => {
                    let idx = att_idx;
                    att_idx += 1;
                    format!("t:{}", idx)
                }
                StreamKind::Data => {
                    let idx = data_idx;
                    data_idx += 1;
                    format!("d:{}", idx)
                }
            };

            match &stream.codec {
                CodecConfig::Copy => {
                    args.push(format!("-c:{}", spec));
                    args.push("copy".to_string());
                }
                CodecConfig::Encode { id, params } => {
                    args.push(format!("-c:{}", spec));
                    args.push(id.clone());

                    if let Some(codec_def) = catalog.get_codec(id) {
                        for param_def in &codec_def.params {
                            // Check conditional activation (e.g. only_if)
                            if !param_def.is_active(params, &codec_def.params) {
                                continue;
                            }

                            // Meta-parameter without ffmpeg flag (used for GUI/logic only)
                            if param_def.flag.is_empty() {
                                continue;
                            }

                            let val_opt = params.get(&param_def.id).or(param_def.default.as_ref());
                            if let Some(val) = val_opt {
                                let raw_flag = param_def.flag.as_str();
                                let flag_with_spec = if (raw_flag.ends_with(":v") && spec.starts_with("v:"))
                                    || (raw_flag.ends_with(":a") && spec.starts_with("a:"))
                                    || (raw_flag.ends_with(":s") && spec.starts_with("s:"))
                                {
                                    format!("{}:{}", &raw_flag[..raw_flag.len() - 2], spec)
                                } else {
                                    format!("{}:{}", raw_flag, spec)
                                };

                                match val {
                                    serde_json::Value::Bool(b) => {
                                        args.push(flag_with_spec);
                                        args.push(if *b { "1".to_string() } else { "0".to_string() });
                                    }
                                    serde_json::Value::Number(n) => {
                                        args.push(flag_with_spec);
                                        args.push(n.to_string());
                                    }
                                    serde_json::Value::String(s) => {
                                        args.push(flag_with_spec);
                                        args.push(s.clone());
                                    }
                                    _ => {}
                                }
                            }
                        }

                        // For NVENC encoders in VBR mode, FFmpeg defaults to 2M target bitrate unless -b:v 0 is explicitly passed.
                        // Automatically emit -b:v 0 in VBR mode so quality is governed solely by -cq.
                        if id.ends_with("_nvenc") {
                            let rc_val = params.get("rc").and_then(|v| v.as_str()).unwrap_or("vbr");
                            if rc_val == "vbr" {
                                let b_flag = format!("-b:{}", spec);
                                if !args.contains(&b_flag) {
                                    args.push(b_flag);
                                    args.push("0".to_string());
                                }
                            }
                        }
                    } else {
                        // Fallback: pass params in deterministic sorted order
                        let mut sorted_keys: Vec<&String> = params.keys().collect();
                        sorted_keys.sort();

                        for k in sorted_keys {
                            let v = &params[k];
                            let raw_flag = format!("-{}", k);
                            let flag_with_spec = if (raw_flag.ends_with(":v") && spec.starts_with("v:"))
                                || (raw_flag.ends_with(":a") && spec.starts_with("a:"))
                            {
                                format!("{}:{}", &raw_flag[..raw_flag.len() - 2], spec)
                            } else {
                                format!("{}:{}", raw_flag, spec)
                            };

                            args.push(flag_with_spec);
                            match v {
                                serde_json::Value::Bool(b) => {
                                    args.push(if *b { "1".to_string() } else { "0".to_string() });
                                }
                                serde_json::Value::Number(n) => {
                                    args.push(n.to_string());
                                }
                                serde_json::Value::String(s) => {
                                    args.push(s.clone());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            // Stream filters (append hw filter_suffix ONLY if this specific stream is actively encoded with an HW codec declaring filter_suffix)
            let is_video_like = matches!(stream.kind, StreamKind::Video | StreamKind::Image);
            let stream_hw_suffix = match &stream.codec {
                CodecConfig::Encode { id, .. } => {
                    catalog.get_codec(id).and_then(|def| def.filter_suffix.clone())
                }
                _ => None,
            };

            let effective_filters: Vec<String> = if is_video_like {
                let mut f = stream.filters.clone();
                if let Some(suffix) = stream_hw_suffix {
                    f.push(suffix);
                }
                f
            } else {
                stream.filters.clone()
            };

            if !effective_filters.is_empty() {
                args.push(format!("-filter:{}", spec));
                args.push(effective_filters.join(","));
            }
        }

        // Single image support: -frames:v 1
        if has_image {
            args.push("-frames:v".to_string());
            args.push("1".to_string());
        }

        // 7. Output file always last
        args.push(job.output.path.to_string_lossy().to_string());

        args
    }

    /// Format command safely for shell display and copy-pasting.
    /// Safely escapes special shell characters using POSIX single-quotes.
    pub fn to_preview_string(args: &[String]) -> String {
        let mut parts = vec!["ffmpeg".to_string()];
        for arg in args {
            if arg.is_empty() {
                parts.push("''".to_string());
            } else if arg.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || c == '_'
                    || c == '-'
                    || c == '.'
                    || c == '/'
                    || c == ':'
                    || c == '='
                    || c == '+'
                    || c == ','
            }) {
                parts.push(arg.clone());
            } else {
                let escaped = arg.replace('\'', r"'\''");
                parts.push(format!("'{}'", escaped));
            }
        }
        parts.join(" ")
    }
}

fn format_seconds(s: f64) -> String {
    if s.fract() == 0.0 {
        format!("{:.0}", s)
    } else {
        format!("{:.3}", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CodecDef;
    use crate::model::{OutStream, StreamKind};
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[test]
    fn test_build_x265_command_matches_spec() {
        let mut catalog = Catalog::new();
        let toml_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codecs/libx265.toml");
        let codec = CodecDef::from_file(&toml_path).unwrap();
        catalog.add_or_replace(codec);

        let mut job = Job::new("in.mov", "out.mkv");
        let mut params = HashMap::new();
        params.insert("crf".to_string(), serde_json::json!(16));
        params.insert("preset".to_string(), serde_json::json!("slow"));
        params.insert("pix_fmt".to_string(), serde_json::json!("yuv420p10le"));

        job.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "libx265",
            params,
        ));

        let args = CommandBuilder::build(&job, &catalog);
        let preview = CommandBuilder::to_preview_string(&args);

        let expected = "ffmpeg -y -i in.mov -map 0:0 -c:v:0 libx265 -crf:v:0 16 -preset:v:0 slow -pix_fmt:v:0 yuv420p10le out.mkv";
        assert_eq!(preview, expected);
    }

    #[test]
    fn test_mp3_cbr_vs_vbr_mutual_exclusion() {
        let mut catalog = Catalog::new();
        let toml_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../codecs/libmp3lame.toml");
        let codec = CodecDef::from_file(&toml_path).unwrap();
        catalog.add_or_replace(codec);

        // Test CBR 320k: should emit -b:a:0 320k and NOT -qscale
        let mut job_cbr = Job::new("in.wav", "out.mp3");
        let mut params_cbr = HashMap::new();
        params_cbr.insert("rate_control".to_string(), serde_json::json!("cbr"));
        params_cbr.insert("bitrate".to_string(), serde_json::json!("320k"));
        params_cbr.insert("qscale".to_string(), serde_json::json!(2)); // even if present, only_if must exclude it!

        job_cbr.output.streams.push(OutStream::new_encode(
            StreamKind::Audio,
            0,
            "libmp3lame",
            params_cbr,
        ));

        let args_cbr = CommandBuilder::build(&job_cbr, &catalog);
        assert!(args_cbr.contains(&"-b:a:0".to_string()));
        assert!(args_cbr.contains(&"320k".to_string()));
        assert!(!args_cbr.iter().any(|a| a.starts_with("-qscale")));

        // Test VBR: should emit -qscale:a:0 and NOT -b
        let mut job_vbr = Job::new("in.wav", "out.mp3");
        let mut params_vbr = HashMap::new();
        params_vbr.insert("rate_control".to_string(), serde_json::json!("vbr"));
        params_vbr.insert("qscale".to_string(), serde_json::json!(0));
        params_vbr.insert("bitrate".to_string(), serde_json::json!("192k")); // should be excluded!

        job_vbr.output.streams.push(OutStream::new_encode(
            StreamKind::Audio,
            0,
            "libmp3lame",
            params_vbr,
        ));

        let args_vbr = CommandBuilder::build(&job_vbr, &catalog);
        assert!(args_vbr.contains(&"-qscale:a:0".to_string()));
        assert!(args_vbr.contains(&"0".to_string()));
        assert!(!args_vbr.iter().any(|a| a.starts_with("-b")));
    }

    #[test]
    fn test_shell_preview_special_chars() {
        let args = vec![
            "-i".to_string(),
            "/path/with spaces/it's cool (1) $dollar.mp4".to_string(),
        ];
        let preview = CommandBuilder::to_preview_string(&args);
        // Special chars should be enclosed in '...' with escaped single quote
        assert!(preview.contains("'/path/with spaces/it'\\''s cool (1) $dollar.mp4'"));
    }

    #[test]
    fn test_deterministic_builder_argument_order() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mp4");
        let mut params = HashMap::new();
        params.insert("crf".to_string(), serde_json::json!(23));
        params.insert("preset".to_string(), serde_json::json!("medium"));
        params.insert("profile".to_string(), serde_json::json!("high"));

        job.output.streams.push(OutStream::new_encode(
            StreamKind::Video,
            0,
            "libx264",
            params,
        ));

        let first = CommandBuilder::build(&job, &catalog);
        for _ in 0..20 {
            let next = CommandBuilder::build(&job, &catalog);
            assert_eq!(first, next, "Builder output must be completely deterministic!");
        }
    }

    #[test]
    fn test_cover_art_stream_counters() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mkv");
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "libx264", HashMap::new()));
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));
        job.output.streams.push(OutStream::new_copy(StreamKind::CoverArt, 2));

        let args = CommandBuilder::build(&job, &catalog);
        // CoverArt should map to video index 1 (v:1)
        assert!(args.contains(&"-c:v:0".to_string()));
        assert!(args.contains(&"-c:a:0".to_string()));
        assert!(args.contains(&"-c:v:1".to_string()));
    }

    #[test]
    fn test_gpu_vaapi_builder_input_args_and_filter_suffix() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mp4");
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "h264_vaapi", HashMap::new()));

        let args = CommandBuilder::build_with_vaapi(&job, &catalog, Some("/dev/dri/renderD129"));
        let joined = args.join(" ");

        // Check input_args before -i
        assert!(joined.contains("-init_hw_device vaapi=va:/dev/dri/renderD129"));
        assert!(joined.contains("-filter_hw_device va"));

        // Position check: input_args should appear before -i
        let hw_pos = args.iter().position(|a| a == "-init_hw_device").unwrap();
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(hw_pos < i_pos, "input_args must appear before -i");

        // Filter suffix check
        assert!(joined.contains("-filter:v:0 format=nv12,hwupload"));
    }

    #[test]
    fn test_av1_nvenc_and_hwaccel_builder() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("input_video.mkv", "output_video.mp4");
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "av1_nvenc", HashMap::new()));

        let args = CommandBuilder::build_with_options(&job, &catalog, None, Some("auto"));
        let joined = args.join(" ");

        // Check hwaccel decode before -i
        let hw_pos = args.iter().position(|a| a == "-hwaccel").unwrap();
        let i_pos = args.iter().position(|a| a == "-i").unwrap();
        assert!(hw_pos < i_pos, "-hwaccel must be before -i");
        assert_eq!(args[hw_pos + 1], "auto");

        // Check video codec
        assert!(joined.contains("-c:v:0 av1_nvenc"));
        // Check pixel format NVENC requirement
        assert!(joined.contains("-pix_fmt:v:0 yuv420p"));
    }

    #[test]
    fn test_nvenc_bitrate_flag_and_audio_copy() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mp4");
        let mut params = HashMap::new();
        params.insert("rc".to_string(), serde_json::json!("vbr"));
        params.insert("cq".to_string(), serde_json::json!(26));
        params.insert("b_v".to_string(), serde_json::json!("0"));
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "av1_nvenc", params));
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));

        let args = CommandBuilder::build_with_options(&job, &catalog, None, Some("auto"));
        let joined = args.join(" ");

        assert!(!joined.contains("-b:v:v"), "Must not contain duplicate stream specifier: {}", joined);
        assert!(joined.contains("-b:v:0 0"), "Expected -b:v:0 0 in: {}", joined);
        assert!(joined.contains("-c:a:0 copy"), "Expected -c:a:0 copy in: {}", joined);
    }

    #[test]
    fn test_nvenc_cbr_and_constqp_parameters() {
        let catalog = Catalog::load_default(false);

        // 1. CBR mode with default 8M
        let mut job_cbr = Job::new("in.mp4", "out.mp4");
        let mut params_cbr = HashMap::new();
        params_cbr.insert("rc".to_string(), serde_json::json!("cbr"));
        job_cbr.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "hevc_nvenc", params_cbr));
        let args_cbr = CommandBuilder::build_with_options(&job_cbr, &catalog, None, None);
        let joined_cbr = args_cbr.join(" ");
        assert!(joined_cbr.contains("-rc:v:0 cbr"));
        assert!(joined_cbr.contains("-b:v:0 8M"), "CBR should emit -b:v:0 8M by default, got: {}", joined_cbr);
        assert!(!joined_cbr.contains("-cq:v:0"), "CBR should not emit -cq");

        // 2. ConstQP mode with default 24
        let mut job_qp = Job::new("in.mp4", "out.mp4");
        let mut params_qp = HashMap::new();
        params_qp.insert("rc".to_string(), serde_json::json!("constqp"));
        job_qp.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "h264_nvenc", params_qp));
        let args_qp = CommandBuilder::build_with_options(&job_qp, &catalog, None, None);
        let joined_qp = args_qp.join(" ");
        assert!(joined_qp.contains("-rc:v:0 constqp"));
        assert!(joined_qp.contains("-qp:v:0 24"), "ConstQP should emit -qp:v:0 24 by default, got: {}", joined_qp);
        assert!(!joined_qp.contains("-b:v:0"), "ConstQP should not emit -b:v:0");
    }

    #[test]
    fn test_stream_copy_does_not_receive_hw_filter_suffix() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mp4");
        // Stream 0: VAAPI encode
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "hevc_vaapi", HashMap::new()));
        // Stream 1: Video stream in copy
        job.output.streams.push(OutStream::new_copy(StreamKind::Video, 1));

        let args = CommandBuilder::build_with_vaapi(&job, &catalog, Some("/dev/dri/renderD128"));
        let joined = args.join(" ");

        // Stream 0 gets the filter
        assert!(joined.contains("-filter:v:0 format=nv12,hwupload"));
        // Stream 1 in copy MUST NOT get a filter flag
        assert!(!joined.contains("-filter:v:1"), "Stream 1 in copy must not receive filters! Got: {}", joined);
    }
}
