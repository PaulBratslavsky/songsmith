# Drift report — 2026-10-28

EGM drift report (`/egm:egm-drift-report`) comparing the design docs in `docs/`
against the code at commit `HEAD` of `import-accuracy-and-additive-takes`.
Checked: COMPOSER-SPEC, IMPORT-ACCURACY-PLAN, RENDER-ROUNDTRIP,
SECTION-SPINE-SPEC, SONG-CREATION-AND-DATA, SONG-FACTS,
STAGE-FREEZE-AND-LYRICS-IMPORT. Skipped: BACKLOG.md and AUDIT-2026-07-06.md
(not designs). Each doc was checked by a separate read-only agent. Only
divergences are listed. "(planned)" marks items the doc itself calls future work.

## Behaviour bugs found (verified by hand)

Most drift below is stale docs. These items are code that breaks a rule the
docs or `CLAUDE.md` set, and each was re-checked directly in the source:

1. ✅ **Fixed 2026-10-28** — a Structure draft now only proposes sections;
   they land on Accept (`spine::apply_structure_proposal`). Test:
   `flow_structure_draft_touches_spine_only_on_accept`. Original finding:
   **Discarding a Structure draft does not undo its section changes.**
   `run_stage` calls `spine::build_run_content` (`core/src/agent.rs:107`), which
   runs `reconcile_structure_run` and creates/renames/deletes/reorders `section`
   rows (`core/src/spine.rs:399-406`) *before* the output is parked as a draft
   (`agent.rs:114-117`). `discard_stage_draft` only deletes the draft row
   (`agent.rs:146-148`). Self-check has the same path (`agent.rs:208-213`).
2. ✅ **Fixed 2026-10-29** — the editor keeps `frozen` through load/save and
   has a 🔒/🔓 toggle per beat (verified in the browser mock: lock → save →
   reload → save → reload stays locked). Core test:
   `locked_lyric_spec_beat_survives_regeneration`. Original finding:
   **Saving the Lyric Spec editor unlocks every beat.** `parseSpec` maps beats to
   `{section_id, section, beat}` and drops `frozen`
   (`frontend/src/components/LyricSpecEditor.tsx:26-32`), and the save writes those
   beats back (`:97-105`). There is also no lock toggle in this editor.
3. ✅ **Fixed 2026-10-29** — `update_progression` is now PARTIAL (only `id`
   required; omitted fields keep their value, `picks: ""` clears). Verified
   through the real mcp-shim; test `update_progression_is_partial`. Original
   finding: **`update_progression` (MCP) is a full replace.** Omitting `picks` writes `""`
   and wipes saved picks (`core/src/tools.rs:360-365`, `core/src/db.rs:1179-1183`).
   Breaks the "`update_*` tools are PARTIAL" house rule.
4. **The Composer keeps its own key/BPM for a song-linked sketch.** Its Key/Tempo
   pickers only change local state (`frontend/src/lib/music/compose/useCompositionState.ts:84-99`)
   and nothing in `components/compose/` or `ComposerRoute.tsx` calls
   `updateSongKey`. "Update linked song" then writes chords resolved in the
   Composer's key without changing `song.key_root`.
5. ✅ **Fixed 2026-10-29** — over MCP, `delete_section` refuses a section
   that Chords / Lyric Spec / Lyrics has content or a 🔒 lock for, and
   `update_section` refuses a locked section (`spine::section_use`). The
   user's own editor is unaffected (Tauri calls the db directly).
   `reorder_sections` is left as-is: it already requires the full id list
   and loses nothing. The `destructive` flag is still not sent to MCP
   clients. Verified through the real mcp-shim; test
   `section_tools_respect_locks_and_content`. Original finding:
   **Claude can rename or delete a locked or content-bearing section.** MCP
   `update_section` / `delete_section` / `reorder_sections` call the db functions
   with no lock or content check (`core/src/tools.rs:283-299`,
   `core/src/db.rs:819-838`). The `destructive: true` flag is never sent to MCP
   clients (`mcp-shim/src/main.rs:68-72`).
