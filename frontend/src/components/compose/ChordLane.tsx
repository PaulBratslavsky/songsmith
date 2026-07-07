// The chord lane — variable-length chord blocks over the composition's
// tick timeline. Blocks can be:
//   - clicked to select (palette then changes the selected block's degree)
//   - dragged by the body to reposition (move)
//   - dragged by the right edge to resize (extend / shrink)
//   - cleared via the × button
// Clicking empty track sets the insertion cursor, where the next palette
// chip drops a chord. All overlap/clamp rules live in spans.ts; this
// component just converts pointer geometry into ticks and calls the
// handlers live so a block follows the cursor as you drag.
//
// Performance (audit Tier-2 #12): the background is NOT a per-tick button
// grid — gridlines are CSS repeating-linear-gradients, the insertion
// cursor is one positioned div, and clicks resolve to a tick through ONE
// hit surface (useLanePointer). Only the chord blocks are real elements.

import { memo, useRef } from 'react';
import type { ChordSpan } from '../../lib/music/compose/types';
import type { DegreeLabel } from '../../lib/music/compose/labels';
import { degreeColor } from '../../lib/music/compose/colors';
import { trackCols, laneGridBackground } from './laneLayout';
import { useSpanDrag } from './useSpanDrag';
import { useLanePointer } from './useLanePointer';

const LANE_H = 56; // px

function ChordLaneImpl({
  chords,
  labels,
  totalTicks,
  selectedId,
  cursor,
  onSelect,
  onSetCursor,
  onMove,
  onResize,
  onRemove,
}: {
  chords: ChordSpan[];
  /** Triad + seventh labels per diatonic degree for the current key. */
  labels: Record<number, DegreeLabel>;
  totalTicks: number;
  /** Id of the selected CHORD (null when the selection is a note) —
   *  scoped per lane so note selections don't re-render this lane. */
  selectedId: string | null;
  /** Insertion-cursor tick; the parent passes -1 while ANY selection
   *  exists (in any lane), which hides the cursor cell. */
  cursor: number;
  onSelect: (id: string | null) => void;
  onSetCursor: (tick: number) => void;
  onMove: (id: string, newStart: number) => void;
  onResize: (id: string, newLength: number) => void;
  onRemove: (id: string) => void;
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const hoverRef = useRef<HTMLDivElement>(null);
  const { begin, onPointerMove, onPointerUp } = useSpanDrag({
    trackRef,
    totalTicks,
    onMove: (id, start) => onMove(id, start),
    onResize,
  });
  const pointer = useLanePointer({
    surfaceRef: trackRef,
    hoverRef,
    totalTicks,
    onPick: (tick) => {
      onSelect(null);
      onSetCursor(tick);
    },
  });
  const TRACK_COLS = trackCols(totalTicks);

  return (
    <div style={{ display: 'flex', alignItems: 'stretch', height: LANE_H }}>
      <div className="cmp-lane-label caps">Chords</div>
      <div style={{ position: 'relative', flex: 1 }}>
        {/* ONE hit surface: CSS gridlines + pointer-derived tick for the
            insertion cursor. Also the measuring track for span drags. */}
        <div
          ref={trackRef}
          style={{
            position: 'absolute',
            inset: 0,
            backgroundImage: laneGridBackground(totalTicks, LANE_H),
          }}
          onClick={pointer.onClick}
          onPointerMove={pointer.onPointerMove}
          onPointerLeave={pointer.onPointerLeave}
          aria-label="Chord lane — click to place the insertion cursor"
        />
        {/* Hover cell marker (moved via direct style writes; no re-render) */}
        <div ref={hoverRef} className="cmp-hover-cell" />

        {/* Insertion cursor — one positioned div at the cursor tick */}
        {selectedId == null && cursor >= 0 && cursor < totalTicks && (
          <div
            style={{
              position: 'absolute',
              pointerEvents: 'none',
              top: 0,
              bottom: 0,
              left: `${(cursor / totalTicks) * 100}%`,
              width: `${100 / totalTicks}%`,
              background: 'var(--paper-3)',
              boxShadow: 'inset 0 0 0 1px var(--accent)',
            }}
          />
        )}

        {/* Foreground: chord blocks, positioned on the same column grid */}
        <div
          style={{
            pointerEvents: 'none',
            position: 'absolute',
            inset: 0,
            display: 'grid',
            gridTemplateColumns: TRACK_COLS,
          }}
        >
          {chords.map((span) => {
            const entry = labels[span.degree];
            const label = entry
              ? span.seventh
                ? entry.seventh
                : entry.triad
              : undefined;
            const selected = span.id === selectedId;
            const color = degreeColor(span.degree);
            // Imported chords print their absolute name (e.g. "Am"); the
            // roman numeral becomes the subtitle. Blank sketches stay
            // degree-based (roman on top, chord name below).
            const titleText = span.name ?? label?.roman ?? String(span.degree);
            const subText = span.name ? (label?.roman ?? '') : (label?.name ?? '');
            return (
              <div
                key={span.id}
                style={{
                  pointerEvents: 'auto',
                  position: 'relative',
                  margin: '2px 0',
                  display: 'flex',
                  cursor: 'grab',
                  userSelect: 'none',
                  flexDirection: 'column',
                  alignItems: 'center',
                  justifyContent: 'center',
                  borderRadius: 'var(--r)',
                  color: '#fff',
                  gridColumn: `${span.start + 1} / span ${span.length}`,
                  backgroundColor: color,
                  outline: selected ? '2px solid #fff' : 'none',
                  outlineOffset: selected ? 1 : 0,
                }}
                onPointerDown={(e) => begin(e, span, 'move')}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onClick={(e) => {
                  e.stopPropagation();
                  onSelect(span.id);
                }}
              >
                <span style={{ fontSize: 13, fontWeight: 700, lineHeight: 1 }}>
                  {titleText}
                </span>
                <span style={{ fontSize: 10, lineHeight: 1.1, opacity: 0.9 }}>
                  {subText}
                </span>
                {/* Clear button — top-left so it never sits under the
                    right-edge resize handle. */}
                <button
                  type="button"
                  className="cmp-x"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onRemove(span.id);
                  }}
                  aria-label="Remove chord"
                >
                  ×
                </button>
                {/* Resize handle (right edge) */}
                <div
                  className="cmp-resize"
                  onPointerDown={(e) => begin(e, span, 'resize')}
                  aria-label="Resize chord"
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// Memoized: with stable handler props + key-derived `labels` + lane-scoped
// selection, editing a note lane or the playhead advancing won't re-render
// the chord lane.
export const ChordLane = memo(ChordLaneImpl);
