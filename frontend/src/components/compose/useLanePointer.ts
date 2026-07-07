// Shared pointer logic for a lane's SINGLE hit surface (audit Tier-2 #12:
// replaces the per-tick <button> grids). The surface is one element per
// lane; (tick, row) are derived from pointer geometry with the same
// tick-width math useSpanDrag uses (laneLayout.tickAtPointer), so clicks
// land on exactly the columns the span blocks occupy.
//
// Hover feedback (the old `.cmp-cell:hover` cell) is preserved by moving a
// single caller-supplied marker div via direct style writes — no React
// state, so pointer movement never re-renders the lane. `onRowEnter` fires
// when the pointer crosses into a new row (the note lanes' hover
// audition); it deliberately does NOT re-fire per tick column.

import { useRef } from 'react';
import { rowAtPointer, tickAtPointer } from './laneLayout';

export function useLanePointer(opts: {
  /** The hit surface itself (its rect spans exactly the tick columns). */
  surfaceRef: React.RefObject<HTMLDivElement | null>;
  /** Absolutely-positioned marker moved under the pointer (hover cell). */
  hoverRef: React.RefObject<HTMLDivElement | null>;
  totalTicks: number;
  /** Row count / height for degree lanes; defaults to one full-height row. */
  rows?: number;
  rowHeight?: number;
  onPick: (tick: number, row: number) => void;
  /** Fired when the pointer enters a different row (hover audition). */
  onRowEnter?: (row: number) => void;
}) {
  const lastRow = useRef(-1);

  const locate = (e: React.PointerEvent | React.MouseEvent) => {
    const surface = opts.surfaceRef.current;
    if (!surface) return null;
    const rect = surface.getBoundingClientRect();
    const rows = opts.rows ?? 1;
    const rowH = opts.rowHeight ?? rect.height;
    return {
      tick: tickAtPointer(rect, e.clientX, opts.totalTicks),
      row: rowAtPointer(rect, e.clientY, rowH, rows),
      rowH,
    };
  };

  const onPointerMove = (e: React.PointerEvent) => {
    const at = locate(e);
    if (!at) return;
    if (at.row !== lastRow.current) {
      lastRow.current = at.row;
      opts.onRowEnter?.(at.row);
    }
    const hover = opts.hoverRef.current;
    if (hover) {
      hover.style.display = 'block';
      hover.style.left = `${(at.tick / opts.totalTicks) * 100}%`;
      hover.style.width = `${100 / opts.totalTicks}%`;
      hover.style.top = `${at.row * at.rowH}px`;
      hover.style.height = `${at.rowH}px`;
    }
  };

  const onPointerLeave = () => {
    lastRow.current = -1;
    const hover = opts.hoverRef.current;
    if (hover) hover.style.display = 'none';
  };

  const onClick = (e: React.MouseEvent) => {
    const at = locate(e);
    if (at) opts.onPick(at.tick, at.row);
  };

  return { onPointerMove, onPointerLeave, onClick };
}