6. ✅ **Fixed 2026-10-29** — `structure.md` no longer asks for `key`/`bpm`
   (notes only), and the legacy path now DROPS them
   (`strip_structure_key_tempo`, was `enforce_song_key_tempo`) instead of
   storing a copy of the song's values. Test
   `structure_output_never_stores_key_tempo`. Original finding:
   **The structure skill still asks the model for key/BPM.**
   `core/src/skills/structure.md:16,24` says "ECHO them exactly in the `key`/`bpm`
   fields". The spine path throws them away, but the legacy fallback
   (`spine.rs:359-376`) stores them via `enforce_song_key_tempo`.

---

## COMPOSER-SPEC.md

- **Stale reference: `Composer.tsx` no longer exists.** Doc l.13 and the port table
  (l.85) name it as top-level wiring. It is now
  `frontend/src/components/compose/Sketchpad.tsx` (renamed in 097cdab), imported at
  `frontend/src/routes/ComposerRoute.tsx:5`.
- **Stale file lists (l.7-8, l.12-13).** Missing from them:
  - in `components/compose/`: `useLanePointer.ts`, `SectionBand.tsx`, `ExportDialog.tsx`, `LyricSheet.tsx`, `NotationView.tsx`
  - in `lib/music/compose/`: `transcription.ts`, `useRenderAudio.ts`, `notation.ts`
- **Stale data model (l.95-107).**
  - The doc gives `BARS = 8, TOTAL_TICKS = 128`. The code has `DEFAULT_BARS` and `DEFAULT_TOTAL_TICKS`, plus a new `MAX_TOTAL_TICKS = 16_000` (`frontend/src/lib/music/compose/types.ts:27-31`).
  - `ChordSpan` has a `seventh: boolean` (`types.ts:52-57`).
  - `Composition` has `bars`, `totalTicks`, `sections` and `lyrics`, with `SCHEMA_VERSION = 3` (`types.ts:114,121-142`).
- **Incomplete model: `Section`.** Doc l.170 gives `{id,name,startTick,lengthTicks}`. The code adds `done?` and `section_id?` (`types.ts:67-80`).
- **Stale "Known constraints" (l.134-136).**
  - "8 bars / 4/4 fixed" was lifted in Phase 4.
  - "7th dropped at resolve" is wrong: `resolveChordMidis` voices the 7th (`playback.ts:79-90`).
  - "`setTimeout` clock" is wrong: N2 replaced it with a lookahead scheduler (`useCompositionPlayback.ts:6-20,46-49,255`).
- **Imported melody/bass lanes are not empty.** Doc l.22 and l.178 say they are. In the code, `applyTakesToComposition` pre-fills them from takes, and `bassFromChordRoots` builds a bass line from chord roots when no bass take exists (`compositionFromSong.ts:189-257`, c1dbde5).
- **Section length comes from spine bars, not the sum of placements.** Doc l.247-256 says the sum. `padSectionToBars` makes the spine bar count own the length, and the spine also owns order and labels (`compositionFromSong.ts:259-296,362-401`).
- **"Open in Composer" is not always active.** Doc l.205 says it is. It is `disabled={!allStagesDone}` (`frontend/src/routes/SongWorkspace.tsx:300-310`).
- **Export backend differs.**
  - Doc l.39-42 says Structure is rendered with `structure_editor_text`. The code uses `render::structure_spine_text` (`core/src/agent.rs:2007-2009,2071-2072`; `core/src/render.rs:106`).
  - The export also syncs the spine, writes `spine_snapshot` and `imported: true` (`agent.rs:1986-2000,2054-2066`), and returns `skipped_frozen`.
  - `merge_frozen_sections` lives in `core/src/freeze.rs:74`.
  - `ExportDialog.tsx:85-96` lists bar-count changes before confirming.
