# Build brief — the visual Composer (Phase 1 + 2: engine + working UI)

Read `docs/COMPOSER-SPEC.md` FIRST — it is the canonical definition. This brief implements **Phase 1
(engine port) + Phase 2 (visual builder UI)**. Persistence (Phase 3) and song import/export (Phase 4)
are explicitly OUT OF SCOPE here, but leave clean seams for them.

## What we're building
A Hookpad-style **visual melody + chords + bass sketchpad**: an 8-bar, scale-degree grid where the
user drags chord blocks + melody/bass notes and loops it through the Web Audio synth; changing the key
transposes everything for free. This is a NEW surface — NOT the existing chord editor.

## Source to port FROM (the reference impl)
`~/programing/music-kb` — port these (read each, adapt imports to Songsmith):
```
client/src/lib/music/compose/{types,spans,playback,useCompositionState,useCompositionPlayback,labels,colors,schema}.ts
client/src/lib/music/audio/synth.ts                 (per-voice additions only)
client/src/components/compose/{Composer,ChordLane,NoteLane,ChordPalette,BeatRuler}.tsx
client/src/components/compose/{useSpanDrag,laneLayout,chordHighlight}.ts
client/src/lib/music/compose/__tests__/*            (port the unit tests if a runner exists)
```

## Target placement (Songsmith) — avoid collisions, reuse the engine
- Port the compose lib into **`frontend/src/lib/music/compose/`** (Songsmith already has the music-kb
  theory engine in `frontend/src/lib/music/theory/` — REUSE its diatonic/degree/scale helpers:
  `theory/diatonic.ts`, `theory/degrees.ts`, `theory/chords.ts`, `theory/scales.ts`. Do NOT duplicate
  theory; wire `playback.ts`'s degree→MIDI + diatonic-triad resolution to those.)
- **NOTE — name collision:** `frontend/src/components/Composer.tsx` ALREADY EXISTS (the chord-section
  editor). Put the NEW visual builder under **`frontend/src/components/compose/`** (so
  `components/compose/Composer.tsx` is the new one). Do not touch the old `components/Composer.tsx`.
- **Synth:** Songsmith's synth is `frontend/src/music/synth.ts` (exports `playChord`, `playChordAt`,
  `playSequence`, `Step`). ADD music-kb's selectable **voices** (piano/string/bass + envelopes/lowpass)
  as an optional trailing arg, keeping all existing call sites working.
- **Route + nav:** add a `/composer` route (component `frontend/src/routes/ComposerRoute.tsx` rendering
  `components/compose/Composer`) and a sidebar **Composer** link in `main.tsx` (register like the other
  routes via `createRoute` + `addChildren`). Place it near Chord Builder.

## Phase 1 — engine (pure, tested)
Port `types.ts` (Composition model + `TICKS_PER_BEAT=4/BEATS_PER_BAR=4/BARS=8/TOTAL_TICKS=128` +
`SCHEMA_VERSION`), `spans.ts` (move/resize/add/remove + clamps + monophonic invariant), `playback.ts`
(degree→MIDI via Songsmith's theory engine, octave bands chords~3/melody~4/bass~2, triad resolve,
`buildSchedule`), `useCompositionState.ts` (reducer: comp + cursor + typed selection), `labels.ts`
(triad labels + chord-tone degrees), `colors.ts`. Keep `schema.ts`'s zod `CompositionSchema` +
`parseStoredComposition`/`reidentify` for later persistence, but NO Strapi/network code. Compile clean.

## Phase 2 — visual builder UI
Port `Composer.tsx` (top-level wiring + transport play/stop/loop + bpm + key picker + name field +
single CSS-calc playhead + Delete/Escape keyboard), `ChordLane.tsx`, `NoteLane.tsx` (melody + bass
piano-rolls, 7 degree rows, octave band), `ChordPalette.tsx` ("Chords in {key}" diatonic chips),
`BeatRuler.tsx`, `useSpanDrag.ts` (pointer-capture move/resize + vertical pitch remap w/ audition via
the synth), `laneLayout.ts`, `chordHighlight.ts`. Lanes stay `memo`'d + presentational. Wire
`useCompositionPlayback.ts` to Songsmith's synth with per-voice playback. Match Songsmith's dark
neon-green aesthetic (look at existing components/styles.css). Composition is in-memory for now
(start with a sensible blank/demo composition; persistence is Phase 3).

## Definition of done
- `cargo build` + `cargo test` green (paste the `test result:` lines) — this phase is frontend-only, so
  mainly ensure nothing breaks.
- `cd frontend && npm run build` succeeds (tsc --noEmit + vite). Fix all type errors from the port.
- If Songsmith's frontend has a test runner (check `frontend/package.json` for vitest/jest), port the
  compose `__tests__` (spans, playback, schema) and run them; otherwise rely on tsc + the visual test
  and note it.
- Extend `scripts/visual_test.py` to navigate to `/composer` and screenshot the builder (chord lane +
  melody + bass lanes + palette + transport). List the screenshot(s).
- Update `docs/COMPOSER-SPEC.md` status (mark Phase 1+2 done) and README (a short "Composer" line).
- Do NOT commit — the parent verifies and commits.

## Rules
- The Composer is degree-based + key-relative (transposition = re-render). Reuse the ported theory
  engine; don't reinvent. Don't touch the existing `components/Composer.tsx` (chord editor) or the
  `/builder` Chord Builder. Match Songsmith style. Keep everything compiling/green.
