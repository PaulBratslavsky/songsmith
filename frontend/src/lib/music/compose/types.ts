// Progression Composer — data model.
//
// A Composition is a *variable-length* sketch expressed in *scale
// degrees* relative to a key. A blank sketch defaults to 8 bars; a song
// imported via compositionFromSong spans the whole song (many sections,
// laid end-to-end). Because chords/notes store degrees, changing `key`
// transposes the whole piece for free. Imported chords additionally carry
// the real chord `name` so the lane can show the absolute chord.
//
// Time is measured in TICKS at sixteenth-note resolution:
//   - 4 ticks per beat, 4 beats per bar  → 16 ticks/bar
//   - N bars                             → N × 16 ticks total
// Chords and melody/bass notes are all variable-length time spans on
// this grid, so a note can be a sixteenth, a quarter, a whole bar, etc.
//
// LENGTH IS PER-COMPOSITION. The old module-level `TOTAL_TICKS`/`BARS`
// constants are now *defaults* (DEFAULT_BARS / DEFAULT_TOTAL_TICKS); a
// composition's real length is `comp.totalTicks`. Helpers that need the
// length take it as an argument (see spans.ts, playback.ts, laneLayout.ts).

import type { PitchClass } from '../types';

export const TICKS_PER_BEAT = 4;
export const BEATS_PER_BAR = 4;
export const TICKS_PER_BAR = TICKS_PER_BEAT * BEATS_PER_BAR; // 16

/** Default length for a fresh blank sketch (unchanged behavior). */
export const DEFAULT_BARS = 8;
export const DEFAULT_TOTAL_TICKS = DEFAULT_BARS * TICKS_PER_BAR; // 128

// Back-compat aliases. Prefer `comp.totalTicks` / `comp.bars`. These are
// kept so any not-yet-migrated reference still compiles, and they equal
// the blank-sketch default.
export const BARS = DEFAULT_BARS;
export const TOTAL_TICKS = DEFAULT_TOTAL_TICKS;

/** Largest length any composition may declare (clamp for stored rows). */
export const MAX_TOTAL_TICKS = 16_000; // ~1000 bars, plenty for a full song

export type KeyMode = 'major' | 'minor';

/** A diatonic scale degree, 1 through 7. */
export type Degree = 1 | 2 | 3 | 4 | 5 | 6 | 7;

/** Any object that occupies a run of ticks on the timeline. */
export type TimeSpan = {
  id: string;
  /** Tick index where it begins, 0..totalTicks-1. */
  start: number;
  /** Duration in ticks, >= 1. */
  length: number;
};

/** A chord placed on the timeline: a diatonic degree over a run of ticks.
 *  `seventh` extends the diatonic triad to its four-note seventh chord
 *  (Imaj7, iim7, V7, viiø …) — quality is still derived from the key, so
 *  the piece transposes for free either way.
 *
 *  `name` is the absolute chord name for *imported* chords (e.g. "Am",
 *  "Bb", "Dm7"); when present the lane shows it instead of the degree
 *  label, and playback resolves the printed chord. A blank sketch leaves
 *  `name` undefined and stays purely degree-based. */
export type ChordSpan = TimeSpan & {
  degree: Degree;
  seventh: boolean;
  /** Absolute chord name (imported songs only). */
  name?: string;
};

/** A monophonic melody/bass note: a degree (+ octave band) over a run of ticks. */
export type NoteSpan = TimeSpan & {
  degree: Degree;
  /** 0 = base octave, 1 = one octave up. Bass always uses 0. */
  octave: 0 | 1;
};

/** A labeled region of the timeline (a song section laid end-to-end). */
export type Section = {
  id: string;
  /** Section label (e.g. "Verse 1", "Chorus"). */
  name: string;
  startTick: number;
  lengthTicks: number;
};

/** A lyric line anchored to a tick (the chord start it sits under). */
export type LyricLine = {
  tick: number;
  text: string;
};

/**
 * Persisted-shape version. Bump when the Composition layout changes in a
 * way that old saved rows can't be read as-is, and add a migration
 * branch in compose/schema.ts → parseStoredComposition. (History: v1 is
 * the tick-based model; the earlier beats-based shape predates this
 * content type, so there are no v0 rows to migrate.)
 *
 * v2 adds `seventh` to ChordSpan; v1 rows are migrated by defaulting it
 * to false (triad).
 * v3 makes length variable (`bars`/`totalTicks`) and adds `sections`,
 * `lyrics`, and an optional chord `name`; older rows default to 8 bars /
 * no sections / no lyrics in compose/schema.ts → parseStoredComposition.
 */
export const SCHEMA_VERSION = 3;

export type Composition = {
  id: string;
  /** Persisted-shape version; see SCHEMA_VERSION. */
  version: number;
  name: string;
  key: { root: PitchClass; mode: KeyMode };
  /** Quarter-note tempo, 60–180. */
  bpm: number;
  /** Length in whole bars (>= 1). */
  bars: number;
  /** Length in ticks = bars × TICKS_PER_BAR. The grid/clamps/playhead read this. */
  totalTicks: number;
  /** Variable-length chord blocks across the timeline. */
  chords: ChordSpan[];
  /** Monophonic melody notes. */
  melody: NoteSpan[];
  /** Monophonic bass notes. */
  bass: NoteSpan[];
  /** Labeled section regions (empty for blank sketches). */
  sections: Section[];
  /** Lyric lines anchored to chord starts (empty for blank sketches). */
  lyrics: LyricLine[];
};

/** Selectable note durations, in ticks. */
export const DURATIONS: Array<{ label: string; ticks: number }> = [
  { label: '1/16', ticks: 1 },
  { label: '1/8', ticks: 2 },
  { label: '1/4', ticks: 4 },
  { label: '1/2', ticks: 8 },
  { label: 'bar', ticks: TICKS_PER_BAR },
];

/** Default new-chord length: one bar. */
export const DEFAULT_CHORD_TICKS = TICKS_PER_BAR;
/** Default tempo for a fresh sketch. */
export const DEFAULT_BPM = 100;

/** An empty composition in the given key, `bars` long (default 8). */
export function emptyComposition(
  id: string,
  name = 'Untitled',
  root: PitchClass = 'C',
  mode: KeyMode = 'major',
  bars: number = DEFAULT_BARS,
): Composition {
  const b = Math.max(1, Math.floor(bars));
  return {
    id,
    version: SCHEMA_VERSION,
    name,
    key: { root, mode },
    bpm: DEFAULT_BPM,
    bars: b,
    totalTicks: b * TICKS_PER_BAR,
    chords: [],
    melody: [],
    bass: [],
    sections: [],
    lyrics: [],
  };
}