- **N3 (MIDI input) is built but not marked done, and the build differs (l.328-330).**
  - `midir` is in `app/src-tauri/Cargo.toml:20`, not in `core/`.
  - The commands are `midi_list_inputs` / `midi_open_input` / `midi_close_input` (`app/src-tauri/src/lib.rs:1323-1357`).
  - Notes are sent as a global `app.emit("midi_note")` event (`lib.rs:1343`), not over a Tauri Channel.
- **(planned) Chat-driven composition editing (l.334-336).** The MCP `save_composition` tool exists (`core/src/tools.rs:74-77`), but nothing reloads the Composer when the row changes. The only `listen` in the Composer is `midi_note` (`Sketchpad.tsx:313`).
- **Undocumented logic.**
  - Changing the key transposes absolute chord names (`useCompositionState.ts:84-96`).
  - Route params `?prog=`, `?comp=`, `?audio=`, `?nudge=` (`ComposerRoute.tsx:24-54,124-128`).
  - Reference-audio playback (`useRenderAudio.ts`).
  - Analysis/stem-transcription import (`compositionFromAnalysis`, `transcription.ts`).
  - Section focus/loop, A/B, and the ✓ done toggle (`Sketchpad.tsx:200-210`).
  - "⚡ Ableton" via `ableton_build_composition` (`Sketchpad.tsx:829-858`, `lib.rs:1274`).
- **Formatting.** l.334 repeats its heading twice on one line.
- Missing links: Phase 1-2 file lists, the export backend (`agent.rs`, `freeze.rs`, `render.rs`), persistence (`models.rs`, `lib.rs`, `ipc/mockApi.ts`), N2 (`music/synth.ts`, `soundfonts.ts`), N3 (`lib.rs` midi_*).

## IMPORT-ACCURACY-PLAN.md

- **Tiers done in code but still marked open.** The header (l.3) lists only T1.0, T1.4, T1.5 and T0.1 as shipped. Commit 3ab7e5e also shipped:
  - **T3.2 (surface `uncertain`).** It is echoed as progress (`core/src/agent.rs:592-596`), stored (`:627-635`) and rendered as "⚠ CHECK THESE" (`core/src/render.rs:73-77`). §3.10 (l.334-335) and §4 (l.368) say it is unused.
  - **T0.2, mostly.** `analysis_summary` has a `measured` snapshot (`agent.rs:1766-1785`). §3.8 (l.300-305) is stale.
  - **T3.5, partly.** `setChordDegree` drops `name` (`spans.ts:125-135`), and a key-root change transposes named chords in the frontend and backend (`useCompositionState.ts:84-95`; `agent.rs:1658`; `midi.rs:816`). §3.11 (l.345-355) is stale.
- **T0.2 incomplete.** `measured` holds `section_count`, not `sections[]` (doc l.407, l.655). There is no `raw` key and no `analysis_raw` column (`core/src/db.rs:104`).
- **(planned) T3.5 remainder.** `setKeyMode` leaves named chords alone (`useCompositionState.ts:97-98`), and the Composer has no chord-name input.
- **T4.3 adapter.** The doc says an unmapped quality falls back to `'X'` and is logged (l.590). `harte()` collapses it to the triad as "degraded-quality" (`analysis/eval.py:150-157,262-264`).
- **T4.4 metrics not as specified.**
  - Downbeat F is replaced by a single first-downbeat ±70 ms check (`eval.py:278-283`).
  - Beat F / CMLt / AMLt are skipped (`eval.py:274-275`), and `mir_eval.beat.evaluate` is never called. Only key and segment evaluation are wired (`eval.py:238,295`). This conflicts with "Shipped" (l.539-540) and Day-2 step 5 (l.660).
  - `tempo_resid_ms`, Pairwise F, NCE and over/underseg are not reported (`eval.py:330-331`).
- **T4.6 gating not implemented.** `eval.py main()` always returns 0 and only prints a delta when `--baseline` is passed (`eval.py:385-404`). `make analyzercheck` passes no baseline (`Makefile:33-35`).
- **(planned) T4 remainder.**
  - No `analyzercheck-llm` target.
  - No `llm_to_intervals`.
  - No demucs stem cache: `run_analyzer` re-runs `analyze.py` every time (`eval.py:321-324`).
  - `new-song-idea-343` has `truth.json` but no audio.
