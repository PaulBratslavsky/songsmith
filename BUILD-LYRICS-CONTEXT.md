# Build brief — richer context for the Lyricist (technical brief + craft examples)

Latest commit 7d7158e. The user: "the songwriting skill needs more context to write better lyrics."
Authorities: `core/src/skills/lyrics.md` (just overhauled — extend, don't weaken), `docs/AUDIT-2026-07-06.md`
conventions. ⚠ User may be live-testing: never kill port 5173, never run visual_test.py.

## Problem
The Lyricist gets craft rules + prior stage text, but ZERO quantitative context: it doesn't know a
section's bar count, the tempo's implication for line length, or how many chord changes a section
carries — so line counts and syllable weights wander (unsingable long lines, sections with the wrong
number of lines for their bars).

## 1. Computed TECHNICAL BRIEF (code — core)
When running the **lyrics** stage (and its self-check), compute and inject a per-section brief from
real data — Structure's `data.sections` ({label, bars}), Chords' `data.sections` ({label,
chords:[{name,beats}]}), and the song's BPM:
```
----- TECHNICAL BRIEF (computed from this song's structure/chords/tempo — honor it) -----
Tempo: 120 BPM half-time feel → a comfortable sung line is roughly 6-10 syllables.
- Verse 1: 8 bars, 8 chord changes (32 beats) → aim for 6-8 lines, one chord change per line.
- Chorus 1: 8 bars, 4 chord changes → aim for 4-6 lines; land the hook on line 1.
- Intro: 4 bars, instrumental (no sung lines — bare chord tags only).
------------------------------------------------------------------------------------------
```
Heuristics (keep simple, document them in code):
- syllable range by tempo: <=90 BPM → 8-12; 91-130 → 6-10; >130 → 4-8 (half-time feel in the
  preset's key_tempo_feel text may be noted but don't over-model).
- suggested lines per sung section: between chord-changes count and bars count, clamped 2..=10.
- a section whose structure role/label implies instrumental (or which has no lyric-spec beat/words)
  → say "instrumental — bare chord tags only".
Factor as a pure `lyrics_technical_brief(song, structure_data, chords_data) -> String` in core
(render module or agent.rs — match current layout), injected into the lyrics-stage user prompt (both
run + self-check). Other stages unchanged. Unit tests on the pure fn (the heuristics + instrumental
detection + missing-data degradation → empty string, prompt unchanged).

## 2. Skill: craft examples + brief compliance (lyrics.md)
- Add a short "HONOR THE TECHNICAL BRIEF" note: line counts and syllable budgets come from the
  brief; if a lyric idea needs more room, prefer more lines over longer lines.
- Add a MICRO-EXAMPLES section (original, written fresh — no real song quotes): 3 tiny before/after
  pairs showing the core moves: (a) AI-purple personification → plain human line with the same
  intent; (b) overlong unsingable line → split to two singable lines; (c) unestablished motif →
  established-then-used. Keep each to 2-4 lines; label them as style calibration, not content to copy.
- Keep every existing rule intact.

## 3. Lyric exemplars on the style preset (the taste lever)
Craft rules are generic; the user's TASTE isn't. Add `lyric_exemplars: String` to `StylePreset`
(models.rs + idempotent ALTER in migrate + ts-rs regen + StyleInput) — a freeform textarea where the
user pastes A FEW LINES they consider great for this project (their own, or fragments typed from
memory as style calibration). Presets UI (`Presets.tsx` / the preset editor): a "Lyric exemplars"
field with hint "a few lines that sound like what you want — calibrates the Lyricist's voice; never
copied". Prompt: when non-empty, `build_system_prompt` (or the lyrics-stage path) appends:
"----- LYRIC EXEMPLARS (calibrate voice/diction/line-length to these; NEVER copy or lightly rework
them) -----\n{exemplars}". Mock parity (mockApi preset shapes). generate_style_preset may leave it
empty (don't force the generator to invent exemplars).

## Definition of done
- `cargo build` + `cargo test --workspace` green with the new unit tests (paste lines).
- `cd frontend && npm run build` green (Presets UI + generated types change).
- Push the updated `lyrics.md` into the live DB builtin row (mirror the pattern:
  `UPDATE skill SET instructions=?, updated_at=datetime('now') WHERE key='songsmith-lyrics' AND source='builtin'`
  via python sqlite3 against "/Users/paul/Library/Application Support/com.songsmithstudio.desktop/songsmith-studio.db").
- Do NOT commit — the parent verifies (including a REAL generation test) and commits.
- Report: files, test lines, an example of the computed brief for a realistic song.
