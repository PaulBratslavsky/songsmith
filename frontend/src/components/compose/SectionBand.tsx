// A labeled region band above the lanes — one block per song section,
// laid end-to-end on the shared tick grid. Purely presentational; aligned
// to the lanes via the same LABEL_W gutter + tick-column grid.

import { memo } from 'react';
import type { Section } from '../../lib/music/compose/types';
import { trackCols } from './laneLayout';

const BAND_COLORS = ['#3b4d63', '#4a5b3b', '#5b3b52', '#3b5b58', '#5b4a3b', '#473b5b'];

function SectionBandImpl({
  sections,
  totalTicks,
}: {
  sections: Section[];
  totalTicks: number;
}) {
  if (!sections.length) return null;
  return (
    <div style={{ display: 'flex', alignItems: 'stretch', height: 22, marginBottom: 2 }}>
      <div className="cmp-lane-label caps">Song</div>
      <div style={{ display: 'grid', flex: 1, gridTemplateColumns: trackCols(totalTicks) }}>
        {sections.map((sec, i) => (
          <div
            key={sec.id}
            title={sec.name}
            style={{
              gridColumn: `${sec.startTick + 1} / span ${Math.max(1, sec.lengthTicks)}`,
              display: 'flex',
              alignItems: 'center',
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
            }}
          >
            {sec.name}
          </div>
        ))}
      </div>
    </div>
  );
}

export const SectionBand = memo(SectionBandImpl);