- **Stale: "nothing reads `.als`" (l.575).** `analysis/als_truth.py:81` `read_als` exists, and the doc's own l.534 says so.
- **Stale: the bare-string schema blocker (l.245-247, l.390).** `core/src/skills/reference.md:19-22` uses `{name, beats}` objects (T0.1 is done).
- **Tempo precision.** The doc claims ~0.01 BPM (l.374), but `analyze.py:415` emits `round(tempo, 1)`.
- **Date mismatch.** The doc says the gate is "Gated 2026-08-11" (l.435). The comment on `RESID_GATE` says it was calibrated 2026-08-03 (`analyze.py:133-149`).
- **Undocumented: a second tempo threshold.** `reference.md:7,11` treats `tempo_resid_ms` above ~40 ms as shaky. The analyzer gates on 5% of the beat period (`analyze.py:201`).
- **Undocumented: BPM range filter.** `json_bpm` discards values outside 20..=300 and falls back to 120 (`agent.rs:1789-1798`). §3.10 (l.329) describes a bare `unwrap_or(120)`.
- **Stale line refs throughout (code moved after `138d9d6`).**
  - `analyze.py` shifted ~70 lines because of `refine_tempo` (e.g. tempo `:183` is now `:251`, whisper `:323` is now `:395`).
  - Other moves:
    - `tools.rs` `run_analyzer` is now `:128-166`
    - `agent.rs` `analysis_summary` is now `:1738-1786`
    - `agent.rs` `analyze_for_composer` is now `:1707-1720`
    - `db.rs` `update_song_key` is now `:711-714`
    - `spans.ts` is now `:125-135`
    - `useCompositionState.ts` is now `:84-98`

## RENDER-ROUNDTRIP.md

- **Stale: "nothing consumes `tempo_resid_ms`/`tempo_refined`" (l.128-130).** `core/src/skills/reference.md:7,11` reads them and flags problems in `uncertain`. `agent.rs:1776-1778` stores them.
- **Incomplete model: `render.analysis` fields (l.32-33, l.264).** The doc's list is missing:
  - the `measured` snapshot: duration, tempo_bpm/coarse/resid/refined, key, section_count, bar_chords (`agent.rs:1766-1781`)
  - separate `key_root`/`key_mode` fields (`:1755-1756`)
- **Undocumented: chord-root bass fallback** in `applyTakesToComposition` (`compositionFromSong.ts:224-252`, c1dbde5).
- **Stale: home of `applyTakesToComposition` (l.270).** It is defined in `lib/music/compose/compositionFromSong.ts:189`, not ComposerRoute.
- **Stale counts (l.278).**
  - The doc says 128 Rust tests. There are 118 `#[test]`/`#[tokio::test]` functions in `core/src` plus 18 `ts(export)` types; neither total is 128, and `CLAUDE.md` says 129. The suite was not run for this report.
  - The doc says 8 flow scenarios. `flow_tests.rs` has 12 (8 non-resume + 4 resume) plus `part_write_streams_tokens_and_honours_cancel`.
