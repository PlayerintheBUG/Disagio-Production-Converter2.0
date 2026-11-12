use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuHwType {
    Nvenc,
    Vaapi,
    Qsv,
    Amf,
}

impl GpuHwType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::Vaapi => "vaapi",
            Self::Qsv => "qsv",
            Self::Amf => "amf",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuEncoderStatus {
    pub id: String,
    pub label: String,
    pub hw_type: GpuHwType,
    pub is_available: bool,
    pub device: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuDetectionResult {
    pub encoders: Vec<GpuEncoderStatus>,
    pub default_vaapi_device: Option<String>,
}

impl GpuDetectionResult {
    pub fn is_encoder_available(&self, codec_id: &str) -> bool {
        self.encoders
            .iter()
            .any(|e| e.id == codec_id && e.is_available)
    }

    pub fn has_any_cuda(&self) -> bool {
        self.encoders
            .iter()
            .any(|e| e.hw_type == GpuHwType::Nvenc && e.is_available)
    }

    pub fn has_any_gpu(&self) -> bool {
        self.encoders.iter().any(|e| e.is_available)
    }

    pub fn has_any_hw_encoder(&self) -> bool {
        self.has_any_gpu()
    }

    pub fn first_available_gpu_encoder(&self) -> Option<&str> {
        self.encoders
            .iter()
            .find(|e| e.is_available)
            .map(|e| e.id.as_str())
    }
}

pub struct GpuBackend;

impl GpuBackend {
    /// List all render device nodes in /dev/dri/ (e.g. renderD128, renderD129)
    pub fn list_render_devices() -> Vec<PathBuf> {
        let dri = Path::new("/dev/dri");
        let mut devices = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dri) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("renderD") {
                    devices.push(entry.path());
                }
            }
        }
        devices.sort();
        devices
    }

    /// Perform a real 0.1-second lavfi execution probe to determine if a GPU encoder actually functions on hardware.
    /// Protected by a 2.5-second timeout to avoid hangs on broken GPU drivers.
    pub fn probe_encoder(
        ffmpeg_bin: &str,
        codec_id: &str,
        hw_type: GpuHwType,
        custom_device: Option<&str>,
    ) -> bool {
        let mut cmd = Command::new(ffmpeg_bin);
        cmd.args(["-hide_banner", "-y"]);

        match hw_type {
            GpuHwType::Nvenc => {
                cmd.args([
                    "-f", "lavfi",
                    "-i", "color=black:s=256x256:d=0.1",
                    "-pix_fmt", "yuv420p",
                    "-c:v", codec_id,
                    "-f", "null",
                    "-",
                ]);
            }
            GpuHwType::Vaapi => {
                let device = custom_device.unwrap_or("/dev/dri/renderD128");
                cmd.args([
                    "-init_hw_device", &format!("vaapi=va:{}", device),
                    "-filter_hw_device", "va",
                    "-f", "lavfi",
                    "-i", "color=black:s=256x256:d=0.1",
                    "-vf", "format=nv12,hwupload",
                    "-c:v", codec_id,
                    "-f", "null",
                    "-",
                ]);
            }
            GpuHwType::Qsv => {
                cmd.args([
                    "-init_hw_device", "qsv=hw",
                    "-filter_hw_device", "hw",
                    "-f", "lavfi",
                    "-i", "color=black:s=256x256:d=0.1",
                    "-vf", "format=nv12,hwupload=extra_hw_frames=64",
                    "-c:v", codec_id,
                    "-f", "null",
                    "-",
                ]);
            }
            GpuHwType::Amf => {
                cmd.args([
                    "-f", "lavfi",
                    "-i", "color=black:s=256x256:d=0.1",
                    "-pix_fmt", "nv12",
                    "-c:v", codec_id,
                    "-f", "null",
                    "-",
                ]);
            }
        }

        let mut child = match cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return false,
        };

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(2500);

        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(_) => {
                    let _ = child.kill();
                    return false;
                }
            }
        }
    }

    /// Detect all known GPU encoders with real hardware verification.
    /// Prioritizes CUDA / NVENC as requested.
    pub fn detect_all(ffmpeg_bin: &str) -> GpuDetectionResult {
        let candidates = [
            // NVENC (CUDA) - Highest Priority
            ("av1_nvenc", "AV1 (NVIDIA NVENC, CUDA)", GpuHwType::Nvenc),
            ("hevc_nvenc", "H.265 (NVIDIA NVENC, CUDA)", GpuHwType::Nvenc),
            ("h264_nvenc", "H.264 (NVIDIA NVENC, CUDA)", GpuHwType::Nvenc),
            // QSV (Intel QuickSync)
            ("av1_qsv", "AV1 (Intel QuickSync)", GpuHwType::Qsv),
            ("hevc_qsv", "H.265 (Intel QuickSync)", GpuHwType::Qsv),
            ("h264_qsv", "H.264 (Intel QuickSync)", GpuHwType::Qsv),
            // AMF (AMD)
            ("av1_amf", "AV1 (AMD AMF)", GpuHwType::Amf),
            ("hevc_amf", "H.265 (AMD AMF)", GpuHwType::Amf),
            ("h264_amf", "H.264 (AMD AMF)", GpuHwType::Amf),
            // VAAPI (Intel/AMD)
            ("av1_vaapi", "AV1 (Intel/AMD VAAPI)", GpuHwType::Vaapi),
            ("hevc_vaapi", "H.265 (Intel/AMD VAAPI)", GpuHwType::Vaapi),
            ("h264_vaapi", "H.264 (Intel/AMD VAAPI)", GpuHwType::Vaapi),
        ];

        let render_devs = Self::list_render_devices();
        let first_dev_str = render_devs.first().map(|d| d.to_string_lossy().to_string());

        let mut statuses = Vec::new();

        for (id, label, hw) in candidates {
            let (is_avail, dev) = match hw {
                GpuHwType::Nvenc | GpuHwType::Qsv | GpuHwType::Amf => {
                    let ok = Self::probe_encoder(ffmpeg_bin, id, hw, None);
                    (ok, None)
                }
                GpuHwType::Vaapi => {
                    let mut working_dev = None;
                    for dev in &render_devs {
                        let dev_str = dev.to_string_lossy();
                        if Self::probe_encoder(ffmpeg_bin, id, hw, Some(&dev_str)) {
                            working_dev = Some(dev_str.to_string());
                            break;
                        }
                    }
                    let ok = working_dev.is_some();
                    (ok, working_dev)
                }
            };

            statuses.push(GpuEncoderStatus {
                id: id.to_string(),
                label: label.to_string(),
                hw_type: hw,
                is_available: is_avail,
                device: dev,
            });
        }

        GpuDetectionResult {
            encoders: statuses,
            default_vaapi_device: first_dev_str,
        }
    }

    /// Return the corresponding CPU codec for seamless "Retry with CPU" fallback
    pub fn get_cpu_counterpart(gpu_codec_id: &str) -> Option<&'static str> {
        match gpu_codec_id {
            "av1_nvenc" | "av1_vaapi" | "av1_qsv" | "av1_amf" => Some("libsvtav1"),
            "hevc_nvenc" | "hevc_vaapi" | "hevc_qsv" | "hevc_amf" => Some("libx265"),
            "h264_nvenc" | "h264_vaapi" | "h264_qsv" | "h264_amf" => Some("libx264"),
            _ => None,
        }
    }

    /// Return the corresponding GPU codec for a CPU codec based on hardware preference
    pub fn get_gpu_counterpart(cpu_codec_id: &str, preferred_hw: &str) -> Option<&'static str> {
        let pref = preferred_hw.to_lowercase();
        match cpu_codec_id {
            "libsvtav1" | "libaom-av1" => {
                if pref.contains("vaapi") {
                    Some("av1_vaapi")
                } else if pref.contains("qsv") {
                    Some("av1_qsv")
                } else if pref.contains("amf") {
                    Some("av1_amf")
                } else {
                    Some("av1_nvenc") // Default prioritize CUDA / NVENC
                }
            }
            "libx265" => {
                if pref.contains("vaapi") {
                    Some("hevc_vaapi")
                } else if pref.contains("qsv") {
                    Some("hevc_qsv")
                } else if pref.contains("amf") {
                    Some("hevc_amf")
                } else {
                    Some("hevc_nvenc") // Default prioritize CUDA / NVENC
                }
            }
            "libx264" => {
                if pref.contains("vaapi") {
                    Some("h264_vaapi")
                } else if pref.contains("qsv") {
                    Some("h264_qsv")
                } else if pref.contains("amf") {
                    Some("h264_amf")
                } else {
                    Some("h264_nvenc") // Default prioritize CUDA / NVENC
                }
            }
            _ => None,
        }
    }

    /// Riconosce nel log di ffmpeg gli errori tipici di un encoder hardware che non parte
    /// (driver mancante, dispositivo non apribile, sessioni esaurite...). Serve a decidere se ha senso
    /// riprovare con la CPU: un errore di disco pieno o di file illeggibile non lo è.
    pub fn looks_like_gpu_failure(log_lines: &[String]) -> bool {
        const GPU_SIGNATURES: &[&str] = &[
            "[avhwdevicecontext", "[hevc_nvenc", "[h264_nvenc", "[av1_nvenc",
            "[h264_vaapi", "[hevc_vaapi", "[av1_vaapi",
            "[h264_qsv", "[hevc_qsv", "[av1_qsv",
            "[h264_amf", "[hevc_amf", "[av1_amf",
            "cannot load libcuda", "cannot load nvcuda", "no capable devices",
            "openencodesession", "device creation failed", "failed to set value",
            "no usable encoding profile", "libva error", "libva info",
            "no device available", "could not open encoder", "hwupload",
            "failed to create directx", "d3d11", "error while opening encoder",
            "amf failed", "nvenc error", "vaapi error",
        ];

        log_lines.iter().rev().take(300).any(|line| {
            let low = line.to_lowercase();
            // Discard lines that contain file path indications to avoid false matches
            if low.contains("from '")
                || low.contains("to '")
                || low.contains("opening '")
                || low.contains("no such file")
                || low.contains("permission denied")
            {
                return false;
            }
            (low.contains("[error]") || low.contains("[fatal]"))
                && GPU_SIGNATURES.iter().any(|sig| low.contains(sig))
        })
    }

    /// Map GPU encoder parameters to equivalent CPU parameters (e.g. cq/qp -> crf, preset mapping).
    pub fn map_gpu_params_to_cpu(
        _gpu_codec: &str,
        cpu_codec: &str,
        gpu_params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> std::collections::HashMap<String, serde_json::Value> {
        let mut cpu_params = std::collections::HashMap::new();

        // 1. Preserve explicit video bitrate target (b_v) when present: it must survive
        //    the fallback, otherwise a size-capped job would silently lose its target.
        let bitrate_target = gpu_params
            .get("b_v")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        // 2. Map quality: cq (nvenc), qp (vaapi), global_quality (qsv), qp_i (amf) -> crf (cpu)
        let quality_val = gpu_params
            .get("cq")
            .or_else(|| gpu_params.get("qp"))
            .or_else(|| gpu_params.get("global_quality"))
            .or_else(|| gpu_params.get("qp_i"))
            .and_then(|v| v.as_f64());

        if let Some(q) = quality_val {
            let crf = match cpu_codec {
                "libsvtav1" => (q.round() as i64).clamp(0, 63),
                "libx265" | "libx264" => (q.round() as i64).clamp(0, 51),
                _ => (q.round() as i64).clamp(0, 51),
            };
            cpu_params.insert("crf".to_string(), serde_json::json!(crf));
        } else if bitrate_target.is_none() {
            let default_crf = if cpu_codec == "libsvtav1" { 28 } else { 23 };
            cpu_params.insert("crf".to_string(), serde_json::json!(default_crf));
        }

        if let Some(ref b) = bitrate_target {
            cpu_params.insert("b_v".to_string(), serde_json::json!(b));
        }

        // 2. Map preset: p1..p7 (nvenc) -> cpu presets
        if let Some(preset_val) = gpu_params.get("preset").and_then(|v| v.as_str()) {
            let mapped_preset = match preset_val {
                "p1" | "p2" => "veryfast",
                "p3" => "faster",
                "p4" | "p5" => "medium",
                "p6" => "slow",
                "p7" => "slower",
                other if ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"].contains(&other) => other,
                _ => "medium",
            };
            cpu_params.insert("preset".to_string(), serde_json::json!(mapped_preset));
        } else {
            cpu_params.insert("preset".to_string(), serde_json::json!("medium"));
        }

        // 3. Preserve bitrate if present
        if let Some(b) = gpu_params.get("bitrate") {
            cpu_params.insert("bitrate".to_string(), b.clone());
        }

        cpu_params
    }

    /// Select the best available GPU encoder based on CPU counterpart, system detection, and hardware preference mode.
    pub fn pick_gpu_encoder(det: &GpuDetectionResult, cpu_id: &str, mode: &str) -> Option<String> {
        let family = match cpu_id {
            "libx265" => "hevc",
            "libx264" => "h264",
            "libsvtav1" | "libaom-av1" => "av1",
            _ => return None,
        };
        let order: &[&str] = match mode.to_lowercase().as_str() {
            "cuda" | "nvenc" => &["nvenc"],
            "vaapi" => &["vaapi"],
            "qsv" => &["qsv"],
            "amf" => &["amf"],
            _ => &["nvenc", "qsv", "amf", "vaapi"],
        };
        order
            .iter()
            .map(|hw| format!("{}_{}", family, hw))
            .find(|id| det.is_encoder_available(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_gpu_failure() {
        let gpu = vec![
            "[info] Input #0, matroska,webm, from 'a.mkv':".to_string(),
            "[AVHWDeviceContext @ 0x55] [error] Cannot load libcuda.so.1".to_string(),
            "[error] Error initializing output stream 0:0".to_string(),
        ];
        assert!(GpuBackend::looks_like_gpu_failure(&gpu));

        let disk = vec![
            "[error] Error writing trailer: No space left on device".to_string(),
            "[info] Conversion failed!".to_string(),
        ];
        assert!(!GpuBackend::looks_like_gpu_failure(&disk));

        // un'informazione (non un errore) che nomina nvenc non basta
        let info_only = vec!["[info]   Stream #0:0: Video: hevc (hevc_nvenc)".to_string()];
        assert!(!GpuBackend::looks_like_gpu_failure(&info_only));

        // Un percorso file contenente stringhe come cuda non deve scambiare l'errore per fallimento GPU
        let path_line = vec!["[error] Error opening /path/to/my_cuda_video.mp4: No such file".to_string()];
        assert!(!GpuBackend::looks_like_gpu_failure(&path_line));
    }

    #[test]
    fn test_pick_gpu_encoder() {
        let det = GpuDetectionResult {
            encoders: vec![
                GpuEncoderStatus {
                    id: "hevc_nvenc".to_string(),
                    label: "H.265 NVENC".to_string(),
                    hw_type: GpuHwType::Nvenc,
                    is_available: true,
                    device: None,
                },
                GpuEncoderStatus {
                    id: "h264_vaapi".to_string(),
                    label: "H.264 VAAPI".to_string(),
                    hw_type: GpuHwType::Vaapi,
                    is_available: true,
                    device: Some("/dev/dri/renderD128".to_string()),
                },
            ],
            default_vaapi_device: None,
        };

        // hevc_nvenc is available
        assert_eq!(GpuBackend::pick_gpu_encoder(&det, "libx265", "auto"), Some("hevc_nvenc".to_string()));
        // h264_vaapi is available, h264_nvenc is not
        assert_eq!(GpuBackend::pick_gpu_encoder(&det, "libx264", "auto"), Some("h264_vaapi".to_string()));
        // cuda mode with no h264_nvenc
        assert_eq!(GpuBackend::pick_gpu_encoder(&det, "libx264", "cuda"), None);
        // av1 is not in detected encoders
        assert_eq!(GpuBackend::pick_gpu_encoder(&det, "libsvtav1", "auto"), None);
        // non-video codec returns None
        assert_eq!(GpuBackend::pick_gpu_encoder(&det, "prores", "auto"), None);
    }

    #[test]
    fn test_cpu_counterparts() {
        assert_eq!(GpuBackend::get_cpu_counterpart("hevc_nvenc"), Some("libx265"));
        assert_eq!(GpuBackend::get_cpu_counterpart("h264_nvenc"), Some("libx264"));
        assert_eq!(GpuBackend::get_cpu_counterpart("av1_nvenc"), Some("libsvtav1"));
        assert_eq!(GpuBackend::get_cpu_counterpart("libx264"), None);
    }

    #[test]
    fn test_gpu_counterparts() {
        assert_eq!(GpuBackend::get_gpu_counterpart("libsvtav1", "cuda"), Some("av1_nvenc"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libsvtav1", "auto"), Some("av1_nvenc"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libsvtav1", "vaapi"), Some("av1_vaapi"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libsvtav1", "qsv"), Some("av1_qsv"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libsvtav1", "amf"), Some("av1_amf"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libx265", "cuda"), Some("hevc_nvenc"));
        assert_eq!(GpuBackend::get_gpu_counterpart("libx264", "cuda"), Some("h264_nvenc"));
    }

    #[test]
    fn test_map_gpu_params_to_cpu() {
        let mut gpu_params = std::collections::HashMap::new();
        gpu_params.insert("cq".to_string(), serde_json::json!(23));
        gpu_params.insert("preset".to_string(), serde_json::json!("p4"));

        let cpu_params = GpuBackend::map_gpu_params_to_cpu("hevc_nvenc", "libx265", &gpu_params);
        assert_eq!(cpu_params.get("crf"), Some(&serde_json::json!(23)));
        assert_eq!(cpu_params.get("preset"), Some(&serde_json::json!("medium")));
        assert!(!cpu_params.contains_key("cq"));
    }
}
