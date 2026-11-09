//! Tabella unificata delle capacità dei contenitori multimediali.
//! Risolve i bug A2, B12, B13 (compatibilità copertina, sottotitoli e stream video).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContainerCaps {
    /// Supporta flussi video standard
    pub video: bool,
    /// Supporta stream di copertina (CoverArt / video MJPEG/PNG/BMP allegati)
    pub cover_art: bool,
    /// Supporta tracce audio multiple
    pub multi_audio: bool,
    /// Supporta tracce di sottotitoli (testo o bitmap)
    pub subtitles: bool,
    /// Supporta allegati generici (es. font TTF/OTF per ASS)
    pub attachments: bool,
}

/// Restituisce le capacità ufficiali supportate dal muxer per l'estensione indicata.
pub fn container_caps(ext: &str) -> ContainerCaps {
    let clean = ext.trim_start_matches('.').to_lowercase();
    match clean.as_str() {
        "mkv" => ContainerCaps {
            video: true,
            cover_art: true,
            multi_audio: true,
            subtitles: true,
            attachments: true,
        },
        "mp4" | "m4v" | "mov" => ContainerCaps {
            video: true,
            cover_art: true,
            multi_audio: true,
            subtitles: true,
            attachments: false,
        },
        // Sottotitoli: WebM accetta solo WebVTT e ffgui non converte i testi, quindi per prudenza
        // non si copiano (con un SRT ffmpeg fallisce: «Only VP8 or VP9 or AV1 video...»)
        "webm" => ContainerCaps {
            video: true,
            cover_art: false,
            multi_audio: true,
            subtitles: false,
            attachments: false,
        },
        "avi" => ContainerCaps {
            video: true,
            cover_art: false,
            multi_audio: true,
            subtitles: false,
            attachments: false,
        },
        // m4a e mka accettano più tracce audio (verificato con ffmpeg 6.1.1)
        "m4a" | "mka" => ContainerCaps {
            video: false,
            cover_art: true,
            multi_audio: true,
            subtitles: false,
            attachments: false,
        },
        // mp3 e flac: una sola traccia audio (con due, ffmpeg rifiuta di scrivere l'intestazione)
        "mp3" | "flac" => ContainerCaps {
            video: false,
            cover_art: true,
            multi_audio: false,
            subtitles: false,
            attachments: false,
        },
        "opus" | "ogg" | "oga" => ContainerCaps {
            video: false,
            cover_art: false, // Muxer Ogg/Opus rifiuta stream video (A2)
            multi_audio: true,
            subtitles: false,
            attachments: false,
        },
        "wav" => ContainerCaps {
            video: false,
            cover_art: false,
            multi_audio: false, // WAVE standard accetta solo un flusso
            subtitles: false,
            attachments: false,
        },
        "ac3" | "aac" => ContainerCaps {
            video: false,
            cover_art: false,
            multi_audio: false,
            subtitles: false,
            attachments: false,
        },
        // Formati immagine: un solo flusso video, niente audio multiplo né sottotitoli
        "webp" | "png" | "jpg" | "jpeg" | "avif" | "gif" | "tif" | "tiff" | "bmp" => ContainerCaps {
            video: true,
            cover_art: false,
            multi_audio: false,
            subtitles: false,
            attachments: false,
        },
        _ => ContainerCaps {
            video: true,
            cover_art: false,
            multi_audio: true,
            subtitles: false,
            attachments: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opus_refuses_cover_art_and_video() {
        let caps = container_caps("opus");
        assert!(!caps.cover_art, "Opus muxer non accetta stream video/copertina");
        assert!(!caps.video);
        assert!(!caps.subtitles);
    }

    #[test]
    fn test_flac_and_mp3_accept_cover_art() {
        assert!(container_caps("flac").cover_art);
        assert!(container_caps("mp3").cover_art);
        assert!(container_caps("m4a").cover_art);
        assert!(container_caps("mkv").cover_art);
        assert!(container_caps("mp4").cover_art);
    }

    #[test]
    fn test_multi_audio_matches_ffmpeg() {
        // verificato con ffmpeg 6.1.1: due tracce audio verso questi contenitori
        for ext in ["mp3", "flac", "wav", "ac3", "aac"] {
            assert!(!container_caps(ext).multi_audio, ".{} accetta una sola traccia audio", ext);
        }
        for ext in ["opus", "ogg", "m4a", "mka", "mkv", "mp4"] {
            assert!(container_caps(ext).multi_audio, ".{} accetta più tracce audio", ext);
        }
    }

    #[test]
    fn test_image_formats_are_known() {
        for ext in ["png", "jpg", "jpeg", "avif", "webp"] {
            let c = container_caps(ext);
            assert!(c.video && !c.multi_audio && !c.cover_art, ".{}", ext);
        }
    }

    #[test]
    fn test_webm_and_avi_capabilities() {
        let webm = container_caps("webm");
        assert!(webm.video);
        assert!(!webm.cover_art);
        assert!(!webm.subtitles, "webm accetta solo WebVTT: non si copiano sottotitoli generici");

        let avi = container_caps("avi");
        assert!(avi.video);
        assert!(!avi.subtitles);
    }
}
