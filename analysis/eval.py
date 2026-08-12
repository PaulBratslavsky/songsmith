#!/usr/bin/env python3
"""Score the import pipeline against ground truth. The thing that makes any
accuracy claim about this app falsifiable.

    make analyzercheck                  # every track, analyzer arm only
    analysis/.venv/bin/python analysis/eval.py --track dopamine
    analysis/.venv/bin/python analysis/eval.py --selftest   # no corpus needed

WHY THIS EXISTS. Before it, the only accuracy claim in the repo was
`docs/RENDER-ROUNDTRIP.md`'s Stay.wav numbers, which measured self-CONSISTENCY,
not correctness — "key confidence" is the Krumhansl correlation, i.e. how sure
the estimator is, not whether it is right (reproduced: confidence rose
0.668 -> 0.939 while the answer stayed wrong). A tempo "fix" that regressed one
track shipped and was only caught because Ableton ground truth happened to
exist for it. See docs/IMPORT-ACCURACY-PLAN.md.

CORPUS LAYOUT
    analysis/bench/tracks/<slug>/audio.wav
    analysis/bench/tracks/<slug>/truth.json

truth.json — ONE home for a track's truth; .lab views are generated on demand,
never stored alongside (a .lab and a JSON disagreeing about a boundary is the
second-copy bug CLAUDE.md warns about):

    {
      "bpm": 90.0,
      "key": "F# minor",                      # mir_eval.key format
      "beats":     [0.51, 1.18, ...],         # optional
      "downbeats": [0.51, 3.18, ...],         # optional
      "chords":   [[0.0, 3.2, "F#:min"], ...] # Harte, optional
      "sections": [[0.0, 12.4, "verse"], ...] # optional
      "provenance": "ableton" | "verified-by-ear" | "guitarset",
      "notes": "how this truth was established"
    }

READING THE OUTPUT. Two numbers are load-bearing and easy to misread:

  * `chord majmin` is a DURATION-weighted mean over the comparable region only.
    Out-of-gamut chords score -1 and mir_eval drops them, so the denominator can
    shrink silently — always read `comparable` alongside it.
  * beat continuity keys are the LONG names. `scores['CMLt']` raises KeyError;
    it is 'Correct Metric Level Total'. CMLt near 0 with AMLt near 1 means an
    octave/phase error with the pulse basically right, not a lost tracker.
"""
import argparse, glob, json, os, subprocess, sys

import numpy as np

try:
    import mir_eval
except ImportError:  # pragma: no cover - the message IS the handling
    sys.exit("mir_eval is missing — it is in analysis/requirements.txt; reinstall the venv")

ROOT = os.path.dirname(os.path.abspath(__file__))
BENCH = os.path.join(ROOT, "bench", "tracks")

# ---------------------------------------------------------------- chord labels

# Pop chord symbol -> Harte quality. Anything not here degrades to its base
# triad and is COUNTED (see `degraded`): degrading keeps root+third, so `majmin`
# stays meaningful, but it would silently flatter `sevenths`, which is exactly
# why the count is reported rather than swallowed.
_QUALITY = {
    "": "maj", "maj": "maj", "M": "maj",
    "m": "min", "min": "min", "-": "min",
    "maj7": "maj7", "M7": "maj7", "Δ7": "maj7", "Δ": "maj7",
    "m7": "min7", "min7": "min7", "-7": "min7",
    "7": "7", "dom7": "7",
    "dim": "dim", "°": "dim", "o": "dim",
    "dim7": "dim7", "°7": "dim7",
    "m7b5": "hdim7", "ø": "hdim7", "ø7": "hdim7", "min7b5": "hdim7",
    "aug": "aug", "+": "aug",
    "sus2": "sus2", "sus4": "sus4", "sus": "sus4",
    "6": "maj6", "m6": "min6", "min6": "min6",
    "9": "9", "maj9": "maj9", "m9": "min9", "min9": "min9",
}
# quality -> the triad it collapses to when we can't represent it exactly
_BASE = {"maj7": "maj", "min7": "min", "7": "maj", "9": "maj", "maj9": "maj",
         "min9": "min", "maj6": "maj", "min6": "min", "dim7": "dim", "hdim7": "dim"}

_NO_CHORD = {"n", "n.c.", "nc", "none", "silence", "-", ""}

