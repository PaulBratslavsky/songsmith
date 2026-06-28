// Progression Composer (the NEW visual builder) — top-level. Owns the
// Composition and assembles the Hookpad-style stacked layout: transport,
// chord palette, then a shared 8-bar timeline with a melody piano-roll,
// the draggable chord lane, and a bass piano-roll. Span/note mutations go
// through the pure helpers in spans.ts; this component only wires state to
// UI.
//
// Time is in ticks (sixteenth resolution). A duration picker sets the
// length of newly-placed chords and notes; everything is draggable and
// resizable afterwards.
//
// NOTE: this is distinct from the chord-section editor in
// components/Composer.tsx and the /builder Chord Builder. Composition
// state is in-memory only — persistence (libSQL) is Phase 3; the
// useCompositionState `load`/`reset` actions + schema.ts are the seams.

import { useEffect, useMemo, useRef, useState } from 'react';
import { PITCH_CLASSES, type PitchClass } from '../../lib/music/types';
import {
  DURATIONS,
  TOTAL_TICKS,
  emptyComposition,
  type Composition,
  type Degree,
  type KeyMode,
} from '../../lib/music/compose/types';
import { LABEL_W } from './laneLayout';
import { useCompositionState } from '../../lib/music/compose/useCompositionState';
import { useCompositionPlayback } from '../../lib/music/compose/useCompositionPlayback';
import { synth } from '../../music/synth';
import {
  keyToScaleSelection,
  resolveMelodyMidi,
  resolveBassMidi,
} from '../../lib/music/compose/playback';
import { getScalePitchClasses } from '../../lib/music/theory/scales';
import { getDiatonicChords } from '../../lib/music/theory/diatonic';
import {
  chordToneDegrees,
  degreeLabel,
  type DegreeLabel,
} from '../../lib/music/compose/labels';
import { degreeColor, hexToRgba } from '../../lib/music/compose/colors';
import type { ChordToneHighlight } from './chordHighlight';
import { BeatRuler } from './BeatRuler';
import { ChordPalette } from './ChordPalette';
import { ChordLane } from './ChordLane';
import { NoteLane } from './NoteLane';

const MELODY_COLOR = '#2563eb';
const BASS_COLOR = '#9333ea';

/** A small demo sketch so the surface isn't blank on first open. */
function demoComposition(id: string): Composition {
  const c = emptyComposition(id, 'Sketch', 'C', 'major');
  c.chords = [
    { id: 'd-c1', degree: 1, seventh: false, start: 0, length: 16 },
    { id: 'd-c2', degree: 5, seventh: false, start: 16, length: 16 },
    { id: 'd-c3', degree: 6, seventh: false, start: 32, length: 16 },
    { id: 'd-c4', degree: 4, seventh: false, start: 48, length: 16 },
  ];
  c.melody = [
    { id: 'd-m1', degree: 1, octave: 0, start: 0, length: 4 },
    { id: 'd-m2', degree: 3, octave: 0, start: 4, length: 4 },
    { id: 'd-m3', degree: 5, octave: 0, start: 8, length: 8 },
    { id: 'd-m4', degree: 5, octave: 0, start: 16, length: 4 },
    { id: 'd-m5', degree: 4, octave: 0, start: 20, length: 4 },
    { id: 'd-m6', degree: 2, octave: 0, start: 24, length: 8 },
  ];
  c.bass = [
    { id: 'd-b1', degree: 1, octave: 0, start: 0, length: 16 },
    { id: 'd-b2', degree: 5, octave: 0, start: 16, length: 16 },
    { id: 'd-b3', degree: 6, octave: 0, start: 32, length: 16 },
    { id: 'd-b4', degree: 4, octave: 0, start: 48, length: 16 },
  ];
  return c;
}

