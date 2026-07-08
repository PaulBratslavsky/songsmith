# Song Facts — single source of truth for cross-stage values (canonical, committed)

User decision (2026-07-08): static values referenced by multiple stages live in ONE place — the
SONG — and stage data references them instead of carrying copies. Copies drift (the structure
regen used to reset the user's key pickers; the enforce splice patched it; this removes the class).

## The facts (live on `song`)
- `title` — the name (already the header field)
- `intent` — the 🎯 north star (North Star feature)
- `key_root` + `key_mode` — the key
- `bpm` — the tempo
(Future candidates: time signature. NOT sections — see below.)

## Rules
1. Stage artifacts MUST NOT store song facts. Structure `data` drops `key`/`bpm`; it keeps
   `keyNote`/`tempoNote` (prose about how the arrangement SERVES the song's key/tempo) + `sections`.
2. Editors read facts from the song and write them via song-level commands (`update_song_key`,
   `update_song_intent`, `update_song_title`) — the Structure editor's pickers stay, but they edit
   the SONG only, never the artifact.
3. Prompts inject facts from the song (already true: THE SONG block + Current song key/tempo).
4. Renderers (`structure_editor_text`, sheet, prompts-stage skill) take facts from the song, not
   from artifact data.
5. READERS of old artifacts ignore embedded `key`/`bpm` (legacy rows keep them harmlessly).
6. Skills that previously "chose" facts (structure) justify/serve them instead; suggestions are
   prose in notes. The `enforce_song_key_tempo` splice remains as a transitional guard for models
   that still emit key/bpm, but the schema no longer expects them.

## Explicitly deferred
- **The section spine** (labels/order/bars) is still duplicated across structure/chords/lyric_spec/
  lyrics data and reconciled by back-fills (import_lyrics, composer export). Moving it to a
  song-level `sections` entity follows the same principle but is a larger refactor touching every
  editor + the Composer + freeze. Decide separately.
