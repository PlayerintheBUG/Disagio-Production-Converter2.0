pub static BUILTIN_CODECS: &[(&str, &str)] = &[
    ("libx265", include_str!("../../codecs/libx265.toml")),
    ("libx264", include_str!("../../codecs/libx264.toml")),
    ("libsvtav1", include_str!("../../codecs/libsvtav1.toml")),
    ("libopus", include_str!("../../codecs/libopus.toml")),
    ("aac", include_str!("../../codecs/aac.toml")),
    ("flac", include_str!("../../codecs/flac.toml")),
    ("libmp3lame", include_str!("../../codecs/libmp3lame.toml")),
    ("prores_ks", include_str!("../../codecs/prores_ks.toml")),
    ("libwebp", include_str!("../../codecs/libwebp.toml")),
    // GPU Encoders (CUDA/NVENC priority, then QSV, AMF, VAAPI)
    ("av1_nvenc", include_str!("../../codecs/av1_nvenc.toml")),
    ("hevc_nvenc", include_str!("../../codecs/hevc_nvenc.toml")),
    ("h264_nvenc", include_str!("../../codecs/h264_nvenc.toml")),
    ("av1_qsv", include_str!("../../codecs/av1_qsv.toml")),
    ("hevc_qsv", include_str!("../../codecs/hevc_qsv.toml")),
    ("h264_qsv", include_str!("../../codecs/h264_qsv.toml")),
    ("av1_amf", include_str!("../../codecs/av1_amf.toml")),
    ("hevc_amf", include_str!("../../codecs/hevc_amf.toml")),
    ("h264_amf", include_str!("../../codecs/h264_amf.toml")),
    ("av1_vaapi", include_str!("../../codecs/av1_vaapi.toml")),
    ("hevc_vaapi", include_str!("../../codecs/hevc_vaapi.toml")),
    ("h264_vaapi", include_str!("../../codecs/h264_vaapi.toml")),
];

pub static BUILTIN_WHISPER_SCRIPT: &str = include_str!("../../tools/whisper_run.py");
