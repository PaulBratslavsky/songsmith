// Adapter: drives the UI from the ported music-kb engine (real CAGED voicings,
// correct chord tones) instead of my hand-rolled shapes. Keeps the existing
// diagram/synth/piano renderers, just fed with engine data.

import { PITCH_CLASSES, type ChordQuality } from "../lib/music/types";
import { getChordPitchClasses } from "../lib/music/theory/chords";
import { parseChordSymbol } from "../lib/music/theory/parse-chord";
import { guitarVoicing, guitarVoicingCount } from "../lib/music/theory/voicings/guitar";
import { pianoVoicing } from "../lib/music/theory/voicings/piano";

/** A guitar fingering for rendering: absolute fret per string (low E→high e;
 *  -1 = muted, 0 = open), the diagram window start, and the shape name. */
export type GuitarShape = { frets: number[]; baseFret: number; label: string };

const noteMidi = (n: { pitchClass: string; octave: number }) => PITCH_CLASSES.indexOf(n.pitchClass as any) + (n.octave + 1) * 12;

/** Quality buttons: display label → engine ChordQuality. */
export const QUALITY_OPTIONS: [string, ChordQuality][] = [
  ["maj", "maj"], ["m", "min"], ["5", "5"], ["dim", "dim"], ["aug", "aug"], ["sus2", "sus2"], ["sus4", "sus4"],
  ["6", "6"], ["m6", "m6"], ["maj7", "maj7"], ["m7", "min7"], ["7", "dom7"], ["m7b5", "m7b5"],
  ["dim7", "dim7"], ["mMaj7", "mMaj7"], ["add9", "add9"], ["9", "9"], ["maj9", "maj9"], ["m9", "m9"],
];

const pcIdx = (pc: string) => PITCH_CLASSES.indexOf(pc as any);

/** Pitch-class indices (0–11) of the chord tones — for the piano view. */
export function chordPcsIdx(rootIdx: number, quality: ChordQuality): number[] {
  return getChordPitchClasses(PITCH_CLASSES[rootIdx], quality).map(pcIdx);
}

/** Piano voicing as MIDI, honoring inversion (which chord tone is the bass). */
export function voicedMidis(rootIdx: number, quality: ChordQuality, inversion: number): number[] {
  return pianoVoicing({ root: PITCH_CLASSES[rootIdx], quality, inversion, voicingIndex: 0 }).map(noteMidi);
}
/** The voiced note names low→high, for display (e.g. first inversion = "E G C"). */
export function voicedNotes(rootIdx: number, quality: ChordQuality, inversion: number): string[] {
  return pianoVoicing({ root: PITCH_CLASSES[rootIdx], quality, inversion, voicingIndex: 0 }).map((n) => n.pitchClass);
}

/** How many guitar voicings (open + barre shapes) exist for this chord. */
export function guitarCount(rootIdx: number, quality: ChordQuality): number {
  return guitarVoicingCount({ root: PITCH_CLASSES[rootIdx], quality, inversion: 0, voicingIndex: 0 });
}

/** The Nth guitar voicing as a fret-per-string diagram shape, or null. */
export function guitarFrets(rootIdx: number, quality: ChordQuality, voicingIndex: number): GuitarShape | null {
  const v = guitarVoicing({ root: PITCH_CLASSES[rootIdx], quality, inversion: 0, voicingIndex });
  if (!v.positions) return null; // quality has no shape → caller shows piano
  // the engine indexes strings 0 = high E … 5 = low E (STANDARD_TUNING_MIDI starts
  // at E4); the diagram renderers expect 0 = low E, so build then reverse.
  const eng = [-1, -1, -1, -1, -1, -1];
  for (const key of v.positions) {
    const [s, f] = key.split("-").map(Number);
    if (s >= 0 && s < 6) eng[s] = f;
  }
  const frets = eng.reverse(); // → [low E, A, D, G, B, high e]
  const pressed = frets.filter((f) => f > 0);
  const baseFret = pressed.length && Math.max(...pressed) > 4 ? Math.min(...pressed) : 1;
  return { frets, baseFret, label: v.shapeName ?? "voicing" };
}

// ---- name-based variants (for Composer chips / export / playback) ----------

type Sel = { rootIdx: number; quality: ChordQuality; pcs: number[] };
export function nameToSel(name: string): Sel | null {
  const p = parseChordSymbol(name);
  if (!p) return null;
  return { rootIdx: pcIdx(p.root), quality: p.quality, pcs: p.pitchClasses.map((pc) => pcIdx(pc)) };
}
export function isValidName(name: string): boolean {
  return parseChordSymbol(name) != null;
}
export function chordPcsByName(name: string): number[] {
  return nameToSel(name)?.pcs ?? [];
}
export function chordMidisByName(name: string, base = 48): number[] {
  let prev = -1;
  return chordPcsByName(name).map((pc) => { let n = base + pc; while (n <= prev) n += 12; prev = n; return n; });
}
export function guitarCountByName(name: string): number {
  const s = nameToSel(name);
  return s ? guitarCount(s.rootIdx, s.quality) : 0;
}
export function guitarFretsByName(name: string, voicingIndex = 0): GuitarShape | null {
  const s = nameToSel(name);
  return s ? guitarFrets(s.rootIdx, s.quality, voicingIndex) : null;
}
/** Number of inversions (= chord tones) available for a chord name. */
export function chordSizeByName(name: string): number {
  return nameToSel(name)?.pcs.length ?? 0;
}
/** Piano voicing as MIDI for a chord name at the given inversion (bass note). */
export function voicedMidisByName(name: string, inversion = 0): number[] {
  const s = nameToSel(name);
  return s ? voicedMidis(s.rootIdx, s.quality, inversion) : [];
}
/** Voiced note names low→high for a chord name at the given inversion. */
export function voicedNotesByName(name: string, inversion = 0): string[] {
  const s = nameToSel(name);
  return s ? voicedNotes(s.rootIdx, s.quality, inversion) : [];
}
