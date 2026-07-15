# BUILD BRIEF — Whole-song-flow test suite (Tier A: deterministic)

Repo: `/Users/paul/programing/songsmith-studio` (Rust workspace: `core` = song_core lib, `app/src-tauri` = Tauri shell, `frontend` = React).

## Goal

A deterministic test suite that walks the WHOLE song flow (create → concept → structure → chords → lyric_spec → lyrics → prompt) through the real orchestration code with a scripted fake Claude, asserting every flow guarantee we've built. Each scenario below reproduces a bug class the user actually hit in live testing. The suite is our answer to "we go back and forth more than we need to" — flow bugs must fail a test before the user finds them.

## Where the code lives

- Orchestration: `core/src/agent.rs` (`run_stage`, `stage_user_prompt`, `create_song_from_lyrics`, `import_lyrics`, `apply_parsed_lyrics`, `parse_pasted_lyrics`)
- Spine: `core/src/spine.rs` (`sync_spine`, `reconcile_structure_run`, `spine_snapshot`), sections CRUD in `core/src/db.rs`
- Freeze: `core/src/freeze.rs` (`merge_frozen_sections`, `save_artifact_guarded`)
- Claude call + mock hook: `core/src/engine.rs` — when env var `SONGSMITH_MOCK_CLAUDE` is set, `call_claude` returns its value verbatim (no CLI spawned)
- Existing test patterns: bottom of `core/src/agent.rs` — `mem_conn()` helper (in-memory libSQL + migrations), `ENV_LOCK` mutex guarding `SONGSMITH_MOCK_CLAUDE` set/remove (tests run in parallel; NEVER touch that env var without holding the lock; always `remove_var` before releasing)

## Deliverable

New file `core/src/flow_tests.rs`, registered as `#[cfg(test)] mod flow_tests;` in `core/src/lib.rs`. COPY the small helpers you need (mem_conn-style setup, an env-lock — reuse `agent::tests` items only if already reachable; otherwise duplicate locally). Do NOT refactor `agent.rs` or move existing tests — append-only integration; the parent session may be editing that file concurrently.

Each test: build a song via `db::create_song` (create a preset first via `db::create_preset`), then drive stages via the real `run_stage`/`save_artifact_guarded` paths with `SONGSMITH_MOCK_CLAUDE` scripted per step.

## Scenarios (one test each; name them `flow_...`)

1. **Happy path lifecycle** — run every stage in order with valid mocked JSON outputs (look at each stage's expected `data` shape in `agent.rs`/`render.rs`; the structure shape is `{keyNote, tempoNote, sections:[{label,bars,role}]}` reconciled against the spine, chords `{sections:[{label|section_id, chords:[{name,beats}]}]}`, lyrics `{sections:[{label|section_id, lines:[...]}]}`). Assert after each run: artifact saved with text rebuilt from data (`render_stage_text`), stage status, spine unchanged unless the stage may change it, and the final prompt-stage text contains no `{PASTE}`-style placeholder and references the song's real key/BPM.
2. **Song facts can't be clobbered** — set the song to F# minor / 109 (`db::update_song_key`), then run structure with a mock that returns `key: "A minor"` / `bpm: 138` inside data. Assert: song row still F# minor/109; saved structure data carries NO key/bpm fields (the `enforce_song_key_tempo` splice / schema drop); `stage_user_prompt` for structure contains the `KEY/TEMPO AUTHORITY` block naming F# minor and 109.
3. **AI can't invent or silently drop sections** — spine of 3 sections; mock structure output inventing a 4th section and omitting one. Assert: no new spine row (D3 no-create), omitted section still present (D2 keep-and-warn), warnings surfaced wherever `reconcile_structure_run` puts them.
4. **Freeze survives regeneration at the write boundary** — freeze one lyrics section, mock a lyrics regen that rewrites every section. Assert the frozen section's lines are byte-identical after save (via `save_artifact_guarded`), other sections updated.
5. **Paste flows** — `create_song_from_lyrics` with a mixed paste: `**Verse 1**` bold headers, `[Chorus]` bracket header, a Suno arrangement tag line `[staccato synth lead riff, electronic kick drum]`, inline `[F#m]` chord tags. Assert: sections split on the real headers only, arrangement tag stays a body line, spine rows match section labels 1:1, key inferred F# minor, lyrics stored verbatim. Then `import_lyrics` of a different sectioning into that song: assert SPINE REPLACE (old rows gone, new labels in order, matched labels keep their row ids).
6. **Reverse context** — song with lyrics artifact but empty concept stage: concept's `stage_user_prompt` carries the derive banner (`LATER_STAGES_BANNER`) and the lyric lines; after the concept has an artifact, a regen prompt carries the reference-only banner (`LATER_STAGES_REGEN_BANNER`) instead.
7. **Revision history + revert** — two lyrics runs (different mocked content) → versions 1,2; `revert_artifact` to v1 → v3 content equals v1, spine restored from snapshot if the spine changed in between.
8. **Approve gates advancement only** — regenerating an unapproved stage overwrites the current artifact (new revision) without touching approval; `approve_stage` marks done and `advance_stage` moves the song pointer; a later stage's run does not silently approve anything.

If a scenario exposes a REAL production bug: do NOT fix production code. Write the failing test, mark it `#[ignore = "real bug: <one line>"]`, and append a `## FINDINGS` section to this file describing exactly what fails and where.

## Constraints

- `cargo test -p song_core` must end green (ignored tests excepted). Run it often.
- Never touch the running dev app: do not kill anything on port 5173, do not run visual_test.py, do not use the live DB (`~/Library/Application Support/com.songsmithstudio.desktop/`). In-memory DBs only.
- Do not commit. The parent session reviews and commits.
- Keep every mock JSON minimal but schema-valid; when unsure of a shape, read the parsing code, not guesses.
