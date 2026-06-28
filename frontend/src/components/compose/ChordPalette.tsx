// The diatonic chord palette — "Chords in {key}". One chip per scale
// degree (I, ii, iii, IV, V, vi, vii°), colored to match the chord
// blocks. Clicking a chip either re-colors the selected block's degree
// or drops a new chord at the cursor (the Composer decides which).

import { useMemo } from 'react';
import type { Composition, Degree } from '../../lib/music/compose/types';
import {
  keyToScaleSelection,
  resolveChordMidis,
} from '../../lib/music/compose/playback';
import { getDiatonicChords } from '../../lib/music/theory/diatonic';
import { triadLabel } from '../../lib/music/compose/labels';
import { degreeColor } from '../../lib/music/compose/colors';
import { synth } from '../../music/synth';
import { SCALE_TYPE_LABELS } from '../../lib/music/theory/scales';

export function ChordPalette({
  comp,
  seventh,
  onPick,
}: {
  comp: Composition;
  /** Whether a placed/previewed chord sounds as a four-note seventh. */
  seventh: boolean;
  onPick: (degree: Degree) => void;
}) {
  const diatonic = useMemo(
    () => getDiatonicChords(keyToScaleSelection(comp)),
    [comp],
  );

  return (
    <div style={{ display: 'flex', flexWrap: 'wrap', alignItems: 'center', gap: 6 }}>
      <span style={{ marginRight: 4, fontSize: 11, color: 'var(--ink-dim)' }}>
        Chords in{' '}
        <span style={{ fontWeight: 600, color: 'var(--ink)' }}>
          {comp.key.root} {SCALE_TYPE_LABELS[comp.key.mode === 'major' ? 'major' : 'minor']}
        </span>
      </span>
      {diatonic.map((c) => {
        const label = triadLabel(c);
        return (
          <button
            key={c.degree}
            type="button"
            className="cmp-chip"
            onClick={() => {
              onPick(c.degree as Degree);
              const midis = resolveChordMidis(comp, c.degree as Degree, seventh);
              if (midis.length) synth.playChord(midis, 700, 'string');
            }}
            style={{ backgroundColor: degreeColor(c.degree), borderColor: degreeColor(c.degree) }}
            title={`${label.name} — click a bar first to place, or a block to change it`}
          >
            <span style={{ fontSize: 13, fontWeight: 700, lineHeight: 1 }}>{label.roman}</span>
            <span style={{ fontSize: 10, lineHeight: 1.1, opacity: 0.9 }}>{label.name}</span>
          </button>
        );
      })}
    </div>
  );
}
