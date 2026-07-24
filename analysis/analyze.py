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
    args = ap.parse_args()

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
            segs, _info = model.transcribe(args.audio, vad_filter=True, beam_size=5)
            for s in segs:
                text = s.text.strip()
                if text:
                    transcript.append({"start": round(s.start, 2), "end": round(s.end, 2), "text": text})
        except Exception as e:  # missing dep / decode failure — never sink the analysis
            transcript = []
            print(f"(lyrics transcription unavailable: {e})", file=sys.stderr)

    out = {
        "duration_sec": round(dur, 2),
        "tempo_bpm": round(tempo, 1),
        "key": key,
        "section_count": len(sections),
        "sections": sections,
        "bar_chords": bar_chords,
        "transcript": transcript,
        "note": "raw perception output — hand to the Reference Analyst skill for labeling + chord cleanup",
    }
    json.dump(out, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
