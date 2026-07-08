# Build brief — backlog batch: frontend (work in THIS order, keep tsc green between items)

Authorities: `docs/BACKLOG.md` + `docs/AUDIT-2026-07-06.md` (Tier 2/3) + `docs/COMPOSER-SPEC.md`.
Latest commit 24aadfc. ⚠ The user is LIVE-TESTING the dev app: do NOT kill anything on port 5173,
do NOT run scripts/visual_test.py, do NOT restart the app (hot reload is fine). Do NOT touch
core/**, app/src-tauri/**, or `mockApi.ts`'s MOCK_TOOLS list (a parallel Rust agent owns those; you
MAY edit the rest of mockApi.ts).

## 1. zod artifact schemas — kill the `any` epicenter (audit Tier-2 #8)
- New `frontend/src/lib/artifacts.ts`: zod schemas per artifact kind (ChordsData {sections:[{label,
  type?, feel?, frozen?, chords:[{name,beats}]}]} — tolerate legacy string chords via coercion;
  LyricsData {sections:[{label, frozen?, lines:string[]}]}; StructureData; LyricSpecData; PromptData
  {stylePrompt,taggedLyrics,notes}) + ONE `parseArtifact(kind, content)` returning a typed result
  (lenient: coerce the known legacy shapes the 5 ad-hoc parsers handle today, null on garbage).
- Replace the five ad-hoc `any` parsers (`artifactData` SongWorkspace.tsx:21, `dataOf` in
  SongSheet/ArrangementBuilder/ComposerRoute, `parse` LyricsEditor.tsx:58) and the FOUR copies of
  chord-shape coercion (Composer.tsx, LyricsEditor, ArrangementBuilder, compositionFromSong) with it.
  Behavior identical for valid data; garbage degrades the same as today.

## 2. ChordPro consolidation (audit Tier-3 #16, partial)
- New `frontend/src/lib/music/chordpro.ts`: `parseLine` (move LyricsEditor's exported
  parseChordProLine here; keep a re-export for compat), `toLine` (lineToChordPro), `stripTags`,
  `extractTags`, `spreadChords` (ONE auto-place implementation — unify LyricsEditor.autoPlaceSection
  and ArrangementBuilder's placeChordsOnLine/proportional spread; keep the LyricsEditor behavior as
  canonical since the user sees it). Point all 7 call-site files at it; delete the duplicates.

## 3. The Composer/Builder rename (audit Tier-3 #14) — mechanical
- `components/Composer.tsx` → `components/SectionChordsEditor.tsx`, export `SectionChordsEditor`
  (update SongWorkspace + ArrangementBuilder imports; the "Builder / manage" TAB LABEL becomes
  "Arrange"). `components/compose/Composer.tsx` → `components/compose/Sketchpad.tsx`, export
  `Sketchpad` (route `/composer` + user-facing "Composer" title UNCHANGED — only code names change;
  update ComposerRoute + visual_test selectors if any target component text that changes). Remove
  the apologizing NOTE header comments. Grep for stragglers.

## 4. Lyric-only sections appear on the Composer timeline (BACKLOG item — decision: INCLUDE them)
- `compositionFromSong.ts`: sections that exist only in the lyrics (deriveSections yields them;
  Chords stage doesn't) currently vanish from the timeline. Include them: a section band with NO
  chord spans, length = max(1 bar, enough ticks for its lines at one line per bar), its lyric lines
  anchored evenly across it. The sheet already shows chips per section — verify such a section's
  chip renders and its lines show (chordless). Playback simply has no chords there (fine).

## 5. Dead code sweep (audit Tier-3 #15 — frontend portion only)
- Delete never-imported: `lib/music/theory/positions.ts`, `roman-analysis.ts`, `chord-scales.ts`,
  `degrees.ts`, `quality-labels.ts`, `voicings/push.ts`, `lib/music/instruments/*/layout.ts` —
  RE-VERIFY zero importers first (grep each; if anything now imports one, keep it and say so).
  Also: `.badge.published` + `.field-chat`/`.field-chat-pop` dead CSS; add the missing `cp-palette`
  rule or drop the className; fix the stale "YT Creator Studio" styles.css header comment and
  ChatPanel's leftover YT empty-state copy (make it songwriting-flavored).

## 6. CommandMap api/mock parity (audit Tier-2 #7) — LAST (touches everything)
- In `ipc/api.ts`: a `type CommandMap = { [command]: { args: ...; result: ... } }` covering every
  command `api.*` dispatches; type `call<C extends keyof CommandMap>` accordingly. In `mockApi.ts`:
  implement the mock as a `const handlers: { [C in keyof CommandMap]: (a: Args<C>) => Result<C> }`
  (or equivalent) so a missing/mistyped handler is a COMPILE error; keep the existing behaviors.
  Fix the known drift: the `analyze_reference` mock orphan (add an api wrapper or delete the case —
  check if anything calls it), and any handler the map reveals as missing. `Artifact.version`
  bigint-vs-number mock mismatch: make the mock emit what the generated type says.

## Definition of done
- After EACH item: `cd frontend && npx tsc --noEmit` clean; after the last: `npm run build` green.
- `cargo test` untouched-green at the end (you shouldn't have changed Rust; just confirm).
- Update `docs/BACKLOG.md` + audit annotations for items done. Do NOT commit — the parent verifies
  and commits (including running the visual test once, after both agents land).
- Report per-item: files, results, anything deferred or found-still-imported.