- **Test-nets table is missing the analyzer nets.** `make analyzertest` (`Makefile:27-29`) and `make analyzercheck` (`Makefile:33-35`, `eval.py`, `bench/baseline.json`) are not in it.
- **Undocumented: the eval harness exists.** `analysis/eval.py`, `als_truth.py`, `make_fixture.py` and `bench/` are there. The doc treats the T4 harness as future (l.124) and lists only `analyze.py` (l.262).
- **(planned) `RESID_GATE` sweep.** It is still hardcoded at 0.05 (`analyze.py:149`), and `eval.py` has no sweep.
- **(planned) `beat_this` beat tracking (l.121).** Absent from the code.
- **Stale: remote-script install path (l.254-256).** The doc says "BOTH" locations but names one. `install_ableton_script` writes to every Live install under `~/Library/Preferences/Ableton` (`app/src-tauri/src/lib.rs:216-233`).
- **Undocumented: the Composer export deletes tracks.** The doc stresses that pushes are additive (l.239-251). `build_midi_tracks` sends `clear_named_tracks` for every exported name plus "Reference" (`core/src/ableton.rs:445-449`), so hand edits on same-named Live tracks are lost.
- **Undocumented: duplicate-label matching.** `clamp_part_take` falls back to the first match once same-label entries run out (`core/src/midi.rs:283-292`). `takes_for_sections` is strictly consume-once (`ableton.rs:656-670`).
- **Undocumented config.** `SONGSMITH_CLAUDE_TIMEOUT_SECS` overrides the `call_claude` timeout (`core/src/engine.rs:44-51,173-177`).
- Missing links: the transcription ladder (l.161-184 → `analyze.py:361-403`), and the analyzer/flowcheck tests (`test_analyze.py`, `eval.py`, `scripts/flowcheck.py`).

## SECTION-SPINE-SPEC.md

- **A Structure regenerate changes the spine before Accept.** See bug #1 above. Neither Accept nor Discard reconciles the spine: the guarded structure save never touches it (`freeze.rs:294-306`).
- **Second copy of section data keyed by label: Melodist/Arranger takes.** `song_melody`/`song_part` store `{sections:[{label, notes}]}` (`core/src/db.rs:119-139`, `midi.rs:267-290`). Every read and write matches on label:
  - the rewrite splice (`agent.rs:950,1024-1031`)
  - `takes_for_sections` (`ableton.rs:654-660`)
  - `applyTakesToComposition` (`compositionFromSong.ts:200`)
  - the rewrite picker (`SongWorkspace.tsx:335`)

  `ableton::song_parts` also drops `section_id` (`ableton.rs:131-138`). Renaming a section on the spine detaches its takes.
- **Second copy: saved compositions linked to a song.** `composition` rows store full sections (`db.rs:74-77`), and FinalRenders saves one per analyzed render (`FinalRenders.tsx:29-31`). Opening one via `?comp=` and exporting it REPLACEs the spine through `sync_spine` (`spine.rs:483-487`).
- **StructureEditor's section list is not live.** It is seeded once in `useState` (`StructureEditor.tsx:80`) and remounts only when the artifact id changes (`SongWorkspace.tsx:448-449`). If the spine changes elsewhere, a save deletes the rows missing from its stale list (`StructureEditor.tsx:111-114`). SectionChordsEditor re-seeds correctly (`SectionChordsEditor.tsx:105-108`).
- **"Deletion of a content-bearing section is a user action only" (l.55-56) is not enforced.** See bug #5.
- **No fuzzy label match (l.43-45).** `attach_ids_non_structure` matches by id, then by exact normalized label (`spine.rs:184-188`, `freeze.rs:52-54`).
- **Duplicate labels share one id.** `import_reference` matches with a first-match `find` on label (`agent.rs:646`), and so does Composer export for new rows (`agent.rs:1996`). Two "Chorus" rows get the same id.
- **Stale: structure artifact shape (l.151-153).** `import_reference` also writes `uncertain` and `imported: true` (`agent.rs:631-636`).
- **Self-contradiction: entry shapes (l.31-33).** The code keeps required `label`/`section` fields alongside `section_id` (`frontend/src/lib/artifacts.ts:51-52,80-81,188`).
- **Stale count.** The doc says "97 core + 2 app tests" (l.181). Core now has 118 test functions.
- **Undocumented: re-migration.** A song with zero spine rows is re-migrated on every startup (`db.rs:207-209`), so deleting every section brings them back on the next launch.
- Missing links: only `spine.rs` and `lib/sections.ts` have paths. Takes storage, compositions and stage drafts are not covered.

## SONG-CREATION-AND-DATA.md

