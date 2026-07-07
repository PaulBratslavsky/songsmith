// Build a full-song Composition from a generated song — the import
// direction of the Composer's flexibility goal (COMPOSER-SPEC.md, DECISION
// 2026-06-29). Lays every section's chords end-to-end on one long,
// variable-length timeline, attaches each section's lyric lines to the
// chord starts they sit under, and keeps melody/bass empty (editable).
//
// Pure + deterministic (no React/audio): given the song key, the Chords
// stage data, and the Lyrics stage data, it returns a Composition that's
// run through parseStoredComposition so the span invariants hold.
//
// Reuse, not duplication:
//   - lyric/chord section pairing reuses deriveSections from the Sheet
//     preview / Builder (components/ArrangementBuilder) — same alignment
//     the existing sheet uses, so the Composer's lyric row matches it.
//   - chord-name parsing reuses the theory engine (parse-chord); the
//     diatonic degree is computed against the song's scale so changing the
//     key still transposes the degree-based playback.

import type {
  Composition,
  ChordSpan,
  Degree,
  KeyMode,
  LyricLine,
  LyricWord,
  Section,
} from './types';
import {
  SCHEMA_VERSION,
  DEFAULT_BPM,
  TICKS_PER_BEAT,
  TICKS_PER_BAR,
} from './types';
import { parseStoredComposition } from './schema';
import { parseChordSymbol } from '../theory/parse-chord';
import { getScalePitchClasses } from '../theory/scales';
import { normalizePitchClass } from '../theory/notes';
import type { PitchClass, ScaleType } from '../types';
import { PITCH_CLASSES } from '../types';
import { deriveSections } from '../../../components/ArrangementBuilder';
import { parseChordProLine } from '../../../components/LyricsEditor';

const uid = (() => {
  let n = 0;
  return (p: string) => `${p}-imp-${(n += 1)}`;
})();

function modeToScaleType(mode: KeyMode): ScaleType {
  return mode === 'major' ? 'major' : 'minor';
}

/**
 * Diatonic degree (1..7) for an absolute chord in the given key. The
 * chord's root PC is matched to the scale; in-scale roots map directly,
 * out-of-key roots pick the nearest scale degree by semitone distance
 * (display still comes from the chord `name`, so the exact quality isn't
 * lost — this only drives degree-based playback when no name parses).
 */
export function degreeForChordName(
  name: string,
  root: PitchClass,
  mode: KeyMode,
): Degree {
  const scalePcs = getScalePitchClasses({ root, type: modeToScaleType(mode) });
  const parsed = parseChordSymbol(name);
  const chordRoot = parsed?.root ?? normalizePitchClass(name) ?? root;
  const exact = scalePcs.indexOf(chordRoot);
  if (exact >= 0) return (Math.min(exact, 6) + 1) as Degree;
  // nearest scale degree by chromatic distance
  const ci = PITCH_CLASSES.indexOf(chordRoot);
  let best = 0;
  let bestDist = 99;
  scalePcs.forEach((pc, i) => {
    const pi = PITCH_CLASSES.indexOf(pc);
    const d = Math.min((ci - pi + 12) % 12, (pi - ci + 12) % 12);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  });
  return (Math.min(best, 6) + 1) as Degree;
}

type RawChord = { name: string; beats: number };

/** Read a chords-stage section's chords as {name, beats}, tolerating
 *  both the {name,beats} object shape and a plain string array. */
function readChords(sec: any): RawChord[] {
  const arr = Array.isArray(sec?.chords) ? sec.chords : [];
  return arr
    .map((c: any) => ({
      name: typeof c === 'string' ? c : (c?.name ?? ''),
      beats: typeof c === 'object' && c?.beats ? Number(c.beats) : 4,
    }))
    .filter((c: RawChord) => c.name);
}

/**
 * Build a Composition spanning the whole song.
 *
 * @param keyRoot  song key root (e.g. "A"); accepts flat/sharp spellings
 * @param keyMode  "major" | "minor"
 * @param chordsData  the Chords stage artifact's `data`
 *                    ({ sections: [{ label, feel?, chords:[{name,beats}] }] })
 * @param lyricsData  the Lyrics stage artifact's `data`
 *                    ({ sections: [{ label, lines:[] }] })
 * @param opts.name  composition name (song title)
 * @param opts.bpm   tempo
 */
