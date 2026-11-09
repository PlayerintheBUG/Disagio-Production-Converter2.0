use crate::builtin_codecs::BUILTIN_WHISPER_SCRIPT;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub default_output_dir: Option<PathBuf>,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_whisper_model")]
    pub preferred_whisper_model: String,
    #[serde(default)]
    pub ffmpeg_path: Option<PathBuf>,
    #[serde(default)]
    pub ffprobe_path: Option<PathBuf>,
    /// Whether GPU encoding is globally enabled
    #[serde(default = "default_gpu_enabled")]
    pub gpu_enabled: bool,
    /// GPU mode: "auto", "cuda", "vaapi", "qsv", "amf"
    #[serde(default = "default_gpu_mode")]
    pub gpu_mode: String,
    /// VAAPI render device (e.g. "/dev/dri/renderD128")
    #[serde(default)]
    pub vaapi_device: Option<String>,
    /// Hardware decoding mode: "auto", "cuda", "vaapi", "qsv", "disabled"
    #[serde(default = "default_hwaccel_decode")]
    pub hwaccel_decode: String,
    /// Maximum concurrent conversion jobs
    #[serde(default = "default_parallel_jobs")]
    pub parallel_jobs: usize,
    /// Whether to automatically fallback to CPU encoding if GPU fails
    #[serde(default = "default_auto_cpu_fallback")]
    pub auto_cpu_fallback: bool,
    /// Maximum concurrent GPU encoding sessions (default 2)
    #[serde(default = "default_max_gpu_sessions")]
    pub max_gpu_sessions: usize,
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_whisper_model() -> String {
    "base".to_string()
}

fn default_gpu_enabled() -> bool {
    true
}

fn default_gpu_mode() -> String {
    "auto".to_string()
}

fn default_hwaccel_decode() -> String {
    "disabled".to_string()
}

fn default_parallel_jobs() -> usize {
    2
}

fn default_auto_cpu_fallback() -> bool {
    true
}

fn default_max_gpu_sessions() -> usize {
    2
}

