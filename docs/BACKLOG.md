# Backlog (small queued items — canonical, committed)

- **Seed new songs' key/BPM from the style preset** (user, 2026-07-07 — approved "yes"): a new song
  under a preset currently gets default key/BPM; the preset's `key_tempo_feel` is prose-only. Parse a
  key + BPM out of `key_tempo_feel` (or add structured fields to StylePreset) and use them as the new
  song's defaults, so e.g. "Sinister Memphis Phonk" (F minor, ~135–145) doesn't silently produce
  A-minor/120 songs. Song-level fields keep overriding the preset after creation.
- **Bridge sections that exist only in the lyrics don't appear on the Composer timeline**
  (pre-existing; noted during sheet v3). The Chords stage is the section spine — decide whether
  lyric-only sections should get a chordless band on the timeline.
- **Remaining audit Tier-2** (docs/AUDIT-2026-07-06.md): CommandMap api/mock parity, zod artifact
  schemas, prompt consolidation (refine_field lacks song context), move Ableton/MIDI logic into core,
  agent.rs module split, serial MCP shim, dead code/rename hygiene (Tier-3).
