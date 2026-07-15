# Section Spine — one source of truth for sections (spec, APPROVED 2026-07-13)

STATUS: COMPLETE — all four phases built 2026-07-14; the spine is the only section authority.

The last big copy-drift class (docs/SONG-FACTS.md deferred it here): sections WERE duplicated across
four stage artifacts and reconciled by label-matching back-fills. This spec normalized them into a
song-level **spine**, the same move that fixed key/BPM — but sections carry per-stage content, so it
was a real refactor. SPEC-FIRST by agreement; implementation was its own session.

## The duplication this removed (verified in code, pre-spine)
- structure `data.sections`: `{type?, label, bars, role, frozen?}` — owned form (now: the artifact
  keeps only `{keyNote, tempoNote}`; the spine IS the sections)
- chords `data.sections`: `{label, feel?, chords:[{name,beats}], frozen?}` — the UI called chords
  "the section spine" (LyricsEditor: "Sections & order come from the Chords stage")
- lyric_spec `data.beats`: `{section, beat}` — keyed by section NAME
- lyrics `data.sections`: `{label, lines(ChordPro), frozen?}`
- Reconciliation was by LABEL everywhere: freeze merge, import_lyrics back-fill, Composer export
  back-fill, deriveSections, ArrangementBuilder, Ableton song_sections. Renames = silent breakage;
  two stages could disagree on order; "who owns sections" was answered differently by different
  files. All of that now goes through `section_id` (labels remain in content-stage data purely as
  human-readable text + legacy-revision tolerance).

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
   ✅ Phase 3 built 2026-07-14: every writer lands on the spine; labels stay in artifact
   data ALONGSIDE `section_id` (removal is Phase 4). Core (`core/src/spine.rs`):
   `build_run_content` replaces the run_stage/self_check save pipeline — a STRUCTURE run
   reconciles into the spine exactly per §Reconciliation (id → exact norm-label →
   same-position+same-NON-EMPTY-type → create at output position; missing rows deleted
   only when content-free, else kept with a ⚠ line in the artifact text — D2; frozen
   sections keep row+entry verbatim; a fresh song's first structure run births the
   spine); NON-structure runs map labels → section_id with create=NEVER (D3, unmatched
   output dropped + ⚠ line); `freeze::merge_frozen_sections` (and through it the MCP
   save/revert guards) matches by section_id FIRST, label fallback — a spine rename no
   longer detaches or duplicates frozen content (the marquee test). Imports
   (`apply_parsed_lyrics`) and the Composer export/create (`sync_spine`) are
   user-authority spine REPLACEs (matched rows keep ids/bars/role; export bars WIN once
   a spine exists — D4); every Phase-3 core write embeds a `spine_snapshot`. Frontend:
   StructureEditor IS the spine editor (D1 — save diffs into create/update/delete/
   reorder_sections, then mirrors into the artifact with ids); SectionChordsEditor's
   add/remove/rename/reorder are the same spine ops (form values kept from the row);
   LyricsEditor/LyricSpecEditor saves carry section_id through; ExportDialog shows
   "Verse 1: 8 → 12 bars" before confirm (D4); mock mirrors sync/attach for import +
   export + id-first frozen merge. Songs with no spine rows keep every legacy path
   byte-identical (all pre-Phase-3 tests unchanged; 93 core + 2 app green). Deferred to
   Phase 4: dropping labels/sections from structure data, snapshot-based restore,
   editor saves embedding snapshots, MCP save_artifact label→id normalization.
4. Cleanup: structure artifact drops sections; UI polish; docs.
   ✅ Phase 4 built 2026-07-14 — the spine is the only section authority end to end:
   - STRUCTURE ARTIFACT DROPS ITS SECTION COPY: every structure writer (StructureEditor
     save, `spine::build_run_content` structure runs, `apply_parsed_lyrics`, Composer
     export/create, `import_reference`, the guarded MCP save) stores only
     `{keyNote, tempoNote}` (+ `spine_snapshot`); the text's SECTION MAP renders FROM
     THE SPINE (`render::structure_spine_text`; notes-only for bare data). Legacy
     revisions that still embed sections render/diff unchanged (readers keep the label
     fallback; `structure_editor_text` still renders embedded sections). The structure
     per-section 🔒 (form-lock) UI is retired with the copy — the user owns the form in
     the spine editor; frozen entries in LEGACY priors are still honored verbatim by
     run reconciliation (tolerance test kept).
   - SNAPSHOT-BASED RESTORE (§Snapshots): `spine::restore_snapshot_rows` re-creates
     missing rows from an artifact's `spine_snapshot` (SAME ids, snapshot label +
     position; existing rows keep their current form — the snapshot only fills gaps)
     before both revert paths: `spine::revert_artifact` (Tauri/user — content restored
     verbatim) and `freeze::revert_artifact_guarded` (MCP — then through the
     freeze/normalize boundary). RevisionHistory cherry-picks splice by section_id
     first, re-create a deleted row (snapshot label/position, else old label appended)
     and re-key the spliced entry; a structure cherry-pick over notes-only revisions
     restores the section's FORM straight onto the spine.
   - FRONTEND SAVES EMBED `spine_snapshot` (lib/sections.ts `spineSnapshot`): the
     Structure/Chords/Lyrics/LyricSpec editors + History restores — spine songs only,
     legacy saves stay byte-identical.
   - MCP `save_artifact` NORMALIZATION: on spine songs the guarded path maps
     label-keyed sections → section_id (create=NEVER, drop + ⚠ like non-structure
     runs) BEFORE the frozen splice, re-renders text, embeds the snapshot; structure
     saves keep notes only (+ ⚠ note when sections were included) and never touch the
     spine — Claude changes sections via the section tools or a structure run.
   - IN-SESSION UNION EDGE: `spine::union_artifact_sections` (reusing the migration's
     union source, `db::artifact_section_labels`) runs when a structure RUN births the
     spine, and via the `union_spine_sections` command when a StructureEditor save
     births it — chords/lyrics-only sections join in first-seen order, default form.
   - Mock parity throughout (notes-only structure, snapshots, snapshot-restoring
     revert, union). 97 core + 2 app tests green (4 new: snapshot restore, MCP save
     normalization ×2, birth union; structure-shape tests now assert the SPINE).

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
