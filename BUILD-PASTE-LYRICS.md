# Build brief — Feature B: paste completed lyrics → processed in the flow

Read `docs/STAGE-FREEZE-AND-LYRICS-IMPORT.md` FIRST — Feature B is the authority (decisions locked:
BOTH entry points; reconcile Structure too; words kept VERBATIM). Build on the current tree
(latest commit 097cdab).

## Outcome
The user has finished lyrics (theirs or from elsewhere) and drops them into the app instead of
generating:
1. **Into an existing song** — a "📋 Paste lyrics" action in the Lyrics stage: paste raw text →
   parsed into sections → preview → confirm → becomes the Lyrics artifact (new revision), and the
   **Structure stage is back-filled** to match (labels/order), so Chords/Arrangement/Composer stay
   consistent. Concept untouched.
2. **New song from lyrics** — a "New from lyrics" entry on the Library: paste → creates a song with
   Structure + Lyrics populated from the paste; Concept left blank/minimal; user picks preset/key/
   BPM as the existing new-song flow allows (inspect Library's current create flow and mirror it).

## Parsing (the verbatim guarantee — core logic, in Rust `core/`)
- **Deterministic first:** split on bracketed section headers (`[Verse 1]`, `[Chorus]`, Suno-style
  `[verse]`; also tolerate `Verse 1:` line-style headers) into sections + plain lines. Words are
  NEVER altered. Lines within a section stay verbatim (they become the Lyrics artifact's `lines` —
  plain text is valid ChordPro with no tags; chords get placed later via the existing editor).
- **Claude fallback ONLY for unlabeled text** (no headers found, >1 non-empty line): one
  `call_claude` asking ONLY for section boundaries + type labels, returning the original lines
  verbatim in a strict JSON shape. **Validate**: every returned line must appear in the input with
  identical text (whitespace-normalized compare); any alteration → discard and fall back to ONE
  section labeled "Lyrics". Use the existing mock-claude hook pattern for tests.
- Implement as `core` functions + Tauri commands:
  - `parse_pasted_lyrics(text) -> {sections: [{label, lines}], used_claude: bool}` — dry-run, no save
    (drives the preview).
  - `import_lyrics(song_id, text) -> ()` — parse, save the Lyrics artifact
    `{kind, text, data:{sections:[{label, lines}]}}` (text rendered like the editors do:
    `[label]\nlines`), then **back-fill the Structure stage**: rewrite its `data.sections` to match
    the pasted labels/order (keep existing bars/role where a label matches the old structure;
    default bars=8, role="" for new ones — inspect StructureEditor's Section shape and the structure
    text renderer in agent.rs and stay consistent). Mark both stages' status sensibly (done/edited
    per existing conventions). Downstream staleness is fine.
  - `create_song_from_lyrics(title, preset_id, text, ...) -> song_id` — mirror the existing
    create_song flow's required inputs, then run the same import.
- **Freeze interaction:** pasting is a deliberate USER action (UI path = user authority), so it
  replaces everything INCLUDING locked sections — but if the current Lyrics artifact has frozen
  sections, the preview must show a clear warning ("this replaces locked sections too") before
  confirm. Carry no frozen flags into the new artifact.
- Register both commands in the invoke handler; add `import_lyrics`-related tools to the MCP
  registry ONLY if trivial (optional — UI-first feature).

## Frontend
- `LyricsEditor.tsx`: a "📋 Paste lyrics" button → modal/panel: big textarea → live preview of the
  parsed sections (call `parse_pasted_lyrics` on debounce or on a Preview button; show
  `used_claude` when the fallback segmented it) → the frozen-replacement warning when applicable →
  Confirm calls `import_lyrics`, invalidates queries, closes.
- `Library.tsx` (or wherever new-song lives): a "New from lyrics" affordance beside the existing
  new-song entry → same modal + the fields the normal create flow needs → `create_song_from_lyrics`
  → navigate to the new song.
- `ipc/api.ts` + `ipc/mockApi.ts` parity: mock implements deterministic parsing in TS (headers
  split only; `used_claude:false`) + both commands mutating the mock DB believably.

## Tests + visuals
- core: (a) deterministic split on headered text (labels/order/lines verbatim, `[x]` and `x:`
  styles); (b) unlabeled text + mock-claude returning VALID segmentation → sections used,
  `used_claude:true`; (c) mock-claude ALTERING a word → single-section fallback (verbatim guarantee
  proven); (d) `import_lyrics` back-fills Structure preserving bars/role for matching labels;
  (e) create_song_from_lyrics produces a song whose Lyrics + Structure match the paste. Keep ALL
  existing tests green (26 core + 2 app).
- `scripts/visual_test.py`: screenshot the paste modal with a parsed preview, and the Lyrics stage
  after import. Kill port 5173 first. List files.
- Update `docs/STAGE-FREEZE-AND-LYRICS-IMPORT.md` (Feature B → ✅ DONE + what/where) and README.

## Definition of done
- `cargo build` + `cargo test` green (paste the lines); `cd frontend && npm run build` succeeds;
  visual test green with the new screenshots. Do NOT commit — the parent verifies and commits.

## Rules
- Words VERBATIM everywhere — the model may only segment, never rewrite; the validator is the
  guarantee, not the prompt. Subscription auth (env_remove key/token) on any claude spawn. Match
  existing style (artifact shape, text renderers, status conventions, dark neon aesthetic). Don't
  break freeze, the Composer, or existing editors.
