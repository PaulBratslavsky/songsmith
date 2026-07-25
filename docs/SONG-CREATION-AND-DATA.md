# How a song is created and where its data lives

The map of the song flow: what happens on each creation path, which table owns which
piece of data, and how a stage "sees" everything else when it runs. Companion specs:
`SONG-FACTS.md` (song-level facts), `SECTION-SPINE-SPEC.md` (sections),
`STAGE-FREEZE-AND-LYRICS-IMPORT.md` (freeze + paste).

## The one principle

**Every piece of song data has exactly ONE home. Everything else references it.**
Copies drift — the stale-key bug, the desynced pickers, and the garbage-section bug were
all a second copy disagreeing with the first. When you wonder "where is X stored," the
answer is always a single table below; anything else showing X is a live view of it.

## The data model (one row → many)

```
style_preset          the sound: genre, mood, influences, key/tempo feel, themes,
                      lyric exemplars. Referenced by songs; never copied into them.
song                  THE FACTS (single source of truth):
                        title · intent (the 🎯 north star) · key_root + key_mode ·
                        bpm · status · style_preset_id
section  (the SPINE)  the song's canonical section list — one row per section:
                        id · position · label · type · bars · role
                      Chords, Lyrics, the Composer, Ableton, and every prompt read it.
stage     (6 rows)    concept → structure → chords → lyric_spec → lyrics → prompt
                      Each has a status (pending/running/done) and an approved flag.
artifact              a stage's content, APPEND-ONLY revisions:
                        {kind, text, data} + spine_snapshot, UNIQUE(stage_id, version)
                      "Current" = highest version. Nothing is ever edited in place —
                      regeneration, edits, and restores all append a new version.
composition           Composer sketches (melody/chords/bass), linked to a song when
                      imported from one.
render                final audio takes: label + file path on disk + source
                      ("import" = the audio the song was imported from) + a stashed
                      `analysis` JSON on imports (the Composer-ready summary: bpm/key/
                      sections with start_sec, transcribed melody/bass note events,
                      and the lyric transcript) so 🎼 Analyze → Composer is instant
                      and ⟳ Resume import can rebuild Lyrics without re-analysis.
skill                 the per-stage instructions Claude runs with (builtin + user).
```

Inside a stage artifact, `data` is the truth and `text` is a rendering of it — the
write boundary (`save_artifact_guarded`) rebuilds `text` from `data`, merges 🔒 frozen
sections back in, and keys everything by `section_id` against the spine.

## Path 1 — blank song from a preset

`create_song` (Library → New song):
1. A `song` row is created referencing the preset. Key/BPM are **seeded** from the
   preset's key/tempo feel (parsed, e.g. "A minor around 120"); title is your working
   title; intent is empty until Concept runs (your seed becomes the intent).
2. Six `stage` rows are created, all pending. The spine starts empty — Structure's
   first run proposes sections, and saving the Structure editor births the spine.
3. You run stages in order. Approving a stage advances the song pointer to the next.

## Path 2 — song from pasted lyrics

`create_song_from_lyrics` (Library → New from lyrics), and its sibling
`import_lyrics` (Lyrics stage → **Paste lyrics** button) for an existing song:

1. **Parse** (`parse_pasted_lyrics`) — deterministic, verbatim-guaranteed:
   - Section headers split the text: `[Verse 1]`, `**Verse 1**`, `Chorus:` styles.
     A header must be *section-like* (a known section word, or a short capitalized
     title) — Suno arrangement tags like `[staccato synth lead riff, electronic kick
     drum]` stay in the lyric body, they never become sections.
   - Inline `[F#m]` chord tags are collected per section.
   - Unlabeled text falls back to Claude segmentation, but with a verbatim validator:
     if the model changes ONE word, the whole segmentation is rejected and the paste
     lands as a single section, untouched. Your words are never rewritten.