- **Stage status values.** The doc says `pending/running/done` plus an approved flag (l.26-27). The code uses `in_progress` (`agent.rs:79`, `models.rs:83`). Approval is per revision on `artifact.approved` (`db.rs:59-63`, `tools.rs:187-195`), and a stage stays `in_progress` after its first run until approved (`agent.rs:115-127`).
- **Tables missing from the model (l.17-47):** `stage_draft` (`db.rs:142-151`), `progression` (`:71`), `outline` (`:107-116`), `setting` (`:84`).
- **Columns missing from the model:**
  - `song.current_stage`, `song.voicings` (`db.rs:49,96`)
  - `style_preset.vocal_range` and `arrangement` (`:44,100`)
  - `artifact.version`, `approved` and `label`
  - `render.is_pick` and `notes`
  - the `imported: true` artifact marker (`db.rs:625-628`, `models.rs:86-101`)
- **The spine is also born by a Structure run** (`spine.rs:399-407`), not only by saving the editor (l.59-60).
- **History restore only re-creates missing rows.** Existing rows keep their current label and position (`spine.rs:76-104`), and re-created rows get default type, bars and role (`db.rs:798`). The doc (l.159-161) says it restores the spine when it differs.
- **Approval doesn't change what later stages see.** `gather_prior_context` uses the current artifact whether approved or not (`agent.rs:325-336`). The prompt header "Approved outputs…" (`agent.rs:536`) and the `run_stage` tool description (`tools.rs:52`) are stale.
- **The user's editor saves skip the write boundary.** The doc (l.49-51) describes `save_artifact_guarded` as the boundary. The Tauri `save_artifact` calls `db::save_artifact` directly (`app/src-tauri/src/lib.rs:488-490`). For spineless songs, the guarded path writes no `spine_snapshot` (`freeze.rs:342-344`).
- **Some artifacts are edited in place.** The doc says "Nothing is ever edited in place" (l.30-31). `migrate_sections` UPDATEs existing artifacts (`db.rs:199-205`).
- **Second key/BPM copy on legacy songs.** `enforce_song_key_tempo` writes them into structure data (`agent.rs:512-525`).
- **Saved compositions copy song data** (key, bpm, sections, chords, melody, bass, lyrics), against the one-home rule (`types.ts:120-142`).
- **Full-replace updates.**
  - MCP `update_progression`; see bug #3.
  - Tauri `update_style_preset`, `update_section`, `update_skill` and `update_progression` are also full replaces (`lib.rs:48,318,524,543`). Only the MCP preset/skill/section arms are partial.
- **Hardcoded key/BPM fallback.** `create_song` falls back to A minor / 120 when parsing fails (`db.rs:659-660`), and so does import (`agent.rs:613-614,1755-1757`).
- **The doc's example doesn't parse.** `parse_feel_bpm` only reads a number placed right before "BPM" (`db.rs:565-580`), so "A minor around 120" (l.57) gets the hardcoded 120.
- **Import picks the preset.** It takes the first preset, or creates an empty "Imported" one, and uses the file name as the title (`agent.rs:598-608`). Path 3 doesn't say this.
- **Path 3 makes four Claude calls, not three (l.91-92).** The Reference Analyst is also a Claude call (`agent.rs:584-588`).
- **`advance_song` goes to the first unfinished or stale stage**, which can be an earlier one (`tools.rs:197-226`). The doc (l.61) says it moves to the next stage.
- **Undocumented: a fourth creation path.** `create_song_from_composition` creates a song, and `export_composition_to_song` replaces the spine and Chords/Structure (`agent.rs:1957-2071`).
- **Undocumented: a key change transposes chords** as a new revision (`agent.rs:1641-1702`, `lib.rs:107-113`).
- **Undocumented: self-check and cancel.** `self_check_stage` always produces a draft (`agent.rs:154-224`), and `cancel_stage` rescues stuck stages (`lib.rs:437-465`).
- **Paste drops lyric locks.** The doc (l.81-82) says locks are kept. `apply_parsed_lyrics` writes no frozen flags into the new Lyrics artifact and replaces the whole Chords artifact when the paste has tags (`agent.rs:1489-1498,1554-1595`).
- **Intent falls back to `hook`** when there is no theme (`agent.rs:722-733`).
- Missing links: no source files are named except `analyze.py`.

