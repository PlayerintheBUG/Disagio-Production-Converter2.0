pub mod catalog;
pub mod model;
pub mod probe;
pub mod validate;
pub mod builder;
pub mod runner;
pub mod queue;
pub mod backends;
pub mod storage;
pub mod builtin_codecs;
pub mod batch;
pub mod cmdview;
pub mod containers;

/// Evita che i processi console figli (ffmpeg, ffprobe, python) aprano
/// una finestra visibile su Windows quando lanciati dall'applicazione GUI.
/// Su altri sistemi non ha effetto.
pub fn hide_console_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