2. **Key inference** (creation path only) — if the paste carries chord tags, the song's
   key is inferred from them (a `[D#m]`-heavy paste makes a D#-minor song). Importing
   into an existing song never touches the key — you may have set it on purpose.
3. **Spine replace** (`apply_parsed_lyrics` → `sync_spine`) — the pasted labels/order
   BECOME the spine (paste is user authority): matched labels keep their row ids (and
   locks), new labels get rows, unmatched old rows are deleted.
4. **Artifacts** — the Lyrics stage gets a `done` artifact holding your words verbatim,
   keyed by `section_id`; the Structure stage is back-filled (section map from the
   spine, chords from the tags land in the Chords stage). Concept stays blank — the
   flow then runs "in reverse" (see below).

## Path 3 — import from an AI render (audio file)

`import_reference` (Library → **⤵ Import reference**) turns a finished AI song (e.g.
a Suno render) into a COMPLETE song mock. Everything runs locally except the three
Claude stage writes (see `RENDER-ROUNDTRIP.md` for the full pipeline and its failure
ladder):

1. **Perception (local)** — `analysis/analyze.py`: demucs stem separation, tempo/key/
   bar-chords/section boundaries (librosa), whisper lyric transcription (vocals stem →
   full-mix fallback, hallucination guards), basic-pitch melody/bass note events.
2. **Cognition (Claude)** — the Reference Analyst skill cleans the raw analysis into a
   real Structure (labeled sections WITH `start_sec`) + Chords.
3. The analysis' sections become the new song's **spine**; Structure + Chords artifacts
   save against it; each transcript line lands in the section whose `[start_sec, next)`
   window contains it → the Lyrics artifact (verbatim words).
4. **Reverse context**: Concept → Lyric Spec → Generation Prompt run from the imported
   content; every stage with content is approved; the song's 🎯 intent fills from the
   concept's theme.
5. The audio attaches as the song's first `render` (source "import") with the analysis
   stashed, and the import ends with an HONEST summary — any skipped piece is named.

**When it doesn't finish** (untranscribable vocals, a failed Claude call, app quit):
the song workspace shows the **⟳ Resume import** banner — resume rebuilds a missing
Lyrics stage from the stashed transcript (or a fresh local pass, or the opt-in
Music.AI cloud add-on when configured in Settings), runs missing stages, approves
everything, and reports fixed-vs-still-failing. **📋 Paste lyrics** in the same banner
is the surest fix for your own Suno songs: the words already exist in Suno and map
onto the sections verbatim.

## "Shouldn't the pasted lyrics be global context?" — they are

The lyrics are NOT copied into a song-level field, for the same reason key/BPM moved
OUT of stage data: one home, no drift. The single home for the sung words is the
**lyrics artifact** (+ the spine for their sectioning). They become global context
through prompt assembly:

Every stage run builds its prompt from LIVE references, not stored copies:
- **System prompt**: the stage's skill + the full style preset + **THE SONG block**
  (title + 🎯 intent — your north star is in every stage, every time).
- **User prompt**, in order:
  - the **SECTIONS block** — the spine, verbatim, when it exists;
  - **earlier stages' content**, rendered fresh from each artifact's `data`;
  - **later stages' content** — this is the reverse-context path: when you start from
    lyrics, an empty Concept/Structure/Chords stage is told the later stages are the
    SOURCE OF TRUTH to derive from; a stage that already has content gets them as
    reference-only, so a regen can't copy stale downstream text back;
  - the computed **TECHNICAL BRIEF** (lyrics stage: bars/chords/tempo → line and
    syllable budgets);
  - structure runs get the **KEY/TEMPO AUTHORITY block** (the song's key/BPM, with
    stale earlier-stage mentions explicitly overruled);
  - your seed text.

So a pasted lyric is referenced by every other stage automatically — Concept derives
the story from it, Chords voices it, the Prompt stage quotes it — all reading the one
copy in the lyrics artifact.

## Regenerate · Approve · History (what each button really does)

- **Run** (first time) saves the result directly as revision v1. **Re-run** parks
  its output as a PENDING DRAFT (one per stage, stored outside History): the
  current version stays live until you click **Accept draft** — which re-guards
  the content (frozen sections, spine) and journals it as a new revision — or
  **Discard**, which deletes it without a trace. A newer re-run replaces an
  unreviewed draft. (Built 2026-07-16; closes the "it updated without approve"
  gap noted below.)
- **Approve & advance** does NOT accept content — content is already saved. It marks
  the stage done and moves the song to the next stage (it's the advancement gate,
  and downstream stages treat approved content as settled).
- **🔒 Freeze** is the "don't touch this" control: a frozen section survives any
  regeneration byte-for-byte, enforced at the write boundary (even for saves coming
  from chat/MCP).
- **History** shows every revision with diffs; restoring appends the old content as a
  new revision (append-only, so a restore is also undoable) and restores the section
  spine from the revision's snapshot when it differs.

(The 2026-07-15 known gap — Re-run replacing content without approval — is closed by
the regenerate-as-draft flow above.)

## Where to look when something's wrong

| Symptom | First place to look |
|---|---|
| Key/BPM "wrong" anywhere | `song.key_root/key_mode/bpm` — every picker and prompt is a view of it |
| Sections mismatched between stages | the `section` spine vs each artifact's `section_id` keys |
| Chords won't align with lyrics | the lyrics artifact's sections vs the chords artifact's — they align only via shared spine rows |
| Content changed unexpectedly | the stage's revision History (nothing is ever lost) |
| A stage ignoring context | its assembled prompt: skill + preset + THE SONG + spine + prior/later context |
