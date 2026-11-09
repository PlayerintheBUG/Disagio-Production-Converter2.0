//! Vista compatta del comando ffmpeg.
//!
//! Esempio di risultato:
//!   ffmpeg -hwaccel auto -i {INPUT} -c:v av1_nvenc -preset p6 -cq 24 -pix_fmt p010le
//!          -c:a flac -compression_level 5 {OUTPUT}.mkv -y
//!
//! Principio: la vista compatta NON ricostruisce il comando da zero. Parte dagli
//! argomenti reali prodotti da `CommandBuilder` (unica fonte di verità) e li ripulisce:
//!   * `-i` e il file di uscita diventano {INPUT} e {OUTPUT}.<estensione>;
//!   * `-hide_banner` sparisce e `-y` va in fondo (ffmpeg lo accetta in qualunque posizione);
//!   * gli specificatori di stream (`:v:0`, `:a:0`) si tolgono solo quando è equivalente,
//!     cioè quando c'è un solo stream di quel tipo (la copertina conta come video);
//!   * `-map` si nasconde solo quando coincide con la scelta automatica di ffmpeg.
//!
//! Le regole sono state verificate con ffmpeg 6.1.1; vedi i test in fondo al file.

use std::path::{Path, PathBuf};

use crate::builder::CommandBuilder;
use crate::catalog::Catalog;
use crate::model::{CodecConfig, Job, StreamKind};
use crate::probe::MediaStream;

pub const INPUT_PLACEHOLDER: &str = "{INPUT}";

/// Categoria di un pezzo del comando (la GUI la usa per i colori).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Program,
    Global,
    Hardware,
    Input,
    Map,
    Video,
    Audio,
    Subtitle,
    Filter,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub text: String,
    pub kind: TokenKind,
}

impl Token {
    fn new(text: impl Into<String>, kind: TokenKind) -> Self {
        Self { text: text.into(), kind }
    }

