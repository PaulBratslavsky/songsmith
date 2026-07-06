# Build brief — Per-section Freeze (regeneration-safe)

Implements **Feature A** of `docs/STAGE-FREEZE-AND-LYRICS-IMPORT.md` (read it first — authority).
Goal: lock individual sections in a stage so regenerating that stage NEVER changes them.

## Model
- Add optional `frozen?: boolean` per entry in a stage artifact's `data.sections[]`. Pure JSON — no DB
  migration. Absent = unfrozen.

## Backend — make `core/src/agent.rs` regeneration merge-aware (the guarantee)
Today `run_stage` (initial gen) AND the self-test/revise pass just `save_artifact(Claude output)`.
Change BOTH to preserve frozen sections:
1. Load the stage's prior current artifact (`db::current_artifact`) and read its `data.sections` +
   which are `frozen`.
2. **Prompt-aware:** if any are frozen, append to the user prompt a clear block: "The following
   sections are FINAL — reproduce them EXACTLY and only (re)write the others:\n<frozen sections
   rendered as text>." (So unlocked output stays coherent with locked content.)
3. **Deterministic splice (after Claude returns), for section-based stages only** (structure, chords,
   lyric_spec, lyrics): parse the new artifact `{kind,text,data}`; for each prior-frozen section,
   replace the matching new `data.sections` entry verbatim (match by `label`, case/space-insensitive;
   if missing, re-insert at its original index) and set `frozen:true` on it. Then **rebuild the
   artifact `text` from the merged `data`** so the human-readable text + downstream
   `gather_prior_context` (which reads `text`) stay consistent — mirror how each stage's editor renders
   text from data (lyrics: `[label]\n lines`; chords: `label: name name ...`; structure/lyric_spec:
   match their editors). Save the merged artifact.
- Non-section stages (concept, prompt) are unaffected.
- Add a small helper (e.g. `merge_frozen_sections(stage_type, prior_json, new_json) -> json`) +
  per-stage text renderers; unit-test it.

## Frontend — lock toggle per section
Add a 🔒 lock/unlock control to each section in: `StructureEditor`, the chords editor
(`components/Composer.tsx`), `LyricSpecEditor`, `LyricsEditor`. Toggling sets `frozen` on that section
in the artifact `data` and saves (same save path as other edits → new revision). Locked sections get a
clear visual state (e.g. a lock badge + subtle dimmed/"protected" styling). A small hint somewhere:
"Locked sections are kept as-is when you regenerate."
- `ipc/mockApi.ts`: parity — persist `frozen` on sections; the mock run-stage path should keep frozen
  sections unchanged so the visual test can show it.

## Tests + visuals
- core: a `run_stage`-level test (use the existing mock-claude pattern) proving a **frozen** section's
  content is identical before/after a regeneration while an unlocked section changes. Plus a unit test
  for `merge_frozen_sections` (match by label, re-insert if dropped, flag carried). Keep ALL tests green.
- `scripts/visual_test.py`: screenshot a stage (e.g. Lyrics or Chords) with one section locked (lock
  badge visible). List it.
- Update `docs/STAGE-FREEZE-AND-LYRICS-IMPORT.md` (Feature A → DONE) + a README line.

## Definition of done
- `cargo build` + `cargo test` green (paste `test result:` lines).
- `cd frontend && npm run build` succeeds.
- Visual screenshot listed.
- Do NOT commit — parent verifies and commits.

## Rules
- A lock is a HARD guarantee (deterministic splice), not just a prompt request. Don't break the normal
  regenerate flow when nothing is frozen (behavior identical to today). Match Songsmith style. Keep the
  artifact `{kind,text,data}` shape + `text` consistent with `data` after merge. Feature B (paste
  lyrics) is a SEPARATE later build — don't do it here.