# Harte writes a slash bass as a SCALE DEGREE, not a note name: "F/A" is
# "F:maj/3" because A is the third of F. Passing the note name through makes
# mir_eval reject the label, and a silent fallback then drops the inversion
# entirely — which would quietly zero out any future `*_inv` metric.
_PC = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
_DEGREE = {0: "1", 1: "b2", 2: "2", 3: "b3", 4: "3", 5: "4",
           6: "b5", 7: "5", 8: "b6", 9: "6", 10: "b7", 11: "7"}


def _pitch_class(note):
    """'F#' -> 6, 'Bb' -> 10. None when it isn't a note name."""
    n = (note or "").strip()
    if not n or n[0].upper() not in _PC:
        return None
    pc = _PC[n[0].upper()]
    for ch in n[1:]:
        if ch == "#":
            pc += 1
        elif ch == "b":
            pc -= 1
        else:
            return None
    return pc % 12


def _bass_degree(root, bass):
    """The Harte '/<degree>' suffix for a slash chord, or '' if unresolvable."""
    r, b = _pitch_class(root), _pitch_class(bass)
    if r is None or b is None:
        return ""
    return "/" + _DEGREE[(b - r) % 12]


class Violations(list):
    """Contract breaches, reported SEPARATELY from accuracy. A dropped section
    or an unparseable chord currently vanishes from mir_eval's comparison rather
    than scoring zero, which would quietly flatter the score."""

    def note(self, kind, detail):
        self.append(f"{kind}: {detail}")


def harte(sym, viol=None, degraded=None):
    """A pop chord symbol -> a Harte label mir_eval accepts.

    'F#m' -> 'F#:min'; bare 'C' stays 'C' (already valid major); 'N.C.' -> 'N';
    'F/A' keeps its bass. Never raises — an unrecognised symbol becomes 'X'
    (mir_eval's "unknown") and is recorded, because crashing the whole
    evaluation over one odd chord is worse than scoring it as unknown.
    """
    s = (sym or "").strip()
    if s.lower() in _NO_CHORD:
        return "N"
    bass_note = ""
    if "/" in s:
        s, _, b = s.partition("/")
        bass_note = b.strip()
    # root: letter + optional accidental(s)
    i = 1 if s else 0
    while i < len(s) and s[i] in "#b":
        i += 1
    root, rest = s[:i], s[i:]
    if not root or root[0].upper() not in "ABCDEFG":
        if viol is not None:
            viol.note("unparseable-chord", sym)
        return "X"
    root = root[0].upper() + root[1:]
    q = _QUALITY.get(rest)
    if q is None:
        q = _QUALITY.get(rest.lower())
    if q is None:
        # unknown extension (add9, 7alt, 11, 13…): keep the triad we can trust
        q = "min" if rest[:1] == "m" and rest[:3] != "maj" else "maj"
        if degraded is not None:
            degraded.append(sym)
    bass = _bass_degree(root, bass_note) if bass_note else ""
    if bass_note and not bass and viol is not None:
        viol.note("unresolvable-bass", sym)
    label = root if q == "maj" and not bass else f"{root}:{q}"
    label += bass
    try:
        mir_eval.chord.encode(label)
    except Exception:
        # a degraded fallback that STILL doesn't parse (weird bass, odd root)
        try:
            label = f"{root}:{_BASE.get(q, 'maj')}"
            mir_eval.chord.encode(label)
        except Exception:
            if viol is not None:
                viol.note("unparseable-chord", sym)
            return "X"
    return label


def merge_adjacent(intervals, labels):
    """Collapse equal neighbours. Per-BAR rows repeat the same chord across a
    held progression; left unmerged they inflate mir_eval's `overseg` and make
    a correct analysis look fragmented."""
    out_i, out_l = [], []
    for (st, en), lab in zip(intervals, labels):
        if out_l and out_l[-1] == lab and abs(out_i[-1][1] - st) < 1e-6:
            out_i[-1][1] = en
        else:
            out_i.append([st, en])
            out_l.append(lab)
    return np.array(out_i, dtype=float), out_l


def bars_to_intervals(bar_chords, end, viol=None, degraded=None):
    """The analyzer's `bar_chords` [{bar,time,chord}] -> (intervals, labels).
    Each bar runs until the next bar's time; the last runs to `end`."""
    rows = [b for b in bar_chords if b.get("time") is not None]
    if not rows:
        return np.zeros((0, 2)), []
    iv, lab = [], []
    for k, b in enumerate(rows):
        st = float(b["time"])
        en = float(rows[k + 1]["time"]) if k + 1 < len(rows) else float(end)
        if en > st:
            iv.append([st, en])
            lab.append(harte(b.get("chord"), viol, degraded))
    return merge_adjacent(iv, lab)


