// The bar-number header row above the lanes. Eight bar labels, each
// spanning its 16 ticks, aligned to the shared track grid.

import { BARS, TICKS_PER_BAR } from '../../lib/music/compose/types';
import { LABEL_W, TRACK_COLS } from './laneLayout';

export function BeatRuler() {
  return (
    <div style={{ display: 'flex', alignItems: 'flex-end' }}>
      <div style={{ width: LABEL_W, flexShrink: 0 }} />
      <div style={{ display: 'grid', flex: 1, gridTemplateColumns: TRACK_COLS }}>
        {Array.from({ length: BARS }, (_, bar) => (
          <div
            key={bar}
            style={{
              gridColumn: `${bar * TICKS_PER_BAR + 1} / span ${TICKS_PER_BAR}`,
              borderLeft: '1px solid var(--line)',
              paddingLeft: 4,
              fontSize: 10,
              fontWeight: 500,
              color: 'var(--ink-faint)',
            }}
          >
            {bar + 1}
          </div>
        ))}
      </div>
    </div>
  );
}
