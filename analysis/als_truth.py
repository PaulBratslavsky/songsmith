#!/usr/bin/env python3
"""Read ground truth out of Ableton `.als` project files.

    als_truth.py --survey ~/Desktop            # rank benchmark candidates
    als_truth.py --truth <project.als> --slug dopamine   # write bench truth.json

WHY. Ground truth is the expensive ingredient in `eval.py`, and the usual
sources are closed to us: Isophonics/Billboard/SALAMI/Harmonix ship annotations
WITHOUT audio, JAAH and CASD are NonCommercial, RWC needs a signed agreement.
An `.als` gives EXACT tempo, meter, section boundaries and the actual MIDI notes
of the harmony — real chord truth rather than a transcriber's best guess — for
audio you own outright. See docs/IMPORT-ACCURACY-PLAN.md (T4.1).

WHAT IT CANNOT TELL YOU. This tier validates code paths and gives exact chord
truth; it does NOT predict accuracy on the app's real input, which is dense
AI-generated renders, not projects built from loops and hand-played MIDI. Read a
good score here as "the pipeline is not broken", never as "the pipeline is
accurate". That claim needs the by-ear render tier.

FORMAT NOTES (verified against Live 12.4.3 files, 2026-08-12)
  * `.als` is gzipped XML.
  * Arrangement times are in BEATS: `MidiClip/CurrentStart|CurrentEnd`, and
    `MidiNoteEvent@Time` relative to its clip. Seconds = beats * 60 / bpm.
  * Notes live under `KeyTrack`, one per pitch: `MidiKey@Value` is the MIDI
    number, and each `MidiNoteEvent` carries `Time`/`Duration`.
  * Recorded/nudged notes carry float jitter (0.003 beats is common), so
    simultaneity needs a tolerance — exact equality finds almost nothing.
"""
import argparse, glob, gzip, json, os, sys
import xml.etree.ElementTree as ET
from collections import Counter

PITCH = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]

# interval-set -> Harte quality, richest first so a 7th is not read as a triad
TEMPLATES = [
    ((0, 4, 7, 11), "maj7"), ((0, 3, 7, 10), "min7"), ((0, 4, 7, 10), "7"),
    ((0, 3, 6, 10), "hdim7"), ((0, 3, 6, 9), "dim7"), ((0, 4, 7, 9), "maj6"),
    ((0, 3, 7, 9), "min6"), ((0, 4, 8), "aug"), ((0, 3, 6), "dim"),
    ((0, 4, 7), "maj"), ((0, 3, 7), "min"), ((0, 2, 7), "sus2"), ((0, 5, 7), "sus4"),
    ((0, 7), "5"), ((0, 4), "maj"), ((0, 3), "min"),
]
DEGREE = {0: "1", 1: "b2", 2: "2", 3: "b3", 4: "3", 5: "4",
          6: "b5", 7: "5", 8: "b6", 9: "6", 10: "b7", 11: "7"}


def _v(el, path, default=None):
    """Ableton stores scalars as <Tag Value="x"/>."""
    e = el.find(path)
    if e is None:
        return default
    return e.get("Value", default)


def name_chord(pitches):
    """A set of MIDI pitches -> a Harte label. Returns None when no template
    fits, which is information: it means that clip is not a clean harmony part
    and the project is a poor chord-truth candidate."""
    if not pitches:
        return None
    bass = min(pitches)
    pcs = sorted({p % 12 for p in pitches})
    best = None
    for root in pcs:
        iv = tuple(sorted((p - root) % 12 for p in pcs))
        for tpl, qual in TEMPLATES:
            if iv == tpl:
                # prefer the reading whose root is the actual bass note
                score = (root == bass % 12, len(tpl))
                if best is None or score > best[0]:
                    best = (score, root, qual)
    if best is None:
        return None
    _, root, qual = best
    label = PITCH[root] if qual == "maj" else f"{PITCH[root]}:{qual}"
    if bass % 12 != root:
        label = (f"{PITCH[root]}:{qual}" if qual == "maj" else label) + "/" + DEGREE[(bass - root) % 12]
    return label


