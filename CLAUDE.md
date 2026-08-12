# Songsmith Studio

A Tauri v2 desktop app (Rust core + React/Vite frontend) that drives the `claude`
CLI to co-write songs, then exports them to Ableton Live. "Claude Code for music
producers": a Suno-style AI render goes in, a human-produced song comes out.

## Agent skills

### Issue tracker

GitHub Issues on `PaulBratslavsky/songsmith`, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical roles, each label string equal to its name (`needs-triage`,
`needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root (created lazily by
`/domain-modeling` — their absence is not a problem to flag). See `docs/agents/domain.md`.

## House rules

**Subscription auth, never an API key.** Every spawned `claude` process must
`env_remove` `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` — that strip is what
keeps the user on their claude.ai subscription with connectors loaded. Prompts go
over STDIN (a prompt can start with `-`, which the CLI's arg parser rejects).

**One home per piece of data.** Copies drift, and every data bug in this app's
history was a second copy disagreeing with the first: the song owns key/BPM/intent,
the `section` table is the only section list, `song_melody`/`song_part` hold written
parts. Anything else showing that data is a live view of it. See
`docs/SONG-CREATION-AND-DATA.md`.

**`update_*` tools are PARTIAL.** Omitted fields keep their current value; an
explicit `""` clears. A full-replace update once blanked a whole style preset.

**Verify against reality, not hope.** Real generations through the mcp-shim, real
files, the user's own screenshots — a model-shaped fix that isn't measured isn't
done. Say plainly when something failed or was skipped.

## Testing

- `cargo test -p song_core` — the contracts (129): the take clamp, bass register
  math, whole-flow scenarios, settings round-trip.
- `make flowcheck` — Tier B: a scratch song through all 6 stages with REAL Claude,
  then 29 coherence checks. ALWAYS via `make` (it rebuilds the shim first; a stale
  shim has produced false failures).
- `cd frontend && npx tsc --noEmit` — the frontend has no test runner by design;
  types plus real-data verification carry it.

## Key docs

- `docs/RENDER-ROUNDTRIP.md` — the AI-render → DAW pipeline, the take contract,
  analyzer accuracy, test nets.
- `docs/IMPORT-ACCURACY-PLAN.md` — measured baseline for key/chord/structure
  detection on real renders, how Chordify does it, and the tiered plan (T0–T4).
- `docs/SONG-CREATION-AND-DATA.md` — how a song is created and where its data lives.
- `docs/SECTION-SPINE-SPEC.md`, `docs/SONG-FACTS.md` — the ownership rules above.
- `docs/BACKLOG.md` — what's open.
