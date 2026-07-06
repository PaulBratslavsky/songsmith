# Build brief — Tier 1 hardening (audit 2026-07-06, items 1, 2, 4)

Read `docs/AUDIT-2026-07-06.md` FIRST (Tier 1). This brief implements the Rust-side Tier 1 fixes:
the freeze write-boundary, call_claude hardening + cancel, and the DB races. The two frontend bugs
(hooks crash, late-lyrics race) are being fixed separately — DO NOT touch
`frontend/src/components/Composer.tsx`, `frontend/src/components/compose/*`, or
`frontend/src/routes/ComposerRoute.tsx`.

## 1. Freeze write-boundary (closes the chat/MCP bypass)
**Trust model:** UI editor saves = the user's authority (can unlock/rewrite anything). Claude-driven
writes (MCP tools, agent output) must NEVER violate frozen sections.
- Add `core`: `save_artifact_guarded(conn, song_id, stage_id, kind, content) -> Result<Artifact>` —
  for section-based stages (structure/chords/lyric_spec/lyrics): load the prior current artifact; if
  its data has frozen sections (`data.sections[]`, or `data.beats[]` for lyric_spec — reuse the same
  section-array detection `merge_frozen_sections` uses), parse the incoming content, run
  `merge_frozen_sections(stage_type, prior_data, incoming_data)`, re-render `text` from the merged
  data via the existing per-stage renderers, and save the merged artifact. If the incoming content's
  JSON doesn't parse AND the prior has frozen sections → return Err (do not save). Non-section
  stages / no frozen sections → plain save (behavior unchanged).
- Use it in `tools.rs` dispatch for **`save_artifact` AND `revert_artifact`** (revert can resurrect
  pre-freeze revisions — apply the same guard to the reverted content). The Tauri `save_artifact`
  command (lib.rs:150) stays direct — it's the user's own editor (this is how unlock works). Document
  the trust model in comments at all three sites.
- Tests: (a) MCP-path save with a frozen prior keeps the frozen section byte-identical and the
  `frozen` flag; (b) MCP-path save with unparseable content + frozen prior errors and prior stays
  current; (c) the direct (UI) path can still unfreeze/rewrite; (d) revert over MCP respects frozen.

## 2. Parse-failure hole in build_merged_content (agent.rs:280-301)
When the prior artifact has frozen sections and `extract_json(raw_text)` is `None` → return Err with
a clear message ("model output had no parseable JSON; refusing to overwrite an artifact with locked
sections") instead of saving `{data:null}`. When nothing is frozen, keep today's lenient fallback.
Also: in `call_claude`'s result handling, check the `result` event's `is_error` / non-"success"
`subtype` — treat as an error (surface the text) rather than saving it as an artifact.

## 3. call_claude hardening + cancel (agent.rs:594-682; lib.rs chat_send:400-428)
- `kill_on_drop(true)` on the Command.
- Drain stderr CONCURRENTLY (spawn a task collecting into a buffer) — never read-after-wait (the
  current pattern deadlocks past ~64KB). Fix in both call_claude and chat_send.
- Wrap the stdout read loop in `tokio::time::timeout` (default 600s; a `SONGSMITH_CLAUDE_TIMEOUT_SECS`
  env override is enough — no settings UI). On timeout: kill child, reap, clear error message.
- `run_stage`: on ANY error path, reset the stage status to its prior value (it's set `in_progress`
  at agent.rs:322 and currently stranded on failure).
- **Cancel:** `AppState` gains a map `running: Mutex<HashMap<String /*stage_id*/, CancelHandle>>`
  (design the handle — e.g. a oneshot/notify that the call_claude loop selects on, killing + reaping
  the child). New Tauri command `cancel_stage(stage_id)` → cancels the in-flight run, resets stage
  status, clears the `inflight` entry, returns cleanly. Register it. Frontend: add `cancelStage` to
  `ipc/api.ts` + mockApi parity, and a **Cancel button in `AIRunPanel.tsx`** shown while a run is
  pending. (AIRunPanel/api.ts/mockApi.ts are yours to edit; the compose/ files are not.)
- `self_check_stage` should set `in_progress` while running (currently no busy signal) and reset on
  error, same as run_stage.

## 4. DB races (db.rs)
- **Migration atomicity:** wrap the lyric_spec retrofit (db.rs:75-82 — ordinal shift + insert) in a
  transaction; propagate errors instead of `let _ =`.
- **Atomic artifact versioning:** compute the version inside the INSERT
  (`INSERT ... SELECT COALESCE(MAX(version),0)+1 FROM artifact WHERE stage_id = ?`) and add
  `CREATE UNIQUE INDEX IF NOT EXISTS ... ON artifact(stage_id, version)` in migrate. Handle the
  no-stage (song-level artifact) case if it exists.
- Transactions on multi-statement writes: `create_song`, `delete_song`, `set_settings`.
- Replace the ~11 write-then-reload `.unwrap()`s (db.rs — create_preset:170, update_song_status:237,
  save_artifact:360, update_skill:422, etc. — grep `.unwrap()` in db.rs) with
  `ok_or_else(|| anyhow!(...))`.
- Set `PRAGMA busy_timeout` (e.g. 5000ms) on connection open in BOTH the app (db::open) and the
  mcp-shim, so concurrent app/shim writes wait instead of erroring "database is locked".

## Definition of done
- `cargo build` + `cargo test` green — ALL existing tests still pass plus the new guard tests
  (paste the `test result:` lines).
- `cd frontend && npm run build` succeeds (api.ts/mockApi/AIRunPanel changes typecheck).
- Update `docs/AUDIT-2026-07-06.md`: mark the fixed Tier-1 items with ✅ FIXED annotations.
- Do NOT commit — the parent verifies and commits.

## Rules
- Preserve the verified-good patterns (registry, mock hook, freeze splice, journaling). Match
  existing style. Keep the mock-claude test hook working. When nothing is frozen and no error
  occurs, all behavior must be byte-identical to today. Do not touch the files listed as
  off-limits at the top.