def read_als(path):
    """Parse one project. Never raises on a malformed file — returns
    {"error": ...} so a survey over hundreds of projects can't be sunk by one."""
    try:
        root = ET.fromstring(gzip.open(path, "rt", errors="replace").read())
    except Exception as e:
        return {"error": f"{type(e).__name__}: {e}"}

    # Live 12 renamed MasterTrack -> MainTrack; older projects still use the old
    # name, so try both or every pre-12 file silently reports no tempo.
    main = root.find(".//MainTrack")
    if main is None:
        main = root.find(".//MasterTrack")
    bpm = None
    if main is not None:
        e = main.find(".//Tempo/Manual")
        if e is None:
            e = main.find(".//Tempo")
        try:
            bpm = float(e.get("Value")) if e is not None and e.get("Value") else None
        except (TypeError, ValueError):
            bpm = None
    # tempo automation makes a single BPM a lie — flag rather than silently use it
    tempo_automated = len(main.findall(".//Tempo//FloatEvent")) > 1 if main is not None else False

    num = _v(root, ".//TimeSignature//Numerator", "4") or "4"
    den = _v(root, ".//TimeSignature//Denominator", "4") or "4"

    tracks = []
    for t in root.findall(".//Tracks/MidiTrack"):
        name = _v(t, "./Name/EffectiveName") or _v(t, "./Name/UserName") or ""
        clips = []
        for c in t.findall(".//MidiClip"):
            try:
                start = float(c.find("./CurrentStart").get("Value"))
                end = float(c.find("./CurrentEnd").get("Value"))
            except Exception:
                continue
            notes = []
            for kt in c.findall(".//KeyTrack"):
                mk = kt.find("./MidiKey")
                if mk is None:
                    continue
                pitch = int(mk.get("Value"))
                for n in kt.findall(".//MidiNoteEvent"):
                    try:
                        notes.append((float(n.get("Time")), float(n.get("Duration")), pitch))
                    except (TypeError, ValueError):
                        continue
            clips.append({"name": _v(c, "./Name") or "", "start": start, "end": end, "notes": notes})
        tracks.append({"name": name, "clips": clips})

    return {"path": path, "bpm": bpm, "tempo_automated": tempo_automated,
            "meter": f"{num}/{den}", "tracks": tracks,
            "creator": root.get("Creator", "")}


def _b2s(beats, bpm):
    return beats * 60.0 / bpm


def sections_from(proj):
    """Section truth from clip NAMES on a track called Sections/Structure/Form —
    which is exactly what this app's own Ableton export writes."""
    for t in proj["tracks"]:
        if t["name"].strip().lower() in ("sections", "section", "structure", "form", "arrangement"):
            out = [(c["start"], c["end"], c["name"] or "section") for c in t["clips"] if c["name"]]
            return sorted(out)
    return []


def chords_from(proj, track_hint=None, tol=0.05):
    """Chord truth from the harmony track's MIDI: group notes that start
    together (within `tol` beats — Ableton leaves float jitter on recorded
    notes) and name the resulting pitch set."""
    cands = [t for t in proj["tracks"] if t["clips"] and any(c["notes"] for c in t["clips"])]
    if track_hint:
        cands = [t for t in cands if track_hint.lower() in t["name"].lower()] or cands
    else:
        named = [t for t in cands if t["name"].strip().lower() in ("chords", "chord", "harmony", "pad", "keys", "piano")]
        cands = named or cands
    if not cands:
        return [], None
    # the most polyphonic track is the harmony one
    def poly(t):
        return sum(len(c["notes"]) for c in t["clips"])
    track = max(cands, key=poly)

    events = []
    for c in track["clips"]:
        for (t0, dur, pitch) in c["notes"]:
            events.append((round(c["start"] + t0, 3), dur, pitch))
    if not events:
        return [], track["name"]
    events.sort()
    groups, cur, cur_t = [], [], None
    for t0, dur, pitch in events:
        if cur_t is None or abs(t0 - cur_t) <= tol:
            cur_t = t0 if cur_t is None else cur_t
            cur.append((pitch, dur))
        else:
            groups.append((cur_t, cur))
            cur, cur_t = [(pitch, dur)], t0
    if cur:
        groups.append((cur_t, cur))

    out = []
    for k, (t0, notes) in enumerate(groups):
        lab = name_chord([p for p, _ in notes])
        if not lab:
            continue
        end = groups[k + 1][0] if k + 1 < len(groups) else t0 + max(d for _, d in notes)
        if end > t0:
            out.append((t0, end, lab))
    return out, track["name"]


