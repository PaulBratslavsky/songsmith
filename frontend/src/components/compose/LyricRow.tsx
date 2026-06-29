// The lyric overlay row, rendered directly under the chord lane. Each
// lyric line is anchored at the tick of the chord it sits under (the song
// lands one chord at the start of each lyric line), left-aligned from that
// column. Purely presentational; aligned via the shared LABEL_W gutter +
// tick-column grid. Empty when the composition carries no lyrics (blank
// sketch).

import { memo } from 'react';
import type { LyricLine } from '../../lib/music/compose/types';
import { LABEL_W, trackCols } from './laneLayout';

function LyricRowImpl({
  lyrics,
  totalTicks,
}: {
  lyrics: LyricLine[];
  totalTicks: number;
}) {
  if (!lyrics.length) return null;
  return (
    <div style={{ display: 'flex', alignItems: 'stretch', minHeight: 20, margin: '4px 0' }}>
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
        Lyrics
      </div>
      <div style={{ position: 'relative', flex: 1 }}>
        {/* One grid ROW per lyric line (stacked top→bottom in song order),
            each line left-anchored at its chord's tick column so it sits
            under the chord it belongs to. */}
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: trackCols(totalTicks),
            gridTemplateRows: `repeat(${lyrics.length}, 18px)`,
          }}
        >
          {lyrics.map((ln, i) => (
            <div
              key={`${ln.tick}-${i}`}
              title={ln.text}
              style={{
                gridColumn: `${Math.min(ln.tick, totalTicks - 1) + 1} / -1`,
                gridRow: i + 1,
                fontSize: 11,
                lineHeight: '18px',
                color: 'var(--ink-dim)',
                whiteSpace: 'nowrap',
                overflow: 'visible',
                pointerEvents: 'none',
                borderLeft: '1px solid var(--line)',
                paddingLeft: 3,
              }}
            >
              {ln.text}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export const LyricRow = memo(LyricRowImpl);
