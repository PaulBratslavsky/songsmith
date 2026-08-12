# Import accuracy: key, chords, structure

STATUS: PLAN (2026-08-01), with **T1.0, T1.4, T1.5 and T0.1 shipped the same day** —
see the DONE markers below. Written after a six-dive research pass (each dive
adversarially fact-checked by an independent skeptic) plus a first-party
measurement pass over 11 real renders on this machine.

The question: `analysis/analyze.py` + the Reference Analyst skill turn an audio
file into key/BPM/sections/chords. How do we make that meaningfully better, and
how would we know we had?

**Read §1 first.** It is the only accuracy evidence that exists on this app's own
material, and it changes the ranking of everything after it.

---

## 1. Measured baseline (2026-08-01, this machine, real renders)

Eleven `.wav` renders from `~/Desktop`, run through the CURRENT analyzer. This is
the first time the import has been measured on anything but one file.

### 1.1 The reported BPM is quantized, and the error compounds into bar drift

Every tempo `analyze.py` has ever emitted lands **exactly** on the grid
`60·(sr/hop)/k` for integer `k` — because `librosa.beat.beat_track` picks a
tempogram lag, and lags are integers. With librosa's defaults (`sr=22050`,
`hop=512` → 43.0664 frames/s):

| lag | BPM | gap to next |
|---|---|---|
| 18 | 143.55 | **7.56** |
| 21 | 123.05 | 5.59 |
| 24 | 107.67 | 4.31 |
| 29 | 89.10 | 2.97 |
| 30 | 86.13 | 2.78 |

At ~144 BPM the reportable values are 7.5 BPM apart. The true tempo is
unrecoverable from that number alone.

It **is** recoverable from the beat times. Least-squares regressing bar index
against bar time (`bar_chords[].time`, already computed) gives:

| track | reported | regressed | resid σ | drift over track |
|---|---|---|---|---|
| new-song-idea-343 (4 mixes) | 89.1 | **90.00** | 7–20 ms | 1.5–2.2 s |
| song-idea-266 (2 versions) | 107.7 | **108.0** | 7 ms | 0.1 s |
| weird-song-25 | 86.1 | **86.00** | 9 ms | 0.2 s |
| bells-and-drums-2 | 123.0 | **121.02** | 9 ms | 0.8 s |
| witch-house-idea-2 | 107.7 | **108.03** | 50 ms | 0.2 s |
| dopamine-v1-final | 143.6 | **140.40** | **104 ms** | **4.61 s** |

Five tracks regress to exactly 90.00 and two to 108.0 — the analyzer was
reporting 89.1 and 107.7 for songs that are plainly at 90 and 108.

Bar 121 of `dopamine-v1-final` is placed **4.6 seconds late** — more than three
bars. Every section `start_sec`, the Composer's audio nudge
(`FinalRenders.tsx:33-40`), the note-event rebase (`transcription.ts:60`) and the
Ableton export inherit it.

The residual σ is a free by-product and a **beat-quality confidence signal the app
does not currently have**: 7–9 ms means the grid is solid; `dopamine`'s 104 ms and
`witch-house`'s 50 ms flag the tracks where beat tracking is genuinely struggling.

### 1.2 Chord stability is material-dependent, and the "better" path is not better

Both preprocessing paths, same file:

| track | agreement (fast vs `--stems`) | key conf fast → stems | distinct chords |
|---|---|---|---|
| song-idea-266 (35 s sparse loop) | **12/12 = 100%** | 0.738 → 0.699 | 3 → 3 |
| dopamine-v1-final (212 s full production) | **58/121 = 48%** | 0.528 → **0.48** | 13 → 12 |

On the sparse loop the analyzer nails `Dm C Am C` and the two paths are identical.
On the dense production **half the bar labels flip** depending on a preprocessing
choice, and the demucs stems path — the one `import_reference_full` actually uses —
scores *worse* on both self-consistency measures. `docs/RENDER-ROUNDTRIP.md:73`'s
claim that stems improve accuracy was measured on one track (Stay.wav); on this
one it reverses.

### 1.3 The full sweep

`in-key%` = share of bars whose triad is diatonic to the key **the analyzer itself
chose** (natural minor/major + V, bVII, iv, IV-in-minor). It is a *self-consistency*
measure, not accuracy — a track with a wrong key can still score 100%.

| track | dur | reported BPM | key | conf | bars | distinct | in-key% |
|---|---|---|---|---|---|---|---|
| bells-and-drums-2 | 63 | 123.0 | D minor | 0.909 | 26 | 6 | 100.0 |
| dopamine-v1-final | 212 | 143.6 | F# minor | **0.528** | 121 | **13** | **85.1** |
| new-song-idea-343-bass | 160 | 89.1 | C# minor | **0.554** | 60 | 5 | **71.7** |
| new-song-idea-343-lead | 171 | 89.1 | C# minor | 0.904 | 62 | 8 | 98.4 |
| new-song-idea-343 | 160 | 89.1 | C# minor | 0.819 | 56 | 6 | 96.4 |
| new-song-idea-orchestra-343 | 213 | 89.1 | C# minor | 0.656 | 80 | 7 | 100.0 |
| song-idea-266 | 36 | 107.7 | A minor | 0.738 | 12 | 3 | 100.0 |
| song-idea-266b | 71 | 107.7 | A minor | 0.669 | 27 | 4 | 88.9 |
| song-progression-327 | 139 | 89.1 | C# minor | 0.690 | 51 | 9 | 96.1 |
| weird-song-25 | 179 | 86.1 | C minor | 0.833 | 59 | 12 | 88.1 |
| witch-house-idea-2 | 56 | 107.7 | E minor | 0.918 | 24 | 7 | 91.7 |

Median key confidence **0.738** (range 0.528–0.918); median in-key **96.1%**;
median distinct chords **7**.

**Read this correctly.** The analyzer is *fine* on sparse, harmonically explicit
material and degrades on dense full productions — exactly the material the app
targets. The two low-confidence tracks (`dopamine` 0.528, `343-bass` 0.554) are
also the two lowest in-key scores, so the KS correlation does carry *some* signal
about its own reliability — but it is uncalibrated and thrown away today.

### 1.4 Timing

| path | wall clock, 212 s track |
|---|---|
| fast (librosa only) | **3.07 s** |
| `--stems --lyrics` (demucs + whisper + basic-pitch) | **69.8 s** |

`analyze.py:27` hardcodes `device="cpu"` for demucs. Measured elsewhere in this
research: demucs CPU 36.5 s vs MPS 9.9 s on a comparable file — **3.67×** available
for a one-line change.

---

## 2. How Chordify does it

### The published system (2012–2016) — fully reimplementable, and the part worth copying

MPTREE (de Haas, Magalhães, Wiering, ISMIR 2012, pp. 295–300) is completely
specified and explicitly **not** machine learning:

| Stage | What they did |
|---|---|
| Chroma | NNLS Chroma via sonic-annotator: 44.1 kHz mono, Hann 16384, hop 2048, log-frequency bins, NNLS note transcription, native tuning correction. Emits **two** 12-D vectors per frame — **bass** and **treble** — "to model the prominent role of the bass note" |
| Beat | QM **Bar and Beat** Tracker — returns beat timestamps **and position-in-bar**. Downbeat came free |
| Dictionary | 48 chords (maj/min/dom7 × 12) **+ N**, binary templates, Euclidean distance, plus a **separate one-hot bass template** scored against the bass chroma |
| Candidates | Not a winner — a **ranked list**. Distances normalised by the best candidate → [0,1] relative preference, cut off at **0.9** |
| Key | Krumhansl-Kessler correlated **per beat**, then DP `M[i,j] = max{M[i-1,j]+K[i,j], M[i-1,j]+K[i,k]+p}`, modulation penalty **p=1**, key segments ≥ **16 beats**. Runs **before** chord selection |
| Merging | Adjacent candidate lists merged by **intersection**, and a merge may only **begin on beat 1 or beat 3** of the bar |
| Selection | HarmTrace CFG error-correcting parser picks the sequence with the lowest insertion/deletion ratio |

