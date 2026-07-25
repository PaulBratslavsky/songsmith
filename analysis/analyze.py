#!/usr/bin/env python3
"""Reference-track analyzer (local-light, no Torch) — the "perception" layer.

    analysis/.venv/bin/python analysis/analyze.py <audio_file> [--json]

Extracts tempo, key, bar-level chord candidates, and structural section
boundaries using librosa only. Output is intentionally raw/noisy: it's meant to
be handed to a "Reference Analyst" Claude skill that does the musical reasoning
(clean the chords, label sections by function) and writes our Structure + Chords.

Keeps audio 100% local. Prints a JSON object to stdout.
"""
import argparse, json, sys
import numpy as np
import librosa


def separate_stems(audio_path):
    """Demucs (htdemucs, CPU) → temp vocals.wav + bass.wav. Returns
    ({name: path}, tmpdir) or ({}, None) if anything is missing/fails."""
    try:
        import pathlib, tempfile
        from demucs.api import Separator, save_audio
        print("separating stems (demucs, CPU — a minute or two)…", file=sys.stderr)
        from contextlib import redirect_stdout
        with redirect_stdout(sys.stderr):  # keep stdout JSON-clean
            sep = Separator(model="htdemucs", device="cpu")
            _origin, separated = sep.separate_audio_file(audio_path)
        tmp = tempfile.mkdtemp(prefix="songsmith-stems-")
        out = {}
        for name in ("vocals", "bass"):
            p = str(pathlib.Path(tmp) / f"{name}.wav")
            save_audio(separated[name], p, samplerate=sep.samplerate)
            out[name] = p
        return out, tmp
    except Exception as e:  # torch missing / OOM / decode — degrade, never sink
        print(f"(stem separation unavailable: {e})", file=sys.stderr)
        return {}, None


def mono_clean(events, min_dur=0.08, min_amp=0.12):
    """Reduce basic-pitch note events to a clean monophonic line: drop the
    quiet/blip notes, then resolve overlaps by truncating the earlier note
    (dropping it if the truncation leaves a blip)."""
    evs = sorted((e for e in events if e["end"] - e["start"] >= min_dur and e["amp"] >= min_amp),
                 key=lambda e: (e["start"], -e["amp"]))
    out = []
    for e in evs:
        keep = True
        while out and e["start"] < out[-1]["end"]:
            prev = out[-1]
            if e["amp"] < 0.6 * prev["amp"] and e["end"] <= prev["end"] + 0.02:
                keep = False  # a quiet blip riding a strong held note
                break
            prev["end"] = round(e["start"], 3)  # a real new onset truncates it
            if prev["end"] - prev["start"] < min_dur:
                out.pop()  # truncation left a blip — evict, recheck the one before
            else:
                break
        if keep:
            out.append(dict(e))
    return out


def transcribe_notes(stems):
    """basic-pitch (ONNX) on the vocals + bass stems → melody/bass note events
    [{start, end, midi, amp}] in seconds. Empty lists on any failure."""
    try:
        # scipy ≥1.13 moved gaussian to signal.windows; basic-pitch still uses
        # the old name — shim it rather than downgrading scipy under librosa
        import scipy.signal
        if not hasattr(scipy.signal, "gaussian"):
            scipy.signal.gaussian = scipy.signal.windows.gaussian
        from basic_pitch import build_icassp_2022_model_path, FilenameSuffix
        from basic_pitch.inference import Model, predict
        model = Model(build_icassp_2022_model_path(FilenameSuffix.onnx))

        def notes_from(path, fmin, fmax):
            # basic-pitch prints "Predicting MIDI for …" to STDOUT — which is
            # our JSON channel; shunt it to stderr or the output won't parse
            from contextlib import redirect_stdout
            with redirect_stdout(sys.stderr):
                _out, _midi, events = predict(
                    path, model, minimum_frequency=fmin, maximum_frequency=fmax,
                    minimum_note_length=80.0)
            evs = [{"start": round(float(s), 3), "end": round(float(e), 3),
                    "midi": int(p), "amp": round(float(a), 3)}
                   for (s, e, p, a, _bends) in events]
            return mono_clean(evs)[:1500]

        print("transcribing melody + bass (basic-pitch)…", file=sys.stderr)
        melody = notes_from(stems["vocals"], 80.0, 1100.0) if "vocals" in stems else []
        bass = notes_from(stems["bass"], 28.0, 300.0) if "bass" in stems else []
        return melody, bass
    except Exception as e:
        print(f"(note transcription unavailable: {e})", file=sys.stderr)
        return [], []

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]

# Krumhansl-Schmuckler key profiles (major, minor)
KS_MAJ = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
KS_MIN = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])


def estimate_key(chroma_mean):
    """Correlate the mean chroma against rotated major/minor profiles."""
    best = (-1e9, "C", "major")
    for pc in range(12):
        for mode, prof in (("major", KS_MAJ), ("minor", KS_MIN)):
            r = np.corrcoef(np.roll(prof, pc), chroma_mean)[0, 1]
            if r > best[0]:
                best = (r, NOTE_NAMES[pc], mode)
    return {"root": best[1], "mode": best[2], "confidence": round(float(best[0]), 3)}


