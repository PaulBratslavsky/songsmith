// Notation view (N1) — pure tick→engraving derivations for the staff view.
// No VexFlow in here: this module turns the Composition's tick spans into
// per-bar note/rest "cells" (with tie flags across bar lines), decomposes
// tick lengths into dotted note values at sixteenth resolution, maps the
// composition key to a VexFlow key-signature spec (minor keys keep their
// minor spec — VexFlow renders the relative-major signature for them), and
// spells MIDI pitches with the key's enharmonics. The ONLY VexFlow consumer
// is components/compose/NotationView.tsx, which renders these derivations.
//
// Degrees→MIDI stays in playback.ts (resolveMelodyMidi / resolveBassMidi) —
// the notation view reuses the exact octave conventions playback sounds.

import type { PitchClass } from '../types';
import {
  octaveFromMidi,
  pitchClassFromMidi,
  pitchClassFromName,
} from '../theory/notes';
import { getScaleNoteNames } from '../theory/scales';
import type { ChordSpan, Composition, KeyMode, NoteSpan } from './types';
import { TICKS_PER_BAR } from './types';
import { keyToScaleSelection } from './playback';
import type { DegreeLabel } from './labels';

// ---------------------------------------------------------------------------
// Key signature: PitchClass (sharp-named) + mode → VexFlow key spec.
// Sharp-named accidental roots pick the CONVENTIONAL signature (Db over C#
// major's 7 sharps, Bb over A# major which doesn't exist); `preferFlats`
// drives the enharmonic spelling of the scale notes to match the signature.
// ---------------------------------------------------------------------------

export type VexKeySpec = { spec: string; preferFlats: boolean };

const MAJOR_SPECS: Record<PitchClass, VexKeySpec> = {
  C: { spec: 'C', preferFlats: false },
  'C#': { spec: 'Db', preferFlats: true },
  D: { spec: 'D', preferFlats: false },
  'D#': { spec: 'Eb', preferFlats: true },
  E: { spec: 'E', preferFlats: false },
  F: { spec: 'F', preferFlats: true },
  'F#': { spec: 'F#', preferFlats: false },
  G: { spec: 'G', preferFlats: false },
  'G#': { spec: 'Ab', preferFlats: true },
  A: { spec: 'A', preferFlats: false },
  'A#': { spec: 'Bb', preferFlats: true },
  B: { spec: 'B', preferFlats: false },
};

const MINOR_SPECS: Record<PitchClass, VexKeySpec> = {
  C: { spec: 'Cm', preferFlats: true },
  'C#': { spec: 'C#m', preferFlats: false },
  D: { spec: 'Dm', preferFlats: true },
  'D#': { spec: 'D#m', preferFlats: false },
  E: { spec: 'Em', preferFlats: false },
  F: { spec: 'Fm', preferFlats: true },
  'F#': { spec: 'F#m', preferFlats: false },
  G: { spec: 'Gm', preferFlats: true },
  'G#': { spec: 'G#m', preferFlats: false },
  A: { spec: 'Am', preferFlats: false },
  'A#': { spec: 'Bbm', preferFlats: true },
  B: { spec: 'Bm', preferFlats: false },
};

export function vexKeySpec(root: PitchClass, mode: KeyMode): VexKeySpec {
  return (mode === 'major' ? MAJOR_SPECS : MINOR_SPECS)[root];
}

// ---------------------------------------------------------------------------
// Pitch spelling: MIDI → VexFlow key string ("eb/4") using the key's scale
// spelling (Bb not A# in F major, E# not F in F# major). Degrees are always
// diatonic, so the scale map covers every pitch the composer can produce;
// anything else (defensive) falls back to the sharp pitch-class name.
// ---------------------------------------------------------------------------

export function midiSpeller(comp: Composition): (midi: number) => string {
  const { preferFlats } = vexKeySpec(comp.key.root, comp.key.mode);
  const names = getScaleNoteNames(keyToScaleSelection(comp), preferFlats);
  const spelling: Partial<Record<PitchClass, string>> = {};
  for (const raw of names) {
    const name = raw.replace(/[0-9]/g, '');
    // Skip double accidentals (F##, Bbb) — the fallback sharp name is
    // clearer on a staff than a glyph VexFlow would have to double-mark.
    if (name.length > 2) continue;
    const pc = pitchClassFromName(name);
    if (pc) spelling[pc] = name;
  }
  return (midi: number) => {
    const pc = pitchClassFromMidi(midi);
    const name = spelling[pc] ?? pc;
    let octave = octaveFromMidi(midi);
    // Wrapped enharmonics change the WRITTEN octave: Cb sounds a B below
    // (written octave +1), B# sounds a C above (written octave −1).
    if (name[0] === 'C' && pc === 'B') octave += 1;
    if (name[0] === 'B' && pc === 'C') octave -= 1;
    return `${name.toLowerCase()}/${octave}`;
  };
}

