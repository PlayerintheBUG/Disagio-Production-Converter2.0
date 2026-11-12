use crate::model::StreamKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProbeError {
    #[error("Failed to run ffprobe: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffprobe failed with exit code: {0}, stderr: {1}")]
    ProcessFailed(i32, String),
    #[error("Failed to parse ffprobe json output: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MediaStream {
    pub index: usize,
    pub kind: StreamKind,
    pub is_attached_pic: bool,
    pub codec_name: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pix_fmt: Option<String>,
    pub fps: Option<f64>,
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub duration_s: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub file: PathBuf,
    pub duration_s: Option<f64>,
    pub format_name: Option<String>,
    pub bit_rate: Option<u64>,
    pub streams: Vec<MediaStream>,
}

impl ProbeResult {
    pub fn video_streams(&self) -> Vec<&MediaStream> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Video).collect()
    }

    pub fn audio_streams(&self) -> Vec<&MediaStream> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Audio).collect()
    }

    pub fn subtitle_streams(&self) -> Vec<&MediaStream> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Subtitle).collect()
    }

    pub fn stream_by_index(&self, index: usize) -> Option<&MediaStream> {
        self.streams.iter().find(|s| s.index == index)
    }
}

// Low-level serde structs for ffprobe JSON format
#[derive(Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    index: usize,
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    pix_fmt: Option<String>,
    r_frame_rate: Option<String>,
    avg_frame_rate: Option<String>,
    channels: Option<u32>,
    sample_rate: Option<String>,
    duration: Option<String>,
    #[serde(default)]
    disposition: Option<HashMap<String, i32>>,
    #[serde(default)]
    tags: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
}

fn parse_frame_rate(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 2 {
        let num: f64 = parts[0].parse().ok()?;
        let den: f64 = parts[1].parse().ok()?;
        if den > 0.0 {
            return Some(num / den);
        }
    } else if let Ok(val) = s.parse::<f64>() {
        if val > 0.0 {
            return Some(val);
        }
    }
    None
}

pub fn probe_file(path: &Path) -> Result<ProbeResult, ProbeError> {
    probe_file_with_program(Path::new("ffprobe"), path)
}

pub fn probe_file_with_program(ffprobe_bin: &Path, path: &Path) -> Result<ProbeResult, ProbeError> {
    let mut cmd = Command::new(ffprobe_bin);
    crate::hide_console_window(&mut cmd);
    let output = cmd
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-i",
        ])
        .arg(path)
        .output()?;

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(ProbeError::ProcessFailed(code, err));
    }

    let parsed: FfprobeOutput = serde_json::from_slice(&output.stdout)?;

    let mut duration_s = None;
    let mut format_name = None;
    let mut bit_rate = None;

    if let Some(f) = parsed.format {
        format_name = f.format_name;
        if let Some(dur_str) = f.duration {
            duration_s = dur_str.parse::<f64>().ok();
        }
        if let Some(br_str) = f.bit_rate {
            bit_rate = br_str.parse::<u64>().ok();
        }
    }

    let streams = parsed
        .streams
        .into_iter()
        .map(|s| {
            let is_attached_pic = s
                .disposition
                .as_ref()
                .and_then(|d| d.get("attached_pic"))
                .map(|&v| v == 1)
                .unwrap_or(false);

            let kind = if is_attached_pic {
                StreamKind::CoverArt
            } else {
                match s.codec_type.as_deref() {
                    Some("video") => StreamKind::Video,
                    Some("audio") => StreamKind::Audio,
                    Some("subtitle") => StreamKind::Subtitle,
                    Some("attachment") => StreamKind::Attachment,
                    Some("data") => StreamKind::Data,
                    _ => StreamKind::Data,
                }
            };

            let fps = s
                .avg_frame_rate
                .as_deref()
                .and_then(parse_frame_rate)
                .or_else(|| s.r_frame_rate.as_deref().and_then(parse_frame_rate));

            let sample_rate = s.sample_rate.as_deref().and_then(|r| r.parse::<u32>().ok());
            let stream_duration = s.duration.as_deref().and_then(|d| d.parse::<f64>().ok());

            let mut language = None;
            let mut title = None;
            if let Some(tags) = s.tags {
                let mut lang_val = None;
                let mut language_val = None;
                for (k, v) in tags {
                    let k_lower = k.to_lowercase();
                    if k_lower == "language" {
                        language_val = Some(v);
                    } else if k_lower == "lang" {
                        lang_val = Some(v);
                    } else if k_lower == "title" {
                        title = Some(v);
                    }
                }
                language = language_val.or(lang_val);
            }

            MediaStream {
                index: s.index,
                kind,
                is_attached_pic,
                codec_name: s.codec_name,
                width: s.width,
                height: s.height,
                pix_fmt: s.pix_fmt,
                fps,
                channels: s.channels,
                sample_rate,
                language,
                title,
                duration_s: stream_duration,
            }
        })
        .collect();

    Ok(ProbeResult {
        file: path.to_path_buf(),
        duration_s,
        format_name,
        bit_rate,
        streams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_synthetic_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let sample_path = temp_dir.path().join("test_sample.mkv");

        // Generate a 1-second sample using ffmpeg lavfi
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=320x240:rate=25",
                "-f",
                "lavfi",
                "-i",
                "sine=duration=1:frequency=440",
                "-c:v",
                "ffv1",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&sample_path)
            .status()
            .expect("ffmpeg lavfi execution failed");

        assert!(status.success(), "Failed to generate synthetic test file");

        let probe = probe_file(&sample_path).expect("probe_file failed");
        assert_eq!(probe.streams.len(), 2);
        assert_eq!(probe.video_streams().len(), 1);
        assert_eq!(probe.audio_streams().len(), 1);

        let v = probe.video_streams()[0];
        assert_eq!(v.width, Some(320));
        assert_eq!(v.height, Some(240));
        assert_eq!(v.fps, Some(25.0));

        let a = probe.audio_streams()[0];
        assert_eq!(a.channels, Some(1));
    }
}
