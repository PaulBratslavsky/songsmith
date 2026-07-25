# The render round-trip: AI song in → human-produced song in your DAW

STATUS: Phases 1–3 SHIPPED 2026-07-24 (+ import error handling, the Music.AI add-on,
and the Ableton Reference audio track).

The north star (user, 2026-07-24): *"a tool that sits between Suno and your DAW —
take an AI song idea and turn it into a human-produced song."* This doc maps that
pipeline end to end: what runs where, what can fail, and every fallback.

```
Suno render (audio file)
   │  ⤵ Import reference          (Library)            → a complete song mock
   │  🎼 Analyze → Composer        (Renders tab)        → an editable Composition
   ▼
LOCAL ANALYSIS (analysis/analyze.py — audio never leaves the machine)
   demucs (htdemucs, CPU) ──► vocals.wav + bass.wav stems
   librosa               ──► tempo · key guess · bar chords · section boundaries
   faster-whisper        ──► lyric transcript  (vocals stem → mix fallback ladder)
   basic-pitch (ONNX)    ──► melody + bass note events (seconds, mono-cleaned)
   ▼
REFERENCE ANALYST (Claude) — corrects key from chord content, snaps tempo, cleans
   chords to the diatonic set, derives the form (repeated lyric blocks find the
   choruses), labels sections WITH start_sec.  Note events are STRIPPED from this
   prompt (they're for the Composer's lanes, not for musical reasoning).
   ▼
THE SONG / THE COMPOSITION
   spine + Structure + Chords + Lyrics (transcript lines by start_sec window)
   Concept → Lyric Spec → Prompt via reverse context · everything approved
   render attached with the ANALYSIS STASHED (summary + note events + transcript)
   ▼
COMPOSER (Phase 2–3)
   melody/bass lanes pre-filled from the transcription (diatonic fold)
   render audio follows the transport (waveform strip, nudge, volume)
   section FOCUS: loop one section, A/B original ↔ your lanes, ✓ done marks
   ▼
ABLETON (⚡ export)
   Composer Chords/Melody/Bass as named MIDI tracks
   + the render as a "Reference" AUDIO clip at bar 1 (mute/solo to A/B in Live)
```

## The transcription fallback ladder (lyrics)

Whisper is a speech model; sung and heavily-processed vocals can defeat it. The
analyzer works down a ladder and keeps the richest result:

1. **Vocals stem** (usually cleanest — but demucs misfiles heavily-processed synth
   vocals into "other", leaving a near-silent stem; verified on a real synthwave
   render where even whisper-medium hallucinated).
2. **Full mix** (when the stem yields < 3 segments).
3. **Hallucination guards** (garbage lyrics in sections are worse than none):
   stock instrumental outputs ("Music", "Thanks for watching") are dropped;
   a transcript that's one or two short phrases looping (≥5 segments, <⅓ unique)
   is discarded whole; a single stray segment is discarded.
4. **Music.AI cloud add-on** (opt-in, Settings → "Music.AI lyrics"): only offered by
   ⟳ Resume import when the local ladder heard nothing AND a key + workflow slug are
   configured. This is the ONLY path where audio leaves the machine, per-song and
   user-initiated. `core/src/musicai.rs`.
5. **📋 Paste lyrics** (always available in the resume banner): for your own Suno
   songs the words exist verbatim in Suno — pasting beats any transcription and goes
   through the same verbatim parser as the lyrics-first flow.

An import that can't fill Lyrics says so (⚠ progress line + "finished with issues"
summary) and leaves the stage pending — which is exactly what makes the ⟳ Resume
banner appear.

## Resume (`agent::resume_import`)

Idempotent completion of a partial import (MCP tool `resume_import`, Tauri command,
⟳ banner): rebuilds a missing Lyrics stage from the render's stashed transcript — or
a fresh analyzer pass, with bar-cumsum section times when the stash predates
`start_sec` — runs any missing Claude stages via reverse context, approves everything
with content (surfacing errors the original import swallowed), fills the intent, and
reports exactly what was fixed vs still failing. Survives the song being deleted
mid-resume. Failed Library imports keep the picked file for one-click ⟳ Retry.
(A hung Claude call can't stall an import: `call_claude` timeboxes and
kill-on-drops the CLI.)

## The melody/bass fold (Phase 2)

`frontend/src/lib/music/compose/transcription.ts` — the Composer's lanes are
diatonic (degree 1–7, octave band 0|1), so absolute-MIDI note events fold lossily
ON PURPOSE (an editable sketch in the vocabulary you compose in, not a piano roll):

- quantize to 16th ticks at the summary bpm;
- shift the whole lane by whole octaves so its median lands in the band;
- per note, pick the (degree, octave) whose resolved playback MIDI is nearest,
  preferring exact pitch-class matches (in-scale notes never land on a neighbor;
  chromatic passing notes snap);
- trim quantization overlaps to keep the lanes monophonic.

Verified on a real render: 936/642 events → 765/514 clean spans, zero overlaps,
bass degrees concentrated on 1/6/7 — the song's actual Am/F/G progression family.
Note transcription reads pitch energy, not words — it works even on songs whose
LYRICS are untranscribable.

## Section build-out (Phase 3)

Click a section band block in the Composer to FOCUS it: the loop confines to its
ticks, the render audio re-seeks to the matching slice (any transport jump >8 ticks
or backward re-seeks), and with audio attached an A/B strip picks what you hear —
`🎵 original` (transport-only mute; edit previews stay audible), `🎹 mine`, or both.
`✓ mark done` persists on the composition (`Section.done`) and dims the block —
per-section bookkeeping while you rebuild. Unfocusing resets A/B so no mute lingers.

## Ableton specifics

- The Composer ⚡ export sends frontend-RESOLVED MIDI (the backend stays theory-free)
  as named tracks, then attaches the render as a **"Reference"** audio clip at bar 1
  on its own audio track (remote script patch v3 adds `create_audio_track`; the clip
  import can take up to 60s — the client widens its socket timeout around it, and any
  failure logs a ⚠ line without sinking the MIDI build).
- Song-level ⚡ builds skip profile-disabled parts entirely (pad/arp/drums off in the
  arrangement profile → no track at all — empty clips read as a broken export).
- Remote-script patches live at BOTH
  `~/Library/Preferences/Ableton/Live 12.4.x/User Remote Scripts/AbletonMCP/__init__.py`
  and need a Live restart (or control-surface toggle) to load.

## Where things live

| Piece | Home |
|---|---|
| Analyzer (stems/whisper/basic-pitch) | `analysis/analyze.py` (uv venv; `settings.analyzer_cmd`) |
| Import pipeline + resume | `core/src/agent.rs` (`import_reference_full`, `resume_import`) |
| Stashed analysis | `render.analysis` (JSON: bpm/key/sections+start_sec/melody/bass/transcript) |
| Music.AI client | `core/src/musicai.rs` (opt-in via `settings.musicai_api_key/_workflow`) |
| MIDI→lane fold | `frontend/src/lib/music/compose/transcription.ts` |
| Focus/A-B/done | `Sketchpad.tsx` + `useCompositionPlayback` (`range`/`mute` opts) + `SectionBand.tsx` |
| Ableton builders | `core/src/ableton.rs` (`build_song`, `build_midi_tracks` + reference audio) |
