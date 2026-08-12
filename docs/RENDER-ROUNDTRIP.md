# The render round-trip: AI song in → human-produced song in your DAW

STATUS: COMPLETE for phases 1–3 (2026-07-24) plus the iteration loop, written
parts (Melodist + Arranger), analyzer v2/v3, and the 2026-07-28/29 audit fixes.

The north star (user, 2026-07-24): *"a tool that sits between Suno and your DAW —
take an AI song idea and turn it into a human-produced song."* This doc maps that
pipeline end to end: what runs where, what can fail, and every fallback.

```
Suno render (audio file)
   │  ⤵ Import reference          (Library)            → a complete song mock
   │  🎼 Analyze → Composer        (Renders tab)        → an editable Composition
   ▼
LOCAL ANALYSIS (analysis/analyze.py — audio never leaves the machine)
   demucs (htdemucs, CPU) ──► vocals · bass · nodrums (= vocals+bass+other) stems
   librosa               ──► tempo · beats · DOWNBEAT phase
                             key + bar chords FROM THE DRUM-FREE STEM
                             (+ bass-root bonus, stay-put smoothing)
                             section boundaries (agglomerative on beat features)
   faster-whisper        ──► lyric transcript  (vocals stem → mix fallback ladder)
   basic-pitch (ONNX)    ──► melody + bass note events (energy-gated, mono-cleaned)
   ▼
REFERENCE ANALYST (Claude) — corrects key from chord content, snaps tempo, cleans
   chords to the diatonic set, derives the form (repeated lyric blocks find the
   choruses), labels sections WITH start_sec.  Note events are STRIPPED from this
   prompt (they're for the Composer's lanes, not for musical reasoning).
   ▼
THE SONG / THE COMPOSITION
   spine + Structure + Chords + Lyrics (transcript lines by start_sec window)
   Concept → Lyric Spec → Prompt via reverse context · everything approved
   render attached with the ANALYSIS STASHED
   (summary + note events + transcript + first_downbeat_sec)
   ▼
COMPOSER (Phase 2–3)
   melody/bass lanes pre-filled from the transcription (diatonic fold, rebased
     onto the downbeat) — and from WRITTEN TAKES when they exist
   render audio follows the transport (waveform strip, auto-nudge, volume)
   section FOCUS: loop one section, A/B original ↔ your lanes, ✓ done marks
   ▼
WRITTEN PARTS (Claude — the 🎛 Variations row)
   🎶 Melodist  → the Lead take (motif-based, per section)
   🎶 Arranger  → bass / pad / chords / arp takes (one skill, part-parameterized)
   ⚡ → Live    → pushes ONE named track, non-destructively
   ▼
ABLETON (⚡ export)
   Sections + Bass/Chords/Pad/Chord melody/Lead/Filler/Arp/Drums
   (a written take REPLACES its part's formula, including its silences)
   + the render as a "Reference" AUDIO clip at bar 1 (mute/solo to A/B in Live)
   ▼
ITERATE
   rebuild sections → export back to the song → 🔄 refresh the Generation Prompt
   → paste into Suno → import v2 → A/B v1 vs v2 on the Renders tab
```

## Analyzer accuracy (v2 2026-07-28, v3 2026-07-29)

The stems are paid for once and used everywhere, and the grid is anchored to a
measured downbeat:

- **Chords/key read the DRUM-FREE stem** — kicks and hats smear chroma.
- **The bass stem votes on roots** (+0.12 on templates rooted at the bar's
  strongest bass pitch class): Cm/Eb/Ab confusions are root confusions.
- **Stay-put smoothing** (mini-Viterbi over per-beat labels, then a bar majority
  vote) stops one noisy beat from flipping a bar.
- **Energy gate on note events**: basic-pitch reports confidence, not loudness,
  so notes sitting in silence (reverb tails, stem bleed) are dropped.
- **`pad=False` on every `util.sync`** — it otherwise emits `len(beats)+1`
  columns and every chord/section was stamped ONE BEAT LATE (audit).
- **Bars start on the measured downbeat phase**, not on `beats[0]`, so bar times
  (and the section `start_sec` derived from them) are musically real.

Measured on one render (Stay.wav, 146 bars): chord changes 92 → 54, distinct
chords 12 → 9, key confidence 0.588 → 0.647, and after v3 the opening reads as a
clean 8-bar `Eb Ab Ab Ab | Cm Cm Cm Cm` cycle with bar 1 on the downbeat.

