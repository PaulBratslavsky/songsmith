# Songsmith Composer — spec (canonical, committed)

This file is the **source of truth** for what "the Composer" means in Songsmith, so it survives
across sessions / context resets. If a future session is unsure what the Composer is, read THIS.

## Status
- **Phase 1 (engine port) — DONE.** Ported to `frontend/src/lib/music/compose/`
  ({types,spans,playback,labels,colors,schema,useCompositionState,useCompositionPlayback}.ts).
  `playback.ts` wires degree→MIDI + diatonic-triad resolution to Songsmith's existing theory
  engine (`lib/music/theory/*`) — no theory duplicated. `synth.ts` (`frontend/src/music/synth.ts`)
  gained selectable voices (piano/string/bass) as a `synth` object; old call sites unchanged.
- **Phase 2 (visual builder UI) — DONE.** New surface under `frontend/src/components/compose/`
  ({Composer,ChordLane,NoteLane,ChordPalette,BeatRuler}.tsx + {useSpanDrag,laneLayout,chordHighlight}.ts),
  rebuilt with Songsmith's plain-CSS dark neon-green aesthetic (no Tailwind). Route `/composer`
  (`routes/ComposerRoute.tsx`) + sidebar **Composer** nav link. Composition is **in-memory** (seeded
  with a demo sketch); `schema.ts`'s zod + `parseStoredComposition`/`reidentify` are kept as the
  load seam but no Strapi/network code exists.
- **Phase 4 (song import — full-song export) — DONE (import direction).** "Open in Composer" in the
  song workspace loads the WHOLE song into `/composer?song=<id>` as one long, horizontally-scrollable,
  multi-section timeline: sections laid end-to-end (labeled `SectionBand`), each section's REAL chords
  on the chord lane (absolute name on the block, roman numeral as subtitle) with the lyric line under
  each chord (`LyricRow`), melody + bass empty/editable across the full song, play/loop across it all.
  Composition length is now **variable** (`bars`/`totalTicks` + `sections[]` + `lyrics[]` on the model;
  optional `name?` on `ChordSpan`); `TOTAL_TICKS` is derived per-composition everywhere (spans clamps,
  `useSpanDrag`, `laneLayout`, `BeatRuler`, playhead, `buildSchedule`). Blank `/composer` (no param) is
  unchanged — defaults to the 8-bar sketch. Builder: `lib/music/compose/compositionFromSong.ts` (pure;
  reuses the theory engine for degree resolution + named-chord MIDI, and `ArrangementBuilder.deriveSections`
  for lyric/chord alignment). Export-back-to-song shipped later — see Phase 5 below.
- **Phase 5 (export back to song) — ✅ DONE (2026-07-07).** The loop is now **bidirectional**:
  an **⤴ Export** button next to Save/Open opens a dialog with the resolved sections preview and
  two destinations — **update the linked song** (overwrite the Chords stage's sections + back-fill
  Structure the way `import_lyrics` does; 🔒 frozen chord/structure sections are listed in the
  dialog and SKIPPED — kept byte-identical via `merge_frozen_sections`, re-inserted if dropped)
  or **create a new song** (preset/title inputs mirroring `create_song_from_lyrics`; key/bpm from
  the composition; lyrics stay empty). Degree→absolute mapping is a pure frontend fn
  (`lib/music/compose/compositionToSong.ts`: `ChordSpan.name` wins, else the diatonic triad/seventh
  for the degree in the key via the theory engine's own labels; ticks→beats, sections grouped —
  a sketch with no sections becomes one "Sketch" section). The backend takes RESOLVED sections
  JSON and stays theory-free: `agent::export_composition_to_song` / `agent::create_song_from_composition`
  (+ matching Tauri commands) render text with the existing `chords_editor_text` /
  `structure_editor_text` renderers, save through the normal artifact conventions, and mark both
  stages done. Covered by four core tests incl. the frozen-skip and the sketch round-trip cases.
