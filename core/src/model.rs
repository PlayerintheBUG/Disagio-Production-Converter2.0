use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    #[default]
    Video,
    Audio,
    Subtitle,
    Image,
    CoverArt,
    Attachment,
    Data,
}

impl StreamKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
            StreamKind::Subtitle => "subtitle",
            StreamKind::Image => "image",
            StreamKind::CoverArt => "coverart",
            StreamKind::Attachment => "attachment",
            StreamKind::Data => "data",
        }
    }

    pub fn ffmpeg_specifier(&self) -> &'static str {
        match self {
            StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => "v",
            StreamKind::Audio => "a",
            StreamKind::Subtitle => "s",
            StreamKind::Attachment => "t",
            StreamKind::Data => "d",
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self, StreamKind::Video | StreamKind::Image | StreamKind::CoverArt)
    }

    pub fn is_cover_art(&self) -> bool {
        matches!(self, StreamKind::CoverArt)
    }

    pub fn is_audio(&self) -> bool {
        matches!(self, StreamKind::Audio)
    }

    pub fn is_subtitle(&self) -> bool {
        matches!(self, StreamKind::Subtitle)
    }

    pub fn is_media(&self) -> bool {
        matches!(
            self,
            StreamKind::Video
                | StreamKind::Audio
                | StreamKind::Subtitle
                | StreamKind::Image
                | StreamKind::CoverArt
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Input {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

impl Input {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            start_time: None,
            duration: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum CodecConfig {
    Copy,
    Encode {
        id: String,
        #[serde(default)]
        params: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutStream {
    pub kind: StreamKind,
    pub source_index: usize,
    pub codec: CodecConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<String>,
}

impl OutStream {
    pub fn new_copy(kind: StreamKind, source_index: usize) -> Self {
        Self {
            kind,
            source_index,
            codec: CodecConfig::Copy,
            filters: Vec::new(),
        }
    }

    pub fn new_encode(
        kind: StreamKind,
        source_index: usize,
        codec_id: impl Into<String>,
        params: HashMap<String, serde_json::Value>,
    ) -> Self {
        Self {
            kind,
            source_index,
            codec: CodecConfig::Encode {
                id: codec_id.into(),
                params,
            },
            filters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub streams: Vec<OutStream>,
}

impl Output {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: None,
            streams: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub input: Input,
    pub output: Output,
}

impl Job {
    pub fn new(input_path: impl Into<PathBuf>, output_path: impl Into<PathBuf>) -> Self {
        Self {
            input: Input::new(input_path),
            output: Output::new(output_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoRule {
    pub codec_id: String,
    #[serde(default)]
    pub params: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale_preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum AudioRule {
    #[default]
    CopyAll,
    EncodeAll {
        codec_id: String,
        params: HashMap<String, serde_json::Value>,
    },
    EncodeFirstCopyRest {
        codec_id: String,
        params: HashMap<String, serde_json::Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum SubtitleRule {
    #[default]
    Ignore,
    CopyCompatible,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct JobTemplate {
    pub video_rule: Option<VideoRule>,
    pub audio_rule: AudioRule,
    pub subtitle_rule: SubtitleRule,
    pub copy_cover_art: bool,
}

impl JobTemplate {
    pub fn instantiate(
        &self,
        input_path: &std::path::Path,
        output_path: &std::path::Path,
        streams: &[crate::probe::MediaStream],
    ) -> Job {
        let mut job = Job::new(input_path, output_path);
        let out_ext = output_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let mut video_count = 0;
        let mut audio_count = 0;

        for stream in streams {
            match stream.kind {
                StreamKind::Video => {
                    video_count += 1;
                    if video_count == 1 {
                        if let Some(ref v_rule) = self.video_rule {
                            let mut out_stream = match v_rule.codec_id.as_str() {
                                "copy" => OutStream::new_copy(StreamKind::Video, stream.index),
                                codec_id => OutStream::new_encode(
                                    StreamKind::Video,
                                    stream.index,
                                    codec_id,
                                    v_rule.params.clone(),
                                ),
                            };

                            if let Some(ref scale) = v_rule.scale_preset {
                                if scale == "1080p (altezza 1080)" || scale == "1080p (1920x1080)" || scale == "1080p" {
                                    out_stream.filters.push("scale=-2:'min(1080,ih)'".into());
                                } else if scale == "720p (altezza 720)" || scale == "720p (1280x720)" || scale == "720p" {
                                    out_stream.filters.push("scale=-2:'min(720,ih)'".into());
                                } else if scale == "480p (altezza 480)" || scale == "480p (854x480)" || scale == "480p" {
                                    out_stream.filters.push("scale=-2:'min(480,ih)'".into());
                                } else if scale == "Personalizzato" {
                                    if let (Some(w), Some(h)) = (v_rule.custom_width, v_rule.custom_height) {
                                        // Ensure even dimensions required by x264/x265 yuv420p
                                        let w_even = ((w / 2) * 2).max(2);
                                        let h_even = ((h / 2) * 2).max(2);
                                        out_stream.filters.push(format!("scale={}:{}", w_even, h_even));
                                    }
                                }
                            }

                            job.output.streams.push(out_stream);
                        }
                    }
                }
                StreamKind::Audio => {
                    audio_count += 1;
                    match &self.audio_rule {
                        AudioRule::CopyAll => {
                            job.output.streams.push(OutStream::new_copy(StreamKind::Audio, stream.index));
                        }
                        AudioRule::EncodeAll { codec_id, params } => {
                            job.output.streams.push(OutStream::new_encode(
                                StreamKind::Audio,
                                stream.index,
                                codec_id,
                                params.clone(),
                            ));
                        }
                        AudioRule::EncodeFirstCopyRest { codec_id, params } => {
                            if audio_count == 1 {
                                job.output.streams.push(OutStream::new_encode(
                                    StreamKind::Audio,
                                    stream.index,
                                    codec_id,
                                    params.clone(),
                                ));
                            } else {
                                job.output.streams.push(OutStream::new_copy(StreamKind::Audio, stream.index));
                            }
                        }
                    }
                }
                StreamKind::Subtitle => {
                    if self.subtitle_rule == SubtitleRule::CopyCompatible {
                        let caps = crate::containers::container_caps(&out_ext);
                        if caps.subtitles {
                            let is_bitmap = stream.codec_name.as_deref().is_some_and(|c| {
                                c == "hdmv_pgs_subtitle" || c == "dvd_subtitle" || c == "dvb_subtitle"
                            });
                            let is_mp4 = matches!(out_ext.as_str(), "mp4" | "m4v" | "mov");
                            if !(is_mp4 && is_bitmap) {
                                job.output.streams.push(OutStream::new_copy(StreamKind::Subtitle, stream.index));
                            }
                        }
                    }
                }
                StreamKind::CoverArt => {
                    if self.copy_cover_art {
                        let caps = crate::containers::container_caps(&out_ext);
                        if caps.cover_art {
                            job.output.streams.push(OutStream::new_copy(StreamKind::CoverArt, stream.index));
                        }
                    }
                }
                StreamKind::Attachment => {
                    let caps = crate::containers::container_caps(&out_ext);
                    if caps.attachments {
                        job.output.streams.push(OutStream::new_copy(StreamKind::Attachment, stream.index));
                    }
                }
                _ => {}
            }
        }

        job
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::MediaStream;

    #[test]
    fn test_scale_preset_min_ih() {
        let mut template = JobTemplate::default();
        template.video_rule = Some(VideoRule {
            codec_id: "libx264".to_string(),
            params: std::collections::HashMap::new(),
            scale_preset: Some("1080p (altezza 1080)".to_string()),
            custom_width: None,
            custom_height: None,
        });

        let streams = vec![
            MediaStream {
                index: 0,
                kind: StreamKind::Video,
                codec_name: Some("h264".to_string()),
                width: Some(3840),
                height: Some(2160),
                ..Default::default()
            }
        ];

        let job = template.instantiate(std::path::Path::new("in.mp4"), std::path::Path::new("out.mp4"), &streams);
        assert_eq!(job.output.streams[0].filters, vec!["scale=-2:'min(1080,ih)'"]);
    }

    #[test]
    fn test_scale_custom_forces_even_dimensions() {
        let mut template = JobTemplate::default();
        template.video_rule = Some(VideoRule {
            codec_id: "libx264".to_string(),
            params: std::collections::HashMap::new(),
            scale_preset: Some("Personalizzato".to_string()),
            custom_width: Some(1921),
            custom_height: Some(1081),
        });

        let streams = vec![
            MediaStream {
                index: 0,
                kind: StreamKind::Video,
                codec_name: Some("h264".to_string()),
                width: Some(1920),
                height: Some(1080),
                ..Default::default()
            }
        ];

        let job = template.instantiate(std::path::Path::new("in.mp4"), std::path::Path::new("out.mp4"), &streams);
        assert_eq!(job.output.streams[0].filters, vec!["scale=1920:1080"]);
    }
}

