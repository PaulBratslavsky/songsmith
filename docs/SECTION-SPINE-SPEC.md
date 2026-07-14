# Section Spine — one source of truth for sections (spec, APPROVED 2026-07-13)

The last big copy-drift class (docs/SONG-FACTS.md deferred it here): sections are duplicated across
four stage artifacts and reconciled by label-matching back-fills. This spec normalizes them into a
song-level **spine**, the same move that fixed key/BPM — but sections carry per-stage content, so it
is a real refactor. SPEC-FIRST by agreement; implementation is its own session.

## Today's duplication (verified in code)
- structure `data.sections`: `{type?, label, bars, role, frozen?}` — owns form
- chords `data.sections`: `{label, feel?, chords:[{name,beats}], frozen?}` — UI calls chords "the
  section spine" (LyricsEditor: "Sections & order come from the Chords stage")
- lyric_spec `data.beats`: `{section, beat}` — keyed by section NAME
- lyrics `data.sections`: `{label, lines(ChordPro), frozen?}`
- Reconciliation by LABEL everywhere: freeze merge, import_lyrics back-fill, Composer export
  back-fill, deriveSections, ArrangementBuilder, Ableton song_sections. Renames = silent breakage;
  two stages can disagree on order; "who owns sections" is answered differently by different files.

## The model
New table `section`:
```
section { id TEXT PK, song_id TEXT, position INTEGER, label TEXT, type TEXT DEFAULT '',
          bars INTEGER DEFAULT 8, role TEXT DEFAULT '', created_at, updated_at }
```
- The SPINE owns **identity, order, label, type, bars, role** (the form).
- STAGES own their per-section **content**, keyed by `section_id` (not label):
  - chords: `{sections: [{section_id, feel?, chords:[…], frozen?}]}`
  - lyrics: `{sections: [{section_id, lines:[…], frozen?}]}`
  - lyric_spec: `{beats: [{section_id, beat}]}` (+ song-level fields unchanged)
  - structure: `data` keeps ONLY `keyNote/tempoNote` — its sections move INTO the spine entirely;
    the Structure stage becomes the spine's editor + the AI's proposal surface.
