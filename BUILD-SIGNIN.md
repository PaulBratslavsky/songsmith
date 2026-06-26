# Build brief — in-app Sign in / Sign out (Connect Claude account)

Port the **"Connect Claude account"** feature from the Work Agent app into Songsmith Studio, and ADD a
**Log out** button. Goal: from Songsmith's Settings, the user can see auth status, **sign in** with their
claude.ai (work) account, **sign out**, and test — all in-app, no terminal.

Auth rule (never violate): subscription `claude` login only — only `env_remove` `ANTHROPIC_API_KEY`/
`ANTHROPIC_AUTH_TOKEN` from spawned processes; never set/require a key (stripping it is also what keeps
connectors enabled).

## Source to port FROM (read these — they are complete + tested)
`/Users/paul/programing/work-agent/app/src-tauri/src/lib.rs`:
- `claude_auth_status` (~684), `extract_url` (~739), `claude_login` (~757, piped stdin + streams
  `login_event`), `claude_login_submit_code` (~847), `claude_login_cancel` (~883), `open_url_inner`/
  `open_url` (~892), `test_claude` (~911), the `AppState.login_child: Mutex<Option<std::process::Child>>`
  field, `api_key_in_env`, `CONNECTORS_URL`, and the invoke-handler registration (~1023).
`/Users/paul/programing/work-agent/frontend/src/routes/Settings.tsx` — the "Connect Claude account" card
  (status badge, Log in with streaming log + Paste-code input + Submit/Cancel, Re-check, Test, connectors
  link, API-key warning). `frontend/src/ipc/api.ts` + `mockApi.ts` — the matching methods + mock parity.

## Target (Songsmith) — read first
`/Users/paul/programing/songsmith-studio/app/src-tauri/src/lib.rs` — it ALREADY has `find_claude` (~240)
and `ensure_claude_bin` (~435) and a claude invocation. Reuse `find_claude`. Check its `AppState`,
settings accessor, and error type (`R<>`/`e2s` equivalents) and adapt names to match Songsmith's style.
`/Users/paul/programing/songsmith-studio/frontend/src/routes/Settings.tsx`, `frontend/src/ipc/api.ts`,
`frontend/src/ipc/mockApi.ts`, and the generated-types dir.

## Deliverables
1. Port these Tauri commands into Songsmith's `lib.rs` (adapt to its AppState/error types), register all in
   the invoke handler: `claude_auth_status`, `claude_login`, `claude_login_submit_code`,
   `claude_login_cancel`, `open_url`, `test_claude`, plus the `login_child` AppState field, `extract_url`,
   `open_url_inner`, `api_key_in_env`, `CONNECTORS_URL`.
2. **NEW — `claude_logout` command:** runs `claude auth logout` (API key stripped) via spawn_blocking,
   returns Ok on success; the UI calls it then re-checks status. (Verified: `claude auth logout` exists.)
3. Songsmith Settings — a **"Connect Claude account"** card mirroring Work Agent's:
   - Status badge: ● connected as `<account>` (`<subscription>`) / ⚠ not signed in / ⚠ API key set.
   - **Log in** → `claude_login`, stream `login_event` into a small log box, show the auth URL prominently
     + Open-browser, a **Paste authentication code** input → Submit (`claude_login_submit_code`) + Cancel,
     auto Re-check on success.
   - **Log out** button (only when signed in) → `claude_logout` → re-check (confirm before logging out).
   - **Re-check** (`claude_auth_status`), **Test** (`test_claude`, "reply READY"), connectors link
     (`open_url` to `CONNECTORS_URL`), and the API-key warning text.
4. `ipc/api.ts`: add `claudeAuthStatus`, `claudeLogin`, `claudeLoginSubmitCode`, `claudeLoginCancel`,
   `claudeLogout`, `openUrl`, `testClaude` (+ the `ClaudeAuthStatus`/`LoginResult`/`LoginSubmitResult`
   types). `ipc/mockApi.ts`: parity — mock a believable signed-in/out flow incl. logout flipping status.

## Definition of done
- `cargo build` (workspace) + `cargo test` green — paste the `test result:` lines.
- `cd frontend && npm run build` succeeds.
- If Songsmith has a `scripts/visual_test.py`, extend it to screenshot the Connect-account card (signed-in
  + the paste-code state) and list the files; otherwise skip gracefully.
- README: a short "Connecting your Claude account (in-app)" section incl. Log out.
- Do NOT commit — the parent verifies and commits.

## Rules
- Subscription only; only strip the key/token. Match Songsmith's existing style/naming (not Work Agent's
  verbatim) — adapt AppState, error helpers, and the frontend `call`/`listen` wrappers to Songsmith. Keep
  everything compiling and green. Run `cargo test` first if it regenerates ts types.
