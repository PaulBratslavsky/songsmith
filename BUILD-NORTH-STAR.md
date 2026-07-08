# Build brief — the North Star: song title + producer intent flow into every stage

Latest commit 668bc11. Root cause (user-confirmed with receipts): `build_system_prompt` includes the
preset + key/BPM but NOT the song's title, and the user's seed is used once in a run then discarded —
so the Concept stage invented a different song ("Mirage in the Mirror" fate-chase) than the user's
title ("Finding you in the sand of time") + seed ("searching for love in desert"), and every later
stage faithfully amplified the drift. ⚠ User may be live-testing: never kill port 5173, never run
visual_test.py; core edits will hot-rebuild the app (accepted).

## 1. `intent` on the song (models/db)
- `intent: String` (`#[serde(default)]`) on `Song`; idempotent `ALTER TABLE song ADD COLUMN intent
  TEXT NOT NULL DEFAULT ''`; SONG_COLS/map/insert wired; `db::update_song_intent(conn, id, intent)`;
  ts-rs regen. Tauri command `update_song_intent` + registry/MCP tool (non-destructive) + mock parity.

## 2. Every stage sees TITLE + INTENT (agent.rs)
In `build_system_prompt`, after the preset block add:
```
----- THE SONG (the producer's brief — the north star; NEVER drift from it) -----
Title: {title}   (the title's plain meaning is part of the brief)
Producer's intent: {intent or "(none stated — honor the title)"}
---------------------------------------------------------------------------------
```
Include for ALL stages. The self-check rubric gains: "0. INTENT: does the output serve the song's
title and the producer's intent? If it drifted into a different song, rewrite toward the brief."

## 3. Seed persistence (run_stage)
When running the CONCEPT stage with a non-empty `user_input` and `song.intent` is empty → save the
seed as the song's intent (trimmed) BEFORE building prompts, so it persists for every later stage and
regeneration. (Only concept; only when empty — never overwrite a user-set intent.)

## 4. Concept skill (concept.md)
Add near the top: "THE BRIEF IS NOT YOURS TO REPLACE — the song TITLE and the producer's INTENT are
the assignment. Your hook/theme/arc must be an INTERPRETATION of them (sharpen, deepen, make
specific) — never a different song. If the title says 'Finding you in the sand of time', the concept
is about finding someone. `alternates` may explore adjacent angles; the primary concept honors the
brief. Only when title and intent are both absent may you invent freely from the preset."

## 5. UI
- Create-song flow (Library new-song form + New-from-lyrics): an optional "What's this song about?
  (one line — the north star)" input → passed through `create_song`/`create_song_from_lyrics` (new
  optional arg, default empty).
- SongWorkspace header: show the intent under the title row as an inline-editable line
  ("🎯 <intent>" / placeholder "🎯 set the song's intent — every stage follows it"); saves via
  `update_song_intent` on blur/Enter (mirror the title-edit pattern).
- api.ts + mockApi parity (CommandMap entries).

## 6. Tests
- `build_system_prompt` contains title + intent (and the "(none stated)" fallback).
- concept run with seed persists intent when empty; does NOT overwrite an existing intent.
- `update_song_intent` round-trip. Keep ALL 68 core + 2 app green.

## Definition of done
cargo build + cargo test green (paste lines); `cd frontend && npm run build` green; concept.md pushed
to the live DB builtin row (python sqlite3 pattern; key='songsmith-concept'); do NOT commit — the
parent verifies (with a real re-aimed generation on the user's song) and commits. Report files/tests.