// ---------------------------------------------------------------------------
// Ticks → cells: one monophonic lane becomes a gap-free run of per-bar
// cells. A span crossing a bar line splits into chunks tied together
// (`tieToNext`); gaps become rest cells (also split per bar, never tied).
// ---------------------------------------------------------------------------

export type VoiceCell = {
  /** Bar index the cell lives in (cells never cross bar lines). */
  bar: number;
  /** Length in ticks, 1..TICKS_PER_BAR. */
  ticks: number;
  /** Resolved MIDI pitch, or null for a rest. */
  midi: number | null;
  /** True when the NEXT cell continues this pitch (tie across the bar). */
  tieToNext: boolean;
};

/**
 * Flatten a lane's spans into gap-free per-bar cells over `totalTicks`
 * (pass the bar-padded length so the final partial bar fills with rests).
 * Overlapping spans are clipped to the running position — the lanes are
 * monophonic by construction, this is just defensive.
 */
export function voiceCells(
  spans: NoteSpan[],
  totalTicks: number,
  resolve: (span: NoteSpan) => number | null,
): VoiceCell[] {
  const sorted = [...spans].sort((a, b) => a.start - b.start);
  const cells: VoiceCell[] = [];

  const pushRun = (start: number, end: number, midi: number | null) => {
    let s = start;
    const first = cells.length;
    while (s < end) {
      const barEnd = (Math.floor(s / TICKS_PER_BAR) + 1) * TICKS_PER_BAR;
      const e = Math.min(end, barEnd);
      cells.push({
        bar: Math.floor(s / TICKS_PER_BAR),
        ticks: e - s,
        midi,
        tieToNext: false,
      });
      s = e;
    }
    if (midi != null) {
      for (let i = first; i < cells.length - 1; i++) cells[i].tieToNext = true;
    }
  };

  let pos = 0;
  for (const span of sorted) {
    const start = Math.max(span.start, pos);
    const end = Math.min(span.start + span.length, totalTicks);
    if (end <= start) continue;
    if (start > pos) pushRun(pos, start, null);
    pushRun(start, end, resolve(span));
    pos = end;
  }
  if (pos < totalTicks) pushRun(pos, totalTicks, null);
  return cells;
}

// ---------------------------------------------------------------------------
// Ticks → note values. The 16-ticks-per-bar grid decomposes greedily into
// plain and dotted values; remainders chain as ties (7 → 6+1, 13 → 12+1).
// Triplets can't exist on a sixteenth grid, so this covers every length.
// ---------------------------------------------------------------------------

export type DurationAtom = {
  /** VexFlow duration code. */
  duration: 'w' | 'h' | 'q' | '8' | '16';
  dotted: boolean;
  ticks: number;
};

const ATOMS: DurationAtom[] = [
  { duration: 'w', dotted: false, ticks: 16 },
  { duration: 'h', dotted: true, ticks: 12 },
  { duration: 'h', dotted: false, ticks: 8 },
  { duration: 'q', dotted: true, ticks: 6 },
  { duration: 'q', dotted: false, ticks: 4 },
  { duration: '8', dotted: true, ticks: 3 },
  { duration: '8', dotted: false, ticks: 2 },
  { duration: '16', dotted: false, ticks: 1 },
];

/** Greedy largest-first decomposition of a tick run (1..16) into values. */
export function decomposeTicks(ticks: number): DurationAtom[] {
  const out: DurationAtom[] = [];
  let left = Math.max(0, Math.floor(ticks));
  while (left > 0) {
    const atom = ATOMS.find((a) => a.ticks <= left);
    if (!atom) break; // unreachable: the 1-tick sixteenth always fits
    out.push(atom);
    left -= atom.ticks;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Chord symbols above the melody staff: the imported name when present,
// else the degree's diatonic label (same labels the grid chips show).
// ---------------------------------------------------------------------------

export function chordSymbolText(
  span: ChordSpan,
  labels: Record<number, DegreeLabel>,
): string {
  if (span.name) return span.name;
  const label = labels[span.degree];
  if (!label) return '';
  return span.seventh ? label.seventh.name : label.triad.name;
}
