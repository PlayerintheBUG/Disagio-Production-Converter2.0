# ffgui – Interfaccia grafica trasparente per FFmpeg

GUI avanzata e trasparente per **FFmpeg** sviluppata in **Rust** (con motore `core` indipendente e GUI reattiva in `egui`).

Non nasconde FFmpeg: genera i comandi in tempo reale, mostra sempre la riga di comando esatta, valida i parametri prima dell'avvio e visualizza il log e l'avanzamento in streaming come in un terminale.

---

## 🎯 Principi di Design

1. **FFmpeg resta il motore**: La GUI genera argomenti e lancia i processi con stream pipe standard, senza reimplementare nulla.
2. **Trasparenza totale**: Il comando generato è sempre visibile in un campo dedicato, con evidenziazione sintattica dei parametri modificati e pulsante rapido per la copia. I log di FFmpeg appaiono in tempo reale in una console monospace virtualizzata ad alte prestazioni con il comando `$ ffmpeg <args>` come prima riga.
3. **Controllo completo**: Tutti i parametri dei codec (CRF/CQ, preset di velocità, formato colore/pixel, bitrate, ecc.) sono esposti tramite controlli dinamici.
4. **Accelerazione Hardware GPU**: Rilevamento automatico hardware NVENC (NVIDIA), VAAPI (Intel/AMD), QSV (Intel) e AMF (AMD), con supporto completo a codifica AV1 hardware (`av1_nvenc`, `av1_vaapi`, `av1_qsv`, `av1_amf`), commutazione rapida CPU ↔ GPU e decodifica hardware (`-hwaccel auto/cuda/vaapi/qsv`).
5. **Dati al posto del codice (Catalogo TOML)**: I codec vivono in file `.toml` all'interno della cartella `codecs/` (e in `~/.config/ffgui/codecs/`). Aggiungere o modificare un codec o i suoi preset non richiede alcuna ricompilazione!
6. **Architettura a due strati**:
   - `ffgui-core`: libreria autonoma (catalog, probe, validate, builder, runner, queue, batch, storage). Testabile al 100% senza aprire alcuna finestra.
   - `ffgui`: applicazione grafica sottile costruita con `egui` / `eframe`.

---

## 🏗 Struttura del Progetto

```
DPC2.0/
├── Cargo.toml                  # Workspace Rust (core + gui)
├── codecs/                     # Definizioni TOML dei codec (dati, non codice!)
│   ├── libx265.toml            # H.265 / HEVC con preset Max, Media, Bassa
│   ├── libx264.toml            # H.264 / AVC
│   ├── libsvtav1.toml          # AV1 SVT (CPU)
│   ├── av1_nvenc.toml          # AV1 NVENC (GPU NVIDIA)
│   ├── av1_vaapi.toml          # AV1 VAAPI (GPU Intel/AMD Linux)
│   ├── hevc_nvenc.toml         # H.265 / HEVC NVENC
│   ├── h264_nvenc.toml         # H.264 / AVC NVENC
│   ├── libopus.toml            # Opus Audio
│   ├── aac.toml                # AAC Audio
│   ├── flac.toml               # FLAC Lossless
│   ├── libmp3lame.toml         # MP3 LAME
│   ├── prores_ks.toml          # Apple ProRes
│   └── libwebp.toml            # WebP Immagine
├── core/                       # Libreria core indipendente
│   ├── src/
│   │   ├── catalog.rs          # Parser TOML, fallback e filtro encoder ffmpeg
│   │   ├── model.rs            # Strutture dati Job, Input, Output, OutStream
│   │   ├── probe.rs            # Analisi stream e metadati tramite ffprobe JSON
│   │   ├── validate.rs         # Validazione a monte (incompatibilità container/filtri/limiti)
│   │   ├── builder.rs          # Modello -> argomenti Vec<String> e stringa preview
│   │   ├── runner.rs           # Process runner, pipe stdout/stderr, progress parsing, kill pulito
│   │   ├── queue.rs            # Coda di lavori (JobQueue) e step pipeline
│   │   ├── batch.rs            # Motore batch: scansione, calcolo CQ, piano live, report
│   │   ├── storage.rs          # Gestione configurazioni XDG, AppSettings e salvataggio preset
│   │   └── backends/
│   │       ├── ffmpeg.rs       # Estrazione audio whisper, muxing sottotitoli ed escape filtri
│   │       ├── gpu.rs          # Rilevamento GPU NVIDIA / VAAPI / QSV / AMF
│   │       └── whisper.rs      # Isolamento venv Python e runner faster-whisper
│   └── src/bin/ffcli.rs        # Tool CLI di verifica e diagnostica automatizzata
├── gui/                        # Interfaccia grafica egui
│   └── src/
│       ├── main.rs             # Tab navigation, drag & drop, layout a 3 colonne e dock
│       ├── app.rs              # Stato dell'applicazione, polling eventi, scan batch asincrono
│       └── ui/
│           ├── stream_panel.rs # Selezione file, riepilogo probe e tracce sorgente
│           ├── controls_panel.rs # Controlli dinamici TOML, slider, dropdown, filtri e whisper
│           ├── queue_panel.rs  # Coda di conversione con progress bar aggregata e preset personali
│           ├── preview_panel.rs# Barra comando live, token diff, copia clipboard e validazione
│           ├── log_panel.rs    # Terminale monospace virtualizzato, progress bar indeterminata/percentuale
│           ├── batch_dialog.rs # Pagina a due colonne per conversione batch
│           └── settings_dialog.rs # Finestra impostazioni (GPU sessions, whisper, log level, fallback CPU)
└── tools/
    └── whisper_run.py          # Script standalone per trascrizione con faster-whisper
```

