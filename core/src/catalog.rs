use crate::builtin_codecs::BUILTIN_CODECS;
use crate::model::StreamKind;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamType {
    Int,
    Float,
    Choice,
    Bool,
    #[serde(rename = "string", alias = "text")]
    Str,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamCondition {
    pub param: String,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
    #[serde(default, rename = "in")]
    pub in_values: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamDef {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub flag: String,
    #[serde(rename = "type")]
    pub param_type: ParamType,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub choices: Option<Vec<String>>,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub only_if: Option<ParamCondition>,
}

pub fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    if a == b {
        return true;
    }
    match (a, b) {
        (serde_json::Value::Number(n1), serde_json::Value::Number(n2)) => {
            if let (Some(f1), Some(f2)) = (n1.as_f64(), n2.as_f64()) {
                (f1 - f2).abs() < 1e-9
            } else {
                false
            }
        }
        (serde_json::Value::String(s1), serde_json::Value::String(s2)) => s1 == s2,
        _ => false,
    }
}

impl ParamDef {
    pub fn is_active(
        &self,
        current_params: &HashMap<String, serde_json::Value>,
        all_params: &[ParamDef],
    ) -> bool {
        if let Some(ref cond) = self.only_if {
            let active_val = current_params
                .get(&cond.param)
                .or_else(|| {
                    all_params
                        .iter()
                        .find(|p| p.id == cond.param)
                        .and_then(|p| p.default.as_ref())
                });

            if let Some(val) = active_val {
                if let Some(ref in_vals) = cond.in_values {
                    in_vals.iter().any(|inv| values_equal(inv, val))
                } else if let Some(ref target) = cond.value {
                    values_equal(val, target)
                } else {
                    true
                }
            } else {
                false
            }
        } else {
            true
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresetDef {
    pub id: String,
    pub label: String,
    pub values: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleCondition {
    #[serde(default)]
    pub param: String,
    #[serde(default, rename = "in")]
    pub in_values: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleConsequence {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub warn: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleDef {
    pub when: RuleCondition,
    #[serde(default)]
    pub unless: Option<RuleCondition>,
    pub then: RuleConsequence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecDef {
    pub id: String,
    pub kind: StreamKind,
    pub label: String,
    pub containers: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub hw: Option<String>,
    #[serde(default)]
    pub input_args: Vec<String>,
    #[serde(default)]
    pub filter_suffix: Option<String>,
    #[serde(default, rename = "param")]
    pub params: Vec<ParamDef>,
    #[serde(default, rename = "preset")]
    pub presets: Vec<PresetDef>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleDef>,
}

impl CodecDef {
    pub fn from_toml_str(content: &str) -> Result<Self, toml::de::Error> {
        let def: CodecDef = toml::from_str(content)?;
        Ok(def)
    }

    pub fn validate_schema(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        for param in &self.params {
            if !ids.insert(&param.id) {
                return Err(format!("Parametro duplicato '{}' nel codec '{}'", param.id, self.id));
            }
            if !param.flag.is_empty() && !param.flag.starts_with('-') {
                return Err(format!(
                    "Il flag '{}' del parametro '{}' nel codec '{}' deve iniziare con '-'",
                    param.flag, param.id, self.id
                ));
            }
            if let Some(ref def) = param.default {
                match param.param_type {
                    ParamType::Int | ParamType::Float => {
                        if let Some(num) = def.as_f64() {
                            if let Some(min) = param.min {
                                if num < min {
                                    return Err(format!(
                                        "Default {} inferiore al minimo {} per '{}' in '{}'",
                                        num, min, param.id, self.id
                                    ));
                                }
                            }
                            if let Some(max) = param.max {
                                if num > max {
                                    return Err(format!(
                                        "Default {} superiore al massimo {} per '{}' in '{}'",
                                        num, max, param.id, self.id
                                    ));
                                }
                            }
                        }
                    }
                    ParamType::Choice => {
                        if let Some(s) = def.as_str() {
                            if let Some(ref ch) = param.choices {
                                if !ch.iter().any(|c| c == s) {
                                    return Err(format!(
                                        "Default '{}' non è tra le scelte per '{}' in '{}'",
                                        s, param.id, self.id
                                    ));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if self.containers.is_empty() {
            return Err(format!("Il codec '{}' non ha specificato nessun contenitore supportato", self.id));
        }

        Ok(())
    }

    pub fn from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let def = Self::from_toml_str(&content)?;
        def.validate_schema()?;
        Ok(def)
    }

    pub fn get_param(&self, id: &str) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }

    pub fn get_preset(&self, id: &str) -> Option<&PresetDef> {
        self.presets.iter().find(|p| p.id == id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub codecs: Vec<CodecDef>,
}

impl Catalog {
    pub fn new() -> Self {
        Self { codecs: Vec::new() }
    }

    pub fn add_or_replace(&mut self, codec: CodecDef) {
        if let Some(pos) = self.codecs.iter().position(|c| c.id == codec.id) {
            self.codecs[pos] = codec;
        } else {
            self.codecs.push(codec);
        }
    }

    pub fn get_codec(&self, id: &str) -> Option<&CodecDef> {
        self.codecs.iter().find(|c| c.id == id)
    }

    pub fn by_kind(&self, kind: StreamKind) -> Vec<&CodecDef> {
        self.codecs.iter().filter(|c| c.kind == kind).collect()
    }

    pub fn default_for_kind(&self, kind: StreamKind) -> Option<&CodecDef> {
        self.by_kind(kind)
            .iter()
            .find(|c| c.is_default)
            .copied()
            .or_else(|| self.by_kind(kind).first().copied())
    }

    /// Load default catalog:
    /// 1. Embedded built-in TOMLs
    /// 2. Local `codecs/` folder (if present)
    /// 3. User config `~/.config/ffgui/codecs/` (if present)
    /// 4. Filters supported encoders via ffmpeg
    pub fn load_default(query_ffmpeg: bool) -> Self {
        let mut catalog = Self::new();

        // 1. Load embedded codecs
        for (_name, toml_str) in BUILTIN_CODECS {
            if let Ok(codec) = CodecDef::from_toml_str(toml_str) {
                catalog.add_or_replace(codec);
            }
        }

        // 2. Load from local codecs/ directory if running in repo
        let local_codecs = Path::new("codecs");
        if local_codecs.is_dir() {
            let _ = catalog.load_dir(local_codecs);
        }

        // 3. Load from ~/.config/ffgui/codecs
        if let Some(user_config) = dirs::config_dir() {
            let user_codecs = user_config.join("ffgui").join("codecs");
            if user_codecs.is_dir() {
                let _ = catalog.load_dir(&user_codecs);
            }
        }

        // 4. Query ffmpeg
        if query_ffmpeg {
            if let Ok(available) = query_available_ffmpeg_encoders() {
                catalog.codecs.retain(|c| available.contains(&c.id));
            }
        }

        catalog
    }

    /// Load codecs from a base directory, then user directory (~/.config/ffgui/codecs).
    pub fn load_from_dirs(
        builtin_dir: Option<&Path>,
        user_dir: Option<&Path>,
        query_ffmpeg: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut catalog = Self::new();

        // Start with embedded
        for (_name, toml_str) in BUILTIN_CODECS {
            if let Ok(codec) = CodecDef::from_toml_str(toml_str) {
                catalog.add_or_replace(codec);
            }
        }

        if let Some(b_dir) = builtin_dir {
            catalog.load_dir(b_dir)?;
        }

        if let Some(u_dir) = user_dir {
            if u_dir.exists() && u_dir.is_dir() {
                catalog.load_dir(u_dir)?;
            }
        }

        if query_ffmpeg {
            if let Ok(available) = query_available_ffmpeg_encoders() {
                catalog.codecs.retain(|c| available.contains(&c.id));
            }
        }

        Ok(catalog)
    }

    pub fn load_dir(&mut self, dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
            .collect();
        entries.sort();

        for path in entries {
            match CodecDef::from_file(&path) {
                Ok(codec) => self.add_or_replace(codec),
                Err(err) => eprintln!("Warning: Failed to load codec from {:?}: {}", path, err),
            }
        }

        Ok(())
    }
}

pub fn query_available_ffmpeg_encoders_with(ffmpeg_bin: &Path) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let output = Command::new(ffmpeg_bin)
        .args(["-hide_banner", "-encoders"])
        .output()?;

    if !output.status.success() {
        return Err("ffmpeg -encoders returned non-zero exit code".into());
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut encoders = HashSet::new();
    let mut in_encoders_section = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Encoders:") {
            in_encoders_section = true;
            continue;
        }
        if !in_encoders_section {
            continue;
        }

        if let Some(rest) = line.get(7..) {
            let mut parts = rest.split_whitespace();
            if let Some(enc_name) = parts.next() {
                encoders.insert(enc_name.to_string());
            }
        }
    }

    Ok(encoders)
}

pub fn query_available_ffmpeg_encoders() -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    query_available_ffmpeg_encoders_with(Path::new("ffmpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_libx265() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let toml_path = manifest_dir.join("../codecs/libx265.toml");
        let codec = CodecDef::from_file(&toml_path).expect("Failed to parse libx265.toml");
        assert_eq!(codec.id, "libx265");
        assert_eq!(codec.kind, StreamKind::Video);
        assert_eq!(codec.containers, vec!["mkv", "mp4", "mov"]);
        assert!(codec.params.iter().any(|p| p.id == "crf"));
        assert!(codec.presets.iter().any(|p| p.id == "max"));
    }

    #[test]
    fn test_all_codecs_in_directory_are_valid() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let codecs_dir = manifest_dir.join("../codecs");

        // Check that every .toml in codecs/ parses without any error
        for entry in fs::read_dir(&codecs_dir).expect("read_dir codecs") {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                let codec = CodecDef::from_file(&path)
                    .unwrap_or_else(|e| panic!("Failed to parse {}: {}", path.display(), e));
                codec.validate_schema()
                    .unwrap_or_else(|e| panic!("Schema error in {}: {}", codec.id, e));
            }
        }

        let catalog = Catalog::load_from_dirs(Some(&codecs_dir), None, false)
            .expect("Catalog load_from_dirs failed");

        assert!(!catalog.codecs.is_empty(), "Should load at least one codec");

        for codec in &catalog.codecs {
            codec.validate_schema().unwrap_or_else(|e| panic!("Schema error in {}: {}", codec.id, e));

            for preset in &codec.presets {
                for (param_id, val) in &preset.values {
                    let param = codec
                        .get_param(param_id)
                        .unwrap_or_else(|| panic!("Preset '{}' in '{}' references unknown param '{}'", preset.id, codec.id, param_id));

                    if let Some(num) = val.as_f64() {
                        if let Some(min) = param.min {
                            assert!(num >= min, "Preset '{}' in '{}' param '{}' value {} < min {}", preset.id, codec.id, param_id, num, min);
                        }
                        if let Some(max) = param.max {
                            assert!(num <= max, "Preset '{}' in '{}' param '{}' value {} > max {}", preset.id, codec.id, param_id, num, max);
                        }
                    }

                    if let Some(s) = val.as_str() {
                        if let Some(ref choices) = param.choices {
                            assert!(choices.iter().any(|c| c == s), "Preset '{}' in '{}' param '{}' value '{}' not in choices: {:?}", preset.id, codec.id, param_id, s, choices);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_encode_synthetic_sample_for_each_codec() {
        let catalog = Catalog::load_default(true);
        let temp_dir = tempfile::tempdir().expect("tempdir");

        for codec in &catalog.codecs {
            // Hardware/GPU encoders require dedicated device initialization and availability
            // and are tested separately in backends::gpu tests.
            if codec.hw.is_some() {
                continue;
            }

            let container = codec.containers.first().map(|s| s.as_str()).unwrap_or("mkv");
            let out_file = temp_dir.path().join(format!("out_{}.{}", codec.id, container));

            let mut args = vec![
                "-hide_banner".to_string(),
                "-y".to_string(),
                "-f".to_string(),
                "lavfi".to_string(),
            ];

            match codec.kind {
                StreamKind::Video => {
                    args.push("-i".to_string());
                    args.push("testsrc=duration=0.2:size=160x120:rate=10".to_string());
                    args.push("-c:v".to_string());
                    args.push(codec.id.clone());
                    if codec.id.starts_with("prores") {
                        args.push("-pix_fmt".to_string());
                        args.push("yuv422p10le".to_string());
                    }
                }
                StreamKind::Audio => {
                    args.push("-i".to_string());
                    args.push("sine=duration=0.2:frequency=440".to_string());
                    args.push("-c:a".to_string());
                    args.push(codec.id.clone());
                }
                StreamKind::Image => {
                    args.push("-i".to_string());
                    args.push("testsrc=duration=0.1:size=160x120:rate=10".to_string());
                    args.push("-frames:v".to_string());
                    args.push("1".to_string());
                    args.push("-c:v".to_string());
                    args.push(codec.id.clone());
                }
                _ => continue,
            }

            args.push(out_file.to_string_lossy().to_string());

            let status = std::process::Command::new("ffmpeg")
                .args(&args)
                .status();

            if let Ok(st) = status {
                assert!(st.success(), "Failed encoding synthetic test sample for codec: {}", codec.id);
            }
        }
    }
}