**Read those v2/v3 numbers narrowly (2026-08-01).** They are n=1 and they are
*self-consistency*, not accuracy: fewer chord changes is the mechanical effect of
any stay-put bonus, and "key confidence" is the KS correlation — how sure the
estimator is, not whether it is right. Re-measured across 13 renders, the drum-free
stems path does **not** uniformly win: on a 212 s full production the stems and
no-stems paths agree on only **48% of bars** (58/121) and the stems path scores
*lower* key confidence (0.48 vs 0.528). On a 35 s sparse loop both paths are
identical and correct. Accuracy here is material-dependent and has never been
measured against ground truth — see `docs/IMPORT-ACCURACY-PLAN.md`.

### v4 (2026-08-01) — the tempo is measured, not read off a lag grid

`librosa.beat.beat_track` returns the tempo of the winning **tempogram lag**, and
lags are integers: with the default `sr`/`hop` the reportable BPMs are
`60·(sr/hop)/k`, a grid **7.6 BPM wide at ~144 BPM**. Every tempo this analyzer
ever emitted sat on that grid (89.1, 107.7, 117.5, 123.0, 143.6 = lags 29, 24, 22,
21, 18).

`refine_tempo()` fits a line through the measured beat times instead — indices
recovered by accumulating each beat's own step, so a skipped beat shifts only
itself — and reads the tempo off the slope. Measured across 13 renders: 89.1 →
**90.00** (6 tracks), 107.7 → **108.00** (3), 123.0 → **121.00**, 117.5 →
**120.00**, 86.1 → **86.00**, 143.6 → **140.40**. That last one was placing bar 121
**4.6 seconds late** — over three bars — which every section `start_sec`, the
Composer's auto-nudge, the note-event rebase and the Ableton session tempo
inherited.

Three fields ride along: `tempo_coarse_bpm` (what the lag grid said),
`tempo_resid_ms` (the fit's residual σ) and `tempo_refined` (whether the fit was
trusted) — a beat-grid **quality** signal the app never had.

**The fit is gated (`RESID_GATE`, 2026-08-11).** Refining a badly-tracked grid is
worse than not refining it: on `loki` the coarse 107.7 was 0.3 BPM off Ableton's
ground-truth 108, and the "refinement" moved it to 109.10 — a real regression. The
residual separates the cases cleanly, judged as a fraction of one beat (absolute ms
would drift with tempo): every confirmed win sits at ≤1.8% of a beat, the confirmed
loss at 12.8%. Above 5%, `refine_tempo` keeps the coarse value and sets
`tempo_refined: false`.

**What the gate costs, stated plainly:** `dopamine-v1-final` — the 4.6 s-of-drift
track that motivated this whole change — has a residual of 24% of a beat and now
falls back to 143.6. Its drift is no longer silently "corrected" to an unverifiable
140.4; it is **flagged** instead. That is the honest outcome, because nobody ever
had ground truth for that track and a 24% residual means no single tempo describes
it. Actually fixing it needs better beat tracking (T2.1 / `beat_this`), not a better
line fit.

The 5% threshold is **provisional — calibrated on n=4** and the first thing the T4
harness should sweep; tracks between ~2% and ~12% are unmeasured. Erring low is the
safe direction, since falling back is exactly the pre-refinement behaviour.
`analysis/test_analyze.py` covers the gate, the skipped-beat path and the
degrade-never-sink cases (`make analyzertest`; no audio required). Nothing consumes
`tempo_resid_ms`/`tempo_refined` yet — routing them into the Reference Analyst's
`uncertain` list is the obvious next step.

**Analyses are snapshots**: fixes do NOT retroactively apply to songs already
imported. Re-run 🎼 Analyze → Composer on a render to restash a corrected one.

## Written parts: the take contract

One shape serves the Melodist (lead) and the Arranger (bass/pad/chords/arp):

```
{ "motif" | "idea": "…", "sections": [ { "label": "Verse 1",
    "notes": [ { "degree": 1-7, "octave": 0|1, "start": <16ths>, "length": <16ths> } ] } ] }
```

- **Degrees, not pitches** — in-key by construction, and the same vocabulary the
  Composer's lanes use, so a take is hand-editable after generation.