### The ablation is the actual finding

RCO on 217 songs (179 Beatles + 20 Queen + 18 Zweieck), MIREX 25-class, 10 ms sampling:

- SIMPLE (best chord per beat) **0.688**
- GROUP (+ the beat-1/beat-3 merge) **0.736** → **+0.048, the largest documented step**
- MPTREE (+ key finding) **0.739**
- MPTREE_key (ground-truth key) **0.741**

Post-hoc Tukey: GROUP→MPTREE and MPTREE→MPTREE_key were **not significant**. The
authors' own hedge, verbatim: *"This difference in performance cannot be attributed
to the merging function alone. However, clearly a lot of the performance gain must
be attributed to this merging function."*

**Metric structure beat tonal grammar.** Ground-truth key bought 0.002. That is the
most decision-relevant number in this document: it argues against spending effort on
smarter harmonic reasoning before the bar grid is right.

### What they ship today

- Two DNNs — a **chord network** and a **beat network**, both spectrogram-in. That is
  the entire published architecture since 2016.
- The beat network predicts **downbeats** as a first-class output; retrained on 4,238
  songs over four days in 2020.
- The product renders **one grid square per beat**, labels drawn only at changes —
  unchanged 2012 → the Feb 2026 support docs.
- Meter is **4/4 with a manual 3/4 toggle**, plus a manual **"shift chords"**
  phase-correction button, still in the Aug 2025 docs. A company with a
  downbeat-predicting neural net still ships a manual phase button. **Downbeat phase
  is genuinely hard, and a manual override is a legitimate product answer.**
- Every song page publishes chords, key, BPM and **tuning frequency**.
- Their vocabulary demonstrably includes sevenths (a D⁷ confirmed on an archived
  Autumn Leaves page). Treat it as "at least sevenths" — a sampling of archived pages
  cannot separate algorithm output from user edits.

### Proprietary / unknowable

Network architectures, spectrogram parameters, exact class count, training corpus
size, **every accuracy number since 2012** (RCO 0.741 is the last they published),
how user edits feed retraining, and whether any grammar post-processing survives.

### Licences (verified — these decide what we can ship)

- **HarmTrace CFG/parser is GPL-3.0-only** (Hackage v2.2.1, © Universiteit Utrecht /
  Oxford / Chordify BV). Strong copyleft — unusable in a shipped Tauri app.
  HarmTrace-**Base** (chord-symbol algebra only) is LGPL-3.0.
- `chordify/tensorflow-haskell` is **byte-identical to upstream** (`ahead_by 0,
  behind_by 0`) — "they serve TF from Haskell" is circumstantial, not confirmed.

### The accuracy ceiling — state it honestly

Chordify's blog summary of Koops et al. (2019) reports 76% root / 59% complex
agreement across 4 annotators on 50 *deliberately ambiguous* Billboard songs. But
Humphrey & Bello (ISMIR 2015, Table 2) measure human-human agreement between two
annotators on RockCorpus at **root 0.932 / majmin 0.905 / sevenths 0.842**, with
automatic systems at majmin 0.723–0.766.

**Do not adopt "0.75 majmin is the human noise floor."** Human agreement is
corpus- and vocabulary-dependent across a ~73%–90% span, and we are not near either
end.

---

## 3. Where Songsmith loses accuracy today

Ranked by cost × confidence.

### 1. Tempo is grid-quantized and never refined — `analyze.py:183-185`

See §1.1. Measured, first-party, n=11. `tempo` comes straight from
`librosa.beat.beat_track` and is reported as truth; the beat times that would fix it
are already in hand. Up to 4.6 s of accumulated bar drift on a real render.
**This is the cheapest large win available.**

### 2. Downbeat phase — `analyze.py:242-247`

```python
phases = [float(np.mean(onset_env[np.asarray(beats[p::4])])) for p in range(4)]
phase = int(np.argmax(phases))
```

Structurally degenerate: kick-on-1-and-3 puts equal energy on two phases;
four-on-the-floor makes all four equal, so the argmax is decided by noise. Three
synthetic pop grooves were reproduced independently — the argmax landed on a
**snare** phase every time, downbeat F-measure **0.000 in 3/3**, while librosa's
beat F was 0.992–1.000. One global phase for the whole song; a single dropped beat
shifts every bar after it with no recovery.

Blast radius: chunks `bar_chords` (`:253`), sets every section `start_sec` (`:284`),
the Composer's audio nudge and the note-event rebase.

**Caveat that decides sequencing: all three failures were on synthesised audio.
Nobody has measured downbeat phase on a real render.**

### 3. The bar majority vote deletes mid-bar changes — `analyze.py:253-261`

A beat-3 change is a 2-2 tie, and `Counter.most_common(1)` is backed by
`heapq.nlargest`, whose `n == 1` short-cut is `max(it, key=key)` — the
**first-inserted** key wins. Verified in this repo's Python:
`Counter(['A','A','B','B'])` → `'A'`, `Counter(['B','B','A','A'])` → `'B'`. The
first half of the bar wins; the second half is deleted. A change on beat 2 is 1-3
and deletes the downbeat chord.

Two-chords-per-bar is the most common pop/EDM pattern this app targets and it
cannot survive this line.

Even if it survived: `reference.md:19` specifies bare strings
`["Am","F","C","E"]`, and `freeze.rs:211-225` coerces every bare string to
`{name, beats: 4}`. **Every imported chord is exactly one bar, by contract.**

### 4. Vocabulary — `analyze.py:150-158`, `:203-204`

24 binary triads. No 7ths, sus, dim/aug, inversions, and critically **no `N`** —
silence, drum breaks and sweeps get a confident triad that propagates into a section
progression at `:281`.

- Subset ambiguity with no tie-break: over an ideal Cmaj7, the Em template {E,G,B}
  and the C template {C,E,G} both score **1.5**.
- `:203` L2-normalises the **chroma**; the templates are never normalised, and `:204`
  is a raw dot product, not a cosine. All 24 triads have norm √3 so this is harmless
  today — but **any 4-note template added naively has norm 2 and wins by
  construction.** Normalising the templates is a *prerequisite* for extending the
  vocabulary, not a cleanup.

### 5. Key — `analyze.py:139-147`, `:194`

One global mean chroma, one winner, `confidence` = a raw Pearson r.

**Corrected mechanism:** the widely-repeated claim that "KS cannot separate relative
keys because the pitch-class content is identical" is **refuted**. `KS_MAJ` and
`KS_MIN` are different vectors (r = 0.6496); on an ideal C-diatonic chroma C major
scores **0.7564** vs A minor **0.7121**, and on an ideal Am-F-C-G chroma C major
**0.935** vs A minor **0.830**. The failure is a **global average + a biased profile
+ emitting only the winner** — all three fixable in the signal layer. A top-2 margin
test is algorithmically available and nobody has tried it.

Also verified: `chroma_cqt` with `tuning=None` **does** auto-estimate tuning inside
`librosa.cqt`. The "no tuning estimation" criticism is wrong; the real gaps are no
CENS/log-compression and no per-segment key.

### 6. Section count is pinned at 8 — `analyze.py:170`, `:268`

`--sections` defaults to 8, and `run_analyzer` (`tools.rs:110-134`) appends only the
audio path, `--lyrics` and `--stems`. Verified: `--sections` appears in exactly two
places repo-wide, both inside `analyze.py` — no caller ever sets it. Every song,
every length, gets 8. `librosa.segment.agglomerative` is a *partitioner* (k is
mandatory) with no repetition or self-similarity modelling — which is what actually
finds a chorus. The repo already knows: `analysis/README.md` says *"raw audio
boundaries noisy; derive form from chord repetition instead."*

