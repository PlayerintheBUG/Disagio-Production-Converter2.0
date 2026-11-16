# 9394bb Guida Rapida a ffgui

ffgui  un'interfaccia grafica **open source** per FFmpeg che ti permette di convertire file audio e video in modo **semplice, trasparente e professionale**.

---

## 4ca Indice
- [4cd Installazione](#-installazione)
- [4c1 Interfaccia Principale](#-interfaccia-principale)
- [3a9 Conversione File Singolo](#-conversione-file-singolo)
- [4ca Conversione Batch](#-conversione-batch)
- [399 Codec Supportati](#-codec-supportati)
- [943 Impostazioni Avanzate](#-impostazioni-avanzate)
- [468 Risoluzione Problemi](#-risoluzione-problemi)

---

## 4cd **Installazione**

### 4bb Requisiti
- **Windows/Linux** (macOS non ancora supportato)
- **FFmpeg e FFprobe** installati e nel PATH
- **Python** (opzionale, solo per trascrizione automatica con Whisper)

### 4cc Da GitHub (Eseguibile Precompilato)
1. Scarica l'eseguibile dalla sezione **Releases** di GitHub:
   - [ffgui Releases](https://github.com/PlayerintheBUG/Disagio-Production-Converter2.0/releases)
2. Scompatta il file `.zip` (Windows) o `.tar.gz` (Linux)
3. Esegui `ffgui.exe` (Windows) o `./ffgui` (Linux)

### 4cc Da Sorgente (Per Sviluppatori)
```bash
# Clona il repository
git clone https://github.com/PlayerintheBUG/Disagio-Production-Converter2.0.git
cd Disagio-Production-Converter2.0

# Compila e esegui
cargo run --bin ffgui

# Oppure compila la versione ottimizzata
cargo build --release
./target/release/ffgui
```

---

## 4c1 **Interfaccia Principale**

L'interfaccia  divisa in **3 colonne principali**:

```
┌─────────────────┬─────────────────┬─────────────────┐
│  STREAM PANEL   │ CONTROLS PANEL  │  PREVIEW PANEL  │
│  (Sorgente)     │  (Impostazioni)  │   (Anteprima)   │
└─────────────────┴─────────────────┴─────────────────┘
```

### 531e51 Pannello Stream (Sinistra)
- **Apri file**: Carica un file audio/video
- **Informazioni file**: Durata, formato, bitrate
- **Elenco stream**: Video, Audio, Sottotitoli con checkbox per **abilitare/disabilitare**
- **Destinazione**: Seleziona dove salvare il file convertito
- **Formato contenitore**: Cambia estensione (mp4, mkv, mov, ecc.)

### 530e80 Pannello Controlli (Centro)
- **Selettore modalit**: 
  - 3a9 **File Singolo** (conversione singola)
  - 4ca **Conversione Batch** (conversione multipla)
  - 4bb **Video** (impostazioni video)
  - 3b5 **Audio** (impostazioni audio)
  - 3ac **Sottotitoli** (gestione sottotitoli)
  - 3f7 **Immagine** (conversione immagini)
- **Codec**: Seleziona il codec (es. libx265, libopus, aac)
- **Parametri**: Configura CRF, bitrate, preset, sample rate, ecc.
- **Filtri**: Taglio, ridimensionamento, rotazione
- **GPU**: Abilita accelerazione hardware (NVENC, VAAPI, QSV, AMF)

### 530e80 Pannello Anteprima (Destra)
- **Comando FFmpeg**: Mostra il comando esatto che verr eseguito
- **Anteprima compatta**: Versione semplificata del comando
- **Copia comando**: Copia negli appunti con un click
- **Valida**: Verifica che il comando sia corretto
- **Esegui**: Avvia la conversione

### 530e50 Pannello Log (In basso)
- Mostra l'output di FFmpeg in tempo reale
- Progress bar con percentuale di completamento
- Velocit di encoding (es. 2.5x)
- Tempo rimanente

---

## 3a9 **Conversione File Singolo**

### Passaggi:
1. **Apri un file** (trascina o clicca "Sfoglia file...")
2. **Seleziona gli stream** da mantenere (spunta/dispunta le checkbox)
3. **Scegli la modalit** (Video, Audio, Immagine, Sottotitoli)
4. **Seleziona il codec** (es. libx265 per video, aac per audio)
5. **Configura i parametri**:
   - **Video**: CRF (18-28), Preset (ultrafast, medium, slow), Pix Format
   - **Audio**: Bitrate, Sample Rate (44100, 48000, 96000), Canali
6. **Seleziona destinazione** e formato contenitore
7. **Clicca "Esegui"** per avviare la conversione

### Esempio: Convertire MP4 in H.265
1. Apri `video.mp4`
2. Seleziona codec video: **libx265**
3. Imposta CRF: **23** (buon compromesso qualit/dimensione)
4. Seleziona formato: **mkv**
5. Clicca **Esegui**

Comando generato:
```bash
ffmpeg -i video.mp4 -c:v libx265 -crf 23 -preset medium -c:a copy output.mkv
```

---

## 4ca **Conversione Batch**

### Passaggi:
1. **Seleziona cartella sorgente** (trascina o clicca "Sfoglia...")
2. **Seleziona estensioni** da convertire (chip colorati)
3. **Configura profili**:
   - **Video**: Archivio H.265, Compatibile H.264, Copia da principale
   - **Audio**: Copia traccia, Ricodifica Opus/AAC
   - **Immagini**: WebP, JPG, PNG
4. **Seleziona destinazione** e naming template
5. **Imposta politica collisioni**: Salta, Sovrascrivi, Rinomina
6. **Clicca "Avvia conversione batch"**

### Funzionalit avanzate:
- **Calcolo spazio**: Mostra spazio necessario e disponibile
- **Piano live**: Tabella con stato di ogni file
- **Esecuzione concorrente**: Pi file in parallelo
- **Fallback CPU**: Se GPU fallisce, ritenta con CPU

---

## 399 **Codec Supportati**

### 3b5 Codec Video
| Codec | Tipo | Qualit | Note |
|-------|------|-----------|------|
| **libx265** | Software | 530530530530530 | HEVC, lossy, ottimo per archiviazione |
| **libx264** | Software | 530530530530 | H.264, compatibile con tutti i dispositivi |
| **libsvtav1** | Software | 530530530 | AV1, codec moderno royalty-free |
| **av1_nvenc** | GPU (NVIDIA) | 530530530530 | AV1 hardware, veloce |
| **hevc_nvenc** | GPU (NVIDIA) | 530530530 | HEVC hardware |
| **h264_nvenc** | GPU (NVIDIA) | 530530530 | H.264 hardware |
| **prores_ks** | Software | 530530530 | ProRes, editing professionale |
| **prores_aw** | Software | 530530530 | ProRes alternativa |
| **dnxhr** | Software | 530530530 | DNxHR, editing Avid |
| **dnxhd** | Software | 530530 | DNxHD, editing legacy |

### 3b5 Codec Audio
| Codec | Tipo | Bitrate | Sample Rate |
|-------|------|---------|-------------|
| **libopus** | Lossy | 64k-320k | 8k-96kHz |
| **aac** | Lossy | 96k-320k | 8k-192kHz |
| **flac** | Lossless | - | 8k-192kHz |
| **libmp3lame** | Lossy | 96k-320k | 8k-48kHz |
| **pcm** | Lossless | - | 8k-192kHz |
| **dsd** | Lossless | - | 44.1k-705.6kHz |

### 3ac Codec Sottotitoli
- **Copy**: Copia sottotitoli senza ricodifica
- **Burn**: Brucia sottotitoli nel video
- **Transcribe**: Trascrizione automatica con Whisper

---

## 943 **Impostazioni Avanzate**

### 527 Pannello Impostazioni (850e80)
- **Livello log FFmpeg**: info, warning, error, quiet
- **Modello Whisper**: base, small, medium, large-v3
- **Sessioni GPU massime**: Numero di encoding GPU simultanei
- **Fallback CPU**: Abilita ritentativo su CPU se GPU fallisce
- **Percorsi personalizzati**: Specificare path di ffmpeg/ffprobe

### 943 Qualit Automatica
- **Manuale**: Usa parametri scelti dall'utente
- **Preset**: Alta/Media/Bassa (valori predefiniti)
- **Adattiva**: Calcola automaticamente CRF/CQ in base a:
  - Risoluzione di destinazione
  - FPS
  - Profondit colore
  - Complessit del video
- **Limite dimensione**: Calcola bitrate per rispettare un target in MB

### 943 Modalit Batch
- **Scansione automatica**: Rileva estensioni e media kind
- **Piano live**: Anteprima di tutti i file da convertire
- **Esecuzione concorrente**: Pi file in parallelo
- **Report dettagliato**: Statistiche e errori

---

## 468 **Risoluzione Problemi**

### 2748 "FFmpeg non trovato nel PATH"
**Soluzione**: Installa FFmpeg e aggiungilo al PATH
- **Windows**: Scarica da [ffmpeg.org](https://ffmpeg.org) e aggiungi al PATH
- **Linux**: `sudo apt install ffmpeg` (Debian/Ubuntu)

### 2748 "Python non trovato"
**Soluzione**: Installa Python 3.8+
- **Windows**: Scarica da [python.org](https://python.org)
- **Linux**: `sudo apt install python3`
- **Note**: Serve solo per la trascrizione automatica con Whisper

### 2748 "Terminali vuoti su Windows"
**Soluzione**: Usa l'eseguibile precompilato da GitHub Releases (gi risolto)
- Non eseguire con `cargo run` (mostra console di debug)
- Esegui direttamente `ffgui.exe`

### 2748 "Non tutti i flussi sono visibili"
**Soluzione**: Usa lo scroll nel pannello Stream
- Il pannello  ora scorrevole automaticamente
- Se ci sono molti sottotitoli, usa lo scroll per vederli tutti

### 2748 "Conversione lenta"
**Soluzioni**:
- Abilita **accelerazione GPU** (NVENC, VAAPI, QSV, AMF)
- Usa **preset pi veloci** (ultrafast, superfast)
- Riduci il **CRF** (valori pi alti = file pi piccoli ma qualit inferiore)
- Diminuisci il **numero di job paralleli** in batch

### 2748 "File di output corrotti"
**Soluzioni**:
- Verifica che ci sia spazio su disco
- Usa contenitore **MKV** (pi robusto)
- Abilita **scrittura atomica** (file .part temporanei)

---

## 4cc **Risorse Utili**
- 310 **Repository**: [GitHub - Disagio-Production-Converter2.0](https://github.com/PlayerintheBUG/Disagio-Production-Converter2.0)
- 419 **Documentazione FFmpeg**: [ffmpeg.org](https://ffmpeg.org)
- 931 **Guida codec**: [FFmpeg Wiki](https://trac.ffmpeg.org/wiki)

---

## 499 **Note di Rilascio**

### Novit Recenti
- 976 **Fix terminali Windows**: Risolto problema terminali vuoti su Windows
- 976 **Nuovi codec audio**: Aggiunti PCM, DSD con supporto sample rate
- 976 **Nuovi codec video**: Aggiunti DNxHR, DNxHD, ProRes AW
- 976 **Sample rate configurabile**: Ora possibile cambiare frequenza audio per tutti i codec
- 976 **Fix scroll stream**: Pannello stream ora scorrevole per molti flussi

---

## 487 **Autori e Licenza**
- **Autore**: PlayerintheBUG
- **Licenza**: [GPL-3.0](https://github.com/PlayerintheBUG/Disagio-Production-Converter2.0/blob/main/LICENSE)
- **Tecnologie**: Rust, egui, FFmpeg

---

**Grazie per usare ffgui!** 499

Se hai domande o segnalazioni, apri una **Issue** su GitHub.
