#!/usr/bin/env python3
"""
whisper_run.py - Runner script for faster-whisper used by ffgui.
Outputs JSON lines for each segment to stdout for live progress tracking,
and writes the final SRT or VTT file.
"""

import argparse
import json
import os
import sys

def ts(seconds: float, sep: str) -> str:
    ms = int(round(seconds * 1000))
    h, ms = divmod(ms, 3600000)
    m, ms = divmod(ms, 60000)
    s, ms = divmod(ms, 1000)
    return f"{h:02d}:{m:02d}:{s:02d}{sep}{ms:03d}"

def format_timestamp_srt(seconds: float) -> str:
    return ts(seconds, ",")

def format_timestamp_vtt(seconds: float) -> str:
    return ts(seconds, ".")

def write_srt(segments, output_path: str):
    with open(output_path, "w", encoding="utf-8") as f:
        for i, seg in enumerate(segments, start=1):
            start_str = format_timestamp_srt(seg.start)
            end_str = format_timestamp_srt(seg.end)
            f.write(f"{i}\n{start_str} --> {end_str}\n{seg.text.strip()}\n\n")

def write_vtt(segments, output_path: str):
    with open(output_path, "w", encoding="utf-8") as f:
        f.write("WEBVTT\n\n")
        for i, seg in enumerate(segments, start=1):
            start_str = format_timestamp_vtt(seg.start)
            end_str = format_timestamp_vtt(seg.end)
            f.write(f"{i}\n{start_str} --> {end_str}\n{seg.text.strip()}\n\n")

def main():
    parser = argparse.ArgumentParser(description="ffgui faster-whisper runner")
    parser.add_argument("--audio", required=True, help="Path to input audio/wav file")
    parser.add_argument("--output", required=True, help="Path to output subtitle file (.srt or .vtt)")
    parser.add_argument("--model", default="base", help="Model name: tiny, base, small, medium, large-v3")
    parser.add_argument("--language", default=None, help="Language code (e.g. it, en) or auto if omitted")
    parser.add_argument("--device", default="cpu", choices=["cpu", "cuda"], help="Computation device")
    parser.add_argument("--compute-type", default="default", help="Compute type: default, float16, int8, etc.")
    parser.add_argument("--vad", action="store_true", help="Enable Voice Activity Detection filter")
    parser.add_argument("--format", default=None, choices=["srt", "vtt"], help="Output subtitle format")

    args = parser.parse_args()

    if not os.path.exists(args.audio):
        sys.stderr.write(f"Error: audio file does not exist: {args.audio}\n")
        sys.exit(1)

    try:
        from faster_whisper import WhisperModel
    except ImportError:
        sys.stderr.write("faster-whisper is not installed in this environment.\n")
        sys.exit(2)

    compute_type = args.compute_type
    if compute_type == "default":
        compute_type = "float16" if args.device == "cuda" else "int8"

    sys.stderr.write(f"Caricamento modello '{args.model}' su {args.device} ({compute_type})...\n")
    model = WhisperModel(args.model, device=args.device, compute_type=compute_type)

    lang = args.language if args.language and args.language.lower() != "auto" else None

    sys.stderr.write("Inizio trascrizione audio...\n")
    segments_gen, info = model.transcribe(
        args.audio,
        language=lang,
        vad_filter=args.vad,
    )

    sys.stderr.write(f"Lingua rilevata: {info.language} (probabilità {info.language_probability:.2f}), durata: {info.duration:.2f}s\n")

    segments_list = []
    for segment in segments_gen:
        segments_list.append(segment)
        # Emit single JSON line for progress tracking
        data = {
            "start": segment.start,
            "end": segment.end,
            "text": segment.text.strip(),
            "total_duration": info.duration,
        }
        print(json.dumps(data), flush=True)

    fmt = args.format
    if not fmt:
        if args.output.lower().endswith(".vtt"):
            fmt = "vtt"
        else:
            fmt = "srt"

    if fmt == "vtt":
        write_vtt(segments_list, args.output)
    else:
        write_srt(segments_list, args.output)

    sys.stderr.write(f"Trascrizione completata con successo: {args.output}\n")

if __name__ == "__main__":
    main()