# ------------------------------------------------------------------- scoring

def _trim(ref_iv, ref_lab, est_iv, est_lab):
    """mir_eval compares over a common span; make it explicit so `comparable`
    can be reported rather than inferred."""
    lo = max(ref_iv[0][0], est_iv[0][0])
    hi = min(ref_iv[-1][1], est_iv[-1][1])
    if hi <= lo:
        return None
    ri, rl = mir_eval.util.adjust_intervals(ref_iv, ref_lab, lo, hi, mir_eval.chord.NO_CHORD)
    ei, el = mir_eval.util.adjust_intervals(est_iv, est_lab, lo, hi, mir_eval.chord.NO_CHORD)
    return ri, rl, ei, el, hi - lo


def score_track(truth, raw, viol):
    """One track, analyzer arm. Returns a flat dict of metrics (missing truth
    for a dimension simply omits it — a partial truth is still useful)."""
    m, degraded = {}, []

    # --- tempo: octave-corrected, because a clean x2 is trivially fixable and a
    # 2% error is not. Reported as a percentage.
    if truth.get("bpm") and raw.get("tempo_bpm"):
        t, e = float(truth["bpm"]), float(raw["tempo_bpm"])
        m["tempo_err_pct"] = 100 * min(abs(e * k - t) / t for k in (0.5, 1.0, 2.0))
        m["tempo_octave"] = min((abs(e * k - t) / t, k) for k in (0.5, 1.0, 2.0))[1]

    # --- key: weighted, so a relative-key miss scores 0.3 rather than 0 and an
    # improvement is visible instead of binary
    if truth.get("key") and raw.get("key"):
        est = f"{raw['key'].get('root','?')} {raw['key'].get('mode','?')}"
        try:
            m["key_weighted"] = mir_eval.key.evaluate(truth["key"], est)["Weighted Score"]
        except Exception as e:
            viol.note("key-format", f"{truth['key']!r} vs {est!r}: {e}")

    # --- chords
    if truth.get("chords") and raw.get("bar_chords"):
        ri = np.array([[c[0], c[1]] for c in truth["chords"]], dtype=float)
        rl = [c[2] for c in truth["chords"]]
        end = float(raw.get("duration_sec") or ri[-1][1])
        ei, el = bars_to_intervals(raw["bar_chords"], end, viol, degraded)
        if len(ei):
            t = _trim(ri, rl, ei, el)
            if t:
                ri2, rl2, ei2, el2, span = t
                dur = mir_eval.util.intervals_to_durations(ri2)
                for name in ("root", "majmin", "thirds", "triads", "sevenths", "mirex"):
                    cmp = getattr(mir_eval.chord, name)(rl2, el2)
                    ok = np.asarray(cmp) >= 0
                    m[f"chord_{name}"] = float(np.sum(dur[ok] * np.asarray(cmp)[ok]) / np.sum(dur[ok])) if ok.any() else 0.0
                    if name == "majmin":
                        # the shrinking-denominator guard: what fraction of the
                        # span was actually comparable at all
                        m["chord_comparable"] = float(np.sum(dur[ok]) / np.sum(dur))
                m["chord_seg"] = mir_eval.chord.seg(ri2, ei2)
                m["chord_transitions"] = len(el2)
                m["chord_ref_transitions"] = len(rl2)
    if degraded:
        m["chord_degraded"] = len(degraded)
        viol.note("degraded-quality", f"{len(degraded)} chord(s) collapsed to a triad, e.g. {degraded[:4]}")

    # --- beats / downbeats: F-measure plus continuity. AMLt high with CMLt low
    # is an octave/phase error, NOT a lost tracker — read them together.
    for field, est_key, tag in (("beats", None, "beat"), ("downbeats", "first_downbeat_sec", "downbeat")):
        ref = truth.get(field)
        if not ref:
            continue
        if tag == "beat":
            continue  # the analyzer does not emit beat times today
        # downbeats: the analyzer emits only the FIRST one, so score phase —
        # is the measured downbeat within 70ms of a true downbeat?
        if raw.get(est_key) is not None:
            d = min(abs(float(raw[est_key]) - float(b)) for b in ref)
            m["downbeat_phase_err_ms"] = 1000 * d
            m["downbeat_phase_ok"] = float(d <= 0.07)

    # --- sections
    if truth.get("sections") and raw.get("sections"):
        ri = np.array([[s[0], s[1]] for s in truth["sections"]], dtype=float)
        rl = [s[2] for s in truth["sections"]]
        est = [s for s in raw["sections"] if s.get("start_sec") is not None]
        if est:
            ei = np.array([[float(s["start_sec"]), float(s["end_sec"])] for s in est], dtype=float)
            el = [str(s.get("index", i)) for i, s in enumerate(est)]
            lo, hi = max(ri[0][0], ei[0][0]), min(ri[-1][1], ei[-1][1])
            if hi > lo:
                ri2, rl2 = mir_eval.util.adjust_intervals(ri, rl, lo, hi, "N")
                ei2, el2 = mir_eval.util.adjust_intervals(ei, el, lo, hi, "N")
                sc = mir_eval.segment.evaluate(ri2, rl2, ei2, el2)
                # gate on @3s, not @0.5s — a producer does not care about 400ms
                # on a section line, and gating tight fails on cosmetic drift
                m["seg_f3"] = sc["F-measure@3.0"]
                m["seg_f05"] = sc["F-measure@0.5"]
                m["seg_count"] = len(est)
                m["seg_ref_count"] = len(rl)
    return m


