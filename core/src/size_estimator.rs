//! Modulo per il calcolo della dimensione stimata del file di output.
//!
//! Questo modulo fornisce funzionalita per calcolare la dimensione approssimativa
//! del file convertito, suddivisa per stream audio e video.

use crate::catalog::Catalog;
use crate::model::{CodecConfig, Job, OutStream, StreamKind};
use crate::probe::ProbeResult;
use std::collections::HashMap;

/// Dimensione stimata per tipologia di stream
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamSizeEstimate {
    /// Dimensione in bytes
    pub bytes: u64,
    /// Dimensione in KB
    pub kb: f64,
    /// Dimensione in MB
    pub mb: f64,
    /// Dimensione in GB
    pub gb: f64,
}

impl StreamSizeEstimate {
    pub fn new(bytes: u64) -> Self {
        Self {
            bytes,
            kb: bytes as f64 / 1024.0,
            mb: bytes as f64 / (1024.0 * 1024.0),
            gb: bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        }
    }

    /// Formatta la dimensione in modo leggibile
    pub fn format(&self) -> String {
        if self.gb >= 1.0 {
            format!("{:.2} GB", self.gb)
        } else if self.mb >= 1.0 {
            format!("{:.2} MB", self.mb)
        } else if self.kb >= 1.0 {
            format!("{:.2} KB", self.kb)
        } else {
            format!("{} B", self.bytes)
        }
    }

    /// Formatta la dimensione in modo compatto (es: "1.23 GB")
    pub fn format_short(&self) -> String {
        if self.gb >= 1.0 {
            format!("{:.2} GB", self.gb)
        } else if self.mb >= 1.0 {
            format!("{:.2} MB", self.mb)
        } else {
            format!("{:.2} KB", self.kb)
        }
    }
}

/// Dimensione stimata totale con suddivisione per tipo
#[derive(Debug, Clone, Copy, Default)]
pub struct EstimatedSize {
    /// Dimensione stimata per stream video
    pub video: StreamSizeEstimate,
    /// Dimensione stimata per stream audio
    pub audio: StreamSizeEstimate,
    /// Dimensione stimata per stream sottotitoli
    pub subtitle: StreamSizeEstimate,
    /// Dimensione totale
    pub total: StreamSizeEstimate,
}

impl EstimatedSize {
    pub fn new(video_bytes: u64, audio_bytes: u64, subtitle_bytes: u64) -> Self {
        Self {
            video: StreamSizeEstimate::new(video_bytes),
            audio: StreamSizeEstimate::new(audio_bytes),
            subtitle: StreamSizeEstimate::new(subtitle_bytes),
            total: StreamSizeEstimate::new(video_bytes + audio_bytes + subtitle_bytes),
        }
    }

    /// Formatta la stima completa
    pub fn format(&self) -> String {
        let mut parts = Vec::new();
        if self.video.bytes > 0 {
            parts.push(format!("Video: {}", self.video.format()));
        }
        if self.audio.bytes > 0 {
            parts.push(format!("Audio: {}", self.audio.format()));
        }
        if self.subtitle.bytes > 0 {
            parts.push(format!("Sottotitoli: {}", self.subtitle.format()));
        }
        parts.push(format!("Totale: {}", self.total.format()));
        parts.join(" | ")
    }

    /// Formatta in modo compatto per l'interfaccia
    pub fn format_compact(&self) -> String {
        let mut parts = Vec::new();
        if self.video.bytes > 0 {
            parts.push(format!("Video ~{}", self.video.format_short()));
        }
        if self.audio.bytes > 0 {
            parts.push(format!("Audio ~{}", self.audio.format_short()));
        }
        if self.subtitle.bytes > 0 {
            parts.push(format!("Sottotitoli ~{}", self.subtitle.format_short()));
        }
        parts.push(format!("Totale ~{}", self.total.format_short()));
        parts.join(" + ")
    }
}

