use crate::runner::{Event, ProcessController, Runner};
use crate::storage::Storage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

pub struct FfmpegBackend;

impl FfmpegBackend {
    /// Builds ffmpeg arguments to extract audio to a 16kHz mono 16-bit PCM WAV.
    /// `audio_track` is the 0-based index among audio streams (0 = first audio stream).
    pub fn build_extract_audio_args(
        input_file: &Path,
        output_wav: &Path,
        audio_track: usize,
    ) -> Vec<String> {
        vec![
            "-hide_banner".to_string(),
            "-y".to_string(),
            "-i".to_string(),
            input_file.to_string_lossy().to_string(),
            "-map".to_string(),
            format!("0:a:{}", audio_track),
            "-vn".to_string(),
            "-ac".to_string(),
            "1".to_string(),
            "-ar".to_string(),
            "16000".to_string(),
            "-c:a".to_string(),
            "pcm_s16le".to_string(),
            output_wav.to_string_lossy().to_string(),
        ]
    }

    /// Converts 2-letter language code (ISO 639-1) or name to 3-letter (ISO 639-2) code
    /// compatible with MP4, MKV and other containers.
    pub fn iso639_2(code: &str) -> String {
        let clean = code.trim().to_lowercase();
        match clean.as_str() {
            "it" | "ita" | "italian" => "ita",
            "en" | "eng" | "english" => "eng",
            "fr" | "fra" | "fre" | "french" => "fra",
            "de" | "deu" | "ger" | "german" => "deu",
            "es" | "spa" | "spanish" => "spa",
            "pt" | "por" | "portuguese" => "por",
            "ru" | "rus" | "russian" => "rus",
            "ja" | "jpn" | "japanese" => "jpn",
            "zh" | "zho" | "chi" | "chinese" => "zho",
            "ko" | "kor" | "korean" => "kor",
            "ar" | "ara" | "arabic" => "ara",
            "nl" | "nld" | "dut" | "dutch" => "nld",
            "pl" | "pol" | "polish" => "pol",
            "sv" | "swe" | "swedish" => "swe",
            "tr" | "tur" | "turkish" => "tur",
            "uk" | "ukr" | "ukrainian" => "ukr",
            "vi" | "vie" | "vietnamese" => "vie",
            "el" | "ell" | "gre" | "greek" => "ell",
            "cs" | "ces" | "cze" | "czech" => "ces",
            "da" | "dan" | "danish" => "dan",
            "fi" | "fin" | "finnish" => "fin",
            "he" | "heb" | "hebrew" => "heb",
            "hi" | "hin" | "hindi" => "hin",
            "hr" | "hrv" | "croatian" => "hrv",
            "hu" | "hun" | "hungarian" => "hun",
            "id" | "ind" | "indonesian" => "ind",
            "ms" | "msa" | "may" | "malay" => "msa",
            "no" | "nor" | "norwegian" | "nb" | "nn" => "nor",
            "ro" | "ron" | "rum" | "romanian" => "ron",
            "sk" | "slk" | "slo" | "slovak" => "slk",
            "bg" | "bul" | "bulgarian" => "bul",
            "ca" | "cat" | "catalan" => "cat",
            "sr" | "srp" | "serbian" => "srp",
            "sl" | "slv" | "slovenian" => "slv",
            "th" | "tha" | "thai" => "tha",
            other => other,
        }
        .to_string()
    }

    /// Extracts an audio track to a 16kHz mono 16-bit PCM WAV for Whisper transcription:
    /// ffmpeg -vn -ac 1 -ar 16000 -c:a pcm_s16le <output.wav>
    pub fn extract_audio_for_whisper(
        input_file: &Path,
        output_wav: &Path,
        audio_track: usize,
        duration_s: Option<f64>,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let args = Self::build_extract_audio_args(input_file, output_wav, audio_track);
        Runner::start_ffmpeg(args, duration_s)
    }