def triad_templates():
    """24 binary triad templates: 12 major, 12 minor (root, third, fifth)."""
    tmpl, labels = [], []
    for pc in range(12):
        maj = np.zeros(12); maj[[pc, (pc + 4) % 12, (pc + 7) % 12]] = 1
        minr = np.zeros(12); minr[[pc, (pc + 3) % 12, (pc + 7) % 12]] = 1
        tmpl.append(maj); labels.append(NOTE_NAMES[pc])
        tmpl.append(minr); labels.append(NOTE_NAMES[pc] + "m")
    return np.array(tmpl), labels


def chord_for(chroma_vec, tmpl, labels):
    v = chroma_vec / (np.linalg.norm(chroma_vec) + 1e-9)
    scores = tmpl @ v
    return labels[int(np.argmax(scores))]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("audio")
    ap.add_argument("--sections", type=int, default=8, help="approx number of sections to find")
    ap.add_argument("--lyrics", action="store_true", help="also transcribe sung lyrics (faster-whisper, local)")
    ap.add_argument("--stems", action="store_true", help="demucs stem separation + basic-pitch melody/bass MIDI (slower)")
    args = ap.parse_args()

    # --- stems first (Phase 2): vocals/bass stems feed BOTH the note
    # transcription and (when --lyrics) a cleaner whisper pass ---
    stems, stems_tmp = separate_stems(args.audio) if args.stems else ({}, None)

    y, sr = librosa.load(args.audio, mono=True)
    dur = librosa.get_duration(y=y, sr=sr)

    # --- tempo + beats ---
    tempo, beats = librosa.beat.beat_track(y=y, sr=sr, units="frames")
    tempo = float(np.atleast_1d(tempo)[0])
    beat_times = librosa.frames_to_time(beats, sr=sr)

    # --- chroma (beat-synced) ---
    chroma = librosa.feature.chroma_cqt(y=y, sr=sr)
    key = estimate_key(chroma.mean(axis=1))
    beat_chroma = librosa.util.sync(chroma, beats, aggregate=np.median)

    # --- bar-level chords (assume 4/4: 4 beats per bar) ---
    tmpl, labels = triad_templates()
    nbeats = beat_chroma.shape[1]
    bar_chords = []
    for b0 in range(0, nbeats, 4):
        seg = beat_chroma[:, b0:b0 + 4]
        if seg.shape[1] == 0:
            break
        t = float(beat_times[b0]) if b0 < len(beat_times) else None
        bar_chords.append({"bar": len(bar_chords) + 1, "time": round(t, 2) if t else None,
                           "chord": chord_for(seg.mean(axis=1), tmpl, labels)})

    # --- structural segmentation (agglomerative on beat-synced features) ---
    mfcc = librosa.feature.mfcc(y=y, sr=sr, n_mfcc=13)
    beat_mfcc = librosa.util.sync(mfcc, beats, aggregate=np.mean)
    feat = np.vstack([librosa.util.normalize(beat_chroma, axis=0),
                      librosa.util.normalize(beat_mfcc, axis=0)])
    nseg = max(2, min(args.sections, feat.shape[1] - 1))
    bounds = librosa.segment.agglomerative(feat, nseg)
    bound_times = librosa.frames_to_time(beats[bounds], sr=sr)
    edges = list(bound_times) + [dur]

    sections = []
    for i in range(len(bound_times)):
        start, end = float(edges[i]), float(edges[i + 1])
        ch = [bc["chord"] for bc in bar_chords if bc["time"] is not None and start <= bc["time"] < end]
        sections.append({
            "index": i + 1,
            "start_sec": round(start, 2), "end_sec": round(end, 2),
            "approx_bars": max(1, round((end - start) / (60.0 / tempo * 4))),
            "chords": ch,
        })

    # --- lyrics transcription (optional; local faster-whisper) ---
    transcript = []
    if args.lyrics:
        try:
            from faster_whisper import WhisperModel
            model = WhisperModel("small", device="cpu", compute_type="int8")
            # NO speech-VAD: it classifies SINGING as non-speech and strips
            # every segment (verified on a real render — 0 vs 20 segments).
            # The isolated vocals stem (when available) transcribes cleaner
            # than the full mix.
            segs, _info = model.transcribe(stems.get("vocals", args.audio), vad_filter=False, beam_size=5)
            for s in segs:
                text = s.text.strip()
                if text:
                    transcript.append({"start": round(s.start, 2), "end": round(s.end, 2), "text": text})
        except Exception as e:  # missing dep / decode failure — never sink the analysis
            transcript = []
            print(f"(lyrics transcription unavailable: {e})", file=sys.stderr)

    # --- melody + bass note events from the stems (basic-pitch, seconds) ---
    melody_notes, bass_notes = transcribe_notes(stems) if stems else ([], [])
    if stems_tmp:
        import shutil
        shutil.rmtree(stems_tmp, ignore_errors=True)

    out = {
        "duration_sec": round(dur, 2),
        "tempo_bpm": round(tempo, 1),
        "key": key,
        "section_count": len(sections),
        "sections": sections,
        "bar_chords": bar_chords,
        "transcript": transcript,
        "melody_notes": melody_notes,
        "bass_notes": bass_notes,
        "note": "raw perception output — hand to the Reference Analyst skill for labeling + chord cleanup",
    }
    json.dump(out, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