    /// Testo da mostrare: i segnaposto restano com'è, il resto tra apici se serve.
    pub fn display_text(&self) -> String {
        match self.kind {
            TokenKind::Program | TokenKind::Output => self.text.clone(),
            _ if self.text == INPUT_PLACEHOLDER => self.text.clone(),
            _ => shell_quote(&self.text),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactCommand {
    pub tokens: Vec<Token>,
    /// true se almeno un indice di stream (`:0`, `:1`) è rimasto perché ci sono più stream dello stesso tipo
    pub kept_specifiers: bool,
    /// true se i `-map` sono stati omessi perché coincidono con la scelta automatica di ffmpeg
    pub map_hidden: bool,
}

impl CompactCommand {
    /// Riga da mostrare, con {INPUT} e {OUTPUT}.
    pub fn to_text(&self) -> String {
        self.tokens
            .iter()
            .map(|t| t.display_text())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Argomenti eseguibili (senza "ffmpeg") con i percorsi veri al posto dei segnaposto.
    pub fn to_args(&self, input: &str, output: &str) -> Vec<String> {
        self.tokens
            .iter()
            .skip(1) // "ffmpeg"
            .map(|t| {
                if t.text == INPUT_PLACEHOLDER {
                    input.to_string()
                } else if t.kind == TokenKind::Output {
                    output.to_string()
                } else {
                    t.text.clone()
                }
            })
            .collect()
    }

    /// Riga pronta da incollare nel terminale, con i percorsi veri.
    pub fn to_shell(&self, input: &str, output: &str) -> String {
        CommandBuilder::to_preview_string(&self.to_args(input, output))
    }
}

/// Quanti stream di ogni tipo ci sono nel file di ingresso (dal probe).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputSummary {
    pub video_like: usize, // video + copertine
    pub audio: usize,
    pub subtitle: usize,
    pub other: usize, // allegati, dati
}

impl InputSummary {
    pub fn from_streams(streams: &[MediaStream]) -> Self {
        let mut s = Self::default();
        for st in streams {
            match st.kind {
                StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => s.video_like += 1,
                StreamKind::Audio => s.audio += 1,
                StreamKind::Subtitle => s.subtitle += 1,
                StreamKind::Attachment | StreamKind::Data => s.other += 1,
            }
        }
        s
    }
}

/// Percorso temporaneo `nome.part.ext` su cui scrive l'app durante la conversione.
pub fn part_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = final_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    match final_path.extension().and_then(|e| e.to_str()) {
        Some(ext) => parent.join(format!("{}.part.{}", stem, ext)),
        None => parent.join(format!("{}.part", stem)),
    }
}

/// Apici POSIX solo quando servono.
pub fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    let safe = arg.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '+' | ',' | '@' | '%')
    });
    if safe {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

// ---------------------------------------------------------------------------
// Costruzione
// ---------------------------------------------------------------------------

/// Opzioni generiche di codec che valgono per qualunque encoder: se si toglie la lettera
/// del tipo (`-b:v` -> `-b`) finirebbero anche sull'altro encoder (per esempio `-profile`
/// esiste sia in x264 sia in aac). Qui la lettera resta sempre.
const KEEP_TYPE_LETTER: &[&str] = &["b", "profile", "level", "tag", "q", "qscale", "flags", "strict"];

/// Opzioni senza valore (le altre ne hanno uno).
const NO_VALUE_FLAGS: &[&str] = &["-hide_banner", "-y", "-n", "-nostdin", "-nostats", "-stats"];

#[derive(Debug, Clone, Copy, Default)]
struct Counts {
    video: usize, // video, immagini, copertine
    audio: usize,
    subtitle: usize,
    attachment: usize,
    data: usize,
}

impl Counts {
    fn from_job(job: &Job) -> Self {
        let mut c = Self::default();
        for s in &job.output.streams {
            match s.kind {
                StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => c.video += 1,
                StreamKind::Audio => c.audio += 1,
                StreamKind::Subtitle => c.subtitle += 1,
                StreamKind::Attachment => c.attachment += 1,
                StreamKind::Data => c.data += 1,
            }
        }
        c
    }

    fn get(&self, letter: char) -> usize {
        match letter {
            'v' => self.video,
            'a' => self.audio,
            's' => self.subtitle,
            't' => self.attachment,
            'd' => self.data,
            _ => 0,
        }
    }
}

fn type_letter(kind: StreamKind) -> char {
    match kind {
        StreamKind::Video | StreamKind::Image | StreamKind::CoverArt => 'v',
        StreamKind::Audio => 'a',
        StreamKind::Subtitle => 's',
        StreamKind::Attachment => 't',
        StreamKind::Data => 'd',
    }
}

/// "-crf:v:0" -> ("crf", 'v', Some(0)); "-c:v" -> ("c", 'v', None); "-ss" -> None
fn parse_flag(flag: &str) -> Option<(&str, char, Option<usize>)> {
    let body = flag.strip_prefix('-')?;
    let mut it = body.splitn(3, ':');
    let name = it.next()?;
    let letter_part = it.next()?;
    let mut chars = letter_part.chars();
    let letter = chars.next()?;
    if chars.next().is_some() || !"vastd".contains(letter) {
        return None;
    }
    let idx = match it.next() {
        Some(s) => Some(s.parse::<usize>().ok()?),
        None => None,
    };
    Some((name, letter, idx))
}

fn flag_name(flag: &str) -> &str {
    flag.trim_start_matches('-').split(':').next().unwrap_or("")
}

/// Un altro stream codificato, di un altro tipo, ha un parametro con lo stesso flag?
fn used_by_other_type(job: &Job, catalog: &Catalog, name: &str, letter: char) -> bool {
    for s in &job.output.streams {
        if type_letter(s.kind) == letter {
            continue;
        }
        if let CodecConfig::Encode { id, .. } = &s.codec {
            if let Some(def) = catalog.get_codec(id) {
                if def.params.iter().any(|p| flag_name(&p.flag) == name) {
                    return true;
                }
            }
        }
    }
    false
}

/// Restituisce il testo del flag e se è rimasto un indice di stream.
fn format_flag(
    name: &str,
    letter: char,
    idx: Option<usize>,
    counts: &Counts,
    job: &Job,
    catalog: &Catalog,
) -> (String, bool) {
    // Più stream dello stesso tipo: serve lo specificatore completo
    if counts.get(letter) > 1 {
        return match idx {
            Some(i) => (format!("-{}:{}:{}", name, letter, i), true),
            None => (format!("-{}:{}", name, letter), false),
        };
    }

    // Un solo stream di questo tipo: l'indice si può togliere
    let text = match name {
        "filter" if letter == 'v' => "-vf".to_string(),
        "filter" if letter == 'a' => "-af".to_string(),
        "filter" => format!("-filter:{}", letter),
        "c" => format!("-c:{}", letter),
        _ => {
            if KEEP_TYPE_LETTER.contains(&name) || used_by_other_type(job, catalog, name, letter) {
                format!("-{}:{}", name, letter)
            } else {
                format!("-{}", name)
            }
        }
    };
    (text, false)
}

fn kind_for(name: &str, letter: char) -> TokenKind {
    if name == "filter" {
        return TokenKind::Filter;
    }
    match letter {
        'v' => TokenKind::Video,
        'a' => TokenKind::Audio,
        's' => TokenKind::Subtitle,
        _ => TokenKind::Global,
    }
}

/// `-map` si può omettere solo se ffmpeg sceglierebbe da solo esattamente gli stessi stream:
/// serve il riepilogo del file di ingresso, al massimo un video, un audio e un sottotitolo,
/// tutti usati, nessun allegato o dato, e nell'ordine automatico (video, audio, sottotitoli).
fn can_hide_map(job: &Job, input: Option<&InputSummary>) -> bool {
    let Some(inp) = input else {
        return false;
    };
    let c = Counts::from_job(job);
    if inp.other != 0 || c.attachment != 0 || c.data != 0 {
        return false;
    }
    if inp.video_like > 1 || inp.audio > 1 || inp.subtitle > 1 {
        return false;
    }
    if c.video != inp.video_like || c.audio != inp.audio || c.subtitle != inp.subtitle {
        return false;
    }
    let mut last = 0u8;
    for s in &job.output.streams {
        let rank = match type_letter(s.kind) {
            'v' => 0u8,
            'a' => 1,
            's' => 2,
            _ => 3,
        };
        if rank < last {
            return false;
        }
        last = rank;
    }
    true
}

/// `args` sono gli argomenti esatti del builder (quelli con `-hide_banner`, `-y`, `-c:v:0`...).
pub fn build_compact(
    job: &Job,
    catalog: &Catalog,
    args: &[String],
    input: Option<&InputSummary>,
) -> CompactCommand {
    let counts = Counts::from_job(job);
    let hide_map = can_hide_map(job, input);

    let mut tokens = vec![Token::new("ffmpeg", TokenKind::Program)];
    let mut kept_specifiers = false;
    let mut map_hidden = false;
    let mut saw_y = false;
    let mut output: Option<Token> = None;

    let n = args.len();
    let mut i = 0;
    while i < n {
        let a = args[i].as_str();

        // l'ultimo argomento è il file di uscita
        if i == n - 1 && !NO_VALUE_FLAGS.contains(&a) {
            let text = match Path::new(a).extension().and_then(|e| e.to_str()) {
                Some(ext) => format!("{{OUTPUT}}.{}", ext),
                None => "{OUTPUT}".to_string(),
            };
            output = Some(Token::new(text, TokenKind::Output));
            break;
        }

        match a {
            "-hide_banner" | "-nostats" | "-stats" | "-nostdin" => {
                i += 1;
                continue;
            }
            "-y" => {
                saw_y = true;
                i += 1;
                continue;
            }
            "-n" => {
                tokens.push(Token::new("-n", TokenKind::Global));
                i += 1;
                continue;
            }
            _ => {}
        }

        let value = args.get(i + 1).cloned().unwrap_or_default();
        match a {
            "-i" => {
                tokens.push(Token::new("-i", TokenKind::Input));
                tokens.push(Token::new(INPUT_PLACEHOLDER, TokenKind::Input));
            }
            "-map" => {
                if hide_map {
                    map_hidden = true;
                } else {
                    tokens.push(Token::new("-map", TokenKind::Map));
                    tokens.push(Token::new(value, TokenKind::Map));
                }
            }
            "-ss" | "-t" => {
                tokens.push(Token::new(a, TokenKind::Input));
                tokens.push(Token::new(value, TokenKind::Input));
            }
            "-hwaccel" | "-hwaccel_device" | "-hwaccel_output_format" | "-init_hw_device"
            | "-filter_hw_device" => {
                tokens.push(Token::new(a, TokenKind::Hardware));
                tokens.push(Token::new(value, TokenKind::Hardware));
            }
            _ => match parse_flag(a) {
                // con indice: è un'opzione di uno stream
                Some((name, letter, Some(idx))) => {
                    let (text, kept) = format_flag(name, letter, Some(idx), &counts, job, catalog);
                    kept_specifiers |= kept;
                    let kind = kind_for(name, letter);
                    tokens.push(Token::new(text, kind));
                    tokens.push(Token::new(value, kind));
                }
                // altre opzioni (anche `-frames:v 1`): si lasciano come sono
                _ => {
                    tokens.push(Token::new(a, TokenKind::Global));
                    tokens.push(Token::new(value, TokenKind::Global));
                }
            },
        }
        i += 2;
    }

    if let Some(o) = output {
        tokens.push(o);
    }
    if saw_y {
        tokens.push(Token::new("-y", TokenKind::Global));
    }

    CompactCommand { tokens, kept_specifiers, map_hidden }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::OutStream;
    use serde_json::json;
    use std::collections::HashMap;
    use std::process::Command;

    fn compact(job: &Job, catalog: &Catalog, hw: Option<&str>, input: Option<&InputSummary>) -> CompactCommand {
        let args = CommandBuilder::build_with_options(job, catalog, None, hw);
        build_compact(job, catalog, &args, input)
    }

    #[test]
    fn esempio_dell_utente_av1_nvenc_e_flac() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mkv");
        let mut v = HashMap::new();
        v.insert("rc".to_string(), json!("vbr"));
        v.insert("preset".to_string(), json!("p6"));
        v.insert("cq".to_string(), json!(24));
        v.insert("pix_fmt".to_string(), json!("p010le"));
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "av1_nvenc", v));
        let mut a = HashMap::new();
        a.insert("compression_level".to_string(), json!(5));
        job.output.streams.push(OutStream::new_encode(StreamKind::Audio, 1, "flac", a));

        let c = compact(&job, &catalog, Some("auto"), None);
        let s = c.to_text();
        assert!(s.starts_with("ffmpeg -hwaccel auto -i {INPUT} "), "{}", s);
        assert!(s.ends_with(" {OUTPUT}.mkv -y"), "{}", s);
        for pezzo in ["-c:v av1_nvenc", "-cq 24", "-preset p6", "-pix_fmt p010le", "-c:a flac", "-compression_level 5"] {
            assert!(s.contains(pezzo), "manca {} in: {}", pezzo, s);
        }
        assert!(!s.contains(":v:0") && !s.contains(":a:0"), "{}", s);
        assert!(!s.contains("-hide_banner"), "{}", s);
        assert!(!c.kept_specifiers);
    }

    #[test]
    fn due_audio_mantengono_gli_indici() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mka");
        job.output.streams.push(OutStream::new_encode(StreamKind::Audio, 1, "libopus", HashMap::new()));
        job.output.streams.push(OutStream::new_encode(StreamKind::Audio, 2, "libopus", HashMap::new()));
        let c = compact(&job, &catalog, None, None);
        let s = c.to_text();
        assert!(s.contains("-c:a:0 libopus") && s.contains("-c:a:1 libopus"), "{}", s);
        assert!(c.kept_specifiers);
    }

    #[test]
    fn video_con_copertina_conta_come_due_video() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mkv");
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "libx264", HashMap::new()));
        job.output.streams.push(OutStream::new_copy(StreamKind::CoverArt, 2));
        let s = compact(&job, &catalog, None, None).to_text();
        // senza indici, -c:v libx264 ricodificherebbe anche la copertina (verificato con ffmpeg)
        assert!(s.contains("-c:v:0 libx264") && s.contains("-c:v:1 copy"), "{}", s);
    }

    #[test]
    fn map_nascosto_solo_se_coincide_con_la_scelta_automatica() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mp4", "out.mkv");
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "libx264", HashMap::new()));
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));
        let input = InputSummary { video_like: 1, audio: 1, subtitle: 0, other: 0 };

        let c = compact(&job, &catalog, None, Some(&input));
        assert!(!c.to_text().contains("-map") && c.map_hidden);

        // senza il riepilogo del file il -map resta
        assert!(compact(&job, &catalog, None, None).to_text().contains("-map 0:0 -map 0:1"));

        // se l'utente ha tolto l'audio, il -map serve per escluderlo
        job.output.streams.pop();
        let c = compact(&job, &catalog, None, Some(&input));
        assert!(c.to_text().contains("-map 0:0") && !c.map_hidden);

        // file con un allegato: mai nascosto
        let with_att = InputSummary { video_like: 1, audio: 1, subtitle: 0, other: 1 };
        job.output.streams.push(OutStream::new_copy(StreamKind::Audio, 1));
        assert!(compact(&job, &catalog, None, Some(&with_att)).to_text().contains("-map"));
    }

    #[test]
    fn opzioni_generiche_tengono_la_lettera_del_tipo() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mkv");
        let mut v = HashMap::new();
        v.insert("rc".to_string(), json!("vbr"));
        v.insert("b_v".to_string(), json!("0"));
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "av1_nvenc", v));
        let s = compact(&job, &catalog, None, None).to_text();
        assert!(s.contains("-b:v 0"), "{}", s);
    }

    #[test]
    fn filtro_singolo_diventa_vf() {
        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mkv");
        let mut st = OutStream::new_encode(StreamKind::Video, 0, "libx264", HashMap::new());
        st.filters.push("scale=-2:1080".to_string());
        job.output.streams.push(st);
        let s = compact(&job, &catalog, None, None).to_text();
        assert!(s.contains("-vf scale=-2:1080"), "{}", s);
    }

    #[test]
    fn quoting_e_segnaposto() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("scale=-2:1080,format=nv12"), "scale=-2:1080,format=nv12");

        let catalog = Catalog::load_default(false);
        let mut job = Job::new("in.mkv", "out.mkv");
        job.output.streams.push(OutStream::new_copy(StreamKind::Video, 0));
        let c = compact(&job, &catalog, None, None);
        let args = c.to_args("/video/it's.mkv", "/out/it's.mkv");
        assert!(args.contains(&"/video/it's.mkv".to_string()));
        assert_eq!(args[args.len() - 2], "/out/it's.mkv");
        assert_eq!(args[args.len() - 1], "-y");
    }

    #[test]
    fn percorso_part() {
        assert_eq!(part_path(Path::new("/a/b/film.mp4")), PathBuf::from("/a/b/film.part.mp4"));
    }

    // --- equivalenza con ffmpeg vero -----------------------------------------------------

    fn md5_of(path: &Path, map: &str) -> String {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-map", map, "-f", "md5", "-"])
            .output()
            .expect("ffmpeg non trovato nel PATH");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn compatto_ed_esatto_producono_lo_stesso_risultato() {
        let ok = Command::new("ffmpeg").arg("-version").output().map(|o| o.status.success()).unwrap_or(false);
        assert!(ok, "ffmpeg non trovato nel PATH: il test di equivalenza ne ha bisogno");

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in.mkv");
        let st = Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i", "testsrc2=d=1:s=160x120:r=25"])
            .args(["-f", "lavfi", "-i", "sine=d=1", "-c:v", "mpeg4", "-c:a", "aac", "-y"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());

        let catalog = Catalog::load_default(false);
        let out_exact = dir.path().join("exact.mkv");
        let out_compact = dir.path().join("compact.mkv");

        let mut job = Job::new(&src, &out_exact);
        let mut v = HashMap::new();
        v.insert("crf".to_string(), json!(30));
        v.insert("preset".to_string(), json!("ultrafast"));
        job.output.streams.push(OutStream::new_encode(StreamKind::Video, 0, "libx264", v));
        let mut a = HashMap::new();
        a.insert("compression_level".to_string(), json!(8));
        job.output.streams.push(OutStream::new_encode(StreamKind::Audio, 1, "flac", a));

        let exact = CommandBuilder::build_with_options(&job, &catalog, None, None);
        let c = build_compact(&job, &catalog, &exact, None);
        let compact_args = c.to_args(src.to_str().unwrap(), out_compact.to_str().unwrap());

        for args in [&exact, &compact_args] {
            let st = Command::new("ffmpeg").args(["-v", "error", "-nostdin"]).args(args).status().unwrap();
            assert!(st.success(), "ffmpeg ha fallito con: {:?}", args);
        }
        assert_eq!(md5_of(&out_exact, "0:v"), md5_of(&out_compact, "0:v"));
        assert_eq!(md5_of(&out_exact, "0:a"), md5_of(&out_compact, "0:a"));
    }
}