### 7. `approx_bars` is computed at the un-snapped tempo — `analyze.py:285`

Uses the raw `beat_track` tempo, while `reference.md:11` then instructs the model to
halve/double it. On any octave-error track **every** section's bar count handed to
the LLM is 2× or 0.5× wrong and is never recomputed. The model's `bars` — not the
measured seconds — becomes the spine (`spine.rs:47-52`, `.unwrap_or(8)`) and drives
the whole Composer timeline, with `padSectionToBars` fabricating "instrumental fill"
to reach the target. Compounds with §1 above: a wrong BPM makes a wrong bar count.

### 8. Nothing is measurable — `agent.rs:1630-1659`

`analysis_summary` copies from `raw` only `melody_notes`, `bass_notes`,
`first_downbeat_sec`, `transcript`. It **drops** `duration_sec`, the measured
`tempo_bpm`, the measured `key{root,mode,confidence}`, `section_count`, the measured
`sections[]` and the **entire `bar_chords[]` timeline**. Nothing else writes them.
After an import there is no record of what the analyzer said vs what the LLM
asserted.

`mir_eval 0.8.2` is installed in the venv, **imported nowhere** (verified), and
**not in `analysis/requirements.txt`** — which contains only `librosa>=0.11` and
`soundfile`. Same for torch, demucs, basic_pitch, faster_whisper: all installed,
none declared.

The only accuracy claim anywhere is `docs/RENDER-ROUNDTRIP.md:73` — *"Stay.wav:
chord changes 92 → 54, distinct chords 12 → 9, key confidence 0.588 → 0.647."*
Fewer chord changes is the mechanical effect of *any* stay-put bonus, and key
*confidence* is the KS correlation — self-assurance, not correctness. Verified on a
drum-free reproduction: confidence rose **0.668 → 0.939 while the answer stayed
wrong**. §1.2 shows the same reversal on a real render.

### 9. No delivery path — `agent.rs:702`, `FinalRenders.tsx:27-28`

`db::set_render_analysis` has exactly one production caller. `analyze_for_composer`
(`agent.rs:1599-1611`) computes and returns but never stashes. The UI prefers the
stash whenever it parses and has a `sections` key. **An improved analyzer cannot
reach any already-imported song** — the opposite of what `docs/BACKLOG.md` tells the
user.

### 10. Zero validation of the LLM output — `agent.rs:604-608`

`unwrap_or("A")` / `unwrap_or("minor")` / `json_bpm(...).unwrap_or(120)`.
`db::update_song_key` (`db.rs:705-708`) is a bare UPDATE — `key_root` can be `"H"`
or `""`. Nothing checks the key's diatonic set against the returned chords (the exact
step `reference.md:10` asks for), nor `start_sec` monotonicity, nor bars-vs-duration.

`uncertain` is in the skill's output schema (`reference.md:14`, `:20`) and is **read
by nothing**.

Chords sections whose label matches no spine row get no `section_id` and are **still
saved** (`agent.rs:631-638`), because the import calls `db::save_artifact` directly
(`:640`), bypassing `freeze::save_artifact_guarded` and the documented drop-with-⚠
at `spine.rs:186-214`. Downstream, `compositionFromSong.ts:363-365` appends them as
extra Composer sections.

### 11. The Composer cannot repair an imported chord

`setChordDegree` (`spans.ts:113-119`) is `{...s, degree}` — it never clears `name`.
And `name` wins everywhere: display (`ChordLane.tsx:133`), playback
(`playback.ts:103-107`), export-back (`compositionToSong.ts:57-58`). Clicking a
different degree on an imported block changes nothing the user can see, hear or
export. There is no chord-name input and no clear-name action.

Worse: `setKeyRoot`/`setKeyMode` (`useCompositionState.ts:74-77`) transpose melody and
bass (degree-based) but **not** named chords, while `ComposerRoute.tsx:143` tells the
user *"Change the key and everything transposes."* The first correction a producer
makes after an import is the key — and it silently shifts the melody a third away
from the chords.

---

## 4. Signal layer vs LLM Reference Analyst

### Right call — keep in the LLM

| Job | Why |
|---|---|
| **Functional labelling** (Intro/Verse/Chorus/Bridge) | SongFormer's benchmark measures Gemini 2.5 Pro at label ACC **0.748** — competitive with All-In-One's 0.740 — but HR.5F **0.423**, the worst strict-boundary score in the table. An LLM given good timings labels about as well as a specialist model and **cannot find boundaries**. That is exactly the current split. |
| **Form from repetition, incl. lyric repetition** | Nothing in the signal layer does this. Lyric repetition alone is a strong chorus detector for sung renders. |
| **Consistent spelling in the chosen key** | Symbolic, cheap; the signal layer has no notion of spelling. |
| **Honest uncertainty** | The channel exists (`uncertain`) and is thrown away. Plumbing bug, not a design error. |

### Papering over a fixable signal problem

| Job | The fixable signal problem |
|---|---|
| **Tempo octave snapping** (`reference.md:11`) | The signal layer holds the beat grid and knows more than the LLM. §1.1 shows it can recover the true tempo to ~0.01 BPM. Worse, `approx_bars` is baked at the pre-snap tempo, so the LLM receives *internally inconsistent evidence* it cannot repair. |
| **Relative major/minor** (`reference.md:10`) | The signal layer emits **one** key with an uncalibrated r and no runner-up. The LLM is asked to resolve an ambiguity it is never shown. Emit top-2 with the margin plus a duration-weighted chord histogram and its job becomes *arbitration* rather than *guessing*. |
| **Chord cleanup toward the diatonic set** (`reference.md:12`) | Narrower than it looks — the instruction whitelists major V, bVII, iv-in-major, bVI. But the whitelist is relative to the key instruction 1 just chose, so borrowed chords are only safe when the key is right, and the LLM is collapsing chords it was never told were uncertain. Chordify's fix is directly portable: hand it a *ranked candidate list with normalised scores*. |
| **Merging over-segmentation** (`reference.md:13`) | The LLM compensates for a hyperparameter pinned at 8 because no caller passes `--sections`. Estimate the count in the signal layer. |

---

## 5. The plan

### T0 — Data-model unblocking

**What is actually blocked, verified against the code:**

| Capability | Blocked? | Where |
|---|---|---|
| 7th / sus / dim chords | **No.** `name` is free text; `midi.rs:8-30` handles maj7/m7/dom7/dim/aug/sus2/sus4 | Blocked only by `analyze.py`'s 24 templates + `reference.md`'s implied triads |
| **Mid-bar (half-bar) chord change** | **No, at integer-beat resolution.** `{name, beats: 2}` works end to end: `chord_events`, `ableton.rs:111`, `SectionChordsEditor.tsx:295` (min=1), `ticksToBeats` | Blocked only by `reference.md:19`'s bare-string schema → `freeze.rs:211-225` → `beats: 4` |
| Sub-beat (eighth) change | **Yes.** `ableton.rs:111` reads `as_i64()`; a float `2.5` silently becomes 4 | Not worth unblocking — Chordify's shipping product is beat-quantised too |
| Slash chords / inversions | **Yes in Rust.** `midi.rs:8-30` falls through to `[0,4,7]` on `"F/A"`; `ParsedChord` has no bass field | `midi.rs:8-30`, `parse-chord.ts:60-67` |
| `N` / no-chord | **Yes, semantically.** `parseChordSymbol('N.C.')` → null → `degreeForChordName` falls back to the key root → **an N.C. region sounds the tonic triad** | `playback.ts:103-108`, `compositionFromSong.ts:81` |
| Modulation / key change | **Yes.** `song` owns one `key_root`/`key_mode`. No per-section key | Model change |
| Non-4/4 | **Yes, end to end.** `analyze.py` hard-codes 4 at `:245`, `:253`, `:285`; `compose/types.ts:23-25` `BEATS_PER_BAR = 4`, `TICKS_PER_BAR = 16` | Model change |
| Section timing on the spine | **Yes.** `section` table (`db.rs:85-90`) has no `start_sec`, no meter | Model change |