- 🔒 freeze stays **per stage per section** (a section's lyrics can be locked while its chords
  aren't) — the flag lives in the stage data entry, now keyed by section_id.

## Who may change the spine
- The **user**: anywhere sections are edited today (StructureEditor add/remove/reorder/rename/bars;
  SectionChordsEditor's add/remove/reorder become spine ops on the same commands).
- The **Structure stage AI run**: proposing sections IS its job — its output reconciles into the
  spine (see Reconciliation). Other stages' runs may NOT create/rename/reorder sections; unmatched
  labels in their output are attached by fuzzy label match or dropped with a visible warning in the
  artifact text (never silently invented).
- **Imports** (paste-lyrics, Composer export): create/update spine rows the way they back-fill
  structure today — user-authority paths.

## Reconciliation (model output → spine), the crux
Claude outputs sections by LABEL (models can't know ids). After a Structure run:
1. Match output sections to spine rows: exact label (case/space-insensitive), else same-position +
   same-type, else NEW row (append at output position).
2. Spine rows not present in the output: DELETED — but only if no stage artifact carries content for
   them; if content exists, keep the row and mark it in the artifact text ("(section dropped by the
   model — kept because Chords/Lyrics still reference it)"). Deletion of a content-bearing section is
   a user action only.
3. Frozen structure sections (form-level lock): keep row + its label/bars/role verbatim (the
   existing merge semantics, now against the spine).
Other stages' runs map output labels → section_id the same way but with create=NEVER.

## Consumers (every touch point, from the code audit)
- run_stage prompts: render the spine once (labels + bars + roles) as the canonical section list all
  stages see; per-stage artifacts render their content keyed through it.
- freeze merge: match by section_id (label fallback for legacy artifacts).
- import_lyrics / create_song_from_lyrics: create spine rows from parsed sections; chords-from-tags
  attaches by section_id.
- Composer: compositionFromSong reads spine (order/bars) + stage content; Composition.sections
  carries `section_id?` so export maps back losslessly; export updates spine bars (user action).
- Ableton builders: song_sections reads the spine directly (drops artifact parsing).
- ArrangementBuilder/deriveSections/SongSheet/editors: read spine + per-stage content by id.
- Revision history restore: an old artifact may reference deleted section_ids — restore re-creates
  missing spine rows from a snapshot embedded in the artifact (see Snapshots) so restores stay
  self-contained.

## Snapshots (keeps journaling honest)
Artifacts stay append-only and must remain meaningful if the spine changes later. Each artifact save
embeds a light `spine_snapshot: [{section_id, label, position}]` alongside `data`. Readers prefer the
live spine; restore uses the snapshot to re-create/re-order missing rows. This keeps the journal
self-contained without making the spine itself versioned.

## Migration (existing songs, one-time, transactional)
1. Build the spine from the STRUCTURE artifact's sections (form owner) in order; if a song has no
   structure artifact, fall back to the Chords artifact's sections; else lyrics'.
2. Union in sections that exist only in other stages (chords/lyrics/beats), appended in first-seen
   order (covers today's "Bridge only in lyrics" case).
3. Rewrite every current artifact's data: label → section_id (case/space-insensitive match), add the
   snapshot. LEGACY revisions are NOT rewritten — readers keep a label-fallback path (same tolerance
   pattern as song-facts).
4. bars/role/type: from structure entries where present, defaults elsewhere.

## Test plan (write these first)
- migration: 3-stage fixture with mismatched labels/case, lyrics-only Bridge, no-structure song.
- reconciliation: rename match, position match, new section, dropped-with-content kept, frozen kept.
- freeze by id: rename a section (spine) → its frozen chords still merge correctly (the label-match
  bug class dies).
- imports: paste-lyrics creates spine; chords-from-tags attach; Composer round-trip via section_id.
- restore: revert an artifact referencing a deleted section → row re-created from snapshot.
- legacy: pre-migration revisions render + diff + restore via label fallback.

## Phasing (one session, but committable checkpoints)
1. Table + models + migration + spine CRUD commands/tools + tests (no consumers switched).
   ✅ Phase 1 built 2026-07-14: `section` table + `Section` model, `db::migrate_sections`
   (runs on every open; per-song idempotent; rewrites current artifacts in place — ids +
   `spine_snapshot` added ADDITIVELY, labels kept, legacy revisions untouched), CRUD
   (`list/create/update/delete/reorder_sections`) as Tauri commands + MCP tools + mock parity.
   App behavior unchanged — no consumer reads the spine yet.
2. Readers switch (render/prompts/Ableton/frontend read spine; label fallback for legacy).
   ✅ Phase 2 built 2026-07-14: every reader prefers the spine, with the legacy label path
   byte-identical when a song has no rows. Core: run_stage/self_check prompts lead with a
   canonical "SECTIONS (canonical)" block (label · bars · role, spine order; "" without rows);
   `ableton::song_sections`/`song_parts` read the spine (chords content attached by section_id,
   exact-label fallback) — chat preamble + all Ableton builders inherit it; `lyrics_technical_brief`
   takes the spine for its section list/bars/roles (chord stats matched id-then-label). Frontend:
   `useSpineSections` (React Query ["sections", songId]) feeds SongSheet + ArrangementBuilder
   (spine-aware `deriveSections`), LyricsEditor (section identity/order from the spine; footer
   copy updated), StructureEditor (section LIST seeded from the spine, display only — save path
   untouched), LyricSpecEditor (beats shown in spine order, matched id-then-label), and
   compositionFromSong/ComposerRoute (section order/bars from the spine; `Composition.sections`
   carries additive `section_id?` for Phase 3's lossless export). Artifact zod schemas expose
   `section_id` additively; mock seeds Cyber Dreams spine rows (+ ids on its artifacts) for
   browser parity. Writers unchanged: label-keyed saves may lag the spine until Phase 3 (readers
   union unmatched artifact sections in, so nothing disappears). Freeze merge stays label-based.
3. Writers switch (editors, run_stage reconciliation, imports, Composer, freeze-by-id).
4. Cleanup: structure artifact drops sections; UI polish; docs.

## USER DECISIONS — ✅ RESOLVED 2026-07-13: user accepted all four recommendations
(D1 structure = spine editor · D2 keep+warn · D3 no section creation by non-structure runs ·
D4 bar changes shown in export dialog). Spec is ready for implementation as its own session.
D1. Structure stage = the spine editor (sections move out of its artifact entirely) — OK? The
    alternative (spine editable everywhere, structure keeps a copy) preserves today's feel but keeps
    a copy alive. RECOMMEND: yes, structure becomes the spine editor.
D2. When a Structure AI run drops a section that still has chords/lyrics content: keep it (spec'd
    above) or delete it with the content? RECOMMEND: keep + warn; deletion is user-only.
D3. May non-structure stage runs CREATE sections when their output invents a label (e.g. lyrics run
    adds an outro)? Spec says NO (drop + warn). RECOMMEND: no — form changes go through Structure or
    the user; keeps the spine stable.
D4. Composer export updating spine bars: silently, or shown in the export dialog as "bars will
    change: Verse 1 8→12"? RECOMMEND: show in dialog.
