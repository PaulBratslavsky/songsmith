# Build brief — Composer: export back to song + done-state "Open in Composer" CTA

Read `docs/COMPOSER-SPEC.md` FIRST — flexibility requirements #2/#3 ("Create a new song directly in
the Composer", "Export a composition FROM the Composer back into a song") and the "Queued: prominent
'Open in Composer' on song completion" block are the authority. Build on the current tree (latest
commit 7f56104). This completes the Composer's bidirectional loop.

## Part A — Export a composition back into a song
The Composer's chords (+ structure) become a real song's Chords/Structure stages.

**Degree→absolute mapping (core of it):** a chord span resolves to an absolute chord name in the
composition's key — `name` when present (imported/full-song chords keep their real names), else the
diatonic triad name for `degree` in `key` (reuse the theory engine's diatonic helpers — the
frontend already has them; do the mapping in the FRONTEND and send resolved data to the backend,
or in Rust if cleaner — pick one, don't duplicate the theory).

**Beats:** span length in ticks → beats (`length / TICKS_PER_BEAT`, min 1, round sensibly).

**Sections:** if the composition has `sections[]`, group chords by section (label preserved,
bars = section lengthTicks/16). A sketch with no sections becomes ONE section ("Sketch", bars =
composition bars) — or split by bar-4 boundaries only if trivial; one section is acceptable.

**Two destinations (both required):**
1. **Update the source song** (when the composition has `song_id`): overwrite the song's Chords
   stage data.sections (labels + chords {name, beats}) and reconcile Structure the same way
   `import_lyrics` back-fills it (labels/order; preserve bars/role/🔒-frozen on label match).
   ⚠️ FREEZE: this is a USER action (UI authority) but chords/structure sections that are 🔒 frozen
   must be respected here — show which sections are locked in the confirm dialog and SKIP them
   (keep the song's frozen section, export the rest). Simpler + safer than silently overwriting.
2. **Create a new song from the composition** (works for any composition, incl. blank sketches):
   mirror `create_song_from_lyrics`'s preset/title inputs → new song with Structure + Chords
   populated from the composition; key/bpm from the composition. Lyrics stay empty (melody/bass
   don't map to stages — they live in the saved composition).

**Backend:** a core fn + Tauri command per destination, e.g.
`export_composition_to_song(song_id, sections_json)` and
`create_song_from_composition(title, preset_id, key_root, key_mode, bpm, sections_json)` — take
the RESOLVED sections (label, bars, chords{name,beats}) so the backend stays theory-free; render
stage text with the existing renderers; save via the existing guarded/artifact conventions; set
stage statuses like import_lyrics does. Register commands; MCP tools optional (skip unless trivial).

**UI (Composer):** an **⤴ Export** button next to Save/Open → small dialog:
- If the comp has a song link: "Update '<song title>'" (with the frozen-sections note when any
  apply) OR "Create new song…" (title + preset fields).
- No song link: just "Create new song…".
- On success: navigate to the song (existing router), toast/status line.

## Part B — Done-state "Open in Composer" CTA (queued 2026-07-07)
When a song's status is done, make the Composer the obvious next step:
- In `SongWorkspace`, when `v.status === "done"`, show a prominent CTA (e.g. a highlighted
  "🎹 Open in Composer" primary-styled button or banner near the top — the existing header button
  can become primary-accented in done state; keep it subtle-but-clear, match the aesthetic).
- In the Library, done songs' rows get a small "🎹" affordance opening `/composer?song=<id>`.

## Tests + visuals
- core: export-to-song test (composition sections → Chords/Structure artifacts match, frozen chord
  section skipped and preserved); create-from-composition test (new song's stages populated, key/bpm
  carried). Keep ALL existing tests green (38 core + 2 app).
- Degree→name mapping: a frontend-side unit isn't runnable (no test runner) — put the mapping in a
  pure lib function (`lib/music/compose/compositionToSong.ts`) and, if mapping lives in TS, ALSO
  cover the round-trip in a Rust test by feeding resolved JSON. State what's covered where.
- `scripts/visual_test.py`: screenshot the Export dialog (`composer-export.png`) and the done-state
  CTA (`song-done-cta.png` — mock a done song). Kill port 5173 first.
- Update `docs/COMPOSER-SPEC.md` (requirements #2/#3 → ✅ DONE; the queued CTA block → ✅ DONE) and
  README (Composer bullet: the loop is now bidirectional).

## Definition of done
- `cargo build` + `cargo test` green (paste lines); `cd frontend && npm run build` succeeds; visual
  test green with the two new screenshots. Do NOT commit — the parent verifies and commits.

## Rules
- Respect 🔒 frozen sections on the update-existing-song path (skip + surface, never silently
  overwrite). Reuse existing renderers/conventions (structure/chords text renderers in agent.rs,
  import_lyrics's back-fill pattern, create-flow inputs). Don't duplicate theory. Don't break
  blank-sketch, full-song import, persistence, the lyric sheet, or freeze. Subscription auth
  untouched (no new claude spawns needed). Match the dark neon aesthetic.
