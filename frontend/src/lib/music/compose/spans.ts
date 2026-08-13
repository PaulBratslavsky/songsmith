// Progression Composer — time-span manipulation. Pure, side-effect-free
// helpers that keep a lane's spans sorted and non-overlapping while the
// user drags blocks, resizes them, and drops new ones. Generic over any
// TimeSpan ({id,start,length}); chord/note specifics are thin builders.
// All clamping lives here so the UI never reasons about neighbours.
//
// Composition length is variable, so the grid-bound helpers
// (freeGapAt/moveSpan/resizeSpan/addChord/addNote) take `totalTicks` as
// an argument rather than reading a module constant.

import type { ChordSpan, Degree, NoteSpan, TimeSpan } from './types';
import { DEFAULT_TOTAL_TICKS } from './types';

export function sortSpans<T extends TimeSpan>(spans: T[]): T[] {
  return [...spans].sort((a, b) => a.start - b.start);
}

/** The span covering `tick`, or null if the tick is empty. */
export function spanAt<T extends TimeSpan>(spans: T[], tick: number): T | null {
  return spans.find((s) => tick >= s.start && tick < s.start + s.length) ?? null;
}

/**
 * The largest free run [start, start+length) that contains `tick` and
 * doesn't collide with any existing span. Returns null if `tick` is
 * already occupied or out of range.
 */
export function freeGapAt(
  spans: TimeSpan[],
  tick: number,
  totalTicks: number = DEFAULT_TOTAL_TICKS,
): { start: number; length: number } | null {
  if (tick < 0 || tick >= totalTicks) return null;
  if (spanAt(spans, tick)) return null;
  let lo = 0;
  let hi = totalTicks;
  for (const s of sortSpans(spans)) {
    const end = s.start + s.length;
    if (end <= tick) lo = Math.max(lo, end);
    if (s.start > tick) {
      hi = Math.min(hi, s.start);
      break;
    }
  }
  return { start: lo, length: hi - lo };
}

export function removeById<T extends TimeSpan>(spans: T[], id: string): T[] {
  return spans.filter((s) => s.id !== id);
}

/** Neighbours of `id` in sorted order: the spans immediately before/after. */
function neighbours<T extends TimeSpan>(
  spans: T[],
  id: string,
  totalTicks: number,
): { prevEnd: number; nextStart: number; self: T | null } {
  const sorted = sortSpans(spans);
  const idx = sorted.findIndex((s) => s.id === id);
  if (idx === -1) return { prevEnd: 0, nextStart: totalTicks, self: null };
  const prevEnd = idx > 0 ? sorted[idx - 1].start + sorted[idx - 1].length : 0;
  const nextStart =
    idx < sorted.length - 1 ? sorted[idx + 1].start : totalTicks;
  return { prevEnd, nextStart, self: sorted[idx] };
}

/** Move a span to `newStart`, clamped so it stays in-grid and non-overlapping. */
export function moveSpan<T extends TimeSpan>(
  spans: T[],
  id: string,
  newStart: number,
  totalTicks: number = DEFAULT_TOTAL_TICKS,
): T[] {
  const { prevEnd, nextStart, self } = neighbours(spans, id, totalTicks);
  if (!self) return spans;
  const maxStart = nextStart - self.length;
  const start = Math.max(prevEnd, Math.min(newStart, maxStart));
  return sortSpans(spans.map((s) => (s.id === id ? { ...s, start } : s)));
}

/** Resize a span to `newLength` ticks, clamped to >= 1 and the next span. */
export function resizeSpan<T extends TimeSpan>(
  spans: T[],
  id: string,
  newLength: number,
  totalTicks: number = DEFAULT_TOTAL_TICKS,
): T[] {
  const { nextStart, self } = neighbours(spans, id, totalTicks);
  if (!self) return spans;
  const maxLength = nextStart - self.start;
  const length = Math.max(1, Math.min(newLength, maxLength));
  return spans.map((s) => (s.id === id ? { ...s, length } : s));
}

// ---- Chord builders ----

export function addChord(
  spans: ChordSpan[],
  id: string,
  degree: Degree,
  tick: number,
  desiredLength: number,
  seventh = false,
  totalTicks: number = DEFAULT_TOTAL_TICKS,
): ChordSpan[] {
  const gap = freeGapAt(spans, tick, totalTicks);
  if (!gap) return spans;
  const length = Math.max(1, Math.min(desiredLength, gap.length));
  const start = Math.min(tick, gap.start + gap.length - length);
  return sortSpans([...spans, { id, degree, seventh, start, length }]);
}

/** Pick a new degree for a chord span.
 *
 *  This CLEARS any imported `name`. `name` wins over `degree` everywhere it is
 *  set — the lane label, playback, and the export back to the song — so leaving
 *  it in place made picking a degree on an imported chord do nothing the user
 *  could see, hear, or export: the Composer was a read-only view of a wrong
 *  analysis (audit 2026-08-13). Picking a degree is an explicit override, so
 *  the printed name it replaces has to go.
 *
 *  To correct the SPELLING without dropping to a degree — keeping qualities the
 *  degree model can't express (Cadd9, F/A) — use the Chords stage editor, which
 *  edits `name` directly (`SectionChordsEditor`, name input + swap popover). */
export function setChordDegree(
  spans: ChordSpan[],
  id: string,
  degree: Degree,
): ChordSpan[] {
  return spans.map((s) => {
    if (s.id !== id) return s;
    const { name: _dropped, ...rest } = s;
    return { ...rest, degree };
  });
}

export function setChordSeventh(
  spans: ChordSpan[],
  id: string,
  seventh: boolean,
): ChordSpan[] {
  return spans.map((s) => (s.id === id ? { ...s, seventh } : s));
}

// ---- Note builders ----

export function addNote(
  spans: NoteSpan[],
  id: string,
  degree: Degree,
  octave: 0 | 1,
  tick: number,
  desiredLength: number,
  totalTicks: number = DEFAULT_TOTAL_TICKS,
): NoteSpan[] {
  const gap = freeGapAt(spans, tick, totalTicks);
  if (!gap) return spans;
  const length = Math.max(1, Math.min(desiredLength, gap.length));
  const start = Math.min(tick, gap.start + gap.length - length);
  return sortSpans([...spans, { id, degree, octave, start, length }]);
}