**T0.1 — `{name, beats}` in the Reference Analyst output schema** — ✅ **DONE 2026-08-01**
- **Shipped:** `core/src/skills/reference.md` — the schema example now shows `{"name":"Am","beats":4}` objects including a half-bar pair, plus a paragraph defining `beats` (4 = a bar in 4/4, two chords in one bar = `beats: 2` each), telling the model not to stretch everything to 4 out of habit and not to split a held chord into repeats, and stating that `romans` stays one entry per chord in the same order.
- **No Rust change needed, verified:** `freeze::normalize_chord_entries` coerces only bare *strings* — objects pass through untouched — and `render::chords_editor_text` (`render.rs:36-40`) already reads `x.as_str()` **or** `x["name"]`. 129 Rust tests and `tsc --noEmit` pass unchanged.
- **Gain:** unblocks half-bar harmony for the whole pipeline; without it a state-of-the-art recognizer still lands as one triad per bar.
- **Still open:** nothing *produces* `beats ≠ 4` yet — the signal layer still votes one chord per bar (T1.1 is what fills this in). This change removes the contract ceiling ahead of the recognizer, so the baseline run reflects the schema we are keeping.
- **Risk:** `SectionChordsEditor` renders a flat chip row — a 16-chord verse becomes a wall of chips. Budget a bar-grid view before pushing past ~2 chords/bar.
- **Metric:** fraction of imported chord entries with `beats ≠ 4` (structurally **0** before this change).

**T0.2 — Persist the raw analyzer JSON**
- **Changes:** `agent.rs:702` — write the raw object alongside the summary (a `raw` key in the existing `analysis` blob, or a new `render.analysis_raw` column). Extend `analysis_summary` (`agent.rs:1630-1659`) to carry `tempo_bpm`, `key{root,mode,confidence}`, measured `sections[]` and `bar_chords[]`.
- **Effort:** hours. **Gain:** every user import becomes an evaluable sample; analyzer-vs-LLM disagreement becomes visible for the first time. `bar_chords` for a 4-minute track is ~150 small objects.
- **Risk:** it duplicates data. Label it explicitly an **immutable perception snapshot**, not a second home for song facts, or it drifts the way `render.analysis` already does.

**T0.3 — Chord-name plumbing fixes**
- (a) `parse-chord.ts:20-58` — add `''` to `TYPE_TO_QUALITY`; verified against this repo's tonal build that `Chord.get('Cadd9').type`, `Chord.get('Cmadd9').type` and `Chord.get('C7alt').type` are **all `''`**, so three table rows are unreachable dead code. (b) Carry the slash bass through `ParsedChord` and `midi.rs:8-30`. (c) `midi.rs:24-26` maps `Cm7b5` → `[0,3,7,10]` (a plain min7) — should be `[0,3,6,10]`. (d) Make `N.C.` silent instead of sounding the tonic.
- **Effort:** hours. **Risk:** low — but `chord_tones`'s tests (`midi.rs:526-535`) cover no slash chord, add9, dim7 or m7b5. Extend them in the same commit.

**T0.4 — Named and deferred, not silently skipped:** sub-beat `beats`, per-section key (modulation), non-4/4 meter, `start_sec`/meter on the `section` table. Real model changes; none on the critical path. Revisit after T4.

---

### T1 — Cheap signal wins