- **Phase 3 (persistence) — ✅ DONE (2026-07-07).** Compositions survive app restarts. libSQL
  `composition` table (`CompositionRow { id, name, song_id: Option, data: the Composition JSON blob,
  created_at, updated_at }`, ts-rs exported, + a light `CompositionMeta` for listings) with CRUD in
  `core/src/db.rs`: `list_compositions` (newest first, no blobs), `get_composition`,
  `save_composition(id: Option, …)` — `None` inserts (the frontend **adopts the minted row id** for
  later saves), `Some` updates in place and bumps `updated_at`; `data` must parse as JSON or the save
  is rejected — and `delete_composition`. Registered as four MCP tools in `tools.rs`
  (delete = destructive) with mock parity, plus four Tauri commands. Composer UI: **💾 Save** next to
  the name field (serializes the reducer state through `CompositionSchema`; dirty tracking via a
  serialized-snapshot baseline → "saved / unsaved changes / not saved yet"), **📂 Open** library
  panel (name, updated, "♪ song" badge when `song_id` is set, per-row × delete with confirm; opening
  loads via `parseStoredComposition` + the reducer's reidentifying `load`, confirm-if-dirty), and
  New-blank confirm-if-dirty. Full-song imports (`?song=`) save with `song_id` and round-trip
  sections/lyrics through the blob (visual-tested: save → reopen → section band + lyric sheet
  intact). Blank-sketch mode, full-song import, the lyric sheet, freeze, and lane interactions are
  unchanged.

## What it is
A **visual melody + chords + bass sketchpad** — a Hookpad-style 8-bar grid where you lay down a
chord progression, a melody, and a bassline visually (drag blocks/notes), and loop it back through
the in-app Web Audio synth. Everything is in **scale degrees relative to a key**, so changing the
key transposes the whole piece for free. It is a **sketch tool, not a DAW**.

It is **NOT**: the per-section chord editor on the song workspace's "Builder / manage" tab, and
**NOT** the standalone "Chord Builder" (`/builder`) route. Those edit absolute chord names/voicings.
The Composer is the degree-based melody/chords/bass visual builder described here.

## Reference implementation to port FROM (the "yt music base" = music-kb)
`~/programing/music-kb` — read `docs/composer.md` (full architecture) then port:

```
client/src/lib/music/compose/
  types.ts                 model + tick/degree constants + SCHEMA_VERSION
  spans.ts                 generic TimeSpan ops (move/resize/add/clamp); monophonic invariant
  schema.ts                zod schema + parseStoredComposition (load validation/migration)
  playback.ts              degree→MIDI resolution + buildSchedule (triads, octave bands)
  useCompositionPlayback.ts setTimeout tick clock → synth
  useCompositionState.ts   reducer (comp + cursor + selection)
  labels.ts                triad labels + chord-tone degrees
  colors.ts                per-degree colors + hexToRgba
client/src/lib/music/audio/synth.ts   per-voice Web Audio synth (piano/string/bass voices)
client/src/components/compose/
  Composer.tsx             top-level wiring + transport + save/load + playhead + keyboard
  ChordLane.tsx            chord block lane (memo, presentational)
  NoteLane.tsx             melody/bass piano-roll lane (memo, presentational)
  ChordPalette.tsx         "Chords in {key}" diatonic chips
  BeatRuler.tsx            bar-number header
  useSpanDrag.ts           shared pointer-drag (move/resize/pitch)
  laneLayout.ts            shared grid geometry (LABEL_W, columns)
  chordHighlight.ts        ChordToneHighlight type
```

## Data model (port verbatim, then adapt persistence)
```ts
const TICKS_PER_BEAT = 4, BEATS_PER_BAR = 4, BARS = 8, TOTAL_TICKS = 128;
type TimeSpan  = { id; start; length };          // ticks
type ChordSpan = TimeSpan & { degree: 1..7 };
type NoteSpan  = TimeSpan & { degree: 1..7; octave: 0|1 };
type Composition = {
  id; version; name;
  key: { root: PitchClass; mode: 'major'|'minor' };
  bpm;                       // 60–180
  chords: ChordSpan[]; melody: NoteSpan[]; bass: NoteSpan[];   // each non-overlapping, sorted
};
```

## UI (stacked lanes over one shared 128-col grid, aligned by a LABEL_W gutter)
```
BeatRuler        bar numbers 1–8
NoteLane melody  7 degree rows (piano-roll), octave band 0|1
ChordLane        variable-length chord blocks (diatonic triads)
NoteLane bass    7 degree rows
```
- Reducer-owned state (`comp` + `cursor` + typed `selected {kind,id}`); lanes are `memo`'d &
  presentational. One shared `useSpanDrag` (pointer-capture; vertical drag remaps degree w/ audition).
- Single CSS-`calc()` playhead in Composer (lanes don't track tick). Chord selection shades its triad
  degrees in the melody grid. `Delete`/`Backspace` removes selection, `Escape` deselects.
- ChordPalette = diatonic chips for the current key. Transport: play/stop/loop + bpm.

## Songsmith integration (differs from music-kb's Strapi)
- **Persistence:** music-kb saves to a Strapi `composition` collection. In Songsmith, store the
  `Composition` JSON in the **libSQL** core (a `composition` table / artifact), mirroring Songsmith's
  existing CRUD + mcp-shim tool pattern — NOT Strapi.
- **Where it lives:** a real **Composer** surface (its own tab in the song workspace and/or a top-level
  route), distinct from "Builder / manage" and "Chord Builder". Add the nav entry actually labeled
  **Composer**.
- **Synth:** Songsmith already has a Web Audio synth (`frontend/src/music/synth.ts`); add the
  per-voice (piano/string/bass) selection from music-kb's synth.
- **Engine reuse:** Songsmith already ported music-kb's `lib/music` theory engine — reuse its
  diatonic/degree helpers; don't duplicate.

## Known constraints (deliberate; from music-kb)
8 bars / 4/4 fixed; monophonic melody+bass; triad chords (7th dropped at resolve); one octave band per
note; `setTimeout` clock (not lookahead). Extension points documented in `music-kb/docs/composer.md` §9.

## Flexibility / import-export (REQUIRED — user, 2026-06-26)
The Composer is a flexible song-structuring hub, bidirectional with the song's chords:
1. **Import a song's generated chords INTO the Composer** — take the chords produced in a song's
   Chords stage and load them onto the Composer's chord lane (map absolute chord names → diatonic
   degrees in the song's key), so the user can then sketch melody + bass over them.
2. **Create a new song directly in the Composer** — start a blank composition with no song attached,
   for free-form idea structuring (then optionally turn it into / attach it to a song).
   **✅ DONE (2026-07-07)** — ⤴ Export → "Create a new song" works for any composition, blank
   sketches included (preset/title inputs; key/bpm from the composition; see Phase 5 above).
3. **Export a composition FROM the Composer back into a song** (final feature) — push the composer's
   chords (and structure) into a song's Chords stage / a new song, so a sketch becomes a real song.
   **✅ DONE (2026-07-07)** — ⤴ Export → "Update the linked song" overwrites Chords + back-fills
   Structure (🔒 frozen sections skipped + surfaced, never silently overwritten; see Phase 5 above).
4. **Save & reopen compositions** — persist each Composition (libSQL `composition` table) with a name;
   list/open/delete them later. (music-kb does this via Strapi; Songsmith uses libSQL + mcp-shim tools.)
   **✅ Shipped with Phase 3 (2026-07-07)** — see the status entry above.

Degree↔absolute mapping is the crux of #1/#3: Composer is degree-based, the song's Chords stage is
absolute chord names — convert through the song's key (reuse the ported `lib/music` theory engine).

### Full-song export + lyric sync (user, 2026-06-29) — NEEDS the 8-bar constraint lifted
The user wants: after generating a song with the regular flow, **export the whole song into the
Composer to see the FULL song** (all sections in sequence), with **chords synced to the lyrics**.
This conflicts with music-kb's load-bearing "fixed 8 bars" constraint — a full song is multi-section
and variable length. So full-song export requires **lifting meter/length into the Composition** (or a
multi-section model) AND **lyric alignment** (the song's Chords stage already lands one chord at the
start of each lyric line; the Sheet preview shows lyrics + inline chords).

**DECISION (2026-06-29): Full-song timeline + lyrics.** Lift the fixed-8-bar limit; lay every
section end-to-end on one long, horizontally-scrollable timeline. Design:
- **Variable length:** `TOTAL_TICKS`/`BARS` become per-composition (derived), not constants. Grid,
  spans, clamps, playhead, and ruler all read the composition's length.
- **Sections:** add `sections: { id, name, startTick, lengthTicks }[]` rendered as a labeled region
  band above the lanes. Source = the Chords stage artifact `data.sections` ({label, chords:{name,beats}}).
- **Chord layout:** lay each section's chords by `beats → ticks` (beats × TICKS_PER_BEAT) end-to-end.
  In full-song (imported) mode the chord lane shows the **real chord NAME** (absolute, from the song) —
  add an optional `name?: string` to `ChordSpan` for imported chords (blank-sketch mode stays
  degree-based). Still resolve to MIDI for playback via the theory engine.
- **Lyric sync:** a lyric row under the chord lane, lyric lines aligned to chord start ticks (the song
  lands one chord at the start of each lyric line). Reuse the Sheet-preview's lyric/chord alignment.
- **Melody/bass:** empty on export, editable across the whole timeline.
- **Trigger:** an "Open in Composer" / "Export to Composer" action in the song workspace → loads that
  song as a full-song Composition into `/composer`.
- This is Phase 4 (import direction) + the full-song extension; export-back-to-song + save (Phase 3)
  follow.

## Lyric display refinement (user, 2026-06-29) — DONE (2026-07-07)
The current full-song lyric row (lines pinned at each chord's tick) cascades right and reads badly.
Replace it with a clean **ChordPro lyric sheet below the timeline** — grouped by section, chord-name
above the word it lands on, left-aligned — **reusing the existing Sheet / Lyric-Spec ChordPro
renderer** (don't re-implement). Keep the timeline (section band + chord/melody/bass lanes) above as the
editing surface; the sheet below is the readable view.
- **Two-way highlight sync:** selecting a chord block (or the playhead passing it) highlights its
  **section** in the sheet AND the **exact active chord/word** (section tint + brighter mark on the
  current chord); hovering/clicking a chord in the sheet highlights the matching timeline block. During
  playback the highlight follows the playhead through the lyrics.
- Build this AFTER the in-flight Freeze feature lands (avoid file conflicts in compose/ components).
- **Shipped 2026-07-07:** `compose/LyricSheet.tsx` (reuses the `.cp-*` ChordPro styles; word-level
  chord anchors threaded through `LyricLine.words` in compositionFromSong from the Lyrics stage's
  `[chord]` tags, parsed by the LyricsEditor's own `parseChordProLine`). LyricRow removed from the
  lane stack. Two-way sync: chord select / sheet-chord click share the ChordLane selection state;
  playback exposes `activeChordId` (set only when the chord under the playhead CHANGES, never per
  tick) so the memo'd sheet re-renders per chord; gentle `scrollIntoView(nearest)` auto-scroll.
  Same build replaced the per-tick lane button grids with CSS-gradient gridlines + one pointer hit
  surface per lane (audit Tier-2 #12).

## Queued: prominent "Open in Composer" on song completion (user, 2026-07-07) — ✅ DONE (2026-07-07)
The header "🎹 Open in Composer" button exists (always visible). ADD, after the current queue
(paste-lyrics → Phase 3 persistence → export-back-to-song): when a song is **marked done**, surface
the Composer path prominently — a clear CTA in the done state of the song workspace (e.g. next to
the "Reopen" control / in the done confirmation moment) and on done songs' Library rows, so opening
the finished song on the full timeline is the natural next step. Small UI affordance; no new
backend.
- **Shipped 2026-07-07:** when `song.status === "done"` the header "🎹 Open in Composer" button
  becomes **primary-accented** (with a done-specific tooltip), and done songs' Library rows get a
  small **🎹** affordance that opens `/composer?song=<id>` (row click still opens the song). No new
  backend; visual-tested (`song-done-cta.png` + a Library-row assertion in `scripts/visual_test.py`).

## Lyric sheet v2 (user test-drive feedback, 2026-07-07) — ✅ DONE (2026-07-07)
The v1 sheet (all sections stacked vertically) is wrong. Requirements:
1. **Show ONE section at a time** — the selected chord's section, else the playing section (follows
   the playhead), else the first. Compact section chips (or prev/next) to browse the others.
2. **Lines flow LEFT-TO-RIGHT** — a section's lyric lines flow inline (wrapping like text, each line
   an inline chunk), chords still printed above the exact words — NOT one line per row stacked down.
3. **Fix play-along line tracking (bug found in test drive):** when a section has more lyric lines
   than chords, `compositionFromSong` anchored every overflow line at the LAST chord's tick, so
   playback "skipped the second half"; and the sheet marked the active chord BY NAME, so repeated
   chords (Dm in both halves) lit both halves at once. Fix: (a) distribute a section's lines evenly
   across the section's tick span when lines ≥ chords (unique, increasing anchors); (b) the sheet
   highlights the ACTIVE LINE — the line whose anchor range contains the playhead — and only that
   line's chord occurrence, never name-matched duplicates.
- **Shipped 2026-07-07:** `compositionFromSong` distributes anchors evenly
  (`sectionStart + round(i * lengthTicks / lines.length)`) when lines ≥ chords, keeps chord-start
  pairing otherwise. `useCompositionPlayback` now also derives `activeLineTick` (greatest lyric
  anchor ≤ playhead, binary search) with the same only-set-on-change discipline as `activeChordId`
  — state changes on line/chord boundaries only, never per tick. `LyricSheet` shows ONE section
  (selected chord's, else the playing one, else the first) with clickable section chips (a chip
  choice holds until the followed section changes); lines flow left-to-right as inline wrapping
  chunks (chords still above their exact words); playback tints the active line and marks only that
  line's chord occurrence by span id; selection still marks the exact occurrence via the in-order
  span mapping. Auto-scroll follows the active line (`block:'nearest'`). Visual-tested:
  `composer-sheet-v2.png` + one-section/chips assertions in `scripts/visual_test.py`.

## Lyric sheet v3 — chords laid from LYRIC PLACEMENTS (user test-drive, 2026-07-07) — ✅ DONE (2026-07-07)
v2 still desyncs when a section's lyrics cycle the progression more times than the Chords stage
lists it (e.g. Chorus: 4 progression chords but 8 ChordPro placements across the lines) — 8 sheet
occurrences can't map onto 4 timeline blocks; duplicates light again. FIX: in `compositionFromSong`,
lay each section's chord spans from the **lyric ChordPro placements in order** (the ground truth of
the sung song): walk the section's lines' tagged chords; each placement becomes ONE span (beats taken
by cycling the section's progression entries by position, default 4 when unknown); section length =
the sum; each line anchors exactly at its first placement's span start (lines with no tags anchor
between neighbors). Sections with NO tagged placements (instrumentals / no lyrics) keep the current
progression-once layout. Result: timeline blocks == sheet occurrences 1:1 (sync exact by
construction — selection and play-along can never light a twin), and the timeline honestly shows the
sung song (Chorus = the progression twice if that's how it's sung).
- **Shipped 2026-07-07:** `compositionFromSong` walks each section's lines' ChordPro tags (the
  word-level `words` model) — every placement becomes ONE span (name = the tag, degree as before),
  beats cycling the section's Chords-stage progression **by position** (`progression[i % n].beats`,
  4 when it's empty); section length = the sum; a tagged line anchors exactly at its first
  placement's span start, untagged lines interpolate between their neighbours' anchors (a `pushLine`
  guard keeps anchors unique + strictly increasing for the sheet keys and the active-line binary
  search). Untagged sections (instrumentals / chordless) keep the progression-once layout.
  `LyricSheet.buildSheetModel`'s claim is now purely POSITIONAL (occurrence i ↔ the section's i-th
  span; the name-based fallback that lit twins is gone — tags past the spans of pre-v3 rows stay
  unlinked/disabled). Playback (`activeChordId`/`activeLineTick`) and export
  (`resolveCompositionSections`, which groups spans by section tick range — a re-export now writes
  the sung layout) needed no changes. Mock "Cyber Dreams" now lists each progression once with
  tagged lyrics cycling it (Chorus 1: 4 chords, 8 placements → 8 blocks); visual-tested:
  `composer-sheet-v3-chorus.png` + spans==occurrences / unique-span-link / 28-block assertions in
  `scripts/visual_test.py`. Saved pre-v3 compositions still load unchanged (schema untouched).

## Notation view (user, 2026-07-13) — REQUIRED, phased
The Composer should offer a SHEET-MUSIC representation in the spirit of MuseScore / Dorico SE
(user-cited: alto clef support, MIDI keyboard input, realistic playback). Honest scoping — three
independent phases, smallest-first:
- **N1 — Staff notation view (frontend-only, VexFlow MIT):** — ✅ DONE (2026-07-13) — a
  "𝄞 Notation" toggle rendering the composition as engraved staves — melody (treble), bass (bass
  clef), chords as symbols above the melody staff (optionally as a third staff of stacked notes);
  clef selector per staff incl. ALTO; key signature from the composition key, 4/4 bars from the
  tick grid, ties across bars, playhead cursor follows playback. Read-only in N1 (the piano-roll
  stays the editor). Degrees→pitches via the existing theory engine; ticks→note values (16th
  resolution) with dotted/tied handling.
  - **Shipped 2026-07-13:** `vexflow` 5.0.0 (MIT, `vexflow/bravura` entry, lazy-loaded so the
    engraving fonts only fetch on first toggle). Transport gets a "▦ Grid / 𝄞 Notation" pair; the
    grid is untouched and stays the editor. `components/compose/NotationView.tsx` renders systems
    of two staves (melody + bass, per-staff clef selects with treble/alto/tenor/bass) that wrap
    responsively (~4 bars per system, ResizeObserver), section labels in accent above the bar where
    a section starts, chord SYMBOLS above the melody staff (`ChordSpan.name` else the diatonic
    triad/seventh label), key signature via a root+mode → VexFlow spec map (minor specs like "Am"
    render the relative-major signature), 4/4 from the tick grid. All tick→engraving math is pure in
    `lib/music/compose/notation.ts`: lanes flatten to gap-free per-bar cells (RESTS fill gaps,
    spans split at bar lines with TIES — cross-system ties draw as two half-ties), greedy
    16th-grid decomposition into plain+dotted values (7→q.+16 etc.), and MIDI spelling with the
    key's enharmonics (Bb in F major, E# in F# major) — degrees→MIDI reuses playback's
    `resolveMelodyMidi`/`resolveBassMidi` (same octave bands the synth sounds). Playback highlights
    the ACTIVE BAR via an overlay div moved between measure rects captured at draw time
    (`activeBar = floor(tick/16)` on the memo'd component — bar-boundary re-renders only, VexFlow
    never redraws per tick; full SVG redraw on comp/key/clef/width change only). Dark theme via
    context ink fill/stroke + accent highlight. Lyric sheet stays below in both views. Triplets
    don't exist on the 16th grid, so no tuplet handling; blank sketches render their 8 bars.
- **N2 — Realistic playback:** — ✅ DONE (2026-07-13) — soundfont-based voices (WebAudioFont or
  soundfont-player, local assets — no network dependency) behind the existing `synth` interface as
  selectable "Piano (sampled)/Strings/Bass" options; the oscillator voices stay as fallback. Timing
  note: consider the AudioContext-lookahead scheduler upgrade here (known setTimeout drift).
  - **Shipped 2026-07-13:** sampled voices + the lookahead-scheduler timing upgrade, frontend-only.
    Assets: three FluidR3_GM instruments vendored from **gleitz/midi-js-soundfonts (MIT; the
    underlying FluidR3_GM soundfont by Frank Wen is also MIT)** — acoustic_grand_piano /
    string_ensemble_1 / acoustic_bass, trimmed to every 3rd semitone in each lane's octave band
    (one velocity layer, MP3 — WKWebView won't decode OGG) as
    `frontend/src/assets/soundfonts/{piano,strings,bass}.json` (~1.4 MB total; license +
    regeneration notes in the README beside them). The JSONs are dynamic-imported (own lazy Vite
    chunks, like NotationView's vexflow — main bundle +3 KB only) and decoded once by
    `music/soundfonts.ts`; playback picks the nearest sample and repitches via playbackRate
    (≤1 semitone). `synth` grew voices `piano-sampled`/`strings-sampled`/`bass-sampled` riding the
    same playNote/playChord API and master gain (mute works) — while chunks load, or if decode
    fails, each falls back to its oscillator sibling so playback never goes silent; old call sites
    (Chord Builder, SectionChordsEditor, Circle of Fifths, ChordPalette) are untouched oscillator
    paths. The transport's Sound control is now mute + a **Synth / Sampled** picker (per-lane
    mapping stays melody=piano, chords=strings, bass=bass; picking Sampled preloads; lane previews
    follow the picker; switching mid-play applies on the next scheduled tick). Timing:
    useCompositionPlayback's setTimeout tick clock became an AudioContext LOOKAHEAD scheduler — a
    25 ms timer schedules notes ~100 ms ahead at exact audio-clock times (playNote/playChord take
    an optional `at` and return a cancel fn); the same timer maps audio time → tick and moves the
    cursor once per boundary, so the exposed API (isPlaying/currentStep/activeChordId/
    activeLineTick/loop) and the per-tick / per-chord / per-line re-render discipline are unchanged
    (playhead CSS-calc, lyric-sheet highlight, notation active bar all work as before).
    Schedule/tempo/voices are still read through refs, so mid-play edits and tempo nudges apply
    from the next scheduled tick without resetting the cursor; on stop, sounding notes ring out
    (old behavior) but not-yet-started lookahead notes are cancelled. With "Synth" selected the
    audio path is the original oscillator code.
- **N3 — MIDI keyboard input:** Tauri's WKWebView has NO Web MIDI — requires a native bridge:
  `midir` crate in the Rust core streaming note events over a Tauri channel; frontend maps notes to
  the cursor position/duration for step entry into melody/bass lanes. Device picker in the transport.
Order N1 → N2 → N3; each is independently shippable. N1 first (biggest visible value, zero native
risk).

## Possible later add-on the user floated## Possible later add-on the user floated
Allow **creating/editing the composition via chat** (Claude over MCP rewrites the Composition JSON and
the grid live-reloads) — after the visual builder + import/export work.
