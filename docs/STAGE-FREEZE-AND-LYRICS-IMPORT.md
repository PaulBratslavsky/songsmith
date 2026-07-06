# Spec — Section Freeze + Paste-Lyrics Import (canonical, committed)

Source of truth for two stage-pipeline features (user, 2026-06-29). Decisions locked via Q&A.

## Background (how the pipeline works today)
Stages: concept → structure → chords → lyric_spec → lyrics → prompt. `core/src/agent.rs::run_stage`
gathers each upstream stage's CURRENT artifact (`gather_prior_context`), prompts Claude, and
`save_artifact` writes a **whole new artifact revision** — it never merges, so regenerating a stage
overwrites everything in it. Section-based artifacts are JSON: lyrics `{sections:[{label,type,lines}]}`,
chords `{sections:[{label,feel,chords:[{name,beats}]}]}`, structure `{sections:[{label,...}]}`.

---

## Feature A — Per-section Freeze (regeneration-safe) — ✅ DONE (2026-06-29)
**Decision:** per-section lock on **all section-based stages** (Structure, Chords, Lyric Spec, Lyrics).
A locked section is a **hard guarantee**: regeneration never changes it.

**Built:** `core/src/agent.rs` — `merge_frozen_sections(stage_type, prior, new)` + `frozen_prompt_block`;
both `run_stage` paths (initial gen + self-test/revise) inject frozen sections into the prompt then
deterministically splice the prior frozen sections back (match by label, re-insert if dropped, rebuild
`text` from merged `data`). 5 tests incl. "frozen section byte-identical after regen" + "no-frozen path
unchanged". 🔒 lock toggle per section in StructureEditor, Composer (chords), LyricSpecEditor,
LyricsEditor (writes `frozen` to the section, saved as a revision). When nothing is frozen, behavior is
identical to before.

### Model
- Add optional `frozen?: boolean` to each entry in a stage artifact's `data.sections[]`. (Persisted in
  the artifact JSON — no schema migration needed; absent = unfrozen.)

### Behavior — make `run_stage` (and the self-test/revise pass) merge-aware
1. Before generating, load the stage's prior current artifact and collect its `frozen` sections.
2. **Prompt-aware:** inject the frozen sections into the generation prompt as "these sections are FINAL
   — reproduce them EXACTLY, only (re)write the others," so the regenerated unlocked sections stay
   coherent with the locked ones.
3. **Deterministic splice (the guarantee):** after Claude returns, parse the new artifact JSON and for
   every section the prior marked `frozen`, overwrite the regenerated section with the frozen one
   verbatim (match by `label`; if Claude dropped/renamed it, re-insert it at its original position) and
   carry the `frozen` flag forward. Then save.
4. Both `run_stage` paths (initial gen + self-test/revise) apply the splice.
- Frozen content survives upstream cascade (staleness may still flag, but content is guaranteed).

### UI
- A 🔒 lock toggle per section in each stage editor: `StructureEditor`, the chords editor
  (`components/Composer.tsx`), `LyricSpecEditor`, `LyricsEditor`. Locked sections show a clear locked
  state. Toggling writes `frozen` into the artifact (save = new revision, same as other edits).

### Edge cases
- New sections Claude adds that didn't exist before: kept (only frozen ones are protected).
- A frozen section whose upstream changed: stays frozen (user opted to keep it); they can unlock to refresh.

---

## Feature B — Paste completed lyrics → processed in the flow
**Decisions:** BOTH entry points; reconcile **Structure too**; **words kept verbatim** (parse/tag only,
never rewrite).

### Entry points
1. **Into the current song's Lyrics stage** — a "Paste lyrics" action in `LyricsEditor`: paste raw text
   → parse into `{sections:[{label,type,lines}]}` → replace the Lyrics artifact (new revision).
2. **New song from pasted lyrics** — a Library entry ("New from lyrics"): paste → create a song → parse
   → populate Structure + Lyrics → status in_progress; user adds Concept/style then runs Chords. Key/BPM
   default (or from a quick prompt), Concept left blank/minimal.

### Parsing (verbatim guarantee)
- **Deterministic first:** split on bracketed section headers (`[Verse 1]`, `[Chorus]`, Suno-style) into
  sections + lines, words untouched. This is the safe path — no model can alter the words.
- **Claude fallback only for unlabeled text:** if there are no headers, ask Claude ONLY to mark section
  boundaries + types, returning the original lines verbatim (strict "do not change any words" prompt).
  Validate the returned lines are a subset/reordering of the input; if it altered words, fall back to a
  single section.

### Reconcile (chosen: Structure too)
- After saving Lyrics, **back-fill the Structure stage** section list to match the pasted lyrics'
  sections (labels/types/order) so Chords / Arrangement / Composer stay consistent. Leave **Concept**
  untouched. Mark downstream stages (chords/prompt) as appropriate (stale is fine).

### UI
- Lyrics stage: a "Paste lyrics" button → modal/textarea → parse → preview → confirm.
- Library: a "New from lyrics" entry alongside the existing new-song flow.

---

## Build order
1. **Feature A (Freeze)** first — it makes regeneration safe (the active pain). Backend `run_stage`
   merge + `frozen` flag + lock UI across the four editors.
2. **Feature B (Paste lyrics)** next — parser + both entry points + Structure back-fill.

## Out of scope (for now)
Freezing across stages in one click; auto-deriving Concept from lyrics; chord re-alignment to imported
lyrics beyond Structure back-fill (chords regenerate normally afterward).