---

## 🚀 Istruzioni d'Uso

### 1. Avviare l'Applicazione
Dalla cartella principale del workspace:
```bash
cargo run --bin ffgui
```
Per compilare la versione ottimizzata finale:
```bash
cargo build --release
```
L'eseguibile compilato si troverà in `target/release/ffgui`.

### 2. File Singolo (`[ 🎬 File Singolo ]`)
1. **Apri un file**: Clicca su **📂 Sfoglia file...** oppure trascina un file audio/video all'interno della finestra.
2. **Esamina gli stream**: La colonna di sinistra mostra tutti i flussi rilevati con codec, risoluzione, fps, frequenza e canali audio.
3. **Scegli la modalità**: Nella colonna centrale puoi passare da `[🎬 Video]`, `[🎵 Audio]`, `[🖼 Immagine]` a `[💬 Sottotitoli]`.
4. **Accelerazione GPU e Codec**: Premi il pulsante GPU per commutare rapidamente al codec con accelerazione hardware (incluso AV1 con fallback su VAAPI/NVENC).
5. **Filtri e Taglio**: Seleziona una risoluzione proporzionale (es. 1080p, 720p) o specifica inizio (`-ss`) e durata (`-t`).
6. **Comando Live**: Il comando esatto di FFmpeg si aggiorna in tempo reale con escaping shell affidabile e copia con un click.
7. **Scrittura Atomica `.part`**: I file vengono salvati inizialmente come `.part` e rinominati solo al termine con successo per evitare file corrotti.

### 3. Conversione Batch a 2 Colonne (`[ 📁 Conversione Batch ]`)
Il tab Batch implementa l'algoritmo intelligente di conversione e archiviazione:
1. **Sorgente con Chip Estensioni**: Seleziona una cartella di origine (o trascinala nella finestra). Trova e visualizza chip interattivi per ogni estensione con conteggio file, permettendo di abilitare/disabilitare estensioni con un click.
2. **Profili per MediaKind**:
   - **Video**: Preset automatici (`Archivio H.265/AV1`, `Compatibile H.264`, o `Copia da Principale`), con calcolo automatico CQ in base a risoluzione e complessità.
   - **Audio**: Opzioni rapide per copia traccia o ricodifica (Opus/AAC).
   - **Immagini**: Conversione di cartelle di foto in WebP/JPG/PNG.
   - **File non multimediali**: Opzione per copiare copertine, file `.srt` e `.nfo`.
3. **Destinazione e Naming Template**:
   - Sottocartella automatica o cartella personalizzata.
   - Struttura ad albero opzionale.
   - Template di ridenominazione con segnaposti (`{nome}`, `{codec}`, `{qualita}`, `{risoluzione}`, `{n}`).
   - Gestione collisioni: Salta, Sovrascrivi o Rinomina aggiungendo `(2)`.
4. **Piano Live e Controllo Spazio**:
   - Tabella interattiva dei file con stato (`NUOVO`, `SOVRASCRIVI`, `SALTA`, `ESCLUSO`).
   - Calcolo dello spazio stimato in output e verifica automatica dello spazio libero effettivo su disco.
5. **Esecuzione Concorrente & Fallback CPU**:
   - Slider per numero di file paralleli e sessioni GPU massime simultanee.
   - **Fallback automatico su CPU**: Se una codifica GPU fallisce per limite di memoria o codec non supportato, il lavoro viene rieseguito automaticamente su CPU con parametri equivalenti.

### 4. Impostazioni Avanzate (`⚙ Impostazioni`)
- Livello di log di FFmpeg (`info`, `warning`, `error`, `quiet`).
- Modello Whisper preferito (`base`, `small`, `medium`, `large-v3`).
- Numero massimo di sessioni GPU concorrenti.
- Checkbox per il fallback automatico su CPU.
- Percorsi personalizzati per i binari di `ffmpeg` e `ffprobe`.

---

## 🧪 Esecuzione dei Test Unitari
Tutti i test del catalogo, validazione delle regole, builder deterministico, scansione e pianificazione batch, synthetic encode test su lavfi e controller di processo:
```bash
cargo test --workspace
```