- `midi::clamp_part_take` is the CONTRACT, not a suggestion: label matching is
  **consume-once** (two "Chorus" rows get their own entries), degrees/octaves
  clamp, notes clip to the section's bars, and mono parts (lead/bass/arp) get
  overlap truncation while poly parts (pad/chords) keep their stacks.
- An **empty section means "sit out"** — both the full build and the single-track
  push honor it, so they can't contradict each other.
- Storage: `song_melody` (lead) and `song_part` (one row per part). Regenerating
  replaces that take only. `midi::part_take_render` holds each part's register
  and velocity band; `takes_for_sections` converts degrees → absolute MIDI.
- A written take also **overrides a profile-disabled part** (an explicit pad take
  beats a genre profile with pads off) and bypasses the section energy map.
- Per-section rewrite: `generate_song_melody(song, section)` sends the existing
  take as context, rewrites ONE section, and splices it back after re-reading
  the stored take (so a concurrent full regeneration can't be reverted).

## The transcription fallback ladder (lyrics)

Whisper is a speech model; sung and heavily-processed vocals can defeat it. The
analyzer works down a ladder and keeps the richest result:

1. **Vocals stem** (usually cleanest — but demucs misfiles heavily-processed synth
   vocals into "other", leaving a near-silent stem; verified on a real synthwave
   render where even whisper-medium hallucinated).
2. **Full mix** (when the stem yields < 3 segments).
3. **Hallucination guards** (garbage lyrics in sections are worse than none):
   stock instrumental outputs ("Music", "Thanks for watching") are dropped;
   a transcript that's one or two short phrases looping (≥5 segments, <⅓ unique)
   is discarded whole; a single stray segment is discarded.
4. **Music.AI cloud add-on** (opt-in, Settings → "Music.AI lyrics"): only offered by
   ⟳ Resume import when the local ladder heard nothing AND a key + workflow slug are
   configured. This is the ONLY path where audio leaves the machine, per-song and
   user-initiated. `core/src/musicai.rs`.
5. **📋 Paste lyrics** (always available in the resume banner): for your own Suno
   songs the words exist verbatim in Suno — pasting beats any transcription and goes
   through the same verbatim parser as the lyrics-first flow.

An import that can't fill Lyrics says so (⚠ progress line + "finished with issues"
summary) and leaves the stage pending — which is exactly what makes the ⟳ Resume
banner appear.

## Resume (`agent::resume_import`)

Idempotent completion of a partial import (MCP tool `resume_import`, Tauri command,
⟳ banner): rebuilds a missing Lyrics stage from the render's stashed transcript — or
a fresh analyzer pass, with bar-cumsum section times when the stash predates
`start_sec` — runs any missing Claude stages via reverse context, approves everything
with content (surfacing errors the original import swallowed), fills the intent, and
reports exactly what was fixed vs still failing. Survives the song being deleted
mid-resume. Failed Library imports keep the picked file for one-click ⟳ Retry.
(A hung Claude call can't stall an import: `call_claude` timeboxes and
kill-on-drops the CLI.)

## The melody/bass fold (Phase 2)

`frontend/src/lib/music/compose/transcription.ts` — the Composer's lanes are
diatonic (degree 1–7, octave band 0|1), so absolute-MIDI note events fold lossily
ON PURPOSE (an editable sketch in the vocabulary you compose in, not a piano roll):

- rebase onto `first_downbeat_sec` (bar 1 of the grid IS the downbeat, and the
  audio is nudged by the same amount — unrebased notes sat late against both);
- quantize to 16th ticks at the summary bpm;
- shift the whole lane by whole octaves so its median lands in the band;
- per note, pick the (degree, octave) whose resolved playback MIDI is nearest,
  preferring exact pitch-class matches (in-scale notes never land on a neighbor;
  chromatic passing notes snap);
- trim quantization overlaps to keep the lanes monophonic.

Verified on a real render: 936/642 events → 765/514 clean spans, zero overlaps,
bass degrees concentrated on 1/6/7 — the song's actual Am/F/G progression family.
Note transcription reads pitch energy, not words — it works even on songs whose
LYRICS are untranscribable.

## Section build-out (Phase 3)

Click a section band block in the Composer to FOCUS it: the loop confines to its
ticks, the render audio re-seeks to the matching slice (a backward jump, a forward
jump >8 ticks, OR a change of the focus `seekKey` — a small forward jump used to
slip past the heuristic), and with audio attached an A/B strip picks what you hear —
`🎵 original` (transport-only mute; edit previews stay audible), `🎹 mine`, or both.
`✓ mark done` persists on the composition (`Section.done`) and dims the block —
per-section bookkeeping while you rebuild. Unfocusing resets A/B so no mute lingers.

