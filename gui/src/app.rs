use ffgui_core::backends::ffmpeg::FfmpegBackend;
use ffgui_core::backends::gpu::GpuDetectionResult;
use ffgui_core::backends::whisper::WhisperBackend;
use ffgui_core::batch::{
    BatchPlan, BatchProfile, BatchSettings, DestSettings, MediaKind,
    OnErrorPolicy, RunOptions, SourceSettings,
};
use ffgui_core::builder::CommandBuilder;
use ffgui_core::catalog::Catalog;
use ffgui_core::model::{CodecConfig, Job, OutStream, StreamKind};
use ffgui_core::probe::{MediaStream, ProbeResult};
use ffgui_core::queue::{JobQueue, StepKind, StepStatus, TranscribeJob};
use ffgui_core::runner::{Event, ProcessController, Runner};
use ffgui_core::storage::{AppSettings, Storage, UserPreset};
use ffgui_core::validate::validate_job;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainTab {
    #[default]
    SingleFile,
    Batch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Video,
    Audio,
    Image,
    Subtitles,
}

#[derive(Debug, Clone)]
pub struct StreamControlState {
    pub enabled: bool,
    pub source_index: usize,
    pub kind: StreamKind,
    pub codec_id: String, // "copy" or codec id like "libx264"
    pub selected_preset: Option<String>,
    pub params: HashMap<String, serde_json::Value>,
    pub scale_preset: String,
    pub custom_width: u32,
    pub custom_height: u32,
}

impl StreamControlState {
    pub fn new_default(stream: &MediaStream, catalog: &Catalog) -> Self {
        let (codec_id, params, selected_preset) = match stream.kind {
            StreamKind::Video => {
                let default_codec = catalog
                    .default_for_kind(StreamKind::Video)
                    .map(|c| c.id.clone())
                    .unwrap_or_else(|| "libx264".to_string());
                let mut p = HashMap::new();
                let mut sel_pre = None;
                if let Some(c) = catalog.get_codec(&default_codec) {
                    for param in &c.params {
                        if let Some(ref def) = param.default {
                            p.insert(param.id.clone(), def.clone());
                        }
                    }
                    if let Some(first_preset) = c.presets.first() {
                        sel_pre = Some(first_preset.id.clone());
                        for (k, v) in &first_preset.values {
                            p.insert(k.clone(), v.clone());
                        }
                    }
                }
                (default_codec, p, sel_pre)
            }
            StreamKind::Audio => {
                // Audio defaults to copy to avoid unwanted re-encoding
                ("copy".to_string(), HashMap::new(), None)
            }
            _ => ("copy".to_string(), HashMap::new(), None),
        };

        Self {
            enabled: true,
            source_index: stream.index,
            kind: stream.kind,
            codec_id,
            selected_preset,
            params,
            scale_preset: "Originale".to_string(),
            custom_width: stream.width.unwrap_or(1920),
            custom_height: stream.height.unwrap_or(1080),
        }
    }

    pub fn apply_preset(&mut self, preset_id: &str, catalog: &Catalog) {
        if let Some(codec) = catalog.get_codec(&self.codec_id) {
            if let Some(preset) = codec.get_preset(preset_id) {
                self.selected_preset = Some(preset_id.to_string());
                for (k, v) in &preset.values {
                    self.params.insert(k.clone(), v.clone());
                }
            }
        }
    }

    pub fn change_codec(&mut self, new_codec_id: &str, catalog: &Catalog) {
        self.codec_id = new_codec_id.to_string();
        self.selected_preset = None;
        self.params.clear();

        if let Some(codec) = catalog.get_codec(new_codec_id) {
            for param in &codec.params {
                if let Some(ref def) = param.default {
                    self.params.insert(param.id.clone(), def.clone());
                }
            }
            // Auto-apply first preset if available
            if let Some(first_preset) = codec.presets.first() {
                self.apply_preset(&first_preset.id, catalog);
            }
        }
    }
}

pub struct SubtitleOptions {
    pub mode: SubtitleMode,
    pub model: String,
    pub language: String,
    pub device: String,
    pub compute_type: String,
    pub vad_filter: bool,
    pub custom_sub_file: Option<PathBuf>,
    pub selected_audio_track: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleMode {
    ExtractAndTranscribeTrack,
    BurnIntoVideo,
    TranscribeOnly,
}

impl Default for SubtitleOptions {
    fn default() -> Self {
        Self {
            mode: SubtitleMode::ExtractAndTranscribeTrack,
            model: "base".to_string(),
            language: "auto".to_string(),
            device: "cpu".to_string(),
            compute_type: "default".to_string(),
            vad_filter: true,
            custom_sub_file: None,
            selected_audio_track: 0,
        }
    }
}

pub struct RunningTask {
    pub item_id: Option<usize>,
    pub desc: String,
    pub controller: ProcessController,
    pub rx: Receiver<Event>,
    pub part_file: Option<(PathBuf, PathBuf)>,
    pub uses_gpu: bool,
    pub progress: f64,
    pub speed: Option<f64>,
    pub fps: Option<f64>,
}

pub type BatchScanResult = (Vec<(String, usize)>, BatchPlan);

pub struct FfGuiApp {
    pub catalog: Catalog,
    pub settings: AppSettings,
    pub mode: AppMode,
    pub input_path: Option<PathBuf>,
    pub output_path: Option<PathBuf>,
    pub probe_result: Option<ProbeResult>,
    pub probe_error: Option<String>,
    pub stream_controls: Vec<StreamControlState>,

    // Trim options
    pub start_time: Option<f64>,
    pub duration: Option<f64>,

    // Subtitle options & status
    pub subtitle_options: SubtitleOptions,
    pub whisper_env_ready: Arc<AtomicBool>,
    pub is_whisper_setup_running: bool,

    // Async probe
    pub is_probing: bool,
    pub rx_probe: Option<Receiver<Result<ProbeResult, String>>>,

    // Atomic output .part handling
    pub current_part_file: Option<(PathBuf, PathBuf)>,

    // GPU detection
    pub gpu_detection: Option<GpuDetectionResult>,
    pub is_detecting_gpu: bool,
    pub rx_gpu_detection: Option<Receiver<GpuDetectionResult>>,

    // Navigation tab
    pub active_tab: MainTab,

    // Modals / Windows
    pub show_settings_window: bool,
    pub show_log_panel: bool,

    // Batch v2 (Section 12)
    pub batch_settings: BatchSettings,
    pub batch_plan_v2: Option<BatchPlan>,
    pub batch_discovered_exts: Vec<(String, usize)>,
    pub is_batch_scanning: bool,
    pub rx_batch_scan: Option<Receiver<BatchScanResult>>,

    // Queue & Execution
    pub queue: JobQueue,
    pub is_queue_running: bool,
    pub active_queue_item_id: Option<usize>,
    pub is_running: bool,
    pub runner_controller: Option<ProcessController>,
    pub rx_events: Option<Receiver<Event>>,
    pub running_tasks: Vec<RunningTask>,
    pub selected_log_job: Option<usize>,
    pub logs: Vec<(String, bool)>, // (line, is_stderr)
    pub current_progress: Option<f64>,
    pub current_time_s: Option<f64>,
    pub current_speed: Option<f64>,
    pub current_fps: Option<f64>,
    pub exit_status: Option<Result<i32, String>>,
    pub auto_scroll_log: bool,
    pub log_filter: String,

    // Vista del comando (compatto / esatto) e lampeggio dei pezzi appena cambiati
    pub cmd_view_mode: crate::ui::preview_panel::CmdViewMode,
    pub cmd_prev_tokens: Vec<String>,
    pub cmd_changed_flags: Vec<bool>,
    pub cmd_changed_at: Option<std::time::Instant>,

