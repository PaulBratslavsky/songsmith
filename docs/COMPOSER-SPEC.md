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
  for lyric/chord alignment). Export-back-to-song is still NOT STARTED.
- **Phase 3 (persistence) — NOT STARTED.** libSQL `composition` table + mcp-shim tools. (Schema v3 +
  `parseStoredComposition`/`reidentify` carry the new fields, so the load seam is ready.)

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
3. **Export a composition FROM the Composer back into a song** (final feature) — push the composer's
   chords (and structure) into a song's Chords stage / a new song, so a sketch becomes a real song.
4. **Save & reopen compositions** — persist each Composition (libSQL `composition` table) with a name;
   list/open/delete them later. (music-kb does this via Strapi; Songsmith uses libSQL + mcp-shim tools.)

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

## Possible later add-on the user floated
Allow **creating/editing the composition via chat** (Claude over MCP rewrites the Composition JSON and
the grid live-reloads) — after the visual builder + import/export work.
