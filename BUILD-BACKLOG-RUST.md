# Build brief — backlog batch: Rust core (work in THIS order, keep green between items)

Authorities: `docs/BACKLOG.md` + `docs/AUDIT-2026-07-06.md` (Tier 2/3). Latest commit 24aadfc.
⚠ The user is LIVE-TESTING the dev app: do NOT kill anything on port 5173, do NOT run
scripts/visual_test.py, do NOT restart the app. cargo build/test are fine (shared target dir —
expect occasional lock waits from the dev watcher). Do NOT touch frontend/src/** except
`frontend/src/ipc/api.ts` + `mockApi.ts` + `ipc/generated/` where an item explicitly says so —
a parallel agent owns the rest of the frontend.

## 1. Seed new songs' key/BPM from the style preset (BACKLOG item, user-approved)
- Core: a pure `parse_key_tempo(feel: &str) -> (Option<(String,String)>, Option<i64>)` that extracts
  a key (root + mode; handle "F minor", "A min", "Dark minor key (F minor / …)" → first explicit
  key mention wins) and a BPM (handle "~135–145 BPM" → midpoint rounded, "120 BPM" → 120) from the
  preset's `key_tempo_feel` prose. Unit-test with the real-world strings: "Dark minor key (F minor /
  cowbell-friendly), ~135–145 BPM with a half-time trap feel" → F minor, 140; "60–75 BPM half-time
  crawl in minor keys (A minor, D minor), swung trap hi-hats" → A minor, ~68 (first key listed);
  empty/unparseable → None.
- Apply at song creation: wherever songs are created (db::create_song callers — the Tauri
  create_song command, create_song_from_lyrics, create_song_from_composition KEEPS its explicit
  key/bpm args), a new song's key_root/key_mode/bpm default from the preset parse when available,
  else current defaults. Song-level fields keep overriding afterwards (unchanged).
- mockApi: mirror (a simple TS parse or hardcode the same behavior for the mock presets) so the
  browser demo matches. Update BACKLOG.md (item → ✅ DONE + where).

## 2. MCP shim: stop being strictly serial (audit Tier-2 #13)
- `mcp-shim/src/main.rs`: dispatch each `tools/call` onto a spawned task with a response channel so
  a long `run_stage` doesn't block `ping`/other calls; responses may return out of order — that's
  valid JSON-RPC (matched by id). Keep stdout writes serialized (a mutex/channel writer). Also
  delete the dead unvalidated MCP token OR enforce it (env `SONGSMITH_TOKEN` match) — pick enforce
  ONLY if trivial end-to-end (config already passes env), else delete token generation + the
  `mcp_config` surface field and note it. Smoke-test the shim manually (echo JSON-RPC lines).

## 3. Move the Ableton/MIDI domain logic out of the Tauri layer (audit Tier-2 #10)
- `app/src-tauri/src/lib.rs` ~798-1229: `chord_tones`, `nearest_pitch`, `part_notes`, `chord_events`,
  `section_parts`, the Ableton socket protocol + builders (`ableton_cmd/_build/_build_clips/
  _build_song`) → move VERBATIM-where-possible into `core/src/midi.rs` + `core/src/ableton.rs`
  (pub fns). lib.rs commands become thin wrappers. Also unify the TRIPLICATED structure-section
  parsing (lib.rs chat preamble + ableton_build + song_sections) through one core fn. Register
  `ableton_build_song` in `core/src/tools.rs` registry+dispatch (destructive: false) so Claude can
  invoke it over MCP; add the mock parity entry in `frontend/src/ipc/mockApi.ts` MOCK_TOOLS (that
  file only — coordinate note: the parallel frontend agent is told not to touch MOCK_TOOLS).
  Move any movable unit logic under core tests (at least: chord_events/part_notes smoke test).

## 4. Split agent.rs (audit Tier-2 #11) — LAST, it's the churny one
- `core/src/agent.rs` (~1300+ lines) → split into focused modules, e.g. `core/src/engine.rs`
  (call_claude, CancelToken, extract_json, timeout), `core/src/render.rs` (the per-stage text
  renderers — COLLAPSE the duplicate renderer pair the audit found: structure_text/chords_text vs
  structure_editor_text/chords_editor_text → one set), `core/src/freeze.rs` (merge_frozen_sections,
  frozen_prompt_block, has_frozen_sections, save/revert guards), keeping `agent.rs` as orchestration
  (run_stage, chat, self_check, refine_field, imports/exports). Pure mechanical moves + re-exports
  (`pub use`) so `crate::agent::X` call sites keep compiling where practical. NO behavior changes.
  All tests must stay green UNCHANGED (44 core + 2 app currently — do not weaken any).

## Definition of done
- After EACH item: `cargo build` + `cargo test --workspace` green. Paste final `test result:` lines.
- `cd frontend && npx tsc --noEmit` clean (for the api/mock touches).
- Update `docs/BACKLOG.md` + `docs/AUDIT-2026-07-06.md` annotations (✅ FIXED + where) for the items done.
- Do NOT commit — the parent verifies and commits. Report per-item: files, results, anything deferred.
