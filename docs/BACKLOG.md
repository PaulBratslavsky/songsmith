# Backlog (small queued items — canonical, committed)

## Open (2026-07-29)

- ✅ RESOLVED (2026-07-30) — **"claude startup costs minutes per stage"** was a
  MISMEASUREMENT. A >300s spawn observed during a service-degradation window was
  blamed on MCP server boot; measured properly, ten global servers add 1.0-1.3s
  and connect asynchronously (`pending`/`needs-auth` never blocks the turn). Stage
  time is real generation time. `call_claude` now passes
  `--mcp-config '{"mcpServers":{}}' --strict-mcp-config` anyway, for ISOLATION —
  single-turn text generation shouldn't have third-party servers in its failure
  path — not for speed. Chat keeps its own servers (`chat_send`).

- **Drums through the Arranger.** Bass/pad/chords/arp/lead are all Claude-written
  takes now (docs/RENDER-ROUNDTRIP.md, "the take contract"); drums are still
  formula-only because they speak GM pitches (36 kick / 38 snare / 42 hat), not
  scale degrees, so they need a second output shape rather than a new part name.
- **`flow_resume_is_idempotent_on_complete_song` flaked once in ~30 full-suite
  runs** (0/20 focused reruns). Most likely the `SONGSMITH_MOCK_CLAUDE`
  process-global env race that `core/src/flow_tests.rs`'s header documents —
  `agent::tests` sets/removes it while flow_tests run. The durable fix is to
  delete that env hook entirely now that every flow test scripts `claude_bin`.
- **Analyses are snapshots.** The analyzer's beat-sync and downbeat fixes
  (2026-07-29) do not retroactively apply to songs imported earlier: their
  stashed `render.analysis` still carries the old one-beat-late chords. Re-run
  🎼 Analyze → Composer per render to restash. A "re-analyze every render"
  batch action would remove the manual step (not built — it overwrites stored
  analyses, so it needs an explicit confirm).
- **Awaiting user hardware/session verification:** 🎹 MIDI keyboard step entry;
  the Ableton "Reference" audio track (needs a Live restart to load remote-script
  patch v3); the Music.AI add-on's live round-trip (needs an API key).

- ✅ DONE (2026-07-07, Rust backlog batch) **Seed new songs' key/BPM from the style preset** (user,
  2026-07-07 — approved "yes"): `core/src/db.rs` now has a pure `parse_key_tempo(feel)` (first
  explicit key mention wins; BPM ranges → rounded midpoint) and `db::create_song` seeds
  key_root/key_mode/bpm from the preset's `key_tempo_feel` when parseable (all creators inherit it;
  composition/reference imports still override with their explicit key right after). Mock parity in
  `frontend/src/ipc/mockApi.ts` (`parseKeyTempo`). Unit tests: `parse_key_tempo_real_world_strings`,
  `create_song_seeds_key_bpm_from_preset_feel`. Song-level fields keep overriding after creation.
- ✅ DONE (2026-07-07, frontend backlog batch) **Bridge sections that exist only in the lyrics
  don't appear on the Composer timeline** (pre-existing; noted during sheet v3). Decision: INCLUDE
  them — `compositionFromSong` now appends a chordless section band for each lyric-only section
  (length = max(1 bar, one bar per lyric line), lines anchored evenly across it; no chord spans, so
  playback is simply silent there). The lyric sheet shows its chip + chordless lines as before.
- **Remaining audit Tier-2** (docs/AUDIT-2026-07-06.md): prompt consolidation (refine_field lacks
  song context). Done 2026-07-07 (Rust backlog batch): move Ableton/MIDI logic into core (#10),
  agent.rs module split (#11), serial MCP shim + dead token deleted (#13). Done 2026-07-07
  (frontend backlog batch): CommandMap api/mock parity (#7), zod artifact schemas in
  `lib/artifacts.ts` (#8), Composer/Builder rename (#14), frontend dead-code sweep (#15), ChordPro
  consolidation into `lib/music/chordpro.ts` (#16, partial) — see the audit annotations.

- ✅ DONE (2026-07-14, four phases) **Section-spine normalization (milestone — spec first, do NOT
  bolt on)** (user-approved direction, 2026-07-08): sections (labels/order/bars) were duplicated
  across structure/chords/lyric_spec/lyrics artifact data and reconciled by back-fills — the biggest
  remaining copy-drift class. Built per docs/SECTION-SPINE-SPEC.md (STATUS: COMPLETE): the `section`
  table is the single spine; stage data references section ids (freeze matches id-first); the
  structure artifact dropped its section copy; Composer/import/export/Ableton/prompts all read the
  spine; restores are snapshot-based.