/// Estrae il bitrate da un parametro, cercando vari nomi possibili
fn extract_bitrate_from_params(params: &HashMap<String, serde_json::Value>) -> Option<u64> {
    // Nomi comuni per il parametro bitrate
    let bitrate_keys = [
        "bitrate", "b", "b_v", "b:a", "video_bitrate", "audio_bitrate", "target_bitrate", "abr", "vbr",
    ];

    for key in &bitrate_keys {
        if let Some(value) = params.get(*key) {
            if let Some(num) = value.as_f64() {
                // Converti da kbps a bps se necessario
                if num > 100_000_000.0 {
                    return Some(num as u64);
                } else if num > 1.0 && num < 100_000.0 {
                    return Some((num * 1000.0) as u64);
                }
            } else if let Some(str_val) = value.as_str() {
                if let Ok(parsed) = parse_bitrate_string(str_val) {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

/// Parsa stringhe di bitrate come "1000k", "1M", "500K", "2000", etc.
fn parse_bitrate_string(s: &str) -> Result<u64, ()> {
    let s = s.trim().to_lowercase();

    if s.ends_with('k') || s.ends_with("kbps") || s.ends_with("kbit") {
        let num_str = s.trim_end_matches(|c: char| c.is_alphabetic() || c == ' ');
        if let Ok(num) = num_str.parse::<f64>() {
            Ok((num * 1000.0) as u64)
        } else {
            Err(())
        }
    } else if s.ends_with('m') || s.ends_with("mbps") || s.ends_with("mbit") {
        let num_str = s.trim_end_matches(|c: char| c.is_alphabetic() || c == ' ');
        if let Ok(num) = num_str.parse::<f64>() {
            Ok((num * 1_000_000.0) as u64)
        } else {
            Err(())
        }
    } else if s.ends_with('g') || s.ends_with("gbps") || s.ends_with("gbit") {
        let num_str = s.trim_end_matches(|c: char| c.is_alphabetic() || c == ' ');
        if let Ok(num) = num_str.parse::<f64>() {
            Ok((num * 1_000_000_000.0) as u64)
        } else {
            Err(())
        }
    } else {
        if let Ok(num) = s.parse::<u64>() {
            if num < 1000 {
                Ok(num * 1000)
            } else {
                Ok(num)
            }
        } else if let Ok(num) = s.parse::<f64>() {
            if num < 1000.0 {
                Ok((num * 1000.0) as u64)
            } else {
                Ok(num as u64)
            }
        } else {
            Err(())
        }
    }
}

/// Estrae il bitrate da un codec usando parametri e preset
fn get_stream_bitrate(stream: &OutStream, catalog: &Catalog) -> Option<u64> {
    match &stream.codec {
        CodecConfig::Copy => {
            None
        }
        CodecConfig::Encode { id, params } => {
            if let Some(bitrate) = extract_bitrate_from_params(params) {
                return Some(bitrate);
            }

            let codec = catalog.get_codec(id)?;

            for param in &codec.params {
                if let Some(ref default) = param.default {
                    if let Some(bitrate) = extract_bitrate_from_params(&HashMap::from([(param.id.clone(), default.clone())])) {
                        return Some(bitrate);
                    }
                }
            }

            get_default_bitrate_for_codec(id)
        }
    }
}

/// Bitrate predefiniti per codec comuni (in bps)
fn get_default_bitrate_for_codec(codec_id: &str) -> Option<u64> {
    let codec_lower = codec_id.to_lowercase();

    if codec_lower.contains("x264") || codec_lower.contains("h264") || codec_lower.contains("avc") {
        Some(2_500_000)
    } else if codec_lower.contains("x265") || codec_lower.contains("h265") || codec_lower.contains("hevc") {
        Some(1_500_000)
    } else if codec_lower.contains("libvpx") || codec_lower.contains("vp9") {
        Some(2_000_000)
    } else if codec_lower.contains("libaom") || codec_lower.contains("av1") {
        Some(1_500_000)
    } else if codec_lower.contains("prores") {
        Some(5_000_000)
    } else if codec_lower.contains("dnxhr") || codec_lower.contains("dnxhd") {
        Some(3_000_000)
    } else if codec_lower.contains("mpeg2") || codec_lower.contains("m2v") {
        Some(4_000_000)
    } else if codec_lower.contains("ffv1") {
        Some(3_000_000)
    } else if codec_lower.contains("aac") {
        Some(128_000)
    } else if codec_lower.contains("opus") || codec_lower.contains("libopus") {
        Some(96_000)
    } else if codec_lower.contains("mp3") || codec_lower.contains("libmp3lame") {
        Some(128_000)
    } else if codec_lower.contains("flac") {
        Some(700_000)
    } else if codec_lower.contains("pcm") || codec_lower.contains("wav") {
        Some(1_411_200)
    } else if codec_lower.contains("dsd") {
        Some(2_822_400)
    } else if codec_lower.contains("vorbis") || codec_lower.contains("libvorbis") {
        Some(128_000)
    } else if codec_lower.contains("sub") || codec_lower.contains("srt") || codec_lower.contains("ass") {
        Some(1000)
    } else {
        None
    }
}

/// Calcola la dimensione stimata per un singolo stream
fn estimate_stream_size(
    stream: &OutStream,
    duration_s: f64,
    catalog: &Catalog,
    probe_result: Option<&ProbeResult>,
) -> u64 {
    if let CodecConfig::Copy = &stream.codec {
        if let Some(probe) = probe_result {
            if let Some(source_stream) = probe.stream_by_index(stream.source_index) {
                if let Some(total_bitrate) = probe.bit_rate {
                    let stream_count = probe.streams.len() as f64;
                    let stream_bitrate = total_bitrate as f64 / stream_count;
                    return (stream_bitrate * duration_s / 8.0) as u64;
                }

                if let Some(avg_bitrate) = estimate_bitrate_from_codec(&source_stream.codec_name, stream.kind) {
                    return (avg_bitrate as f64 * duration_s / 8.0) as u64;
                }
            }
        }
        return match stream.kind {
            StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => (2_500_000.0 * duration_s / 8.0) as u64,
            StreamKind::Audio => (128_000.0 * duration_s / 8.0) as u64,
            _ => (1000.0 * duration_s / 8.0) as u64,
        };
    }

    if let Some(bitrate_bps) = get_stream_bitrate(stream, catalog) {
        return (bitrate_bps as f64 * duration_s / 8.0) as u64;
    }

    match stream.kind {
        StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => (2_500_000.0 * duration_s / 8.0) as u64,
        StreamKind::Audio => (128_000.0 * duration_s / 8.0) as u64,
        _ => (1000.0 * duration_s / 8.0) as u64,
    }
}

/// Stima il bitrate medio per un codec in base al nome
fn estimate_bitrate_from_codec(codec_name: &Option<String>, kind: StreamKind) -> Option<u64> {
    let codec_name = codec_name.as_deref()?.to_lowercase();

    match kind {
        StreamKind::Video => {
            if codec_name.contains("h264") || codec_name.contains("x264") || codec_name.contains("avc") {
                Some(2_500_000)
            } else if codec_name.contains("h265") || codec_name.contains("x265") || codec_name.contains("hevc") {
                Some(1_500_000)
            } else if codec_name.contains("vp9") || codec_name.contains("vpx") {
                Some(2_000_000)
            } else if codec_name.contains("av1") {
                Some(1_500_000)
            } else if codec_name.contains("prores") {
                Some(5_000_000)
            } else {
                Some(2_500_000)
            }
        }
        StreamKind::Audio => {
            if codec_name.contains("pcm") || codec_name.contains("wav") {
                Some(1_411_200)
            } else if codec_name.contains("flac") {
                Some(700_000)
            } else if codec_name.contains("aac") || codec_name.contains("mp3") {
                Some(128_000)
            } else if codec_name.contains("opus") {
                Some(96_000)
            } else if codec_name.contains("dsd") {
                Some(2_822_400)
            } else {
                Some(128_000)
            }
        }
        _ => Some(1000),
    }
}

/// Calcola la dimensione stimata totale per un job
pub fn estimate_job_size(
    job: &Job,
    catalog: &Catalog,
    probe_result: Option<&ProbeResult>,
) -> EstimatedSize {
    let duration_s = job.input.duration
        .or_else(|| probe_result.and_then(|p| p.duration_s))
        .unwrap_or(0.0);

    let mut video_bytes: u64 = 0;
    let mut audio_bytes: u64 = 0;
    let mut subtitle_bytes: u64 = 0;

    for stream in &job.output.streams {
        let size = estimate_stream_size(stream, duration_s, catalog, probe_result);

        match stream.kind {
            StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => {
                video_bytes += size;
            }
            StreamKind::Audio => {
                audio_bytes += size;
            }
            StreamKind::Subtitle => {
                subtitle_bytes += size;
            }
            StreamKind::Attachment | StreamKind::Data => {
                video_bytes += size;
            }
        }
    }

    EstimatedSize::new(video_bytes, audio_bytes, subtitle_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Input, Output, OutStream, StreamKind};
    use crate::probe::{MediaStream, ProbeResult};
    use std::path::PathBuf;

    #[test]
    fn test_stream_size_estimate_new() {
        let size = StreamSizeEstimate::new(1_048_576);
        assert_eq!(size.bytes, 1_048_576);
        assert!((size.kb - 1024.0).abs() < 0.01);
        assert!((size.mb - 1.0).abs() < 0.01);
        assert!(size.gb < 0.01);
    }

    #[test]
    fn test_stream_size_estimate_format() {
        let size = StreamSizeEstimate::new(1_048_576);
        assert_eq!(size.format(), "1.00 MB");

        let size = StreamSizeEstimate::new(1_073_741_824);
        assert_eq!(size.format(), "1.00 GB");

        let size = StreamSizeEstimate::new(500);
        assert_eq!(size.format(), "500 B");
    }

    #[test]
    fn test_extract_bitrate_from_params() {
        let mut params = HashMap::new();
        params.insert("bitrate".to_string(), serde_json::Value::from(2000));
        assert_eq!(extract_bitrate_from_params(&params), Some(2_000_000));

        let mut params = HashMap::new();
        params.insert("b".to_string(), serde_json::Value::from(1500));
        assert_eq!(extract_bitrate_from_params(&params), Some(1_500_000));

        let mut params = HashMap::new();
        params.insert("b_v".to_string(), serde_json::Value::from(2500.0));
        assert_eq!(extract_bitrate_from_params(&params), Some(2_500_000));
    }

    #[test]
    fn test_parse_bitrate_string() {
        assert_eq!(parse_bitrate_string("1000k"), Ok(1_000_000));
        assert_eq!(parse_bitrate_string("1M"), Ok(1_000_000));
        assert_eq!(parse_bitrate_string("500K"), Ok(500_000));
        assert_eq!(parse_bitrate_string("2000"), Ok(2_000_000));
        assert_eq!(parse_bitrate_string("1.5M"), Ok(1_500_000));
    }

    #[test]
    fn test_estimate_job_size_basic() {
        let catalog = Catalog::default();
        let mut job = Job::new("input.mp4", "output.mp4");

        job.output.streams.push(OutStream::new_copy(StreamKind::Video, 0));
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));

        let probe = ProbeResult {
            file: PathBuf::from("input.mp4"),
            duration_s: Some(60.0),
            format_name: Some("mp4".to_string()),
            bit_rate: Some(3_000_000),
            streams: vec![
                MediaStream {
                    index: 0,
                    kind: StreamKind::Video,
                    codec_name: Some("h264".to_string()),
                    ..Default::default()
                },
                MediaStream {
                    index: 1,
                    kind: StreamKind::Audio,
                    codec_name: Some("aac".to_string()),
                    ..Default::default()
                },
            ],
        };

        let estimate = estimate_job_size(&job, &catalog, Some(&probe));

        assert!(estimate.total.bytes > 0);
        assert!(estimate.video.bytes > 0);
        assert!(estimate.audio.bytes > 0);
    }
}