## SONG-FACTS.md

- **The structure skill still asks for key/BPM** (`core/src/skills/structure.md:16,24`). See bug #6.
- **The transitional guard writes facts instead of stripping them.** `enforce_song_key_tempo` always writes key/BPM into structure data (`agent.rs:518-519`) on the legacy fallback (`spine.rs:359-376`), which is still reachable when the output has no JSON or no sections.
- **A guarded save on a spineless song keeps whatever key/BPM the model sends**, with no strip or splice (`freeze.rs:335-348`).
- **The renderer reads facts from artifact data, not the song.** `structure_editor_text(d: &Value)` renders KEY/TEMPO from `d.key`/`d.bpm` (`render.rs:53-63`), against Rule 4 (l.21-22).
- **Prompts carry stale key lines.** Prior context renders through `structure_editor_text` (`agent.rs:311-316,334,350`), so legacy rows put a stale `**KEY:**` line next to the live key (`agent.rs:472`). `change_song_key` doesn't re-render structure (`agent.rs:1660-1670`).
- **The Composer keeps its own key/BPM.** See bug #4. The copy also drives the Ableton tempo (`Sketchpad.tsx:852`) and the export (`ExportDialog.tsx:117-118`).
- **Render analyses store their own key/BPM** (`agent.rs:1755-1757`). Analyze→Composer reads them, not the song (`FinalRenders.tsx:27-30`, `compositionFromSong.ts:154-160`).
- **The concept artifact stores its own `title`** (`skills/concept.md:27`, `ConceptEditor.tsx:26,76`). It is copied to the song only on an explicit click (`:69,77`).
- **The Generation Prompt bakes key/BPM into `stylePrompt`** (`skills/prompt.md:20,32`), and nothing refreshes it after a change.
- **Undocumented: changing the key writes an artifact.** `update_song_key` goes through `change_song_key`, which saves a transposed chords revision (`lib.rs:107-113`, `agent.rs:1640-1670`). The doc (l.17-19) says it edits the song only.
- **Intent is set from model artifacts.** It comes from concept `theme`/`hook` on import (`agent.rs:729-737`, with no empty check) and on resume (`:889-897`), as well as from the seed (`:72-75`).
- **No MCP tool for key/BPM.** There is no `update_song_key` in `tools.rs` or `mcp-shim`, so an agent can't use the sanctioned path.
- **Stale: Structure data no longer has `sections`** (l.15-16); it is now `{keyNote, tempoNote}`, plus `uncertain` on imports.
- **Stale code comments.**
  - `agent.rs:509-510` says the spine path splices key/BPM. It drops them (`spine.rs:404-410`).
  - `StructureEditor.tsx:10-15` says root/mode/bpm are "never part of this draft", but `StructureData` still carries copies that are never read.
- **(planned) Time signature.** There is no column, and 4/4 is hardcoded as `BEATS_PER_BAR = 4` (`types.ts:24`).
- Missing links: no source files are named. The Composer, render analysis, concept title and MCP areas are not covered.

## STAGE-FREEZE-AND-LYRICS-IMPORT.md

- **No 🔒 toggle in StructureEditor (l.22, l.43-45).** It has only ↑/↓/× buttons (`StructureEditor.tsx:113-116`), and its save writes `{keyNote, tempoNote}` only (`:138-143`). Since Phase 4, Structure locks can't be set from the UI, but `freeze.rs:26-28` still lists `structure` as a freezable stage.
- **No 🔒 toggle in LyricSpecEditor, and saving it unlocks beats.** See bug #2.
- **Lyric Spec locks live in `data.beats[]`, keyed by `section`** (`freeze.rs:4-5,34-38`). The doc (l.27) says `data.sections[]`.
- **Lyrics sections have no `type`.** The doc (l.9) gives `{label,type,lines}`. In the code:
  - `ParsedSection` is `{label, lines}` (`agent.rs:1186-1189`)
  - the import saves `{section_id,label,lines}` (`:1551-1552`)
  - `LyricsEditor.tsx:288` saves no `type`
