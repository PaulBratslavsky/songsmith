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
        # drum-free mix (vocals+bass+other): drums smear chroma — chord/key
        # detection runs on THIS when available (analyzer v2, 2026-07-28)
        p = str(pathlib.Path(tmp) / "nodrums.wav")
        save_audio(separated["vocals"] + separated["bass"] + separated["other"], p, samplerate=sep.samplerate)
        out["nodrums"] = p
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


def energy_gate(events, stem_path):
    """Drop note events whose window is essentially SILENT in the stem.
    basic-pitch reports model confidence, not loudness — a near-silent stem
    (demucs misfiling, reverb tails) still yields hundreds of ghost notes
    (verified: 823 'melody notes' from a near-silent vocals stem)."""
    if not events:
        return events
    y, sr = librosa.load(stem_path, mono=True)
    rms = librosa.feature.rms(y=y)[0]
    hop = 512
    thr = max(0.004, 0.06 * float(np.percentile(rms, 95)))
    out = []
    for e in events:
        a = int(e["start"] * sr / hop)
        b = min(len(rms), max(a + 1, int(e["end"] * sr / hop)))
        if a < len(rms) and float(np.mean(rms[a:b])) >= thr:
            out.append(e)
    if len(out) < len(events):
        print(f"(energy gate dropped {len(events) - len(out)}/{len(events)} silent-window notes)", file=sys.stderr)
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
            return mono_clean(energy_gate(evs, path))[:1500]

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

    # --- chroma (beat-synced) — from the DRUM-FREE mix when stems exist:
    # kicks/hats smear the harmonic profile (analyzer v2) ---
    y_harm = y
    if "nodrums" in stems:
        y_harm, _ = librosa.load(stems["nodrums"], sr=sr, mono=True)
        y_harm = y_harm[:len(y)] if len(y_harm) > len(y) else y_harm
    chroma = librosa.feature.chroma_cqt(y=y_harm, sr=sr)
    key = estimate_key(chroma.mean(axis=1))
    beat_chroma = librosa.util.sync(chroma, beats, aggregate=np.median)

    # --- beat-level chord scores: triad templates + a BASS-ROOT bonus (the
    # bass stem knows the root better than mid-register chroma — Cm/Eb/Ab
    # confusions are root confusions), then a stay-put smoothing pass so one
    # noisy beat can't flip a chord (flicker) while real changes still win ---
    tmpl, labels = triad_templates()
    nbeats = beat_chroma.shape[1]
    normed = beat_chroma / (np.linalg.norm(beat_chroma, axis=0, keepdims=True) + 1e-9)
    scores = tmpl @ normed  # 24 × nbeats
    if "bass" in stems and nbeats > 0:
        yb, _ = librosa.load(stems["bass"], sr=sr, mono=True)
        bsync = librosa.util.sync(librosa.feature.chroma_cqt(y=yb, sr=sr), beats, aggregate=np.median)
        for b in range(min(nbeats, bsync.shape[1])):
            col = bsync[:, b]
            if float(col.max()) > 1e-4:  # silent bass beat → no opinion
                pc = int(np.argmax(col))
                scores[2 * pc, b] += 0.12      # major triad rooted on the bass pc
                scores[2 * pc + 1, b] += 0.12  # minor triad rooted on the bass pc
    STAY = 0.10
    if nbeats > 0:
        dp = scores[:, 0].copy()
        back = np.zeros((24, nbeats), dtype=int)
        for b in range(1, nbeats):
            prev_best = int(np.argmax(dp))
            dp_new = np.empty(24)
            for c in range(24):
                stay_score = dp[c] + STAY
                if stay_score >= dp[prev_best]:
                    dp_new[c] = stay_score + scores[c, b]
                    back[c, b] = c
                else:
                    dp_new[c] = dp[prev_best] + scores[c, b]
                    back[c, b] = prev_best
            dp = dp_new
        path = np.zeros(nbeats, dtype=int)
        path[-1] = int(np.argmax(dp))
        for b in range(nbeats - 1, 0, -1):
            path[b - 1] = back[path[b], b]
        beat_labels = [labels[int(i)] for i in path]
    else:
        beat_labels = []

    # --- bar-level chords (assume 4/4): majority vote of the smoothed beats ---
    from collections import Counter
    bar_chords = []
    for b0 in range(0, nbeats, 4):
        seg = beat_labels[b0:b0 + 4]
        if not seg:
            break
        t = float(beat_times[b0]) if b0 < len(beat_times) else None
        bar_chords.append({"bar": len(bar_chords) + 1, "time": round(t, 2) if t else None,
                           "chord": Counter(seg).most_common(1)[0][0]})

    # --- first DOWNBEAT: the 4-beat phase whose beats carry the most onset
    # energy — makes the Composer's auto-nudge tight instead of approximate ---
    first_downbeat = 0.0
    if len(beats) >= 8:
        onset_env = librosa.onset.onset_strength(y=y, sr=sr)
        phases = [float(np.mean(onset_env[np.asarray(beats[p::4])])) for p in range(4)]
        first_downbeat = float(beat_times[int(np.argmax(phases))])

    # --- structural segmentation (agglomerative on beat-synced features) ---
    mfcc = librosa.feature.mfcc(y=y, sr=sr, n_mfcc=13)
    beat_mfcc = librosa.util.sync(mfcc, beats, aggregate=np.mean)
    feat = np.vstack([librosa.util.normalize(beat_chroma, axis=0),
                      librosa.util.normalize(beat_mfcc, axis=0)])
    nseg = max(2, min(args.sections, feat.shape[1] - 1))
    bounds = librosa.segment.agglomerative(feat, nseg)
    # librosa.util.sync yields len(beats)+1 columns, so a boundary can land ON
    # index len(beats) — indexing `beats` with it raises IndexError and the
    # analyzer dies with no JSON at all (audit 2026-07-28). Clip to the last
    # real beat.
    bounds = np.clip(np.asarray(bounds), 0, len(beats) - 1)
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

            def transcribe(path):
                # NO speech-VAD: it classifies SINGING as non-speech and strips
                # every segment (verified on a real render — 0 vs 20 segments)
                segs, _info = model.transcribe(path, vad_filter=False, beam_size=5)
                out = []
                for s in segs:
                    text = s.text.strip()
                    # whisper's stock instrumental hallucinations — not lyrics
                    if text and text.lower().strip(" .!♪") not in ("music", "thanks for watching", "thank you for watching"):
                        out.append({"start": round(s.start, 2), "end": round(s.end, 2), "text": text})
                # hallucination signature: one or two short phrases looping
                # ("Music"×50, "Thank you very much."×12) — worse than nothing,
                # they'd land in the Lyrics stage as real words
                uniq = {e["text"].lower() for e in out}
                if len(out) >= 5 and len(uniq) * 3 < len(out):
                    print(f"(dropping hallucinated transcript: {len(out)} segments, {len(uniq)} unique)", file=sys.stderr)
                    return []
                if len(out) < 2:
                    return []  # a single stray segment is noise, not lyrics
                return out

            # Fallback ladder: the isolated vocals stem usually transcribes
            # cleanest — but demucs can misfile heavily-processed synth
            # vocals into "other", leaving a near-silent vocals stem
            # (verified on a real synthwave render: stem 0 segments, mix 6).
            # When the stem yields almost nothing, retry on the full mix and
            # keep whichever heard more.
            candidates = ([("vocals stem", stems["vocals"])] if "vocals" in stems else []) + [("full mix", args.audio)]
            for tag, path in candidates:
                got = transcribe(path)
                print(f"lyrics via {tag}: {len(got)} segments", file=sys.stderr)
                if len(got) > len(transcript):
                    transcript = got
                if len(transcript) >= 3:
                    break  # good enough — skip the extra pass
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
        "first_downbeat_sec": round(first_downbeat, 3),
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