    pub toast_message: Option<(String, std::time::Instant)>,

    // User Presets
    pub user_presets: Vec<UserPreset>,
    pub new_preset_name: String,
}

impl FfGuiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let _ = Storage::ensure_dirs();
        let settings = Storage::load_settings();

        // Section 7: Global spacing, solid scrollbars and comfortable hitboxes
        cc.egui_ctx.style_mut(|s| {
            s.spacing.scroll = eframe::egui::style::ScrollStyle::solid();
            s.spacing.interact_size.y = 26.0;
            s.spacing.button_padding = eframe::egui::vec2(10.0, 5.0);
            s.spacing.combo_height = 360.0;
        });

        // Load catalog from embedded codecs and user config
        let catalog = Catalog::load_default(false);
        let user_presets = Storage::list_presets();

        let whisper_env_ready = Arc::new(AtomicBool::new(false));
        let ready_clone = whisper_env_ready.clone();
        thread::spawn(move || {
            let ready = WhisperBackend::is_env_ready();
            ready_clone.store(ready, Ordering::SeqCst);
        });

        let (tx_gpu, rx_gpu) = std::sync::mpsc::channel();
        let ffmpeg_prog = settings
            .ffmpeg_path
            .as_ref()
            .and_then(|p| p.to_str())
            .unwrap_or("ffmpeg")
            .to_string();
        thread::spawn(move || {
            let res = ffgui_core::backends::gpu::GpuBackend::detect_all(&ffmpeg_prog);
            let _ = tx_gpu.send(res);
        });

        let mut profiles = BTreeMap::new();
        profiles.insert(MediaKind::Video, BatchProfile::archive_video(None));
        profiles.insert(MediaKind::Audio, BatchProfile::default_audio());
        profiles.insert(MediaKind::Image, BatchProfile::default_image());

        let mut active_exts = HashSet::new();
        for ext in &["mp4", "mkv", "mov", "avi", "webm", "flac", "mp3", "wav", "m4a", "ogg", "opus", "jpg", "jpeg", "png", "webp"] {
            active_exts.insert(ext.to_string());
        }

        let batch_settings = BatchSettings {
            source: SourceSettings {
                folder: PathBuf::new(),
                recursive: true,
                active_exts,
            },
            dest: DestSettings::default(),
            profiles,
            overrides: HashMap::new(),
            run: RunOptions {
                parallel_jobs: settings.parallel_jobs,
                max_gpu_sessions: settings.max_gpu_sessions,
                on_error: OnErrorPolicy::Continue,
                generate_report: true,
            },
        };