**T1.0 — Refine the tempo by regressing the beat grid** — ✅ **DONE 2026-08-01**
- **Shipped:** `refine_tempo()` in `analysis/analyze.py`, called right after `beat_track`. Recovers each beat's index by accumulating its **own** step (`round(ibi / median_ibi)`, clipped ≥1) rather than its distance from the first beat — distance-from-start drifts a whole beat within ~100 beats if the median period is off by 1%, which made the first attempt decline on 4 of 5 tracks. Least-squares fits index → time; tempo comes off the slope. Emits `tempo_coarse_bpm` (what librosa's lag grid said) and `tempo_resid_ms` (the fit's residual σ). `approx_bars` picks up the refined tempo automatically. Declines and keeps the coarse value on <8 beats, a non-positive slope, or a >15% disagreement.
- **Measured on 13 real renders after the change:**

| coarse → refined | n | resid σ |
|---|---|---|
| 89.1 → **90.00** | 6 | 7.0–24.2 ms |
| 107.7 → **108.00** | 3 | 7.3–39.5 ms |
| 123.0 → **121.00** | 1 | 8.2 ms |
| 117.5 → **120.00** | 1 | 8.9 ms |
| 143.6 → **140.40** | 1 | **102.4 ms** |
| 86.1 → **86.00** | 1 | 8.7 ms |

  Every track lands on a musically plausible tempo, and `dopamine-v1-final` — the one that was 4.6 s adrift — is both corrected and *flagged* by a residual an order of magnitude above the others.
- **Gated 2026-08-11 (`RESID_GATE = 0.05`).** Ableton ground truth from 6 matching `.als` projects showed the refinement is only trustworthy when a single tempo really describes the beats: 3 exact wins (90, 121.0084, 120 — all at ≤1.8% of a beat of residual) and **1 real regression** (`loki`: truth 108, coarse 107.7, "refined" 109.10, residual 12.8%). Above 5% of a beat the coarse value is kept and `tempo_refined: false` is emitted. **Cost:** `dopamine-v1-final` (24% residual) now falls back to 143.6 — its 4.6 s drift is flagged rather than corrected, which is honest, since it never had ground truth and no single tempo fits it. Fixing that track needs T2.1, not a better line fit. The threshold is provisional (n=4); the 2–12% band is unmeasured and is the first thing T4 should sweep.
- **Tested:** `analysis/test_analyze.py` (7 cases: the lag-quantized correction, jitter tolerance, the `loki` fallback, a skipped beat, the coarse-disagreement guard, too-few-beats, and degenerate input that must never raise). stdlib `unittest`, **no audio required** — which matters, see below. Wired as `make analyzertest`, and `make test` now runs it before the Rust suite.
- **⚠ The calibration corpus is gone.** The `~/Desktop/*.wav` renders these numbers came from were deleted between 2026-08-04 and 08-11 (absent from Desktop, app data dir, Trash, Music, iCloud; Spotlight finds none). The 400 `.als` projects remain, so the ground-truth **tempos** survive but the **audio does not**. T4's tier-1 golden set has to be rebuilt — by re-exporting audio from the `.als` sessions, which is now the cheapest path to exact truth since tempo/meter/MIDI all come from the same file.
- **Metric:** octave-corrected tempo error on the golden set; accumulated bar drift = |true bar-N time − predicted|.

**T1.1 — Chord *segments* + per-beat candidate list, replacing the bar vote** — `analyze.py:249-261`
- **Changes:** emit `chord_segments: [{start, end, name}]` (run-length encoded from the smoothed beat labels) **plus** `beat_candidates: [{beat, time, cands:[{name, rel}]}]` where `rel` is Chordify's normalisation — each candidate's distance over the best, kept if ≥ a cutoff (they used 0.9). Keep `bar_chords` one release for compatibility.
- **Effort:** days. Depends on T0.1 to survive to the artifact.
- **Gain:** beat-3 changes stop being deleted by a tie-break, and the Reference Analyst gets an **ambiguity signal** — it can tell which beats are already certain. That is exactly the division of labour HarmTrace implemented.
- **Metric:** mir_eval `overseg`/`underseg`/`seg` and transitions-per-song. Reference point: a CRNN emits 167 transitions/song against a ground truth of 104; HMM smoothing brings it to 102 **with accuracy unchanged (60.0 → 60.0)** — smoothing buys segmentation quality, not accuracy.

**T1.2 — Add `N`, and normalise the templates** — `analyze.py:150-158`, `:203-204`
- **Changes:** (a) L2-normalise the templates so `scores` is a genuine cosine — **before adding any 4-note template**, or the norm-2 vs norm-√3 asymmetry hands every beat to the bigger template. (b) Add an `N` state gated on beat energy / spectral flatness (`energy_gate` at `:76` already computes what is needed).
- **Effort:** hours. **Gain:** intros, drum breaks and sweeps stop injecting a confident wrong triad into a section progression.
- **Metric:** mir_eval `majmin` **plus the comparable-duration fraction** — out-of-gamut chords return −1 and are silently dropped from the weighted mean, so `majmin` alone can be computed over a shrinking denominator.
- **Explicitly do NOT** add 7th templates here. Oudre et al. measured maj+min **0.70** → maj+min7 **0.64** — minor-7th templates *hurt*, via precisely the relative-major confusion this app already has. (The maj+7+min7 cell is **0.66**; the 0.63 in circulation is 7+min7 with no major template at all.) 7ths belong in T2.2, behind a model that can score them.

**T1.3 — Key: emit the ranking, not just the winner** — `analyze.py:139-147`, `:194`
- **Changes:** return the full 24-key ranking (or top-3 with margins) plus a duration-weighted chord-root histogram. Chordify's per-beat key DP (p=1, ≥16-beat segments) is a T2-sized change their own ablation values at +0.002 RCO — measure before building it.
- **Effort:** hours for the ranking; days for the DP.
- **Risk:** **do not ship "first-and-last chord decides the tonic" as a standalone rule.** Reproduced on an Am-F-C-G track where `estimate_key` says C major: with the measured phase the bars came out `['F','C','G','Am',…]` — first chord **F**, last chord **G**, histogram F 4 / C 4 / G 4 / **Am 3**, i.e. Am is the *least*-dwelt triad and the rule votes against it. The first/last rule is downstream of a correct bar grid.
- **Metric:** `mir_eval.key.evaluate` weighted score (relative-key error scores 0.3, so improvement is visible). Note the asymmetry: the 0.5 fifth bonus applies only upward, so ref/est order matters.

**T1.4 — Delete verified dead code and a stale comment** — ✅ **DONE 2026-08-01**
- **Shipped:** `chord_for` deleted (verified called by nothing repo-wide). The stale comment above the `np.clip` rewritten — it described `pad=True` behaviour that no longer applies and actively misled anyone touching the beat indexing, which is exactly the code T1.0/T1.1 must touch.
- **Deliberately NOT done:** the `np.clip` itself was kept. It is dead today (`util.sync(pad=False)` returns `len(beats)-1` columns, so boundaries max out at `len(beats)-2`), but it was added after a real crash that killed the whole analysis with no JSON at all. A one-line guard whose failure mode is "lose the entire analysis" is worth keeping even when the current call path can't reach it; the comment now says so.

**T1.5 — Harden `run_analyzer`** — ✅ **DONE 2026-08-01**
- **Shipped:** `core/src/tools.rs` — `kill_on_drop(true)` plus a wall-clock timeout via `analyzer_timeout()` (1800 s, overridable with `SONGSMITH_ANALYZER_TIMEOUT_SECS`), mirroring `engine.rs`'s `claude_timeout()`. Failures now report elapsed seconds, so the timeout can be calibrated from real runs rather than guessed.
- **Gain:** the analyzer is the **longer** of the two subprocesses (§1.4: 69.8 s with stems, and whisper can push it minutes) and was the only one with neither guard. A hang hung the import forever; cancelling orphaned a Python process on a core.
- **Note:** the timeout is deliberately generous (30 min). It exists to stop a *hung* analyzer, not to police a slow one — a real 8-minute track on slow hardware must not be killed.

**T1.6 — Use MPS for demucs** — `analyze.py:27`
- One line: `device="cpu"` → auto-select MPS when available. Measured 36.5 s → 9.9 s (**3.67×**) on a comparable file. Keep a CPU fallback; MPS has historically produced different output on some torch versions, so gate it behind a settings flag and diff the stems once on the golden set before defaulting it on.

---

### T2 — Model swap

**T2.1 — `beat_this` for beats AND downbeats**
- **Changes:** replace `librosa.beat.beat_track` (`analyze.py:183`) and delete the phase argmax (`:242-247`). Take `beats` and `downbeats` from `File2Beats(checkpoint_path='final0', device='cpu', dbn=False)`; derive tempo by the T1.0 regression; derive beats-per-bar from the median beat-index gap between downbeats instead of assuming 4.
- **Licence:** MIT for code **and published weights** — verified verbatim. The caveat: the next README sentence says *"some of the training files are fully copyrighted or under limited Creative Commons licenses, and it is up to the user to assess whether this may impact their use case."* That assessment is pushed onto you.
- **Cost, measured:** `torch 2.13.0` already in the venv (demucs pulls it). Two runs: **1.17 s and 1.21 s for a 181 s file** (~150× realtime) on CPU. Install delta **~4.4 MB**. Checkpoints: `final0` **81 MB**, `small0` **8.5 MB** (GTZAN 88.8/77.2 vs 89.1/78.3 — the small model is a real option). Ship it pre-cached or the first run downloads 81 MB.
- **Expected gain:** GTZAN beat F1 **89.1 ± 0.3**, downbeat F1 **78.3 ± 0.4**, no DBN. Harmonix 8-fold (closest to this app's material): beat **95.8**, downbeat **90.7**. SOTA downbeat is only ~78 on GTZAN — downbeat is materially harder than beat.
- **Risks:**
  - **It does not emit `beat_positions`.** The API is `beats, downbeats = File2Beats(...)(path)` — two arrays. `beat_positions` is an **allin1** field. Meter must be derived.
  - **No tempo prior** → it can lock to half-time on sparse grooves (measured 69.8 BPM on a true-140 track where librosa got 143.6). Cross-check its median IBI against librosa and flag a 2×/0.5× disagreement rather than silently trusting either.
  - Continuity metrics trail DBN systems (downbeat CMLt 67.3 vs Hung et al. 71.5). The "can fail on difficult and underrepresented genres" caveat is in the **paper**, not the README.
  - **Do not repeat "+2.5 F1 over madmom."** It is **+3.5** beat (89.1 vs 85.6) and +14.3 downbeat, cross-paper, on different GTZAN subsets (993 vs 999 tracks).
- **Metric:** `mir_eval.beat.evaluate` on downbeat sequences. **API trap:** the returned keys are `'Correct Metric Level Total'` / `'Any Metric Level Total'`, **not** `'CMLt'`/`'AMLt'` — the short names raise `KeyError`. Read CMLt≈0 with AMLt≈1 as "octave or phase error, pulse is right"; both near 0 as "genuinely lost."
- **Rust-native alternative:** `beat-this-rs` v1.0.0, MIT, pure-Rust `rten` backend, bundles a 10 MB small model, F-measure **1.0** vs the Python reference. Its own benchmark shows it **losing** at length (13:48 file: 12.1 s vs Python 11.9 s) — the case for it is deployability, not speed. `tools.rs:110-134` already treats the analyzer as a swappable `<audio> [--lyrics] [--stems]` → JSON contract, so it can land behind the same interface later.

**T2.2 — BTC for chords — needs a spike first**
- **Changes:** vendor `btc_model.py` + `utils/transformer_modules.py` + a copy of `idx2voca_chord` from `jayg996/BTC-ISMIR19` (MIT), plus the 12.2 MB `btc_model_large_voca.pt`. Feed per-frame posteriors into the beat-sync + segment logic from T1.1.
- **Verified:** MIT weights, `btc_model_large_voca.pt` = 12,229,576 B, **3,036,842 parameters** (loaded and counted twice). An end-to-end run was reproduced **in this repo's venv**: 233 segments on a 257 s file, qualities `['7','dim','dim7','maj6','maj7','min','min7','sus2','sus4']` + N, ~1.3 s dominated by audio decoding. 170-class vocabulary.
- **Two traps the spike must handle:**
  1. The repo's `test.py` **cannot run** here: `utils/hparams.py` calls `yaml.load(f)` without a Loader; `utils/chords.py` uses `np.int`/`np.bool` at module scope in six places *and* imports **pandas, which is not installed**. The workable path vendors the three files above and bypasses `utils/chords.py` and `utils/hparams.py` entirely — that path needs exactly one `np.float` fix.
  2. **A real timestamp bug in BTC's own code:** `utils/mir_eval_modules.py` computes `10.0/108 = 0.0925926 s` per frame while the true hop is `2048/22050 = 0.0928798 s` — **0.31% drift, ~0.8 s over a 257 s track.** Vendored code must compute frame times from the true hop.
- **Expected gain — honestly:** BTC makes maj7/min7/dom7/sus2/sus4/maj6/dim/dim7/hdim7/aug/X/**N** representable at all, which 24 binary triads cannot do. **The accuracy delta on this app's material is unknown.** The circulating "+13 to +20 points" figure is **refuted** — it subtracts an F-measure on one Beatles song (0.503), an Average Overlap Score over 180 Beatles songs (0.70) and a WCSR over 471 pop/rock tracks (82.7) as if they were one metric. Published BTC numbers (Root 83.5 / Thirds 80.8 / Triads 75.9 / Sevenths 71.8 / Tetrads 65.5 / MajMin 82.3 / MIREX 80.8) are on Isophonics + Robbie Williams + UsPop2002 — 1960s–2000s pop/rock, nothing like a Suno render.
- **Risk:** the field is at a glass ceiling — post-BTC models have moved large-vocabulary metrics by **1–3 points in seven years**. If BTC underperforms on renders, that is information, not failure.

**T2.3 — Ruled out. Written down so it is not relitigated.**

| Package | Blocker (verified) |
|---|---|
| **madmom** | Source BSD-2, but **all model/data files are CC BY-NC-SA 4.0** — *"You must not use the material for commercial purposes"*, explicitly covering pickled processors. PyPI 0.16.1 is from 2018-11-14; only git `main` works on Python 3.12. Chord recognition is **maj/min only, 25 classes** — buys no vocabulary. |
| **allin1** | Architecturally ideal (300 K params; Harmonix beat 0.958 / downbeat 0.915 / HR.5F 0.660; MIT; emits exactly `intro/outro/break/bridge/inst/solo/verse/chorus`) — but hard-depends on madmom-from-git, inheriting the NC licence. **Correction:** `pip install natten` **succeeds** here (natten 0.21.7, exit 0). allin1 breaks on an **API change**: `dinat.py` imports `natten1dav, natten1dqkrpb, natten2dav, natten2dqkrpb` and NATTEN 0.21.7 exports none. Open issue #30; repo untouched since 2024-05-09. |
| **Essentia** | AGPL-3.0 (verified via GitHub API); models CC BY-NC-ND. Its `ChordsDetection` *"finds the best matching major or minor triad"* over HPCP — a **downgrade** from what we have. A native `macosx_arm64` cp312 wheel does exist (20.4 MB), contra the common claim — the blocker is licence, not packaging. |
| **Chordino / NNLS-Chroma** | GPL-2.0-or-later. Also `chord-extractor` (GPL-2.0 **and** `requires_python <3.12` while this venv is 3.12.13 — cannot install) and `autochord` (Apache-2.0 code executing the GPL plugin; maj/min-only at a self-reported 67.33%; model fetched from Google Drive at import). |
| **crema** | **Measured to fail here:** `keras.models.load_model(...)` under this venv's keras 3.15.0 / tf 2.16.2 raises `TypeError: … Argument 'name' must be a string and cannot contain character '/'. Received: name=cqt/mag`. |
| **HarmTrace CFG** | **GPL-3.0-only.** Only HarmTrace-Base is LGPL. |
| **ACR_seq2seq** (Kim & Park 2026) | Code released — but **no licence file**, a harder blocker than no code. |

---

### T3 — UX + correction loop

**T3.1 — Forced re-analyze** — `FinalRenders.tsx:27-28`, `agent.rs:1599-1611`
- Treat the stash as a cache with an explicit `↻ Re-analyze` override; make `analyze_for_composer` write via `db::set_render_analysis`. **Without this nothing in T1 or T2 reaches an existing song.** Explicit button, explicit confirm (it overwrites a stash the producer may rely on).

**T3.2 — Surface `uncertain`** — `reference.md:14,20` → the Structure artifact / a banner
- Hours. With human agreement well short of 1.0, telling the producer *where to look* is worth more than a marginal accuracy point.

**T3.3 — Validate the LLM output before it becomes song truth** — new function called from `agent.rs:588`, shared with `analyze_for_composer`
- Checks: `key_root` is a note name; `key_mode ∈ {major, minor}`; `bpm` within tolerance of `tempo_bpm × {0.5, 1, 2}`; `start_sec` monotonic; section bars vs `duration_sec` within ~15%; every chords section matches a spine row.
- **Warn-and-record**, hard-fail only on grid-corrupting values. **Metric:** validation warnings per import; contract violations reported *separately* from accuracy (a dropped section currently vanishes from mir_eval rather than scoring zero).

**T3.4 — Route the import's chords save through the guard** — `agent.rs:640`
- `db::save_artifact` → `freeze::save_artifact_guarded` (or at minimum `normalize_chord_entries`). Today the artifact holds bare strings while the stash holds `{name, beats}` objects, and `flowcheck.py:223` asserts the normalized shape that imported songs would fail. **There is currently no test covering `import_reference_full` at all** — write one first.

**T3.5 — Make the Composer able to repair a chord** — `spans.ts:113-119`, `useCompositionState.ts:74-77`, `ComposerRoute.tsx:143`
- Clear-or-rewrite `name` on a degree pick; add an explicit chord-name field; and either transpose named chords on a key change **or** delete the "everything transposes" claim.
- **Gain:** turns the Composer from a read-only view of a wrong analysis into the repair surface the docs already promise — which *lowers the accuracy bar the analyzer has to clear.* Chordify ships a manual phase button for the same reason.
- **Risk:** clearing `name` loses the imported spelling (Bb vs A#). Prefer editing `name` in place.

---

### T4 — Measurement harness (gates everything above)

**T4.1 — The golden set: 12–16 tracks, three tiers**

| Tier | n | Why |
|---|---|---|
| **Self-produced Ableton exports** | 4–5 | From the `.als`: exact tempo, time signature, arrangement locators (named sections), MIDI clips on harmony tracks — **exact truth for all five quantities**, audio you own, and the **only** tier where `sevenths`/`tetrads` mean anything. *(Nothing in this codebase reads `.als` today — `ableton.rs` only pushes into Live. The reader is ~a day of unbuilt work.)* |
| **Verified-by-ear Suno renders** | 6–8 | The actual input distribution. Use the app's generated spec as a **prior to check against, never as truth** — Suno transposes, changes mode and reorders sections, so trusting the spec builds a benchmark that rewards the LLM for hallucinating back toward the prompt. Budget **45–90 min/track**. |
| **GuitarSet excerpts** | 2 | Zenodo 3371780, **CC BY 4.0 with audio** (verified), chords + beats + downbeats + key. Solo acoustic guitar, so it validates code paths rather than predicting accuracy — but it survives a reinstall and can be committed publicly. |

**T4.2 — `truth.json`, one per track, as the single home**
```json
{"key":"F# minor","bpm":100.0,"beats":[…],"downbeats":[…],
 "chords":[[start,end,"F#:min"],…],"sections":[[start,end,"verse"],…],
 "provenance":"ableton|verified-by-ear|guitarset","notes":"…"}
```
Harte syntax from the start. Generate `.lab` views on demand — a `.lab` and a JSON
disagreeing about a boundary is exactly the second-copy bug `CLAUDE.md` warns about.

**T4.3 — `analysis/eval.py` + `make analyzercheck` / `make analyzercheck-llm`**
- Add **`mir_eval>=0.8`** to `analysis/requirements.txt` (installed, undeclared).
- Two adapters do the real work: `harte(sym)` mapping `'F#m'→'F#:min'` (note: bare `'C'` **already validates** as Harte major — only the `m` form fails), and `bars_to_intervals(bar_chords, end)` with **equal-neighbour merging** (unmerged per-bar rows inflate `overseg`). The LLM adapter additionally expands each section's looped progression across its bars at the reported bpm from `start_sec`, joins `chords.sections` to `structure.sections` by label consume-once (the rule `midi::clamp_part_take` already uses), and emits **contract violations separately**. It must **not raise** on an unmapped quality — fall back to `'X'` and log.
- Split the targets: analyzer-only is deterministic, offline, no Claude, runs on every `analyze.py` edit; the LLM arm costs real generations. Copy `flowcheck.py`'s `env_remove` of `ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN` — house rule.
- **Cache demucs stems per track** (hash of the audio) or 14 tracks × `--stems` is 20–30 min and nobody runs it habitually.

**T4.4 — Gate on five numbers**

| Metric | Why this one |
|---|---|
| **Downbeat F-measure** | A wrong bar grid corrupts section `start_sec`, the audio nudge and the note rebase simultaneously. Gate hardest here. |
| **Key weighted score** (`mir_eval.key`) | Partial credit — relative 0.3, fifth 0.5, parallel 0.2 — so improvement is visible instead of binary. |
| **Octave-corrected tempo error** | `min` over ×½/×1/×2 of `\|est−ref\|/ref`. A 2% error is unusable for DAW export; a clean ×2 is trivially fixable. Score raw and post-LLM separately. **This is the metric T1.0 moves.** |
| **Chord `majmin`** + **comparable-duration fraction** | `majmin` is lenient about extensions (verified: ref `A:min7` vs est `A:min` = **1.0**), right for a triad-only analyzer. Out-of-gamut chords return **−1 and are dropped**, so print the fraction or the score shrinks its own denominator. |
| **Boundary F@3s** | Not F@0.5s — a producer does not care about 400 ms on a section line. |

Diagnostics, printed but not gated: `root`, `sevenths`, `mirex`, `overseg`/`underseg`/`seg`, beat F / CMLt / AMLt, Pairwise F, NCE F, segment count vs true, transitions/song, and **`tempo_resid_ms`** from T1.0.

**T4.5 — Statistics, or the harness will manufacture improvements**
- With ~14 tracks a mean `majmin` moving 0.74 → 0.77 is not evidence. Report a **per-track delta table**, **win/loss/tie** with a Wilcoxon signed-rank, and a **paired bootstrap over tracks** (10 k draws).
- The LLM arm is nondeterministic. **Measure its run-to-run spread on a fixed analyzer JSON (≥3 repeats) before judging any skill edit** — if the spread is ±4 points, no smaller edit is evaluable. Measure once, write the number into `docs/`, then drop to single runs.
- `docs/BACKLOG.md` already records this repo getting burned by a real-looking number attributed to the wrong cause (the MCP-startup mismeasurement). A harness reporting unstable means will do it again with more authority.

**T4.6 — Gate on regression, not absolute targets**
Commit `baseline.json`; require it updated in the same commit as the change that moves it, with the delta table in the message. Do **not** set an absolute floor from the first run — that encodes today's bugs as acceptable — and do not set it at "the human noise floor of 0.75", which is refuted.

**T4.7 — Do not build on the public corpora; record why**

| Corpus | Blocker (verified) |
|---|---|
| Isophonics | Annotations only, **no licence stated anywhere**; timed against specific CD masters |
| McGill Billboard | CC0 annotations, **no audio** |
| SALAMI | CC0 annotations, no audio; DDMAL's own repo points at YouTube |
| Harmonix | No audio; documented path is DTW-aligning YouTube (a ToS problem). **Correction: the repo LICENSE is MIT** — the "CC BY 4.0" is the ISMIR per-paper footer |
| JAAH | **Correction: CC BY-NC-SA 4.0**, verified via the Zenodo REST API — **not** CC BY 4.0. Plus it needs two Smithsonian box sets |
| RWC | Ships audio, but research-use-only, no redistribution, per a signed agreement |
| CASD | CC BY-NC-SA 4.0. Cite its numbers; don't ship the data |

State plainly in the docs that a golden-set `majmin` is **not** comparable to an
Isophonics `majmin`.

---

## 6. Recommended first move

**One week: the measurement harness (T4) + raw persistence (T0.2), 6 tracks not 14
— and ship T1.0 immediately, because it is already measured.**

### Why not `beat_this` first

Every number that would justify beat_this-first was measured on GTZAN, Ballroom or
Harmonix. The one place a downbeat failure was reproduced, it was on **synthesised
audio** — three synthetic grooves. Nobody has measured downbeat phase on a real
render. The house rule applies exactly: *a model-shaped fix that isn't measured
isn't done.*

The harness is also 3–5 days — the same order as beat_this — and it converts every
subsequent change from a guess into a measurement.

**T1.0 is the exception.** It is not a guess: §1.1 measured the defect on 11 real
tracks, the fix is ~15 lines, and the correct answers (90.00, 108.0, 86.00) are
unambiguous. Ship it in the same week and let the harness confirm it.

### Exactly what to build

**Day 1 — plumbing**
1. `analysis/requirements.txt`: add `mir_eval>=0.8`.
2. `agent.rs:702` + `:1630-1659`: persist the raw analyzer JSON; add `tempo_bpm`, `key{root,mode,confidence}`, measured `sections[]` and `bar_chords[]` to the summary. Label it an immutable perception snapshot.
3. `analysis/bench/tracks/<slug>/{audio.wav, truth.json}` + the schema above.

**Day 2 — adapters** (the only genuinely fiddly code)
4. `analysis/eval.py`: `harte(sym)`; `bars_to_intervals` with equal-neighbour merging; `llm_to_intervals(structure, chords)` expanding looped progressions at the reported bpm from `start_sec`, joining by label consume-once, emitting contract violations separately.
5. Wire the four `mir_eval.*.evaluate` calls. **Use `'Correct Metric Level Total'` / `'Any Metric Level Total'`, not `'CMLt'`/`'AMLt'`.**

**Day 3 — 3 Ableton tracks**
6. `.als` reader (gzipped XML): tempo + automation, time signature, arrangement locators → sections, harmony-track MIDI clips → chords. Render each to WAV. Exact, free, yours.

**Day 4 — 3 Suno renders**
7. Annotate by ear against a DAW, using the app's generated spec as a prior. Provenance `verified-by-ear`. The 11 tracks in §1.3 are the obvious candidate pool — start with `dopamine-v1-final` (the worst) and `song-idea-266` (the best) so the set spans the range.

**Day 5 — targets and baseline**
8. `make analyzercheck` (deterministic, offline) and `make analyzercheck-llm` (with `env_remove`). Per-track table + summary + baseline diff. Stem cache keyed on audio hash.
9. Run it. Commit `baseline.json`. Put the numbers into `docs/RENDER-ROUNDTRIP.md` **as measurements with n stated**, replacing the Stay.wav self-consistency prose.

### Ship alongside — hours each, no harness needed

- **T1.0** (tempo regression) — measured, do it first
- T1.4 (delete `chord_for`, the dead `np.clip`, the stale `pad=True` comment)
- T1.5 (`run_analyzer` timeout + `kill_on_drop`)
- T1.6 (demucs on MPS, behind a flag)
- T0.1 (`{name, beats}` in `reference.md:19`) — before the baseline run, so the baseline reflects the schema you are keeping

### What the first run should change

If **downbeat F ≥ 0.9** on real renders, T2.1 drops down the list and T1.1/T1.2
(segments, `N`, template normalisation) rise to the top. If it is near 0 as the
synthetic tests suggest, T2.1 is unambiguously next and everything else waits —
Chordify's own ablation says metric structure dominates tonal reasoning.

---

## 7. Open questions that need a measurement, not an opinion

1. **What is the downbeat F-measure on real Suno renders?** The whole §3 ranking pivots on this; the only evidence is 3/3 failures on synthesised audio. *Measured by:* T4, tier-2 tracks.
2. **Does `beat_this` survive Suno renders?** Trained on real recordings; an AI render is out of distribution differently than a synth click track. Its downbeat head also fired on nearly every beat of a synthetic 3/4 waltz (F1 0.511) while the paper reports Ballroom downbeat 95.3 — unexplained. *Measured by:* T4 tier-2 plus one real 3/4 track.
3. **What does BTC actually score on this app's material versus 24 triads?** Genuinely unknown. *Measured by:* T4 `majmin` + `sevenths`, RAW vs BTC arms.
4. **Is `nseg = 8` actually wrong?** Nobody has counted true sections on a real render. Renders are short and highly regular; a fixed prior of 8 may beat unsupervised estimation, making this the **lowest**-value item despite looking like an obvious defect. Field ceiling: best MIREX on SALAMI is F@0.5s **54.09 ± 18.50%** against ~90% human agreement. *Measured by:* count true sections on 6 renders **before writing any code**.
5. **What is the Reference Analyst's run-to-run variance on a fixed analyzer JSON?** Precondition for judging any `reference.md` edit. *Measured by:* 3 repeats × 6 tracks.
6. **Does the LLM cleanup net-help, and which of its four jobs?** Run three arms — RAW, LLM, and a per-track ORACLE (whichever scored better) — and judge on **wins/losses and the single worst regression**, not the mean. An LLM that rescues five tracks by +15 and wrecks three by −20 shows a positive mean while feeling unreliable. The oracle gap is the headroom a better prompt could recover.
7. **How often do `+0.12` (`analyze.py:212-213`) and `STAY = 0.10` (`:214`) change the answer, and in which direction?** Both landed in one commit (`e1af96d`) alongside four other changes, justified only by aggregate self-consistency on one file. `scores ∈ [0, √3]`, but **the real inter-template margin distribution has never been measured**. *Measured by:* dump the margin histogram over the golden set, then sweep each constant independently.
8. **Does removing drums (`analyze.py:190-193`) help or hurt?** §1.2 measured it **hurting** on `dopamine-v1-final` — 48% bar agreement with the fast path, lower key confidence, lower in-key% — while a separate reproduction returned the same *wrong* key at confidence 0.939 vs 0.668. Confidence is not correctness, and this path is documented as the correct one. *Measured by:* T4 key + chord scores with and without `--stems`.
9. **Where does the import latency budget go, and does whisper belong on the default path?** §1.4: fast path **3.07 s**, `--stems --lyrics` **69.8 s**. demucs is 36.5 s CPU / 9.9 s MPS. `analyze.py:323-330` can run whisper **twice** when the vocals stem comes back near-silent — it breaks on `len(transcript) >= 3`, not on stem energy, though `energy_gate` at `:76` already computes what is needed.
10. **Do imported songs currently fail `flowcheck.py:223`'s normalized-chords assertion?** Predicted yes — the import bypasses `normalize_chord_entries` — but unverified because **no test covers `import_reference_full` at all**. *Measured by:* T3.4's prerequisite test.

---

## 8. Corrections carried forward — do not re-quote the originals

Each of these was asserted somewhere in the research and then refuted by an
independent check. They are recorded so they do not come back.

| Refuted / corrected | Use instead |
|---|---|
| "BTC buys +13 to +20 pts maj/min" | Gain unknown on this app's material; BTC's value is vocabulary (170 classes incl. N) |
| "HMM smoothing is worth +27.6 pts" | Cross-song subtraction of two different Beatles songs. Smoothing changed accuracy **0.0** (60.0→60.0) while cutting transitions 167→102 |
| "beat_this beats madmom by +2.5 F1" | **+3.5** beat / +14.3 downbeat, cross-paper, different GTZAN subsets |
| "0.75 majmin is the human noise floor" | Human-human is 0.73 (4 annotators, ambiguous Billboard) to **0.905 majmin / 0.932 root** (2 annotators, RockCorpus) |
| "chroma_cqt does no tuning estimation" | It auto-estimates via `librosa.cqt` → `estimate_tuning` |
| "the `np.clip` at :274 can collapse boundaries" | `sync(pad=False)` returns `len(beats)-1`; the clip is **dead** and the comment above it is stale |
| "KS can't separate relative keys — identical pitch content" | Profiles differ (r=0.6496); C major 0.935 vs A minor 0.830 on ideal Am-F-C-G. A top-2 margin test is available |
| "first/last chord gives A minor deterministically" | Reproduced: bars came out F…G with **Am the least-dwelt triad**. Requires a correct bar grid first |
| "`pip install natten` fails on Apple Silicon" | It succeeds; allin1 breaks on a NATTEN **API** change |
| "beat_this weights' licence unverified" | Weights **are MIT**; the real caveat is copyrighted training data, pushed onto the user |
| "beat_this emits `beat_positions`" | API is `(beats, downbeats)`. `beat_positions` is an **allin1** field |
| "HarmTrace CFG is LGPL-3.0" | **GPL-3.0-only.** Only HarmTrace-Base is LGPL |
| "JAAH / Harmonix are CC BY 4.0" | JAAH is **CC BY-NC-SA 4.0**; Harmonix's repo is **MIT** |
| "mir_eval.beat returns `CMLt`" | Keys are `'Correct Metric Level Total'` / `'Any Metric Level Total'` |
| "Oudre maj+7+min7 = 0.63" | **0.66**. The 0.63 cell is 7+min7 with no major template |
| "`reference.md` instruction 3 destroys bVII/iv-in-major" | Both are explicitly whitelisted. The risk is narrower: the whitelist is relative to the key instruction 1 chose |
| "mir_eval rejects `'C'`" | Bare roots validate as major. Only `'F#m'` raises |
| "Chordify replaced HarmTrace with DL in 2016" | The post announces a new algorithm; it never says replaced. HarmTrace-Base is still in their back-end |
| "demucs MPS is 2.59× CPU" | **3.67×** (9.94 s vs 36.49 s) |
| "the librosa fast path is 0.79 s" | **3.07 s** measured on a 212 s render (§1.4) |

---

## Provenance

- §1 is first-party: 11 renders from `~/Desktop`, current `analyze.py`, this machine, 2026-08-01. Raw JSON was written to a scratch dir and is not committed; re-run to reproduce.
- §2–§5, §7–§8 come from six research dives, each fact-checked by an independent skeptic instructed to refute. Claims that survived are stated plainly; claims that did not are in §8.
- Every code claim (`file:line`) was read from this repo at HEAD `138d9d6`.