## Ableton specifics

- The Composer ⚡ export sends frontend-RESOLVED MIDI (the backend stays theory-free)
  as named tracks, then attaches the render as a **"Reference"** audio clip at bar 1
  on its own audio track (remote script patch v3 adds `create_audio_track`; the clip
  import can take up to 60s — the client widens its socket timeout around it, and any
  failure logs a ⚠ line without sinking the MIDI build).
- Song-level ⚡ builds skip profile-disabled parts entirely (pad/arp/drums off in the
  arrangement profile → no track at all — empty clips read as a broken export), and
  notes are resolved BEFORE `create_clip` so a partially-covering take can't leave
  blank clips behind.
- `build_take_track` is the ADDITIVE push (2026-08-03): it creates a NEW track for
  the take — `Lead`, then `Lead 2`, `Lead 3`, … — and touches nothing else: no tempo
  change, no rebuild, and **no deletion**. Takes accumulate so you can A/B them by
  soloing, and hand edits you made to an earlier take survive.
  It used to send `clear_named_tracks`, which **deletes** every track of that name —
  so each push silently threw away the previous take *and* any editing done to it in
  Live (user-hit, 2026-08-03). The free name is chosen by reading the set's existing
  track names back (`get_session_info` → `track_count`, then `get_track_info` per
  index) — deliberately only commands the shipped remote script already has, so this
  needs no Live restart. `free_take_track_name` matches case-insensitively and
  ignores padding, and reuses a gap (`Lead`+`Lead 3` → `Lead 2`).
  A full ⚡ Build still rebuilds its own `Lead`/`Bass`/… tracks; take tracks with a
  numeric suffix are left alone.
- Drums remain formula-only (GM pitches, not scale degrees — a different vocabulary
  from the take contract).
- Remote-script patches live at BOTH
  `~/Library/Preferences/Ableton/Live 12.4.x/User Remote Scripts/AbletonMCP/__init__.py`
  and need a Live restart (or control-surface toggle) to load.

## Where things live

| Piece | Home |
|---|---|
| Analyzer (stems/whisper/basic-pitch) | `analysis/analyze.py` (uv venv; `settings.analyzer_cmd`) |
| Import pipeline + resume | `core/src/agent.rs` (`import_reference_full`, `resume_import`) |
| Stashed analysis | `render.analysis` (JSON: bpm/key/sections+start_sec/melody/bass/transcript/first_downbeat_sec) |
| Music.AI client | `core/src/musicai.rs` (opt-in via `settings.musicai_api_key/_workflow`) |
| MIDI→lane fold | `frontend/src/lib/music/compose/transcription.ts` |
| Focus/A-B/done | `Sketchpad.tsx` + `useCompositionPlayback` (`range`/`mute` opts) + `SectionBand.tsx` |
| Ableton builders | `core/src/ableton.rs` (`build_song`, `build_take_track`, `build_midi_tracks` + reference audio) |
| Written parts | skills `melodist.md` / `arranger.md`; `agent::generate_song_melody` / `generate_song_part`; `midi::clamp_part_take` / `take_notes_abs`; tables `song_melody` / `song_part` |
| Takes → Composer lanes | `get_song_takes` + `applyTakesToComposition` (ComposerRoute) |
| A/B renders (v1 vs v2) | `frontend/src/components/RenderAB.tsx` |
| Setup doctor + bundled Live script | `run_doctor` / `install_ableton_script` (`app/src-tauri/resources/AbletonMCP_init.py`) |

## Test nets

| Layer | What it covers |
|---|---|
| `cargo test -p song_core` (128) | the contracts: bass root pitch class across all 144 root pairs, consume-once duplicate labels, Settings round-trip (every field, so a new setting can't ship dead), the take clamp, 8 whole-flow scenarios incl. 4 resume paths |
| `make flowcheck` | Tier B: a scratch song through all 6 stages with REAL Claude, then 29 deterministic coherence checks. ALWAYS via make (it rebuilds the shim first — a stale shim once produced false failures) |
| the audit workflow | `docs/`-adjacent history: two multi-agent passes (2026-07-28/29) whose findings were each adversarially verified by two independent skeptics before any fix landed |