fn default_preset_version() -> u32 {
    1
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            default_output_dir: None,
            log_level: default_log_level(),
            preferred_whisper_model: default_whisper_model(),
            ffmpeg_path: None,
            ffprobe_path: None,
            gpu_enabled: default_gpu_enabled(),
            gpu_mode: default_gpu_mode(),
            vaapi_device: None,
            hwaccel_decode: default_hwaccel_decode(),
            parallel_jobs: default_parallel_jobs(),
            auto_cpu_fallback: default_auto_cpu_fallback(),
            max_gpu_sessions: default_max_gpu_sessions(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPreset {
    #[serde(default = "default_preset_version")]
    pub version: u32,
    pub name: String,
    pub description: Option<String>,
    pub mode: String, // "video", "audio", "image"
    pub container: Option<String>,
    pub video_codec: Option<String>,
    pub video_preset: Option<String>,
    #[serde(default)]
    pub video_params: HashMap<String, serde_json::Value>,
    pub video_scale: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_preset: Option<String>,
    #[serde(default)]
    pub audio_params: HashMap<String, serde_json::Value>,
}

pub struct Storage;

impl Storage {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .map(|p| p.join("ffgui"))
            .unwrap_or_else(|| PathBuf::from(".config/ffgui"))
    }

    pub fn presets_dir() -> PathBuf {
        Self::config_dir().join("presets")
    }

    pub fn codecs_dir() -> PathBuf {
        Self::config_dir().join("codecs")
    }

    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .map(|p| p.join("ffgui"))
            .unwrap_or_else(|| PathBuf::from(".local/share/ffgui"))
    }

    pub fn whisper_env_dir() -> PathBuf {
        Self::data_dir().join("whisper")
    }

    pub fn cache_dir() -> PathBuf {
        dirs::cache_dir()
            .map(|p| p.join("ffgui"))
            .unwrap_or_else(|| PathBuf::from(".cache/ffgui"))
    }

    pub fn whisper_script_path() -> PathBuf {
        Self::data_dir().join("whisper_run.py")
    }

    pub fn ensure_dirs() -> Result<(), std::io::Error> {
        fs::create_dir_all(Self::config_dir())?;
        fs::create_dir_all(Self::presets_dir())?;
        fs::create_dir_all(Self::codecs_dir())?;
        fs::create_dir_all(Self::data_dir())?;
        fs::create_dir_all(Self::whisper_env_dir())?;
        fs::create_dir_all(Self::cache_dir())?;
        Ok(())
    }

    pub fn ensure_whisper_script() -> Result<PathBuf, std::io::Error> {
        Self::ensure_dirs()?;
        let path = Self::whisper_script_path();
        let needs_write = match fs::read_to_string(&path) {
            Ok(existing) => existing != BUILTIN_WHISPER_SCRIPT,
            Err(_) => true,
        };
        if needs_write {
            atomic_write(&path, BUILTIN_WHISPER_SCRIPT)?;
        }
        Ok(path)
    }

    pub fn settings_file_path() -> PathBuf {
        Self::config_dir().join("settings.json")
    }

    pub fn load_settings() -> AppSettings {
        let path = Self::settings_file_path();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                match serde_json::from_str(&content) {
                    Ok(settings) => return settings,
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to parse settings.json ({}), backing up to settings.json.bak",
                            e
                        );
                        let bak_path = Self::config_dir().join("settings.json.bak");
                        let _ = fs::copy(&path, &bak_path);
                    }
                }
            }
        }
        AppSettings::default()
    }

    pub fn save_settings(settings: &AppSettings) -> Result<(), Box<dyn std::error::Error>> {
        Self::ensure_dirs()?;
        let path = Self::settings_file_path();
        let content = serde_json::to_string_pretty(settings)?;
        atomic_write(&path, &content)?;
        Ok(())
    }

    pub fn list_presets() -> Vec<UserPreset> {
        let dir = Self::presets_dir();
        if !dir.exists() {
            return Vec::new();
        }

        let mut presets = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(preset) = serde_json::from_str::<UserPreset>(&content) {
                            presets.push(preset);
                        }
                    }
                }
            }
        }
        presets.sort_by(|a, b| a.name.cmp(&b.name));
        presets
    }

    pub fn save_preset(preset: &UserPreset) -> Result<(), Box<dyn std::error::Error>> {
        Self::ensure_dirs()?;
        let filename = preset_filename(&preset.name);
        let path = Self::presets_dir().join(filename);
        let content = serde_json::to_string_pretty(preset)?;
        atomic_write(&path, &content)?;
        Ok(())
    }

    pub fn delete_preset(name: &str) -> Result<(), std::io::Error> {
        let dir = Self::presets_dir();
        if !dir.exists() {
            return Ok(());
        }

        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(preset) = serde_json::from_str::<UserPreset>(&content) {
                            if preset.name == name {
                                let _ = fs::remove_file(path);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), std::io::Error> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write;
    temp.write_all(content.as_bytes())?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

fn preset_filename(name: &str) -> String {
    // Deterministic FNV-1a 64-bit hash
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .take(24)
        .collect();
    format!("{}_{:016x}.json", sanitized, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_serialization() {
        let settings = AppSettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.log_level, "info");
        assert_eq!(deserialized.preferred_whisper_model, "base");
    }

    #[test]
    fn test_preset_filenames_do_not_collide() {
        let f1 = preset_filename("a b");
        let f2 = preset_filename("a_b");
        assert_ne!(f1, f2, "Collision between 'a b' and 'a_b'!");
    }

    #[test]
    fn test_preset_filename_is_deterministic() {
        let name = "My 4K HDR Profile";
        let f1 = preset_filename(name);
        let f2 = preset_filename(name);
        assert_eq!(f1, f2);
        assert!(f1.starts_with("My_4K_HDR_Profile_"));
    }
}
