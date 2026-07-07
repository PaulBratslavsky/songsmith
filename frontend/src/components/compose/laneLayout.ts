// Shared geometry for the composer lanes so the beat ruler, melody
// piano-roll, chord lane, and bass piano-roll all line their columns up.
// Every lane is a flex row of [fixed-width label gutter][flex-1 track];
// the track is an N-column grid (N = the composition's totalTicks =
// bars × 4 beats × 4 sixteenths). Length is per-composition, so the
// column count is passed in rather than read from a constant.

import { TICKS_PER_BEAT, TICKS_PER_BAR } from '../../lib/music/compose/types';

/** Width of the left label gutter, shared by every lane.
 *  Must match the `.cmp-lane-label` width in styles.css. */
export const LABEL_W = '2.75rem';

/** Minimum px width per bar so long songs scroll instead of squashing. */
export const BAR_MIN_PX = 96;

/** CSS grid-template-columns for a lane track: one column per tick. */
export function trackCols(totalTicks: number): string {
  return `repeat(${totalTicks}, minmax(0, 1fr))`;
}

/** Width in px of one tick column for a track of the given pixel width.
 *  The single shared pointer↔tick conversion (useSpanDrag's drag deltas
 *  and the lanes' hit surfaces both go through this). */
export function tickWidth(trackWidthPx: number, totalTicks: number): number {
  return trackWidthPx / totalTicks;
}

/** Tick index under a pointer, clamped into the track. */
export function tickAtPointer(
  rect: DOMRect,
  clientX: number,
  totalTicks: number,
): number {
  const raw = Math.floor((clientX - rect.left) / tickWidth(rect.width, totalTicks));
  return Math.max(0, Math.min(totalTicks - 1, raw));
}

/** Row index (0-based, top row = 0) under a pointer, clamped. */
export function rowAtPointer(
  rect: DOMRect,
  clientY: number,
  rowHeight: number,
  rows: number,
): number {
  const raw = Math.floor((clientY - rect.top) / rowHeight);
  return Math.max(0, Math.min(rows - 1, raw));
}

/**
 * CSS background layers that draw the lane gridlines without any per-tick
 * DOM: a 2px bar line on every bar downbeat, a 1px beat line on every
 * quarter, and (when `rowHeight` is given) a 1px row separator along each
 * row's bottom edge. Percentage-period repeating gradients track the lane
 * width, so the lines stay glued to the same fractional tick columns the
 * span blocks use.
 */
export function laneGridBackground(totalTicks: number, rowHeight?: number): string {
  const bars = totalTicks / TICKS_PER_BAR;
  const beats = totalTicks / TICKS_PER_BEAT;
  const layers = [
    `repeating-linear-gradient(to right, var(--ink-faint) 0 2px, transparent 2px calc(100% / ${bars}))`,
    `repeating-linear-gradient(to right, var(--line) 0 1px, transparent 1px calc(100% / ${beats}))`,
  ];
  if (rowHeight) {
    layers.push(
      `repeating-linear-gradient(to bottom, transparent 0 ${rowHeight - 1}px, var(--line) ${rowHeight - 1}px ${rowHeight}px)`,
    );
  }
  return layers.join(', ');
}