export function Composer({ initialRoot = 'C' }: { initialRoot?: PitchClass }) {
  // Composition + edit state (comp, cursor, selected) live in the reducer;
  // only ephemeral UI stays local here. Seeded with a small demo sketch.
  const { comp, cursor, selected, actions } = useCompositionState(
    initialRoot,
    demoComposition,
  );
  const selectedId = selected?.id ?? null; // for lane render (selection ring)

  const [muted, setMuted] = useState(false);
  const [durTicks, setDurTicks] = useState(4); // default 1/4 note
  const [loop, setLoop] = useState(true);
  // Sticky placement mode: newly-dropped chords are sevenths while on.
  const [seventhMode, setSeventhMode] = useState(false);

  const { isPlaying, currentStep, toggle, stop } = useCompositionPlayback(comp, {
    loop,
  });

  useEffect(() => {
    synth.setMuted(muted);
  }, [muted]);

  // Note placement carries the duration-picker length, read via a ref so
  // the stable handler bundles below don't churn when the duration changes.
  const durRef = useRef(durTicks);
  durRef.current = durTicks;

  // Key-derived data (changes only when the key changes) so the memoized
  // lanes don't re-render on note/chord edits or playback ticks.
  const scaleSel = useMemo(
    () => keyToScaleSelection(comp),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- key-only
    [comp.key.root, comp.key.mode],
  );
  const pcs = useMemo(() => getScalePitchClasses(scaleSel), [scaleSel]);
  const labels = useMemo(() => {
    const m: Record<number, DegreeLabel> = {};
    for (const c of getDiatonicChords(scaleSel)) m[c.degree] = degreeLabel(c);
    return m;
  }, [scaleSel]);

  // Latest preview fns (depend on key) read through a ref so the stable
  // handler bundles don't change identity when the composition edits.
  const previewRef = useRef<{ melody: (d: Degree) => void; bass: (d: Degree) => void }>({
    melody: () => {},
    bass: () => {},
  });
  previewRef.current = {
    melody: (d) => {
      const midi = resolveMelodyMidi(comp, { degree: d, octave: 0 });
      if (midi != null) synth.playNote(midi, 260, 'piano');
    },
    bass: (d) => {
      const midi = resolveBassMidi(comp, { degree: d, octave: 0 });
      if (midi != null) synth.playNote(midi, 260, 'bass');
    },
  };

  // Stable handler bundles (actions is stable; dur/preview via refs) so
  // the memoized lanes only re-render when their own data changes.
  const melodyHandlers = useMemo(
    () => ({
      onPlace: (degree: Degree, tick: number) => actions.placeNote('melody', degree, tick, durRef.current),
      onSelect: (id: string | null) => (id ? actions.select('melody', id) : actions.deselect()),
      onMove: (id: string, s: number, degree: Degree) => actions.moveNote('melody', id, s, degree),
      onResize: (id: string, l: number) => actions.resizeNote('melody', id, l),
      onRemove: (id: string) => actions.removeNote('melody', id),
      previewNote: (d: Degree) => previewRef.current.melody(d),
    }),
    [actions],
  );
  const bassHandlers = useMemo(
    () => ({
      onPlace: (degree: Degree, tick: number) => actions.placeNote('bass', degree, tick, durRef.current),
      onSelect: (id: string | null) => (id ? actions.select('bass', id) : actions.deselect()),
      onMove: (id: string, s: number, degree: Degree) => actions.moveNote('bass', id, s, degree),
      onResize: (id: string, l: number) => actions.resizeNote('bass', id, l),
      onRemove: (id: string) => actions.removeNote('bass', id),
      previewNote: (d: Degree) => previewRef.current.bass(d),
    }),
    [actions],
  );
  const chordHandlers = useMemo(
    () => ({
      onSelect: (id: string | null) => (id ? actions.select('chord', id) : actions.deselect()),
      onSetCursor: actions.selectBar,
      onMove: actions.moveChord,
      onResize: actions.resizeChord,
      onRemove: actions.removeChord,
    }),
    [actions],
  );

  // ---- keyboard: delete selected, escape to deselect ----
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const typing =
        el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA' || el.tagName === 'SELECT');
      if (e.key === 'Escape') {
        actions.deselect();
        return;
      }
      if ((e.key === 'Delete' || e.key === 'Backspace') && selected && !typing) {
        e.preventDefault();
        if (selected.kind === 'chord') actions.removeChord(selected.id);
        else actions.removeNote(selected.kind, selected.id);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selected, actions]);

  // ---- selected-chord tone highlight in the melody grid ----
  const selectedChord =
    selected?.kind === 'chord'
      ? comp.chords.find((s) => s.id === selected.id)
      : undefined;
  const melodyHighlight = useMemo<ChordToneHighlight | null>(
    () =>
      selectedChord
        ? {
            degrees: new Set(
              chordToneDegrees(selectedChord.degree, selectedChord.seventh),
            ),
            start: selectedChord.start,
            length: selectedChord.length,
            color: hexToRgba(degreeColor(selectedChord.degree), 0.28),
          }
        : null,
    [selectedChord],
  );

  const clearAll = () => {
    stop();
    actions.clearAll();
  };

  return (
    <div className="col" style={{ gap: 14 }}>
      {/* Transport */}
      <div className="card cmp-transport">
        <button
          type="button"
          className="primary"
          onClick={toggle}
          style={{ minWidth: 84 }}
        >
          {isPlaying ? '■ Stop' : '▶ Play'}
        </button>

        <div className="row" style={{ alignItems: 'center', gap: 6 }}>
          <span className="cmp-cap">Key</span>
          <div className="row" style={{ flexWrap: 'wrap', gap: 3 }}>
            {PITCH_CLASSES.map((pc) => (
              <button
                key={pc}
                type="button"
                className={'sm' + (comp.key.root === pc ? ' primary' : '')}
                onClick={() => actions.setKeyRoot(pc)}
              >
                {pc}
              </button>
            ))}
          </div>
        </div>

        <div className="row" style={{ gap: 3 }}>
          {(['major', 'minor'] as KeyMode[]).map((m) => (
            <button
              key={m}
              type="button"
              className={'sm' + (comp.key.mode === m ? ' primary' : '')}
              onClick={() => actions.setKeyMode(m)}
              style={{ textTransform: 'capitalize' }}
            >
              {m}
            </button>
          ))}
        </div>

        <label className="row" style={{ margin: 0, alignItems: 'center', gap: 8, textTransform: 'none' }}>
          <span className="cmp-cap">Tempo</span>
          <input
            type="range"
            min={60}
            max={180}
            value={comp.bpm}
            onChange={(e) => actions.setBpm(Number(e.target.value))}
            style={{ width: 96, padding: 0 }}
          />
          <span style={{ width: 56, fontVariantNumeric: 'tabular-nums', color: 'var(--ink-dim)' }}>
            {comp.bpm} BPM
          </span>
        </label>

        <div className="row" style={{ alignItems: 'center', gap: 4 }}>
          <span className="cmp-cap">Note</span>
          <div className="row" style={{ gap: 3 }}>
            {DURATIONS.map((d) => (
              <button
                key={d.ticks}
                type="button"
                className={'sm' + (durTicks === d.ticks ? ' primary' : '')}
                onClick={() => setDurTicks(d.ticks)}
              >
                {d.label}
              </button>
            ))}
          </div>
        </div>

        <button type="button" className="sm" onClick={() => setMuted((m) => !m)}>
          {muted ? '🔇 Muted' : '🔊 Sound'}
        </button>

        <button
          type="button"
          className={'sm' + (loop ? ' primary' : '')}
          onClick={() => setLoop((l) => !l)}
          title="Loop playback"
        >
          ↻ Loop {loop ? 'on' : 'off'}
        </button>

        <button type="button" className="sm ghost" onClick={clearAll} style={{ marginLeft: 'auto' }}>
          Clear
        </button>
      </div>

      {/* Name field (in-memory; persistence is Phase 3) */}
      <div className="row" style={{ flexWrap: 'wrap', gap: 8, alignItems: 'center' }}>
        <input
          type="text"
          value={comp.name}
          onChange={(e) => actions.setName(e.target.value)}
          placeholder="Composition name"
          style={{ width: 200 }}
        />
        <button
          type="button"
          className="sm ghost"
          onClick={() => {
            stop();
            actions.reset(comp.key.root, comp.key.mode);
          }}
        >
          New blank
        </button>
        <span className="faint" style={{ fontSize: 11 }}>
          in-memory sketch · saving comes in Phase 3
        </span>
      </div>

      {/* Palette */}
      <div className="row" style={{ flexWrap: 'wrap', alignItems: 'center', gap: 12 }}>
        <ChordPalette
          comp={comp}
          seventh={selectedChord ? selectedChord.seventh : seventhMode}
          onPick={(degree) => actions.pickDegree(degree, seventhMode)}
        />
        <button
          type="button"
          className={
            'sm' + ((selectedChord ? selectedChord.seventh : seventhMode) ? ' primary' : '')
          }
          onClick={() => {
            if (selectedChord) {
              actions.setChordSeventh(selectedChord.id, !selectedChord.seventh);
            } else {
              setSeventhMode((m) => !m);
            }
          }}
          aria-pressed={selectedChord ? selectedChord.seventh : seventhMode}
          title={
            selectedChord
              ? 'Toggle the selected chord between triad and seventh'
              : 'Place new chords as four-note sevenths'
          }
        >
          7th
        </button>
      </div>
      <p className="faint" style={{ marginTop: -6, fontSize: 11 }}>
        {selectedChord
          ? 'Chord selected — pick a palette chip to change it, "7th" to toggle the seventh, drag its body to move, the right edge to extend, or × to remove.'
          : `Click a beat in the chord lane to set where the next chord lands, then a palette chip. "7th" ${seventhMode ? 'is on — new chords are sevenths.' : 'makes new chords sevenths.'} Click melody/bass cells to add notes at the chosen duration.`}
      </p>

      {/* Timeline */}
      <div className="card" style={{ overflowX: 'auto' }}>
        <div style={{ position: 'relative', minWidth: 820 }}>
          {/* Moving playhead — spans all lanes at the current tick. The
              track starts after the LABEL_W gutter, so offset by it. */}
          {currentStep != null && (
            <div
              style={{
                pointerEvents: 'none',
                position: 'absolute',
                bottom: 0,
                top: 16,
                zIndex: 10,
                width: 2,
                background: 'var(--accent)',
                opacity: 0.7,
                left: `calc(${LABEL_W} + (100% - ${LABEL_W}) * ${(currentStep + 0.5) / TOTAL_TICKS})`,
              }}
            />
          )}
          <BeatRuler />
          <div style={{ marginTop: 4 }}>
            <NoteLane
              lane="melody"
              notes={comp.melody}
              pcs={pcs}
              color={MELODY_COLOR}
              highlight={melodyHighlight}
              selectedId={selectedId}
              {...melodyHandlers}
            />
          </div>
          <div style={{ margin: '6px 0' }}>
            <ChordLane
              chords={comp.chords}
              labels={labels}
              selectedId={selectedId}
              cursor={cursor}
              {...chordHandlers}
            />
          </div>
          <NoteLane
            lane="bass"
            notes={comp.bass}
            pcs={pcs}
            color={BASS_COLOR}
            selectedId={selectedId}
            {...bassHandlers}
          />
          <div className="row" style={{ marginTop: 8, gap: 16, fontSize: 10, color: 'var(--ink-faint)' }}>
            <span className="row" style={{ gap: 4, alignItems: 'center' }}>
              <span style={{ display: 'inline-block', height: 8, width: 8, borderRadius: 2, backgroundColor: MELODY_COLOR }} />
              Melody
            </span>
            <span className="row" style={{ gap: 4, alignItems: 'center' }}>
              <span style={{ display: 'inline-block', height: 8, width: 8, borderRadius: 2, backgroundColor: BASS_COLOR }} />
              Bass
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