export function compositionFromSong(
  keyRoot: string,
  keyMode: string,
  chordsData: any,
  lyricsData: any,
  opts: { id?: string; name?: string; bpm?: number } = {},
): Composition {
  const root: PitchClass = normalizePitchClass(keyRoot) ?? 'C';
  const mode: KeyMode = keyMode === 'major' ? 'major' : 'minor';

  const cSecs: any[] = Array.isArray(chordsData?.sections) ? chordsData.sections : [];

  // Section label → lyric lines, from the Sheet-preview's own alignment
  // (deriveSections), so the Composer lyric sheet matches the Sheet. Each
  // line keeps its word-level ChordPro breakdown (the [chord]-tag anchors),
  // which the lyric sheet uses to print chord names above the exact words.
  const derived = deriveSections(chordsData, lyricsData);
  const lyricLinesByLabel = new Map<string, LyricWord[][]>();
  for (const d of derived) {
    const lines = d.lyrics
      .map((l) => parseChordProLine(l).filter((w) => w.text || w.chord))
      .filter((words) => words.some((w) => w.text.trim().length > 0));
    lyricLinesByLabel.set(d.label, lines);
  }

  const chords: ChordSpan[] = [];
  const sections: Section[] = [];
  const lyrics: LyricLine[] = [];
  let tick = 0;

  for (const sec of cSecs) {
    const label: string = sec.label || sec.type || 'Section';
    const raw = readChords(sec);
    if (!raw.length) {
      // a chordless section (e.g. a bare Intro) — give it one bar so the
      // band still shows it.
      sections.push({ id: uid('sec'), name: label, startTick: tick, lengthTicks: TICKS_PER_BAR });
      tick += TICKS_PER_BAR;
      continue;
    }
    const sectionStart = tick;
    const chordStarts: number[] = [];
    for (const rc of raw) {
      const length = Math.max(1, Math.round(rc.beats * TICKS_PER_BEAT));
      chordStarts.push(tick);
      chords.push({
        id: uid('span'),
        degree: degreeForChordName(rc.name, root, mode),
        seventh: false,
        name: rc.name,
        start: tick,
        length,
      });
      tick += length;
    }
    sections.push({
      id: uid('sec'),
      name: label,
      startTick: sectionStart,
      lengthTicks: tick - sectionStart,
    });

    // Anchor this section's lyric lines (lyric-sheet-v2 fix). When the
    // CHORDS outnumber the lines, pair line i with chord start i (the song
    // lands one chord at the start of each lyric line). When the LINES
    // outnumber (or equal) the chords, distribute them evenly across the
    // section's tick span instead — the old code pinned every overflow
    // line to the LAST chord's tick, so play-along tracking skipped the
    // back half of the section. Either way every line gets a unique,
    // strictly increasing anchor. `words` carries the word-level chord
    // anchors for the lyric sheet.
    const lines = lyricLinesByLabel.get(label) ?? [];
    const sectionLength = tick - sectionStart;
    lines.forEach((words, i) => {
      const anchor =
        lines.length >= chordStarts.length
          ? sectionStart + Math.round((i * sectionLength) / lines.length)
          : (chordStarts[i] ?? sectionStart);
      const text = words.map((w) => w.text).filter(Boolean).join(' ');
      lyrics.push({ tick: anchor, text, words });
    });
  }

  // Round the total length up to whole bars (so the ruler ends clean).
  const totalTicks = Math.max(TICKS_PER_BAR, Math.ceil(tick / TICKS_PER_BAR) * TICKS_PER_BAR);
  const bars = totalTicks / TICKS_PER_BAR;

  const draft: Composition = {
    id: opts.id ?? uid('comp'),
    version: SCHEMA_VERSION,
    name: opts.name || 'Imported song',
    key: { root, mode },
    bpm: opts.bpm ?? DEFAULT_BPM,
    bars,
    totalTicks,
    chords,
    melody: [],
    bass: [],
    sections,
    lyrics,
  };

  // Run through the validator so invariants hold (sorted/in-range). If it
  // somehow fails to parse, fall back to the unvalidated draft rather than
  // crashing the editor.
  return parseStoredComposition(draft) ?? draft;
}
