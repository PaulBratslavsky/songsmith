// Progression Composer — playback hook. Owns the tick clock (sixteenth
// resolution) and is the only piece that touches the synth. The pure
// schedule comes from buildSchedule; this walks the tick cursor and
// fires the synth with each layer's own voice + sustain.
//
// The schedule and tempo are read through refs so that editing the
// composition *while it plays* takes effect on the next tick WITHOUT
// re-arming the interval (which would snap the cursor back to tick 0).
// Only starting/stopping and the loop flag re-arm the clock.

import { useEffect, useMemo, useRef, useState } from 'react';
import { synth } from '../../../music/synth';
import type { Composition } from './types';
import { buildSchedule, msPerTick } from './playback';
import { spanAt } from './spans';

export type CompositionPlayback = {
  isPlaying: boolean;
  /** Current tick cursor 0..totalTicks-1, or null when stopped. */
  currentStep: number | null;
  /** Id of the chord span under the playhead, or null (stopped / gap).
   *  Derived from the tick but only SET when it changes, so consumers
   *  (the lyric sheet) re-render per chord, never per tick. */
  activeChordId: string | null;
  play: () => void;
  stop: () => void;
  toggle: () => void;
};

export function useCompositionPlayback(
  comp: Composition,
  opts: { loop?: boolean } = {},
): CompositionPlayback {
  const loop = opts.loop ?? true;
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentStep, setCurrentStep] = useState<number | null>(null);
  const [activeChordId, setActiveChordId] = useState<string | null>(null);

  const eventsByStep = useMemo(() => {
    const map = new Map<number, ReturnType<typeof buildSchedule>[number]>();
    for (const e of buildSchedule(comp)) map.set(e.step, e);
    return map;
  }, [comp]);

  // Latest schedule + tempo, read live by the running interval so edits
  // (and tempo nudges) apply on the next tick without resetting position.
  const eventsRef = useRef(eventsByStep);
  eventsRef.current = eventsByStep;
  const tickMsRef = useRef(msPerTick(comp.bpm));
  tickMsRef.current = msPerTick(comp.bpm);
  // Read live so a key/length change while playing wraps at the right tick.
  const totalTicksRef = useRef(comp.totalTicks);
  totalTicksRef.current = comp.totalTicks;
  // Chord spans, read live for the active-chord derivation.
  const chordsRef = useRef(comp.chords);
  chordsRef.current = comp.chords;

  const stepRef = useRef(0);
  // Last derived chord id — state is only set when this changes, so the
  // per-tick clock never causes a per-tick re-render downstream.
  const lastChordIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (!isPlaying) {
      setCurrentStep(null);
      lastChordIdRef.current = null;
      setActiveChordId(null);
      return;
    }

    const noteActiveChord = (step: number) => {
      const id = spanAt(chordsRef.current, step)?.id ?? null;
      if (id !== lastChordIdRef.current) {
        lastChordIdRef.current = id;
        setActiveChordId(id);
      }
    };

    const fire = (step: number) => {
      const e = eventsRef.current.get(step);
      if (!e) return;
      const tickMs = tickMsRef.current;
      // Each layer sustains for its span length and uses its own voice.
      if (e.chord) synth.playChord(e.chord, tickMs * (e.chordTicks ?? 1) * 0.97, 'string');
      if (e.melody != null) synth.playNote(e.melody, tickMs * (e.melodyTicks ?? 1) * 0.95, 'piano');
      if (e.bass != null) synth.playNote(e.bass, tickMs * (e.bassTicks ?? 1) * 0.97, 'bass');
    };

    stepRef.current = 0;
    setCurrentStep(0);
    noteActiveChord(0);
    fire(0);

    // Self-scheduling timeout (re-read tickMs each tick) so a live tempo
    // change takes effect without re-arming and losing the cursor.
    let timer: ReturnType<typeof setTimeout>;
    const advance = () => {
      const next = stepRef.current + 1;
      if (next >= totalTicksRef.current) {
        if (!loop) {
          setIsPlaying(false);
          setCurrentStep(null);
          return;
        }
        stepRef.current = 0;
      } else {
        stepRef.current = next;
      }
      setCurrentStep(stepRef.current);
      noteActiveChord(stepRef.current);
      fire(stepRef.current);
      timer = setTimeout(advance, tickMsRef.current);
    };
    timer = setTimeout(advance, tickMsRef.current);

    return () => clearTimeout(timer);
  }, [isPlaying, loop]);

  return {
    isPlaying,
    currentStep,
    activeChordId,
    play: () => setIsPlaying(true),
    stop: () => setIsPlaying(false),
    toggle: () => setIsPlaying((p) => !p),
  };
}