def survey(paths):
    """Rank projects as benchmark candidates so the export chore is 6 projects,
    not 400. Scored on what actually makes a track useful as truth."""
    rows = []
    for p in paths:
        pr = read_als(p)
        if "error" in pr:
            rows.append((os.path.basename(p), None, 0, 0, 0, False, pr["error"][:30]))
            continue
        secs = sections_from(pr)
        chords, ctrack = chords_from(pr)
        distinct = len({c[2] for c in chords})
        # an untouched 120.0 with almost no MIDI is Ableton's default on a
        # project someone opened and abandoned — not evidence of tempo
        default_ish = pr["bpm"] == 120.0 and sum(len(c["notes"]) for t in pr["tracks"] for c in t["clips"]) < 40
        rows.append((os.path.basename(p)[:-4], pr["bpm"], len(secs), len(chords), distinct,
                     pr["tempo_automated"] or default_ish, ctrack or ""))
    rows.sort(key=lambda r: (r[2] > 0, r[4], r[3]), reverse=True)
    print(f"{'project':38} {'bpm':>7} {'secs':>5} {'chords':>7} {'distinct':>9}  {'harmony track':<16} flag")
    for name, bpm, ns, nc, nd, flag, ct in rows:
        b = f"{bpm:7.2f}" if bpm else "      -"
        print(f"{name[:38]:38} {b} {ns:5d} {nc:7d} {nd:9d}  {str(ct)[:16]:<16} {'⚠ suspect tempo' if flag else ''}")
    good = [r for r in rows if r[1] and not r[5] and r[3] >= 4 and r[4] >= 2]
    print(f"\n{len(good)} of {len(rows)} look usable as chord truth "
          f"(real tempo, >=4 chord events, >=2 distinct chords).")
    print("Export audio for those, then: als_truth.py --truth <file.als> --slug <name>")
    return 0


def write_truth(path, slug, key=None, track_hint=None):
    pr = read_als(path)
    if "error" in pr:
        sys.exit(f"could not read {path}: {pr['error']}")
    if not pr["bpm"]:
        sys.exit("no tempo found — cannot convert beats to seconds")
    bpm = pr["bpm"]
    secs = [(_b2s(a, bpm), _b2s(b, bpm), n) for a, b, n in sections_from(pr)]
    chords, ctrack = chords_from(pr, track_hint)
    truth = {
        "bpm": bpm,
        "meter": pr["meter"],
        "provenance": "ableton",
        "notes": f"extracted from {os.path.basename(path)} by als_truth.py"
                 + (" — TEMPO IS AUTOMATED, single-bpm truth is approximate" if pr["tempo_automated"] else "")
                 + (f"; chords from the '{ctrack}' track" if ctrack else ""),
    }
    if key:
        truth["key"] = key
    if secs:
        truth["sections"] = [[round(a, 3), round(b, 3), n] for a, b, n in secs]
    if chords:
        truth["chords"] = [[round(_b2s(a, bpm), 3), round(_b2s(b, bpm), 3), n] for a, b, n in chords]

    d = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bench", "tracks", slug)
    os.makedirs(d, exist_ok=True)
    out = os.path.join(d, "truth.json")
    json.dump(truth, open(out, "w"), indent=2)
    print(f"wrote {out}")
    print(f"  bpm {bpm}  meter {pr['meter']}  sections {len(secs)}  chords {len(chords)}"
          f"  distinct {len({c[2] for c in chords})}")
    if chords:
        print("  first chords:", ", ".join(c[2] for c in chords[:8]))
    if pr["tempo_automated"]:
        print("  ⚠ tempo automation present — the single bpm above is approximate")
    print(f"\nNow put the rendered audio at {os.path.join(d, 'audio.wav')} and run: make analyzercheck")
    return 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--survey", metavar="DIR", help="scan a directory tree for .als candidates")
    ap.add_argument("--truth", metavar="ALS", help="write bench truth.json from one project")
    ap.add_argument("--slug", help="bench track slug (with --truth)")
    ap.add_argument("--key", help='e.g. "F# minor" — .als does not record key, so pass it')
    ap.add_argument("--track", help="harmony track name hint (default: the most polyphonic)")
    ap.add_argument("--limit", type=int, default=400)
    a = ap.parse_args()
    if a.survey:
        paths = sorted(glob.glob(os.path.join(os.path.expanduser(a.survey), "**", "*.als"), recursive=True))
        paths = [p for p in paths if "Backup" not in p][: a.limit]
        if not paths:
            sys.exit(f"no .als files under {a.survey}")
        return survey(paths)
    if a.truth:
        if not a.slug:
            sys.exit("--truth needs --slug")
        return write_truth(os.path.expanduser(a.truth), a.slug, a.key, a.track)
    ap.print_help()
    return 1


if __name__ == "__main__":
    sys.exit(main())
