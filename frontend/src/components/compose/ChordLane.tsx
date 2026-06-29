// The chord lane — variable-length chord blocks over the 128-tick
// timeline. Blocks can be:
//   - clicked to select (palette then changes the selected block's degree)
//   - dragged by the body to reposition (move)
//   - dragged by the right edge to resize (extend / shrink)
//   - cleared via the × button
// Empty ticks are click targets that set the insertion cursor, where the
// next palette chip drops a chord. All overlap/clamp rules live in
// spans.ts; this component just converts pointer geometry into ticks and
// calls the handlers live so a block follows the cursor as you drag.

import { memo, useRef } from 'react';
import type { ChordSpan } from '../../lib/music/compose/types';
import type { DegreeLabel } from '../../lib/music/compose/labels';
import { degreeColor } from '../../lib/music/compose/colors';
import { LABEL_W, trackCols, isBarStart, isBeatStart } from './laneLayout';
import { useSpanDrag } from './useSpanDrag';

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
  selectedId: string | null;
  cursor: number;
  onSelect: (id: string | null) => void;
  onSetCursor: (tick: number) => void;
  onMove: (id: string, newStart: number) => void;
  onResize: (id: string, newLength: number) => void;
  onRemove: (id: string) => void;
}) {
  const trackRef = useRef<HTMLDivElement>(null);
  const { begin, onPointerMove, onPointerUp } = useSpanDrag({
    trackRef,
    totalTicks,
    onMove: (id, start) => onMove(id, start),
    onResize,
  });
  const TRACK_COLS = trackCols(totalTicks);

  return (
    <div style={{ display: 'flex', alignItems: 'stretch', height: 56 }}>
      <div
        style={{
          width: LABEL_W,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'flex-end',
          paddingRight: 6,
          fontSize: 9,
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: '0.06em',
          color: 'var(--ink-faint)',
        }}
      >
        Chords
      </div>
      <div style={{ position: 'relative', flex: 1 }}>
        {/* Background: clickable tick cells + bar gridlines + cursor */}
        <div
          ref={trackRef}
          style={{ display: 'grid', height: '100%', gridTemplateColumns: TRACK_COLS }}
        >
          {Array.from({ length: totalTicks }, (_, step) => {
            const isCursor = step === cursor && selectedId == null;
            return (
              <button
                key={step}
                type="button"
                className="cmp-cell"
                onClick={() => {
                  onSelect(null);
                  onSetCursor(step);
                }}
                style={{
                  borderBottom: '1px solid var(--line)',
                  borderLeft: isBarStart(step)
                    ? '2px solid var(--ink-faint)'
                    : isBeatStart(step)
                      ? '1px solid var(--line)'
                      : 'none',
                  background: isCursor ? 'var(--paper-3)' : undefined,
                  boxShadow: isCursor ? 'inset 0 0 0 1px var(--accent)' : undefined,
                }}
                aria-label={`Tick ${step + 1}`}
              />
            );
          })}
        </div>

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

// Memoized: with stable handler props + key-derived `labels`, editing a
// note lane or the playhead advancing won't re-render the chord lane.
export const ChordLane = memo(ChordLaneImpl);