    /// Builds arguments to embed subtitles as a separate track (soft subtitles).
    pub fn build_mux_subtitles_args(
        video_file: &Path,
        subtitles_file: &Path,
        output_file: &Path,
        language: Option<&str>,
        is_default: bool,
    ) -> Vec<String> {
        let out_ext = output_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mkv")
            .to_lowercase();

        let sub_codec = match out_ext.as_str() {
            "mp4" | "m4v" | "mov" => "mov_text",
            _ => "srt",
        };

        let mut args = vec![
            "-hide_banner".to_string(),
            "-y".to_string(),
            "-i".to_string(),
            video_file.to_string_lossy().to_string(),
            "-i".to_string(),
            subtitles_file.to_string_lossy().to_string(),
            "-map".to_string(),
            "0:v?".to_string(),
            "-map".to_string(),
            "0:a?".to_string(),
            "-map".to_string(),
            "1:0".to_string(),
            "-c:v".to_string(),
            "copy".to_string(),
            "-c:a".to_string(),
            "copy".to_string(),
            "-c:s:0".to_string(),
            sub_codec.to_string(),
        ];

        if let Some(lang) = language {
            let clean = lang.trim();
            if !clean.is_empty() && clean != "auto" {
                let code_3 = Self::iso639_2(clean);
                args.push("-metadata:s:s:0".to_string());
                args.push(format!("language={}", code_3));
            }
        }

        if is_default {
            args.push("-disposition:s:s:0".to_string());
            args.push("default".to_string());
        }

        args.push(output_file.to_string_lossy().to_string());
        args
    }

    /// Embed subtitles as a separate track (soft subtitles).
    pub fn mux_subtitles_track(
        video_file: &Path,
        subtitles_file: &Path,
        output_file: &Path,
        language: Option<&str>,
        is_default: bool,
        duration_s: Option<f64>,
    ) -> Result<(ProcessController, Receiver<Event>), std::io::Error> {
        let args = Self::build_mux_subtitles_args(
            video_file,
            subtitles_file,
            output_file,
            language,
            is_default,
        );
        Runner::start_ffmpeg(args, duration_s)
    }

    /// Prepares a safe subtitle file path in cache to avoid fragile filter escaping issues
    /// (e.g. single quotes, colons, brackets or non-ascii filenames).
    pub fn prepare_safe_subtitle_file(
        original_path: &Path,
        unique_id: &str,
    ) -> Result<PathBuf, std::io::Error> {
        let _ = Storage::ensure_dirs();
        let ext = original_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("srt");
        let safe_path = Storage::cache_dir().join(format!("sub_burn_safe_{}.{}", unique_id, ext));
        fs::copy(original_path, &safe_path)?;
        Ok(safe_path)
    }

    /// Escape file path for use inside ffmpeg filter string (e.g. subtitles='...')
    pub fn escape_filter_path(path: &Path) -> String {
        let path_str = path.to_string_lossy();
        let mut escaped = String::with_capacity(path_str.len() * 2);

        for ch in path_str.chars() {
            match ch {
                '\\' => escaped.push_str(r"\\"),
                '\'' => escaped.push_str(r"'\'"),
                ':' => escaped.push_str(r"\:"),
                '[' => escaped.push_str(r"\["),
                ']' => escaped.push_str(r"\]"),
                ';' => escaped.push_str(r"\;"),
                ',' => escaped.push_str(r"\,"),
                '=' => escaped.push_str(r"\="),
                _ => escaped.push(ch),
            }
        }

        escaped
    }

    /// Constructs filter for burning subtitles into video.
    pub fn burn_subtitles_filter(subtitles_file: &Path) -> String {
        let escaped = Self::escape_filter_path(subtitles_file);
        format!("subtitles='{}'", escaped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_filter_path() {
        let path = PathBuf::from("/home/user/my file [test]: 1.srt");
        let escaped = FfmpegBackend::escape_filter_path(&path);
        assert!(escaped.contains(r"\:"));
        assert!(escaped.contains(r"\["));
        assert!(escaped.contains(r"\]"));
    }

    #[test]
    fn test_burn_subtitles_filter_with_apostrophe_safe() {
        let temp = tempfile::tempdir().unwrap();
        let sub_file = temp.path().join("it's cool.srt");
        fs::write(&sub_file, "1\n00:00:01,000 --> 00:00:02,000\nTest\n\n").unwrap();

        let safe = FfmpegBackend::prepare_safe_subtitle_file(&sub_file, "123").unwrap();
        let filter = FfmpegBackend::burn_subtitles_filter(&safe);
        assert!(!filter.contains("it's cool"));
        assert!(filter.contains("sub_burn_safe_123.srt"));
    }

    #[test]
    fn test_iso639_2() {
        assert_eq!(FfmpegBackend::iso639_2("it"), "ita");
        assert_eq!(FfmpegBackend::iso639_2("en"), "eng");
        assert_eq!(FfmpegBackend::iso639_2("ita"), "ita");
        assert_eq!(FfmpegBackend::iso639_2("de"), "deu");
    }

    #[test]
    fn test_build_extract_audio_args() {
        let in_path = Path::new("test.mkv");
        let out_path = Path::new("audio.wav");
        let args = FfmpegBackend::build_extract_audio_args(in_path, out_path, 2);
        let joined = args.join(" ");
        assert!(joined.contains("-map 0:a:2"));
        assert!(joined.contains("-c:a pcm_s16le"));
    }
}