- **The Structure artifact holds no section list.** It is `{keyNote, tempoNote}`, and sections live in the `section` table (`agent.rs:1564-1571`, `freeze.rs:294-307`). The doc (l.10, l.89-90) is stale.
- **Claude can bypass locks.**
  - MCP section tools: see bug #5.
  - MCP `import_lyrics` (`tools.rs:38,265`) replaces locked Lyrics and Chords with no warning. The doc (l.65) makes that a user action shown with a warning.
- **A paste replaces locked Chords too, but only Lyrics locks trigger the warning** (`agent.rs:1574-1596`, `LyricsEditor.tsx:204-207,235-236`).
- **Freeze code lives in `freeze.rs`, not `agent.rs`** (doc l.18). `merge_frozen_sections` is at `freeze.rs:74` and `frozen_prompt_block` at `:135`; agent.rs only re-exports them.
- **The splice matches by `section_id` first, then by label** (`freeze.rs:68-72,116-119`). The doc (l.20, l.36) says label only.
- **Run output goes through `spine::build_run_content`** (`agent.rs:107,211`; `spine.rs:352`), not a direct splice-and-save (l.20-21).
- **Re-runs and self-checks become drafts**, not revisions (`agent.rs:110-117,213`), and `accept_stage_draft` re-guards them (`:138-144`). The doc (l.8, l.38) says revisions.
- **The chords editor is `SectionChordsEditor.tsx`** (lock at `:125,266-269`), not `Composer.tsx` (l.22, l.44).
- **Undocumented: the write-boundary trust model.** `save_artifact_guarded` / `revert_artifact_guarded` guard Claude's MCP saves (`freeze.rs:236-372`, `tools.rs:313-319`). The user's Tauri save is the unlock authority (`lib.rs:482-490`).
- **Undocumented: saves are refused or rewritten.**
  - Unparseable model output is refused when the prior artifact has locks (`freeze.rs:191-195,259-262`).
  - `text` is regenerated from `data` on every guarded save, even when nothing is locked (`freeze.rs:340-348`). This contradicts "identical to before" (l.23-24).
- **Undocumented: other lock interactions.**
  - Composer export skips locked sections (`agent.rs:1957`, `ExportDialog.tsx:175-178`).
  - `change_song_key` transposes locked chord sections too (`agent.rs:1658-1697`).
- **A third paste entry point.** The "📋 Paste lyrics" banner calls `importLyrics` with no preview or lock warning (`SongBanners.tsx:61-70,89-105`). The doc (l.73-78) lists two.
- **The parser accepts more headers than documented.** It also accepts `**Verse 1**` and has a `section_like` filter for Suno tags (`agent.rs:1214-1245`).
- **Undocumented: `imported: true`** stops staleness flags (`agent.rs:1553-1556,1569,1592`), and a paste replaces the whole spine through `sync_spine` (`:1542-1546`).
- **The derive banner applies only to empty stages.** Regenerations use `LATER_STAGES_REGEN_BANNER` (`agent.rs:367,182,450-452,541`). The doc (l.104-108) says every stage.
- **Undocumented extras.** The MCP tool `create_song_from_lyrics` (`tools.rs:37`), and an extra optional `intent` on the Tauri command (`lib.rs:359-366`).
- **Self-contradiction on reordering (l.85-86).** The doc says the validator accepts a reordering. `rebuild_from_input` rejects any reorder or dropped line (`agent.rs:1419-1464`), which matches l.61-62.
- **Stale counts.** The doc says 5 tests each for A and B (l.21, l.69). There are about 14 freeze tests and about 11 paste tests.
- **Stale banner text** (l.105-106). The actual constant is `LATER_STAGES_BANNER` (`agent.rs:360`).
- Missing links: MCP tools, the freeze write boundary (`freeze.rs`, `tools.rs`, `mcp-shim`), the spine interaction, the draft flow and the unlock trust model.
