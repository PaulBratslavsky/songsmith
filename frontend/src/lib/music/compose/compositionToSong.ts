// Export a Composition BACK into a song — the final direction of the
// Composer's flexibility goal (COMPOSER-SPEC.md requirements #2/#3). The
// mirror of compositionFromSong: it resolves the Composer's degree-based
// chord spans to ABSOLUTE chord names in the composition's key and groups
// them into the Chords-stage section shape ({label, bars, chords:[{name,
// beats}]}), so the backend can stay theory-free (it just renders + saves
// the resolved JSON through the existing artifact conventions).
//
// Pure + deterministic (no React/audio/IPC), so it's the single testable
// seam for the degree→name mapping:
//   - an imported chord keeps its real `name` verbatim (full-song chords
//     round-trip untouched);
//   - a degree-only chord (blank sketch) resolves to the diatonic triad —
//     or four-note seventh when `seventh` is set — for that degree in the
//     composition's key, via the SAME theory engine + labels the palette
//     and playback use (getDiatonicChords + triadLabel/seventhLabel — no
//     duplicated theory).
//
// The Rust side covers the round-trip contract by feeding the resolved
// JSON this module produces into export/create tests (see
// core/src/agent.rs: sketch_resolved_sections_round_trip_* tests).

import type { ChordSpan, Composition, KeyMode } from './types';
import { TICKS_PER_BEAT, TICKS_PER_BAR } from './types';
import { getDiatonicChords } from '../theory/diatonic';
import { triadLabel, seventhLabel } from './labels';
import type { PitchClass, ScaleType } from '../types';

/** One Chords-stage section, fully resolved (what the backend receives).
 *  `section_id` is the song's spine row the section came from (docs/SECTION-
 *  SPINE-SPEC.md — carried by full-song imports since Phase 2) so the export
 *  maps back losslessly even across renames; absent on sketches. */
export type ResolvedChord = { name: string; beats: number };
export type ResolvedSection = {
  label: string;
  bars: number;
  chords: ResolvedChord[];
  section_id?: string;
};

function modeToScaleType(mode: KeyMode): ScaleType {
  return mode === 'major' ? 'major' : 'minor';
}

/**
 * Absolute chord name for a chord span in the given key. `name` wins when
 * present (imported chords keep their printed name); otherwise the
 * diatonic triad (or seventh) for `degree` in the key, from the theory
 * engine's own labels — e.g. C major degree 6 → "Am", degree 5 + seventh
 * → "G7"; A minor degree 1 → "Am".
 */
export function chordNameForSpan(
  span: Pick<ChordSpan, 'degree' | 'seventh' | 'name'>,
  root: PitchClass,
  mode: KeyMode,
): string {
  const printed = span.name?.trim();
  if (printed) return printed;
  const chord = getDiatonicChords({ root, type: modeToScaleType(mode) }).find(
    (c) => c.degree === span.degree,
  );
  if (!chord) return root; // 7-note scales always resolve; defensive only
  return (span.seventh ? seventhLabel(chord) : triadLabel(chord)).name;
}

/** Span length in ticks → whole beats (min 1, rounded). */
export function ticksToBeats(lengthTicks: number): number {
  return Math.max(1, Math.round(lengthTicks / TICKS_PER_BEAT));
}

/**
 * Resolve a whole Composition into Chords-stage sections. With
 * `sections[]` (full-song imports) each chord lands in the section whose
 * tick range contains its start (chords past the last section fold into
 * it); a sketch with no sections becomes ONE "Sketch" section spanning
 * the composition. Chords stay in timeline order; section `bars` comes
 * from its tick length (min 1, rounded to whole bars).
 */
export function resolveCompositionSections(comp: Composition): ResolvedSection[] {
  const { root, mode } = comp.key;
  const chords = [...comp.chords].sort((a, b) => a.start - b.start);
  const resolve = (span: ChordSpan): ResolvedChord => ({
    name: chordNameForSpan(span, root, mode),
    beats: ticksToBeats(span.length),
  });

  if (!comp.sections.length) {
    return [
      {
        label: 'Sketch',
        bars: Math.max(1, Math.round(comp.totalTicks / TICKS_PER_BAR)),
        chords: chords.map(resolve),
      },
    ];
  }

  const sections = [...comp.sections].sort((a, b) => a.startTick - b.startTick);
  const out: ResolvedSection[] = sections.map((s) => ({
    label: s.name || 'Section',
    bars: Math.max(1, Math.round(s.lengthTicks / TICKS_PER_BAR)),
    chords: [],
    ...(s.section_id ? { section_id: s.section_id } : {}),
  }));
  for (const span of chords) {
    let idx = sections.findIndex(
      (s) => span.start >= s.startTick && span.start < s.startTick + s.lengthTicks,
    );
    if (idx < 0) {
      // outside every section (resized/trailing chord) — fold into the
      // nearest one so no chord is silently dropped from the export.
      idx = span.start < sections[0].startTick ? 0 : sections.length - 1;
    }
    out[idx].chords.push(resolve(span));
  }
  return out;
}
