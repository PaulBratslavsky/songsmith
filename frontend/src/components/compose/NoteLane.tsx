// A piano-roll lane for melody or bass. Seven degree rows (degree 7 on
// top, 1 at the bottom, each labeled with its note name in the current
// key) over the composition's variable-length tick timeline. Click an
// empty cell to drop a note of the current duration; drag a note's body
// to move it in time AND pitch (up/down across rows), drag its right edge
// to resize, or double-click to remove. Monophonic: notes never overlap
// in time (clamped in spans.ts). Hovering a row auditions the pitch.
//
// Performance (audit Tier-2 #12): the background is NOT a per-tick button
// grid — gridlines are CSS repeating-linear-gradients and all placement /
// hover geometry runs through ONE hit surface (useLanePointer, sharing
// useSpanDrag's tick-width math). Only the note spans are real elements.
//
// Presentational: the parent supplies the per-degree note labels (`pcs`)
// and a `previewNote` audition callback, so this component holds no
// theory/audio knowledge and can be memoized — editing another lane or
// advancing the playhead won't re-render it.

import { memo, useRef } from 'react';
import type { Degree, NoteSpan } from '../../lib/music/compose/types';
import type { PitchClass } from '../../lib/music/types';
import { LABEL_W, trackCols, laneGridBackground } from './laneLayout';
import type { ChordToneHighlight } from './chordHighlight';
import { useSpanDrag } from './useSpanDrag';
import { useLanePointer } from './useLanePointer';

const DEGREES: Degree[] = [7, 6, 5, 4, 3, 2, 1];
const ROW_H = 20; // px, must match the row height below
/** Grid row (1-based, top=1) for a degree, given DEGREES order. */
const rowForDegree = (degree: number) => 8 - degree;
/** Degree for a 0-based row index from the top (inverse of rowForDegree). */
const degreeForRow = (row: number) => (7 - row) as Degree;

function NoteLaneImpl({
  lane,
  notes,
  pcs,
  color,
  totalTicks,
  highlight,
  selectedId,
  onPlace,
  onSelect,
  onMove,
  onResize,
  onRemove,
  previewNote,
}: {
  lane: 'melody' | 'bass';
  notes: NoteSpan[];
  /** Note label per degree, index 0 = degree 1, for the current key. */
  pcs: PitchClass[];
  color: string;
  totalTicks: number;
  highlight?: ChordToneHighlight | null;
  /** Id of the selected note IN THIS LANE (null when the selection is
   *  elsewhere) — scoped per lane so selecting in one lane doesn't
   *  re-render the others. */
  selectedId: string | null;
  onPlace: (degree: Degree, tick: number) => void;
  onSelect: (id: string | null) => void;
  /** Move a note to a new start tick and (for body drags) a new degree. */
  onMove: (id: string, newStart: number, newDegree: Degree) => void;
  onResize: (id: string, newLength: number) => void;
  onRemove: (id: string) => void;
  /** Audition a degree (hover + drag-across-rows). */
  previewNote: (degree: Degree) => void;
}) {
  const overlayRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const hoverRef = useRef<HTMLDivElement>(null);
  const { begin, onPointerMove, onPointerUp } = useSpanDrag({
    trackRef: overlayRef,
    totalTicks,
    rowHeight: ROW_H,
    onMove: (id, start, degree) => onMove(id, start, degree as Degree),
    onResize,
    onDegreeChange: (degree) => previewNote(degree as Degree),
  });
  const pointer = useLanePointer({
    surfaceRef,
    hoverRef,
    totalTicks,
    rows: DEGREES.length,
    rowHeight: ROW_H,
    onPick: (tick, row) => onPlace(degreeForRow(row), tick),
    onRowEnter: (row) => previewNote(degreeForRow(row)),
  });
  const TRACK_COLS = trackCols(totalTicks);
  const laneH = ROW_H * DEGREES.length;

  return (
    <div style={{ display: 'flex', alignItems: 'stretch' }}>
      {/* Label gutter: one label per degree row */}
      <div style={{ width: LABEL_W, flexShrink: 0 }}>
        {DEGREES.map((degree) => (
          <div
            key={degree}
            className="cmp-lane-label"
            style={{ height: ROW_H, gap: 4, lineHeight: 1 }}
          >
            <span style={{ fontVariantNumeric: 'tabular-nums' }}>{degree}</span>
            <span style={{ fontWeight: 500, color: 'var(--ink-dim)' }}>{pcs[degree - 1] ?? ''}</span>
          </div>
        ))}
      </div>

      <div style={{ position: 'relative', flex: 1, height: laneH }}>
        {/* ONE hit surface: CSS gridlines + pointer-derived (tick, degree)
            for click-to-add and hover audition. */}
        <div
          ref={surfaceRef}
          style={{
            position: 'absolute',
            inset: 0,
            backgroundImage: laneGridBackground(totalTicks, ROW_H),
          }}
          onClick={pointer.onClick}
          onPointerMove={pointer.onPointerMove}
          onPointerLeave={pointer.onPointerLeave}
          aria-label={`${lane} lane — click to add a note`}
        />
        {/* Hover cell marker (moved via direct style writes; no re-render) */}
        <div ref={hoverRef} className="cmp-hover-cell" />

        {/* Selected-chord tone shading: one strip per chord-tone degree */}
        {highlight &&
          [...highlight.degrees].map((degree) => (
            <div
              key={degree}
              style={{
                position: 'absolute',
                pointerEvents: 'none',
                top: (rowForDegree(degree) - 1) * ROW_H,
                height: ROW_H,
                left: `${(highlight.start / totalTicks) * 100}%`,
                width: `${(highlight.length / totalTicks) * 100}%`,
                backgroundColor: highlight.color,
              }}
            />
          ))}

        {/* Foreground: one overlay spanning all rows, so notes can move
            across rows (pitch) as well as columns (time). Same box as the
            hit surface, so useSpanDrag's tick math lines up. */}
        <div
          ref={overlayRef}
          style={{
            pointerEvents: 'none',
            position: 'absolute',
            inset: 0,
            display: 'grid',
            gridTemplateColumns: TRACK_COLS,
            gridTemplateRows: `repeat(${DEGREES.length}, ${ROW_H}px)`,
          }}
        >
          {notes.map((note) => {
            const selected = note.id === selectedId;
            return (
              <div
                key={note.id}
                style={{
                  pointerEvents: 'auto',
                  position: 'relative',
                  margin: '1px 0',
                  display: 'flex',
                  cursor: 'grab',
                  userSelect: 'none',
                  alignItems: 'center',
                  borderRadius: 2,
                  gridColumn: `${note.start + 1} / span ${note.length}`,
                  gridRow: rowForDegree(note.degree),
                  backgroundColor: color,
                  outline: selected ? '1px solid #fff' : 'none',
                }}
                onPointerDown={(e) => begin(e, note, 'move')}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onClick={(e) => {
                  e.stopPropagation();
                  onSelect(note.id);
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  onRemove(note.id);
                }}
                title="Drag to move (time + pitch), right edge to resize, double-click to remove"
              >
                <div
                  className="cmp-resize"
                  onPointerDown={(e) => begin(e, note, 'resize')}
                  aria-label="Resize note"
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

// Memoized: with stable handler props + key-derived `pcs` + lane-scoped
// selection, editing the other lane or advancing the playhead won't
// re-render this one.
export const NoteLane = memo(NoteLaneImpl);
