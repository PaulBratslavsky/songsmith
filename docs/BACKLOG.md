# Backlog (small queued items — canonical, committed)

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

- **Section-spine normalization (milestone — spec first, do NOT bolt on)** (user-approved direction,
  2026-07-08): sections (labels/order/bars) are duplicated across structure/chords/lyric_spec/lyrics
  artifact data and reconciled by back-fills — the biggest remaining copy-drift class (see
  docs/SONG-FACTS.md "Explicitly deferred"). Plan: a song-level `sections` entity as the single
  spine; stage data references section ids; freeze flags move to the spine; Composer/import/export
  read it. Touches every editor + Composer + freeze + imports — needs its own spec + test plan.
