// A labeled region band above the lanes — one block per song section,
// laid end-to-end on the shared tick grid. Aligned to the lanes via the
// same LABEL_W gutter + tick-column grid.
//
// Phase 3 build-out: blocks are clickable — clicking FOCUSES a section
// (loop playback confined to it, audio slice follows); clicking again
// unfocuses. A ✓-marked section renders dimmed with a check (the user's
// "this one is rebuilt" bookkeeping). Both are optional — without the
// callbacks the band stays purely presentational (Notation view).

import { memo } from 'react';
import type { Section } from '../../lib/music/compose/types';
import { trackCols } from './laneLayout';

const BAND_COLORS = ['#3b4d63', '#4a5b3b', '#5b3b52', '#3b5b58', '#5b4a3b', '#473b5b'];

function SectionBandImpl({
  sections,
  totalTicks,
  focusId,
  onFocus,
}: {
  sections: Section[];
  totalTicks: number;
  /** id of the focused section (loop target), or null */
  focusId?: string | null;
  /** click a block to focus it (click again to unfocus) */
  onFocus?: (id: string) => void;
}) {
  if (!sections.length) return null;
  return (
    <div style={{ display: 'flex', alignItems: 'stretch', height: 22, marginBottom: 2 }}>
      <div className="cmp-lane-label caps">Song</div>
      <div style={{ display: 'grid', flex: 1, gridTemplateColumns: trackCols(totalTicks) }}>
        {sections.map((sec, i) => {
          const focused = focusId === sec.id;
          return (
            <div
              key={sec.id}
              title={
                (sec.done ? '✓ done · ' : '') +
                sec.name +
                (onFocus ? (focused ? ' — click to unfocus' : ' — click to focus + loop') : '')
              }
              onClick={onFocus ? () => onFocus(sec.id) : undefined}
              style={{
                gridColumn: `${sec.startTick + 1} / span ${Math.max(1, sec.lengthTicks)}`,
                display: 'flex',
                alignItems: 'center',
                gap: 3,
                padding: '0 6px',
                borderRadius: 3,
                margin: '0 1px',
                fontSize: 10,
                fontWeight: 600,
                color: '#fff',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                backgroundColor: BAND_COLORS[i % BAND_COLORS.length],
                cursor: onFocus ? 'pointer' : undefined,
                opacity: sec.done && !focused ? 0.45 : 1,
                outline: focused ? '2px solid var(--accent, #7aa2f7)' : undefined,
                outlineOffset: -2,
              }}
            >
              {sec.done ? '✓ ' : ''}
              {sec.name}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export const SectionBand = memo(SectionBandImpl);