# ------------------------------------------------------------------ the corpus

def load_tracks(only=None):
    out = []
    for d in sorted(glob.glob(os.path.join(BENCH, "*"))):
        slug = os.path.basename(d)
        if only and only not in slug:
            continue
        tp = os.path.join(d, "truth.json")
        audio = next((p for p in glob.glob(os.path.join(d, "audio.*"))), None)
        if not os.path.exists(tp):
            continue
        out.append((slug, audio, json.load(open(tp))))
    return out


def run_analyzer(audio, stems=False):
    py = os.path.join(ROOT, ".venv", "bin", "python")
    cmd = [py, os.path.join(ROOT, "analyze.py"), audio] + (["--stems"] if stems else [])
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip()[-400:])
    return json.loads(r.stdout)


AGG = ["tempo_err_pct", "key_weighted", "chord_majmin", "chord_comparable",
       "chord_root", "chord_sevenths", "chord_seg", "downbeat_phase_ok", "seg_f3"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--track", help="substring of a track slug")
    ap.add_argument("--stems", action="store_true", help="run the deep (demucs) analyzer path")
    ap.add_argument("--baseline", help="baseline.json to diff against")
    ap.add_argument("--write-baseline", help="write results here")
    ap.add_argument("--selftest", action="store_true", help="verify the adapters; needs no corpus")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    tracks = load_tracks(args.track)
    if not tracks:
        print(f"No tracks in {BENCH}.\n"
              "Each track is a directory with audio.wav + truth.json — see this file's docstring.\n"
              "Run --selftest to verify the adapters without a corpus.")
        return 1

    results, all_viol = {}, {}
    for slug, audio, truth in tracks:
        viol = Violations()
        if not audio:
            print(f"{slug}: truth.json but no audio.* — skipped")
            continue
        try:
            raw = run_analyzer(audio, args.stems)
        except Exception as e:
            print(f"{slug}: ANALYZER FAILED — {e}")
            continue
        results[slug] = score_track(truth, raw, viol)
        if viol:
            all_viol[slug] = list(viol)

    cols = [c for c in AGG if any(c in r for r in results.values())]
    w = max([len(s) for s in results] + [10])
    print(f"{'track':{w}} " + " ".join(f"{c.replace('chord_','').replace('_',' ')[:11]:>12}" for c in cols))
    for slug, r in results.items():
        print(f"{slug:{w}} " + " ".join(f"{r[c]:12.3f}" if c in r else f"{'-':>12}" for c in cols))
    if results:
        print(f"{'MEAN':{w}} " + " ".join(
            f"{np.mean([r[c] for r in results.values() if c in r]):12.3f}"
            if any(c in r for r in results.values()) else f"{'-':>12}" for c in cols))

    if all_viol:
        print("\nCONTRACT VIOLATIONS (reported separately from accuracy):")
        for slug, vs in all_viol.items():
            for v in vs:
                print(f"  {slug}: {v}")

    if args.baseline and os.path.exists(args.baseline):
        base = json.load(open(args.baseline))
        print("\nvs baseline (per-track delta; a mean alone is not evidence at this n):")
        wins = losses = 0
        for slug, r in results.items():
            b = base.get(slug, {})
            for c in cols:
                if c in r and c in b:
                    d = r[c] - b[c]
                    if abs(d) < 1e-9:
                        continue
                    better = d < 0 if c.endswith("_pct") or c.endswith("_ms") else d > 0
                    wins, losses = wins + better, losses + (not better)
                    print(f"  {'✓' if better else '✗'} {slug} {c}: {b[c]:.3f} -> {r[c]:.3f} ({d:+.3f})")
        print(f"  {wins} better / {losses} worse")

    if args.write_baseline:
        json.dump(results, open(args.write_baseline, "w"), indent=2, sort_keys=True)
        print(f"\nbaseline written to {args.write_baseline}")
    return 0


def selftest():
    """Verify the adapters on synthetic data. Exists because the corpus can
    vanish — the render set these were calibrated on was deleted — and a
    measurement tool nobody can run is worse than none."""
    fails = []

    def check(name, got, want):
        if got != want:
            fails.append(f"{name}: got {got!r}, want {want!r}")

    check("harte bare", harte("C"), "C")
    check("harte minor", harte("F#m"), "F#:min")
    check("harte maj7", harte("Cmaj7"), "C:maj7")
    check("harte m7", harte("Am7"), "A:min7")
    check("harte dom7", harte("G7"), "G:7")
    check("harte slash -> degree", harte("F/A"), "F:maj/3")
    check("harte slash minor", harte("Am/C"), "A:min/b3")
    check("harte nc", harte("N.C."), "N")
    check("harte flat root", harte("Bbm"), "Bb:min")
    deg = []
    check("harte add9 degrades", harte("Cadd9", None, deg), "C")
    check("harte add9 counted", len(deg), 1)
    check("harte junk", harte("???"), "X")
    # every emitted label must be one mir_eval will actually accept
    for sym in ["C", "F#m", "Cmaj7", "Am7", "G7", "F/A", "N.C.", "Bbm", "Cadd9", "Cm7b5", "Gsus4", "Ddim"]:
        lab = harte(sym)
        try:
            mir_eval.chord.encode(lab)
        except Exception as e:
            fails.append(f"harte({sym!r}) -> {lab!r} which mir_eval rejects: {e}")

    iv, lab = bars_to_intervals(
        [{"bar": 1, "time": 0.0, "chord": "Am"}, {"bar": 2, "time": 2.0, "chord": "Am"},
         {"bar": 3, "time": 4.0, "chord": "F"}], end=6.0)
    check("bars merge held chord", lab, ["A:min", "F"])
    check("bars merge intervals", iv.tolist(), [[0.0, 4.0], [4.0, 6.0]])

    # a perfect analysis must score 1.0 — if this drifts, the scoring is wrong,
    # not the analyzer
    truth = {"bpm": 120.0, "key": "A minor",
             "chords": [[0.0, 2.0, "A:min"], [2.0, 4.0, "F"]]}
    raw = {"tempo_bpm": 120.0, "duration_sec": 4.0,
           "key": {"root": "A", "mode": "minor"},
           "bar_chords": [{"bar": 1, "time": 0.0, "chord": "Am"}, {"bar": 2, "time": 2.0, "chord": "F"}]}
    v = Violations()
    m = score_track(truth, raw, v)
    check("perfect tempo", round(m["tempo_err_pct"], 6), 0.0)
    check("perfect key", m["key_weighted"], 1.0)
    check("perfect majmin", round(m["chord_majmin"], 6), 1.0)
    check("perfect comparable", round(m["chord_comparable"], 6), 1.0)
    # a relative-key miss must score 0.3, not 0 — partial credit is the point
    m2 = score_track({"key": "A minor"}, {"key": {"root": "C", "mode": "major"}}, Violations())
    check("relative key partial credit", m2["key_weighted"], 0.3)
    # half-time must be caught as an octave error, not a 50% miss
    m3 = score_track({"bpm": 120.0}, {"tempo_bpm": 60.0}, Violations())
    check("octave-corrected tempo", round(m3["tempo_err_pct"], 6), 0.0)
    check("octave factor recorded", m3["tempo_octave"], 2.0)

    for f in fails:
        print("FAIL " + f)
    print(f"\nselftest: {'FAILED' if fails else 'OK'} ({len(fails)} failure(s))")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
