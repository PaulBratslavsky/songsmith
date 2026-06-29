# Build brief — Composer: full-song export + lyric sync

Read `docs/COMPOSER-SPEC.md` FIRST (esp. the "DECISION (2026-06-29): Full-song timeline + lyrics"
block — it is the authority). This implements: **export a generated song into the Composer as one
long, scrollable, multi-section timeline with chords + lyrics synced**, plus melody/bass editable
across the whole song. Builds on the committed Phase 1+2 Composer (commit c9bf065).

## Outcome
From an open song, the user clicks **"Open in Composer"** → the Composer opens showing the WHOLE song:
sections laid end-to-end (labeled region band), each section's real chords on the chord lane with the
**lyric line under each chord**, melody + bass lanes spanning the full song (empty, editable), and
play/loop across the whole thing. Changing key still transposes (degree resolution preserved).

## Key change: lift the fixed-8-bar constraint
Today `types.ts` has constants `BARS=8`, `TOTAL_TICKS=128`. Make composition length **variable**:
- Add `bars` (or `totalTicks`) to the `Composition` model; derive `TOTAL_TICKS` from it everywhere
  (spans clamps, `useSpanDrag` pixel↔tick math, `laneLayout` columns, `BeatRuler`, the playhead
  calc, `buildSchedule`). Keep blank-sketch default = 8 bars (existing behavior unchanged).
- Add `sections: { id, name, startTick, lengthTicks }[]` to the `Composition` (empty for blank
  sketches). Render a **labeled section band** above the lanes, aligned to the same grid.
- Add optional `name?: string` to `ChordSpan`. When present (imported songs) the chord lane block
  shows that **real chord name**; when absent (blank sketch) it shows the diatonic degree label as now.
  Playback resolves a named chord to MIDI via the theory engine (parse the name → pitches), and a
  degree chord as before.
- Add an optional lyric overlay: `lyrics?: { tick, text }[]` (or carry `lyric?` on chord spans) — a
  row rendered under the chord lane, each line left-aligned at its tick.

## Building the export (song → Composition)
A pure builder `compositionFromSong(...)` (in `lib/music/compose/`):
- **Input:** the song's key (`key_root`/`key_mode`), the **Chords stage** artifact
  `data.sections: { label, feel?, chords: { name, beats }[] }[]`, and the **Lyrics** stage content.
- **Layout:** for each section in order, lay its chords by `beats → ticks` (beats × TICKS_PER_BEAT),
  accumulating `startTick`. Section `lengthTicks` = sum of its chords' beats in ticks. Build the
  `sections[]` band from the labels + computed spans. Total length = the sum (round up to whole bars).
- **Chords:** one `ChordSpan` per chord `{ start, length, name, degree }` — set `name` to the absolute
  chord name; also compute the diatonic `degree` in the song's key (reuse the ported theory engine;
  for out-of-key chords pick the nearest/duplicate-safe degree and rely on `name` for display).
- **Lyrics:** align lyric lines to chord starts. The song lands one chord at the start of each lyric
  line — pair each section's chords with that section's lyric lines (from the Lyrics artifact) in
  order; attach each lyric line's text at its chord's `startTick`. REUSE the Sheet-preview component's
  existing lyric/chord parsing/alignment logic (find it under `frontend/src` — the "Sheet preview"
  tab) rather than re-deriving ChordPro parsing.
- **Melody/bass:** empty.
- Run the result through the existing span validation so invariants hold.

## UI / wiring
- **Trigger:** add an **"Open in Composer"** button in `routes/SongWorkspace.tsx` (near the header /
  the Composer/Chords tab). It loads the built Composition into the `/composer` route. Simplest robust
  approach: pass the songId to `/composer` (route search param `?song=<id>`); `ComposerRoute` fetches
  the song's chords+lyrics, builds the Composition, and loads it via the reducer's `load`. Without the
  param, `/composer` stays the blank-sketch tool as today.
- **Composer view:** horizontal scroll for long songs; the section band + lyric row scroll in sync
  with the lanes (shared grid width). Keep transport/key/loop working across the full length. Show the
  song name. A way back to blank sketch ("New blank" already exists).
- Lanes stay `memo`'d/presentational; section band + lyric row are their own light components.

## Mock parity
`ipc/mockApi.ts`: ensure a mock song with chords (sections + named chords + beats) and lyrics exists so
`/composer?song=<id>` renders a believable full song in the browser/visual test.

## Definition of done
- `cargo build` + `cargo test` green (paste `test result:` lines).
- `cd frontend && npm run build` succeeds (fix all type errors from the model change — many files read
  TOTAL_TICKS; update them all).
- `scripts/visual_test.py`: screenshot `/composer?song=<mock id>` showing the full-song timeline —
  multiple labeled sections, named chords, the lyric row under the chords, melody+bass lanes. List it.
- Blank `/composer` (no song) still works exactly as before (8-bar sketch) — screenshot still valid.
- Update `docs/COMPOSER-SPEC.md` status (full-song export DONE) + README line.
- Do NOT commit — the parent verifies and commits.

## Rules
- Don't break the blank-sketch Composer (Phase 1+2) — variable length must default to 8 bars when no
  song. Don't touch the old `components/Composer.tsx` chord editor or `/builder`. Reuse the theory
  engine + the Sheet-preview alignment; don't duplicate. Degree resolution for transposition stays.
- Persistence (Phase 3) and export-back-to-song are OUT OF SCOPE — leave seams.