        Self {
            active_tab: MainTab::SingleFile,
            catalog,
            settings,
            mode: AppMode::Video,
            input_path: None,
            output_path: None,
            probe_result: None,
            probe_error: None,
            stream_controls: Vec::new(),
            start_time: None,
            duration: None,
            subtitle_options: SubtitleOptions::default(),
            whisper_env_ready,
            is_whisper_setup_running: false,
            is_probing: false,
            rx_probe: None,
            current_part_file: None,
            gpu_detection: None,
            is_detecting_gpu: true,
            rx_gpu_detection: Some(rx_gpu),
            show_settings_window: false,
            show_log_panel: true,
            batch_settings,
            batch_plan_v2: None,
            batch_discovered_exts: Vec::new(),
            is_batch_scanning: false,
            rx_batch_scan: None,
            queue: JobQueue::new(),
            is_queue_running: false,
            active_queue_item_id: None,
            is_running: false,
            runner_controller: None,
            rx_events: None,
            running_tasks: Vec::new(),
            selected_log_job: None,
            logs: Vec::new(),
            current_progress: None,
            current_time_s: None,
            current_speed: None,
            current_fps: None,
            exit_status: None,
            auto_scroll_log: true,
            log_filter: String::new(),
            cmd_view_mode: crate::ui::preview_panel::CmdViewMode::Compact,
            cmd_prev_tokens: Vec::new(),
            cmd_changed_flags: Vec::new(),
            cmd_changed_at: None,
            toast_message: None,
            user_presets,
            new_preset_name: String::new(),
        }
    }

    pub fn ffmpeg_bin(&self) -> PathBuf {
        self.settings
            .ffmpeg_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("ffmpeg"))
    }

    pub fn ffprobe_bin(&self) -> PathBuf {
        self.settings
            .ffprobe_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("ffprobe"))
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast_message = Some((msg.into(), std::time::Instant::now()));
    }

    pub fn set_mode(&mut self, new_mode: AppMode) {
        if self.mode == new_mode {
            return;
        }
        self.mode = new_mode;

        // 1. Update output extension according to mode
        let ext = match new_mode {
            AppMode::Video | AppMode::Subtitles => "mkv",
            AppMode::Audio => "mp3",
            AppMode::Image => "webp",
        };
        if let Some(ref mut out) = self.output_path {
            out.set_extension(ext);
        }

        // 2. Adjust enabled streams based on mode (Bug 3.8)
        let mut first_video_seen = false;
        for ctrl in &mut self.stream_controls {
            ctrl.enabled = match new_mode {
                AppMode::Video | AppMode::Subtitles => {
                    matches!(ctrl.kind, StreamKind::Video | StreamKind::Audio | StreamKind::Subtitle | StreamKind::CoverArt)
                }
                AppMode::Audio => {
                    let caps = ffgui_core::containers::container_caps(ext);
                    ctrl.kind == StreamKind::Audio || (ctrl.kind == StreamKind::CoverArt && caps.cover_art)
                }
                AppMode::Image => {
                    if ctrl.kind == StreamKind::Video || ctrl.kind == StreamKind::Image {
                        if !first_video_seen {
                            first_video_seen = true;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
            };
        }

        // 3. Adjust default codecs
        match new_mode {
            AppMode::Video => {
                for ctrl in &mut self.stream_controls {
                    if ctrl.kind == StreamKind::Video {
                        // If it was switched to libwebp, restore default video codec
                        if ctrl.codec_id == "libwebp" {
                            let def_id = self.catalog.default_for_kind(StreamKind::Video)
                                .map(|c| c.id.clone())
                                .unwrap_or_else(|| "libx264".to_string());
                            ctrl.change_codec(&def_id, &self.catalog);
                        }
                    }
                }
            }
            AppMode::Audio => {
                for ctrl in &mut self.stream_controls {
                    if ctrl.kind == StreamKind::Audio && ctrl.codec_id == "copy" {
                        if let Some(def_codec) = self.catalog.default_for_kind(StreamKind::Audio) {
                            ctrl.change_codec(&def_codec.id, &self.catalog);
                        }
                    }
                }
            }
            AppMode::Image => {
                for ctrl in &mut self.stream_controls {
                    if (ctrl.kind == StreamKind::Video || ctrl.kind == StreamKind::Image) && ctrl.enabled {
                        if let Some(def_codec) = self.catalog.default_for_kind(StreamKind::Image) {
                            ctrl.change_codec(&def_codec.id, &self.catalog);
                        }
                    }
                }
            }
            AppMode::Subtitles => {}
        }
    }

    pub fn open_file(&mut self, path: PathBuf) {
        self.input_path = Some(path.clone());
        self.probe_error = None;
        self.probe_result = None;
        self.stream_controls.clear();
        self.is_probing = true;

        let (tx, rx) = std::sync::mpsc::channel();
        self.rx_probe = Some(rx);

        let ffprobe_prog = self.ffprobe_bin();
        thread::spawn(move || {
            let res = ffgui_core::probe::probe_file_with_program(&ffprobe_prog, &path).map_err(|e| e.to_string());
            let _ = tx.send(res);
        });
    }

    pub fn poll_probe(&mut self) {
        if let Some(ref rx) = self.rx_probe {
            if let Ok(res) = rx.try_recv() {
                self.is_probing = false;
                self.rx_probe = None;
                match res {
                    Ok(probe) => {
                        self.probe_error = None;
                        if let Some(ref path) = self.input_path {
                            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
                            let parent = self
                                .settings
                                .default_output_dir
                                .clone()
                                .unwrap_or_else(|| path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf());

                            let ext = match self.mode {
                                AppMode::Video | AppMode::Subtitles => "mkv",
                                AppMode::Audio => "mp3",
                                AppMode::Image => "webp",
                            };

                            let out_name = format!("{}_converted.{}", stem, ext);
                            self.output_path = Some(parent.join(out_name));
                        }

                        // Initialize stream controls
                        self.stream_controls = probe
                            .streams
                            .iter()
                            .map(|s| StreamControlState::new_default(s, &self.catalog))
                            .collect();

                        // Apply mode-specific enabling
                        let mode = self.mode;
                        let mut first_video = false;
                        for ctrl in &mut self.stream_controls {
                            ctrl.enabled = match mode {
                                AppMode::Video | AppMode::Subtitles => {
                                    matches!(ctrl.kind, StreamKind::Video | StreamKind::Audio | StreamKind::Subtitle | StreamKind::CoverArt)
                                }
                                AppMode::Audio => {
                                    matches!(ctrl.kind, StreamKind::Audio | StreamKind::CoverArt)
                                }
                                AppMode::Image => {
                                    if ctrl.kind == StreamKind::Video || ctrl.kind == StreamKind::Image {
                                        if !first_video {
                                            first_video = true;
                                            true
                                        } else {
                                            false
                                        }
                                    } else {
                                        false
                                    }
                                }
                            };
                        }

                        self.probe_result = Some(probe);
                    }
                    Err(e) => {
                        self.probe_error = Some(format!("Errore analisi file ffprobe: {}", e));
                    }
                }
            }
        }
    }

    pub fn poll_gpu_detection(&mut self) {
        if let Some(ref rx) = self.rx_gpu_detection {
            if let Ok(res) = rx.try_recv() {
                self.is_detecting_gpu = false;
                self.rx_gpu_detection = None;
                self.gpu_detection = Some(res);
            }
        }
    }

    pub fn build_current_job(&self) -> Option<Job> {
        self.build_job_with_quality().map(|(job, _)| job)
    }

    /// Come `build_current_job`, ma ritorna anche le righe di riepilogo della
    /// qualità applicata (modalità Preset/Fissa/Auto) per il log.
    pub fn build_job_with_quality(&self) -> Option<(Job, Vec<String>)> {
        let input_path = self.input_path.as_ref()?;
        let output_path = self.output_path.as_ref()?;

        let mut job = Job::new(input_path, output_path);
        job.input.start_time = self.start_time;
        job.input.duration = self.duration;

        for ctrl in &self.stream_controls {
            if !ctrl.enabled {
                continue;
            }

            // In Audio mode, skip video streams
            if self.mode == AppMode::Audio && (ctrl.kind == StreamKind::Video || ctrl.kind == StreamKind::Image) {
                continue;
            }

            // In Image mode, mark video as Image stream so builder emits -frames:v 1
            let stream_kind = if self.mode == AppMode::Image && ctrl.kind == StreamKind::Video {
                StreamKind::Image
            } else {
                ctrl.kind
            };

            let mut out_stream = match ctrl.codec_id.as_str() {
                "copy" => OutStream::new_copy(stream_kind, ctrl.source_index),
                codec_id => OutStream::new_encode(
                    stream_kind,
                    ctrl.source_index,
                    codec_id,
                    ctrl.params.clone(),
                ),
            };

            // Proportional scaling (height fixed, proportional width) to never deform aspect ratio
            if ctrl.scale_preset == "1080p (altezza 1080)" || ctrl.scale_preset == "1080p (1920x1080)" {
                out_stream.filters.push("scale=-2:1080".into());
            } else if ctrl.scale_preset == "720p (altezza 720)" || ctrl.scale_preset == "720p (1280x720)" {
                out_stream.filters.push("scale=-2:720".into());
            } else if ctrl.scale_preset == "480p (altezza 480)" || ctrl.scale_preset == "480p (854x480)" {
                out_stream.filters.push("scale=-2:480".into());
            } else if ctrl.scale_preset == "Personalizzato" {
                out_stream.filters.push(format!("scale={}:{}", ctrl.custom_width, ctrl.custom_height));
            }

            job.output.streams.push(out_stream);
        }

        let mut quality_summaries = Vec::new();
        if let Some(probe) = self.probe_result.as_ref() {
            let src_size = std::fs::metadata(input_path).ok().map(|m| m.len());
            quality_summaries = ffgui_core::batch::apply_quality_to_job(
                &mut job,
                &self.settings.video_quality,
                &self.settings.audio_quality,
                Some(probe),
                src_size,
            );
        }

        Some((job, quality_summaries))
    }

    pub fn is_stream_hw_encoded(&self, stream: &OutStream) -> bool {
        match &stream.codec {
            CodecConfig::Encode { id, .. } => {
                self.catalog.get_codec(id).map(|c| c.hw.is_some()).unwrap_or(false)
            }
            _ => false,
        }
    }

    pub fn build_job_args(&self, job: &Job) -> Vec<String> {
        let hwaccel = if self.settings.hwaccel_decode == "disabled" || self.settings.hwaccel_decode == "none" {
            None
        } else if self.settings.hwaccel_decode == "auto" {
            let uses_gpu = job.output.streams.iter().any(|s| self.is_stream_hw_encoded(s));
            let det_has_gpu = self.gpu_detection.as_ref().map(|d| d.has_any_hw_encoder()).unwrap_or(false);
            if uses_gpu || det_has_gpu {
                Some("auto")
            } else {
                None
            }
        } else {
            Some(self.settings.hwaccel_decode.as_str())
        };

        let mut args = CommandBuilder::build_with_options(
            job,
            &self.catalog,
            self.settings.vaapi_device.as_deref(),
            hwaccel,
        );

        if !args.iter().any(|a| a == "-loglevel") {
            let lvl = if self.settings.log_level.is_empty() {
                "info".to_string()
            } else {
                self.settings.log_level.clone()
            };
            args.insert(0, lvl);
            args.insert(0, "-loglevel".to_string());
        }

        args
    }

    /// Prepares atomic .part output path, creating parent directory on disk (A1).
    pub fn prepare_output_part(job: &mut Job) -> std::io::Result<(PathBuf, PathBuf)> {
        let final_path = job.output.path.clone();
        let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        let stem = final_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
        let ext = final_path.extension().and_then(|s| s.to_str()).unwrap_or("mkv");
        let part = parent.join(format!("{}.part.{}", stem, ext));
        job.output.path = part.clone();
        Ok((part, final_path))
    }

    pub fn begin_output(&mut self, job: &mut Job) -> std::io::Result<(PathBuf, PathBuf)> {
        let (part, final_path) = Self::prepare_output_part(job)?;
        self.current_part_file = Some((part.clone(), final_path.clone()));
        Ok((part, final_path))
    }

    pub fn start_current_job(&mut self) {
        if self.is_running {
            return;
        }

        let Some((mut job, quality_lines)) = self.build_job_with_quality() else {
            return;
        };

        let issues = validate_job(&job, &self.catalog);
        if let Some(err) = issues.iter().find(|i| i.is_error()) {
            self.set_toast(format!("Impossibile avviare: {}", err.message));
            self.logs.push((format!("ERRORE VALIDAZIONE: {}", err.message), true));
            return;
        }

        let (part_path, final_dest) = match self.begin_output(&mut job) {
            Ok(res) => res,
            Err(e) => {
                self.set_toast(format!("Impossibile creare cartella di output: {}", e));
                self.logs.push((format!("ERRORE I/O: {}", e), true));
                return;
            }
        };

        let args = self.build_job_args(&job);
        let total_dur = if let Some(d) = self.duration {
            Some(d)
        } else if let Some(probe_dur) = self.probe_result.as_ref().and_then(|p| p.duration_s) {
            if let Some(ss) = self.start_time {
                Some((probe_dur - ss).max(0.1))
            } else {
                Some(probe_dur)
            }
        } else {
            None
        };

        let uses_gpu = job.output.streams.iter().any(|s| self.is_stream_hw_encoded(s));

        let cmd_str = format!("$ {} {}", self.ffmpeg_bin().display(), args.join(" "));
        match Runner::start_ffmpeg_with_program(&self.ffmpeg_bin(), args, total_dur) {
            Ok((controller, rx)) => {
                self.is_running = true;
                self.active_queue_item_id = None;
                self.logs.clear();
                self.logs.push((cmd_str, false));
                for line in &quality_lines {
                    self.logs.push((line.clone(), false));
                }
                self.current_progress = Some(0.0);
                self.current_speed = None;
                self.current_fps = None;
                self.exit_status = None;

                let task = RunningTask {
                    item_id: None,
                    desc: format!("{:?}", final_dest.file_name().unwrap_or_default()),
                    controller: controller.clone(),
                    rx,
                    part_file: Some((part_path, final_dest)),
                    uses_gpu,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                };
                self.running_tasks.push(task);
                self.runner_controller = Some(controller);
            }
            Err(e) => {
                self.logs.push((format!("Errore avvio processo: {}", e), true));
                self.exit_status = Some(Err(e.to_string()));
            }
        }
    }

    pub fn start_sample_test_job(&mut self, sample_dur: f64) {
        if self.is_running {
            return;
        }

        let Some(mut job) = self.build_current_job() else {
            self.set_toast("Seleziona prima un file di input");
            return;
        };

        let issues = validate_job(&job, &self.catalog);
        if let Some(err) = issues.iter().find(|i| i.is_error()) {
            self.set_toast(format!("Impossibile avviare prova: {}", err.message));
            self.logs.push((format!("ERRORE VALIDAZIONE: {}", err.message), true));
            return;
        }

        let parent = job.output.path.parent().unwrap_or_else(|| Path::new("."));
        let stem = job.output.path.file_stem().and_then(|s| s.to_str()).unwrap_or("sample");
        let ext = job.output.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let sample_path = parent.join(format!("{}_test_{}s.{}", stem, sample_dur as i64, ext));
        job.output.path = sample_path;

        let (part_path, final_dest) = match self.begin_output(&mut job) {
            Ok(res) => res,
            Err(e) => {
                self.set_toast(format!("Impossibile creare cartella di output: {}", e));
                self.logs.push((format!("ERRORE I/O: {}", e), true));
                return;
            }
        };
        job.input.duration = Some(sample_dur);

        let uses_gpu = job.output.streams.iter().any(|s| self.is_stream_hw_encoded(s));

        let args = self.build_job_args(&job);
        let cmd_str = format!("$ {} {}", self.ffmpeg_bin().display(), args.join(" "));
        match Runner::start_ffmpeg_with_program(&self.ffmpeg_bin(), args, Some(sample_dur)) {
            Ok((controller, rx)) => {
                self.is_running = true;
                self.active_queue_item_id = None;
                self.logs.clear();
                self.logs.push((cmd_str, false));
                self.current_progress = Some(0.0);
                self.current_speed = None;
                self.current_fps = None;
                self.exit_status = None;

                let task = RunningTask {
                    item_id: None,
                    desc: format!("Prova 10s: {:?}", final_dest.file_name().unwrap_or_default()),
                    controller: controller.clone(),
                    rx,
                    part_file: Some((part_path, final_dest)),
                    uses_gpu,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                };
                self.running_tasks.push(task);
                self.runner_controller = Some(controller);
            }
            Err(e) => {
                self.logs.push((format!("Errore avvio test campione: {}", e), true));
                self.exit_status = Some(Err(e.to_string()));
            }
        }
    }

    pub fn cancel_current_job(&mut self) {
        for task in &self.running_tasks {
            task.controller.cancel();
        }
        if let Some(ref controller) = self.runner_controller {
            controller.cancel();
        }
        self.logs.push(("Richiesta interruzione processo inviata...".into(), true));
    }

    pub fn start_queue(&mut self) {
        if !self.queue.has_pending() {
            self.set_toast("Nessun lavoro in attesa nella coda");
            return;
        }

        self.is_queue_running = true;
        self.spawn_next_queue_jobs();
    }

    pub fn stop_queue(&mut self) {
        self.is_queue_running = false;
        for task in &self.running_tasks {
            task.controller.cancel();
        }
        if let Some(ref ctrl) = self.runner_controller {
            ctrl.cancel();
        }
        self.set_toast("Coda fermata dall'utente");
    }

    /// Prepares and starts an individual queue item as a RunningTask
    fn spawn_queue_item(&mut self, item_id: usize) -> Result<RunningTask, String> {
        let (step, desc, item_duration_s) = {
            let Some(item) = self.queue.find_mut(item_id) else {
                return Err("Elemento non trovato".to_string());
            };
            item.status = StepStatus::Running;
            (item.step.clone(), item.description.clone(), item.duration_s)
        };

        match step {
            StepKind::Ffmpeg(mut job) => {
                let (part_path, final_dest) = Self::prepare_output_part(&mut job)
                    .map_err(|e| format!("Errore creazione cartella destinazione: {}", e))?;

                let issues = validate_job(&job, &self.catalog);
                if let Some(err) = issues.iter().find(|i| i.is_error()) {
                    return Err(format!("Errore validazione: {}", err.message));
                }

                let uses_gpu = job.output.streams.iter().any(|s| self.is_stream_hw_encoded(s));
                let args = self.build_job_args(&job);
                let cmd_str = format!("$ {} {}", self.ffmpeg_bin().display(), args.join(" "));
                let dur = item_duration_s.or(job.input.duration);
                let (ctrl, rx) = Runner::start_ffmpeg_with_program(&self.ffmpeg_bin(), args, dur)
                    .map_err(|e| e.to_string())?;

                self.logs.push((format!("[#{}] {}", item_id, cmd_str), false));

                Ok(RunningTask {
                    item_id: Some(item_id),
                    desc,
                    controller: ctrl,
                    rx,
                    part_file: Some((part_path, final_dest)),
                    uses_gpu,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                })
            }
            StepKind::Transcribe(transcribe_job) => {
                let (ctrl, rx) = WhisperBackend::start_transcription(&transcribe_job, None)
                    .map_err(|e| e.to_string())?;
                Ok(RunningTask {
                    item_id: Some(item_id),
                    desc,
                    controller: ctrl,
                    rx,
                    part_file: None,
                    uses_gpu: false,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                })
            }
            StepKind::FfmpegArgs { args, duration_s } => {
                let dur = item_duration_s.or(duration_s);
                let cmd_str = format!("$ {} {}", self.ffmpeg_bin().display(), args.join(" "));
                let (ctrl, rx) = Runner::start_ffmpeg_with_program(&self.ffmpeg_bin(), args, dur)
                    .map_err(|e| e.to_string())?;

                self.logs.push((format!("[#{}] {}", item_id, cmd_str), false));

                Ok(RunningTask {
                    item_id: Some(item_id),
                    desc,
                    controller: ctrl,
                    rx,
                    part_file: None,
                    uses_gpu: false,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                })
            }
            StepKind::BurnSubtitles { job, srt, pipeline_id } => {
                let sub_path = FfmpegBackend::prepare_safe_subtitle_file(&srt, &pipeline_id)
                    .map_err(|e| format!("Errore preparazione sottotitoli: {}", e))?;
                let filter = FfmpegBackend::burn_subtitles_filter(&sub_path);

                let mut burn_job = *job;
                if let Some(v_stream) = burn_job.output.streams.iter_mut()
                    .find(|s| s.kind == StreamKind::Video)
                {
                    v_stream.filters.push(filter);
                }

                let (part_path, final_dest) = Self::prepare_output_part(&mut burn_job)
                    .map_err(|e| format!("Errore creazione cartella: {}", e))?;

                let issues = validate_job(&burn_job, &self.catalog);
                if let Some(err) = issues.iter().find(|i| i.is_error()) {
                    return Err(format!("Errore validazione: {}", err.message));
                }

                let uses_gpu = burn_job.output.streams.iter().any(|s| self.is_stream_hw_encoded(s));
                let args = self.build_job_args(&burn_job);
                let cmd_str = format!("$ {} {}", self.ffmpeg_bin().display(), args.join(" "));
                let dur = item_duration_s.or(burn_job.input.duration);
                let (ctrl, rx) = Runner::start_ffmpeg_with_program(&self.ffmpeg_bin(), args, dur)
                    .map_err(|e| e.to_string())?;

                self.logs.push((format!("[#{}] {}", item_id, cmd_str), false));

                Ok(RunningTask {
                    item_id: Some(item_id),
                    desc,
                    controller: ctrl,
                    rx,
                    part_file: Some((part_path, final_dest)),
                    uses_gpu,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                })
            }
            StepKind::FfmpegTemplate { input, output, template } => {
                let (part_path, final_dest) = {
                    let parent = output.parent().unwrap_or_else(|| Path::new("."));
                    let stem = output.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
                    let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
                    let part = parent.join(format!("{}.part.{}", stem, ext));
                    (part, output.clone())
                };

                let uses_gpu = template.video_rule.as_ref().map(|v| {
                    self.catalog.get_codec(&v.codec_id).map(|c| c.hw.is_some()).unwrap_or(false)
                }).unwrap_or(false);

                let controller = ProcessController::new_empty();
                let controller_thread = controller.clone();

                let (tx, rx) = std::sync::mpsc::channel();
                let ffprobe_bin = self.ffprobe_bin();
                let ffmpeg_bin = self.ffmpeg_bin();
                let catalog = self.catalog.clone();
                let vaapi_device = self.settings.vaapi_device.clone();
                let hwaccel_decode = self.settings.hwaccel_decode.clone();
                let log_level = self.settings.log_level.clone();
                let det_has_gpu = self.gpu_detection.as_ref().map(|d| d.has_any_gpu()).unwrap_or(false);
                let part_path_clone = part_path.clone();

                thread::spawn(move || {
                    if controller_thread.is_cancelled() {
                        let _ = tx.send(Event::Failed {
                            reason: "Processo annullato dall'utente".to_string(),
                        });
                        return;
                    }

                    // Background probe without blocking GUI frame
                    let probe = match ffgui_core::probe::probe_file_with_program(&ffprobe_bin, &input) {
                        Ok(p) => p,
                        Err(e) => {
                            let _ = tx.send(Event::Failed {
                                reason: format!("Errore analisi file {}: {}", input.display(), e),
                            });
                            return;
                        }
                    };

                    if controller_thread.is_cancelled() {
                        let _ = tx.send(Event::Failed {
                            reason: "Processo annullato dall'utente".to_string(),
                        });
                        return;
                    }

                    let mut job = template.instantiate(&input, &output, &probe.streams);
                    let parent = part_path_clone.parent().unwrap_or_else(|| Path::new("."));
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        let _ = tx.send(Event::Failed {
                            reason: format!("Errore creazione cartella destinazione: {}", e),
                        });
                        return;
                    }
                    job.output.path = part_path_clone;

                    let src_size = std::fs::metadata(&input).ok().map(|m| m.len());
                    let quality_lines = ffgui_core::batch::apply_quality_to_job(
                        &mut job,
                        &template.video_quality,
                        &template.audio_quality,
                        Some(&probe),
                        src_size,
                    );
                    for line in quality_lines {
                        let _ = tx.send(Event::Log { line, is_stderr: false });
                    }

                    let issues = validate_job(&job, &catalog);
                    if let Some(err) = issues.iter().find(|i| i.is_error()) {
                        let _ = tx.send(Event::Failed {
                            reason: format!("Errore validazione: {}", err.message),
                        });
                        return;
                    }

                    let hwaccel = if hwaccel_decode == "disabled" || hwaccel_decode == "none" {
                        None
                    } else if hwaccel_decode == "auto" {
                        let job_uses_gpu = job.output.streams.iter().any(|s| match &s.codec {
                            CodecConfig::Encode { id, .. } => catalog.get_codec(id).map(|c| c.hw.is_some()).unwrap_or(false),
                            _ => false,
                        });
                        if job_uses_gpu || det_has_gpu {
                            Some("auto")
                        } else {
                            None
                        }
                    } else {
                        Some(hwaccel_decode.as_str())
                    };

                    let mut args = CommandBuilder::build_with_options(
                        &job,
                        &catalog,
                        vaapi_device.as_deref(),
                        hwaccel,
                    );

                    if !args.iter().any(|a| a == "-loglevel") {
                        let lvl = if log_level.is_empty() {
                            "info".to_string()
                        } else {
                            log_level.clone()
                        };
                        args.insert(0, lvl);
                        args.insert(0, "-loglevel".to_string());
                    }

                    let cmd_str = format!("$ {} {}", ffmpeg_bin.display(), args.join(" "));
                    let _ = tx.send(Event::Log {
                        line: cmd_str,
                        is_stderr: false,
                    });

                    let dur = item_duration_s.or(probe.duration_s);
                    let (sub_ctrl, sub_rx) = match Runner::start_ffmpeg_with_program(&ffmpeg_bin, args, dur) {
                        Ok(res) => res,
                        Err(e) => {
                            let _ = tx.send(Event::Failed {
                                reason: format!("Errore avvio ffmpeg: {}", e),
                            });
                            return;
                        }
                    };

                    controller_thread.attach_from(&sub_ctrl);
                    if controller_thread.is_cancelled() {
                        sub_ctrl.cancel();
                    }

                    while let Ok(event) = sub_rx.recv() {
                        let _ = tx.send(event);
                    }
                });

                Ok(RunningTask {
                    item_id: Some(item_id),
                    desc,
                    controller,
                    rx,
                    part_file: Some((part_path, final_dest)),
                    uses_gpu,
                    progress: 0.0,
                    speed: None,
                    fps: None,
                })
            }
        }
    }

    /// Spawns pending queue jobs up to parallel_jobs limit, respecting GPU concurrency (Section 4.1)
    pub fn spawn_next_queue_jobs(&mut self) {
        if !self.is_queue_running {
            return;
        }

        let max_parallel = self.settings.parallel_jobs.max(1);
        let max_gpu = self.settings.max_gpu_sessions.max(1);

        while self.running_tasks.len() < max_parallel {
            let Some(next_id) = self.queue.next_runnable_id() else {
                break;
            };

            // Check GPU limit if next job uses GPU
            let next_uses_gpu = self.queue.items.iter().find(|i| i.id == next_id).map(|item| {
                match &item.step {
                    StepKind::Ffmpeg(job) => job.output.streams.iter().any(|s| {
                        match &s.codec {
                            CodecConfig::Encode { id, .. } => self.catalog.get_codec(id).map(|c| c.hw.is_some()).unwrap_or(false),
                            _ => false,
                        }
                    }),
                    StepKind::BurnSubtitles { job, .. } => job.output.streams.iter().any(|s| {
                        match &s.codec {
                            CodecConfig::Encode { id, .. } => self.catalog.get_codec(id).map(|c| c.hw.is_some()).unwrap_or(false),
                            _ => false,
                        }
                    }),
                    StepKind::FfmpegTemplate { template, .. } => {
                        template.video_rule.as_ref().map(|v| {
                            self.catalog.get_codec(&v.codec_id).map(|c| c.hw.is_some()).unwrap_or(false)
                        }).unwrap_or(false)
                    }
                    _ => false,
                }
            }).unwrap_or(false);

            if next_uses_gpu {
                let current_gpu_count = self.running_tasks.iter().filter(|t| t.uses_gpu).count();
                if current_gpu_count >= max_gpu {
                    break; // Wait for a GPU task to finish
                }
            }

            match self.spawn_queue_item(next_id) {
                Ok(task) => {
                    self.logs.push((format!("[Coda #{}] Avvio: {}", next_id, task.desc), false));
                    self.running_tasks.push(task);
                    self.is_running = true;
                }
                Err(err) => {
                    if let Some(it) = self.queue.find_mut(next_id) {
                        it.status = StepStatus::Failed;
                        it.error_message = Some(err.clone());
                    }
                    let cancelled = self.queue.cascade_cancel_dependencies();
                    for c_id in cancelled {
                        self.logs.push((
                            format!("[Coda] Lavoro dipendente #{} annullato perché il lavoro #{} è fallito", c_id, next_id),
                            true,
                        ));
                    }
                    self.logs.push((format!("[Coda #{}] Errore: {}", next_id, err), true));
                }
            }
        }

        if self.running_tasks.is_empty() && (!self.queue.has_pending() || self.queue.next_runnable_id().is_none()) {
            self.is_queue_running = false;
            self.is_running = false;
        }
    }

    #[allow(dead_code)]
    pub fn process_next_queue_item(&mut self) {
        self.spawn_next_queue_jobs();
    }

    pub fn start_subtitles_pipeline(&mut self) {
        if self.is_running {
            self.set_toast("Un lavoro è già in esecuzione");
            return;
        }

        let Some(input_path) = self.input_path.as_ref() else {
            self.set_toast("Seleziona prima un file video/audio di input");
            return;
        };

        let Some(output_path) = self.output_path.as_ref() else {
            self.set_toast("Seleziona una destinazione di salvataggio");
            return;
        };

        // Bug 3.1: Check if audio stream exists in input file
        if self.subtitle_options.custom_sub_file.is_none() {
            let has_audio = self
                .probe_result
                .as_ref()
                .map(|p| p.streams.iter().any(|s| s.kind == StreamKind::Audio))
                .unwrap_or(true);

            if !has_audio {
                self.set_toast("Nessuna traccia audio presente nel file");
                return;
            }
        }

        let duration_s = self
            .duration
            .or_else(|| self.probe_result.as_ref().and_then(|p| p.duration_s));

        let lang_opt = if self.subtitle_options.language == "auto" || self.subtitle_options.language.trim().is_empty() {
            None
        } else {
            Some(self.subtitle_options.language.trim().to_string())
        };

        // Bug 3.7: Generate unique pipeline ID for temporary files
        let pipeline_id = format!(
            "{}_{}",
            self.queue.len(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );

        let srt_output = if let Some(ref custom_sub) = self.subtitle_options.custom_sub_file {
            custom_sub.clone()
        } else {
            if !self.whisper_env_ready.load(Ordering::SeqCst) {
                self.set_toast("L'ambiente Whisper non è pronto. Clicca su 'Installa ambiente Whisper'!");
                return;
            }

            let _ = Storage::ensure_dirs();
            let cache_dir = Storage::cache_dir();
            let temp_wav = cache_dir.join(format!("sub_audio_{}.wav", pipeline_id));

            let srt_out = match self.subtitle_options.mode {
                SubtitleMode::TranscribeOnly => output_path.with_extension("srt"),
                _ => cache_dir.join(format!("sub_transcribed_{}.srt", pipeline_id)),
            };

            // Step 1: Extract Audio (Bug 3.1: pass selected_audio_track)
            let audio_track = self.subtitle_options.selected_audio_track;
            let extract_args = FfmpegBackend::build_extract_audio_args(input_path, &temp_wav, audio_track);
            self.queue.push_with_duration(
                format!("1. Estrazione audio #{} per Whisper ({})", audio_track, input_path.file_name().unwrap_or_default().to_string_lossy()),
                StepKind::FfmpegArgs {
                    args: extract_args,
                    duration_s,
                },
                duration_s,
            );

            // Step 2: Transcribe
            let transcribe_job = TranscribeJob {
                audio_file: temp_wav,
                output_file: srt_out.clone(),
                model: self.subtitle_options.model.clone(),
                language: lang_opt.clone(),
                device: self.subtitle_options.device.clone(),
                compute_type: self.subtitle_options.compute_type.clone(),
                vad_filter: self.subtitle_options.vad_filter,
            };
            self.queue.push_with_duration(
                format!("2. Trascrizione Whisper ({})", self.subtitle_options.model),
                StepKind::Transcribe(transcribe_job),
                duration_s,
            );

            srt_out
        };

        // Step 3: Mux or Burn
        match self.subtitle_options.mode {
            SubtitleMode::ExtractAndTranscribeTrack => {
                let mux_args = FfmpegBackend::build_mux_subtitles_args(
                    input_path,
                    &srt_output,
                    output_path,
                    lang_opt.as_deref(),
                    true,
                );
                self.queue.push_with_duration(
                    format!("3. Mux sottotitoli in {:?}", output_path.file_name().unwrap_or_default()),
                    StepKind::FfmpegArgs {
                        args: mux_args,
                        duration_s,
                    },
                    duration_s,
                );
            }
            SubtitleMode::BurnIntoVideo => {
                let mut burn_job = Job::new(input_path, output_path);
                burn_job.input.start_time = self.start_time;
                burn_job.input.duration = self.duration;

                // Find the current video stream's codec (don't force libx264 if user set a different one, and avoid "copy")
                let video_codec = self.stream_controls.iter()
                    .find(|c| c.kind == StreamKind::Video && c.enabled)
                    .map(|c| {
                        if c.codec_id == "copy" {
                            self.catalog.default_for_kind(StreamKind::Video)
                                .map(|d| d.id.clone())
                                .unwrap_or_else(|| "libx264".to_string())
                        } else {
                            c.codec_id.clone()
                        }
                    })
                    .unwrap_or_else(|| "libx264".to_string());

                let video_src = self.stream_controls.iter()
                    .find(|c| c.kind == StreamKind::Video && c.enabled)
                    .map(|c| c.source_index)
                    .unwrap_or(0);

                let audio_src = self.stream_controls.iter()
                    .find(|c| c.kind == StreamKind::Audio && c.enabled)
                    .map(|c| c.source_index)
                    .unwrap_or(0);

                let v_stream = OutStream::new_encode(StreamKind::Video, video_src, &video_codec, HashMap::new());
                burn_job.output.streams.push(v_stream);
                burn_job.output.streams.push(OutStream::new_copy(StreamKind::Audio, audio_src));

                // Bug 3.2 & 3.7: Queue BurnSubtitles step for execution-time safe copy
                self.queue.push_with_duration(
                    format!("3. Codifica video con sottotitoli impressi in {:?}", output_path.file_name().unwrap_or_default()),
                    StepKind::BurnSubtitles {
                        job: Box::new(burn_job),
                        srt: srt_output,
                        pipeline_id,
                    },
                    duration_s,
                );
            }
            SubtitleMode::TranscribeOnly => {}
        }

        self.set_toast("Pipeline sottotitoli accodata ed avviata!");
        self.start_queue();
    }
}

fn is_stats_line(s: &str) -> bool {
    let t = s.trim_start();
    let stripped = if t.starts_with('[') && t.contains("] ") {
        t.split_once("] ").map(|(_, r)| r).unwrap_or(t).trim_start()
    } else {
        t
    };
    stripped.starts_with("frame=") || stripped.starts_with("size=")
}

fn append_or_update_log(logs: &mut Vec<(String, bool)>, new_line: String, is_stderr: bool) {
    let is_stats = is_stats_line(&new_line);
    let can_update = if is_stats {
        if let Some((last, _)) = logs.last() {
            let last_prefix = if last.starts_with('[') && last.contains("] ") {
                last.split_once("] ").map(|(p, _)| p)
            } else {
                None
            };
            let new_prefix = if new_line.starts_with('[') && new_line.contains("] ") {
                new_line.split_once("] ").map(|(p, _)| p)
            } else {
                None
            };
            last_prefix == new_prefix && is_stats_line(last)
        } else {
            false
        }
    } else {
        false
    };

    if can_update {
        *logs.last_mut().unwrap() = (new_line, is_stderr);
    } else {
        logs.push((new_line, is_stderr));
        if logs.len() > 30000 {
            logs.drain(0..5000);
        }
    }
}

impl FfGuiApp {
    pub fn poll_runner_events(&mut self) {
        // Poll rx_events (for Whisper setup and direct runner channels)
        let mut toast_to_set = None;
        if let Some(ref rx) = self.rx_events {
            let mut finished = false;
            while let Ok(event) = rx.try_recv() {
                match event {
                    Event::Log { line, is_stderr } => {
                        append_or_update_log(&mut self.logs, line, is_stderr);
                    }
                    Event::Progress {
                        time_s,
                        percent,
                        speed,
                        fps,
                    } => {
                        self.current_time_s = Some(time_s);
                        self.current_progress = percent;
                        self.current_speed = speed;
                        self.current_fps = fps;
                    }
                    Event::Finished { exit_code } => {
                        finished = true;
                        self.exit_status = Some(Ok(exit_code));
                        if self.is_whisper_setup_running {
                            let ready = WhisperBackend::is_env_ready();
                            self.whisper_env_ready.store(ready, Ordering::SeqCst);
                            self.is_whisper_setup_running = false;
                            if ready {
                                toast_to_set = Some("Ambiente Whisper installato con successo!".to_string());
                            } else {
                                toast_to_set = Some("Installazione Whisper terminata ma ambiente non pronto.".to_string());
                            }
                        }
                    }
                    Event::Failed { reason } => {
                        finished = true;
                        self.exit_status = Some(Err(reason.clone()));
                        self.logs.push((format!("Errore: {}", reason), true));
                        if self.is_whisper_setup_running {
                            self.is_whisper_setup_running = false;
                            toast_to_set = Some(format!("Installazione Whisper fallita: {}", reason));
                        }
                    }
                }
            }
            if finished {
                self.rx_events = None;
            }
        }
        if let Some(msg) = toast_to_set {
            self.set_toast(msg);
        }

        let mut finished_indices = Vec::new();

        for (idx, task) in self.running_tasks.iter_mut().enumerate() {
            let mut task_finished = false;
            let mut task_exit_code = None;
            let mut task_failed_reason = None;

            while let Ok(event) = task.rx.try_recv() {
                match event {
                    Event::Log { line, is_stderr } => {
                        let formatted_line = if let Some(id) = task.item_id {
                            format!("[#{}] {}", id, line)
                        } else {
                            line.clone()
                        };

                        if let Some(id) = task.item_id {
                            if let Some(item) = self.queue.find_mut(id) {
                                let is_stats = is_stats_line(&line);
                                if is_stats && item.logs.last().map(|l| is_stats_line(l)).unwrap_or(false) {
                                    *item.logs.last_mut().unwrap() = line.clone();
                                } else {
                                    item.logs.push(line.clone());
                                    if item.logs.len() > 10000 {
                                        item.logs.drain(0..1000);
                                    }
                                }
                            }
                        }

                        append_or_update_log(&mut self.logs, formatted_line, is_stderr);
                    }
                    Event::Progress {
                        time_s,
                        percent,
                        speed,
                        fps,
                    } => {
                        task.progress = percent.unwrap_or(0.0);
                        task.speed = speed;
                        task.fps = fps;

                        if task.item_id.is_none() {
                            self.current_time_s = Some(time_s);
                            self.current_progress = percent;
                            self.current_speed = speed;
                            self.current_fps = fps;
                        } else if let Some(id) = task.item_id {
                            if let Some(item) = self.queue.find_mut(id) {
                                if let Some(p) = percent {
                                    item.progress = p;
                                }
                            }
                        }
                    }
                    Event::Finished { exit_code } => {
                        task_finished = true;
                        task_exit_code = Some(exit_code);
                    }
                    Event::Failed { reason } => {
                        task_finished = true;
                        task_failed_reason = Some(reason);
                    }
                }
            }

            if task_finished {
                finished_indices.push((idx, task_exit_code, task_failed_reason));
            }
        }

        // Handle finished tasks in reverse order to preserve indices
        for (idx, exit_code_opt, failed_reason) in finished_indices.into_iter().rev() {
            let task = self.running_tasks.remove(idx);
            let is_success = exit_code_opt == Some(0);

            if let Some((part_path, final_path)) = task.part_file {
                if is_success {
                    if part_path.exists() {
                        if let Some(parent) = final_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(e) = std::fs::rename(&part_path, &final_path) {
                            self.logs.push((format!("Errore rinomina file definitivo: {}", e), true));
                        }
                    }
                } else if part_path.exists() {
                    let _ = std::fs::remove_file(&part_path);
                }
            }

            if let Some(id) = task.item_id {
                if let Some(item) = self.queue.find_mut(id) {
                    if is_success {
                        item.status = StepStatus::Finished;
                        item.progress = 100.0;
                        item.exit_code = Some(0);
                    } else {
                        // Check automatic CPU fallback on GPU failure (B9)
                        let mut can_fallback_cpu = false;
                        let was_cancelled = task.controller.is_cancelled();
                        let gpu_failure =
                            ffgui_core::backends::gpu::GpuBackend::looks_like_gpu_failure(&item.logs);
                        if task.uses_gpu && self.settings.auto_cpu_fallback && !was_cancelled && gpu_failure {
                            match &mut item.step {
                                StepKind::Ffmpeg(job) => {
                                    for s in &mut job.output.streams {
                                        if let CodecConfig::Encode { id, params } = &mut s.codec {
                                            if let Some(cpu_codec) = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(id) {
                                                let new_params = ffgui_core::backends::gpu::GpuBackend::map_gpu_params_to_cpu(id, cpu_codec, params);
                                                *id = cpu_codec.to_string();
                                                *params = new_params;
                                                can_fallback_cpu = true;
                                            }
                                        }
                                    }
                                }
                                StepKind::BurnSubtitles { job, .. } => {
                                    for s in &mut job.output.streams {
                                        if let CodecConfig::Encode { id, params } = &mut s.codec {
                                            if let Some(cpu_codec) = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(id) {
                                                let new_params = ffgui_core::backends::gpu::GpuBackend::map_gpu_params_to_cpu(id, cpu_codec, params);
                                                *id = cpu_codec.to_string();
                                                *params = new_params;
                                                can_fallback_cpu = true;
                                            }
                                        }
                                    }
                                }
                                StepKind::FfmpegTemplate { template, .. } => {
                                    if let Some(ref mut v) = template.video_rule {
                                        if let Some(cpu_codec) = ffgui_core::backends::gpu::GpuBackend::get_cpu_counterpart(&v.codec_id) {
                                            let new_params = ffgui_core::backends::gpu::GpuBackend::map_gpu_params_to_cpu(&v.codec_id, cpu_codec, &v.params);
                                            v.codec_id = cpu_codec.to_string();
                                            v.params = new_params;
                                            can_fallback_cpu = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }

                        if can_fallback_cpu {
                            self.logs.push((
                                format!("[GPU Fallback] Lavoro #{}: errore GPU rilevato. Riconversione automatica con CPU mappando la qualità richiesta...", id),
                                true,
                            ));
                            item.status = StepStatus::Pending;
                            item.progress = 0.0;
                        } else {
                            if was_cancelled {
                                item.status = StepStatus::Cancelled;
                                item.error_message = Some("Processo annullato dall'utente".to_string());
                            } else {
                                item.status = StepStatus::Failed;
                                item.exit_code = exit_code_opt;
                                item.error_message = failed_reason.or_else(|| exit_code_opt.map(|c| format!("Uscita con codice {}", c)));
                            }
                            let cancelled = self.queue.cascade_cancel_dependencies();
                            for c_id in cancelled {
                                self.logs.push((
                                    format!("[Coda] Lavoro dipendente #{} annullato perché il lavoro #{} è fallito o annullato", c_id, id),
                                    true,
                                ));
                            }
                        }
                    }
                }
            } else {
                // Standalone single job
                if is_success {
                    self.exit_status = Some(Ok(0));
                } else if let Some(code) = exit_code_opt {
                    self.exit_status = Some(Ok(code));
                } else if let Some(reason) = failed_reason {
                    self.exit_status = Some(Err(reason));
                }
            }
        }

        // Section 4.1: Compute aggregated progress across queue items
        if self.is_queue_running && !self.queue.items.is_empty() {
            let running_sum: f64 = self.running_tasks.iter().map(|t| t.progress).sum();
            let done = self.queue.items.iter().filter(|i| i.status == StepStatus::Finished).count() as f64;
            let total = self.queue.items.len().max(1) as f64;
            self.current_progress = Some(((done * 100.0 + running_sum) / total).clamp(0.0, 100.0));
        }

        // Section 3.1: Clean exit condition only when both tasks and rx_events are idle
        if self.running_tasks.is_empty() && self.rx_events.is_none() {
            self.is_running = false;
            self.runner_controller = None;
        }

        if self.is_queue_running {
            self.spawn_next_queue_jobs();
        }
    }

    pub fn trigger_batch_scan(&mut self) {
        if self.batch_settings.source.folder.as_os_str().is_empty() || !self.batch_settings.source.folder.exists() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let settings = self.batch_settings.clone();
        self.is_batch_scanning = true;
        self.rx_batch_scan = Some(rx);
        thread::spawn(move || {
            let res = ffgui_core::batch::scan_and_plan_batch(&settings);
            let _ = tx.send(res);
        });
    }

    pub fn poll_batch_scan(&mut self) {
        if let Some(ref rx) = self.rx_batch_scan {
            if let Ok((exts, plan)) = rx.try_recv() {
                self.is_batch_scanning = false;
                self.rx_batch_scan = None;
                self.batch_discovered_exts = exts;
                self.batch_plan_v2 = Some(plan);
            }
        }
    }

    pub fn enqueue_batch_plan_v2(&mut self, auto_start: bool) {
        let Some(ref plan) = self.batch_plan_v2 else {
            self.set_toast("Nessun piano batch disponibile");
            return;
        };

        let mut queued_count = 0;
        for item in &plan.items {
            if item.state == ffgui_core::batch::PlanState::ExistsSkip {
                continue;
            }

            if item.is_copy_only {
                let src = item.input.clone();
                let dst = item.output.clone();
                if let Some(parent) = dst.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::copy(&src, &dst) {
                    self.logs.push((format!("Errore copia file {}: {}", src.display(), e), true));
                } else {
                    self.logs.push((format!("Copiato: {} -> {}", src.display(), dst.display()), false));
                }
                continue;
            }

            let template = if let Some(ov) = self.batch_settings.overrides.get(&item.input) {
                if ov.exclude {
                    continue;
                }
                if let Some(ref prof) = ov.profile {
                    prof.to_job_template()
                } else if let Some(prof) = self.batch_settings.profiles.get(&item.media_kind) {
                    prof.to_job_template()
                } else {
                    continue;
                }
            } else if let Some(prof) = self.batch_settings.profiles.get(&item.media_kind) {
                prof.to_job_template()
            } else {
                continue;
            };

            let desc = format!(
                "Batch [{}] {}",
                item.media_kind.as_str(),
                item.input.file_name().unwrap_or_default().to_string_lossy()
            );

            self.queue.push(
                desc,
                StepKind::FfmpegTemplate {
                    input: item.input.clone(),
                    output: item.output.clone(),
                    template,
                },
            );
            queued_count += 1;
        }

        self.set_toast(format!("Aggiunti {} elementi alla coda", queued_count));
        if auto_start {
            self.start_queue();
        }
    }

    pub fn copy_main_panel_to_video_profile(&mut self) -> bool {
        let Some(ref _in_path) = self.input_path else {
            self.set_toast("Nessun file aperto nel pannello principale da cui copiare");
            return false;
        };

        let v_ctrl = self.stream_controls.iter().find(|c| c.kind == StreamKind::Video && c.enabled);
        let a_ctrl = self.stream_controls.iter().find(|c| c.kind == StreamKind::Audio && c.enabled);

        let v_rule = v_ctrl.map(|c| ffgui_core::model::VideoRule {
            codec_id: c.codec_id.clone(),
            params: c.params.clone(),
            scale_preset: if c.scale_preset == "Originale" { None } else { Some(c.scale_preset.clone()) },
            custom_width: if c.scale_preset == "Personalizzato" { Some(c.custom_width) } else { None },
            custom_height: if c.scale_preset == "Personalizzato" { Some(c.custom_height) } else { None },
        });

        let a_rule = match a_ctrl {
            Some(c) if c.codec_id == "copy" => ffgui_core::model::AudioRule::CopyAll,
            Some(c) => ffgui_core::model::AudioRule::EncodeAll {
                codec_id: c.codec_id.clone(),
                params: c.params.clone(),
            },
            None => ffgui_core::model::AudioRule::CopyAll,
        };

        let ext = self.output_path.as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("mkv")
            .to_string();

        let prof = BatchProfile {
            name: "Copiato da Principale".to_string(),
            target_container: ext,
            video_rule: v_rule,
            audio_rule: a_rule,
            subtitle_rule: ffgui_core::model::SubtitleRule::CopyCompatible,
            copy_cover_art: true,
            video_quality: self.settings.video_quality.clone(),
            audio_quality: self.settings.audio_quality,
        };

        self.batch_settings.profiles.insert(MediaKind::Video, prof);
        self.set_toast("Profilo video aggiornato con le impostazioni del pannello principale!");
        self.trigger_batch_scan();
        true
    }
}
