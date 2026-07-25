// Progression Composer — playback hook. Owns the playback clock
// (sixteenth-tick resolution) and is the only piece that touches the
// synth. The pure schedule comes from buildSchedule; this walks the tick
// cursor and fires the synth with each layer's own voice + sustain.
//
// N2 timing upgrade: notes are no longer fired from a setTimeout tick
// (which drifts) — a LOOKAHEAD scheduler runs on a coarse ~25ms timer
// and schedules every tick's audio ~100ms ahead at EXACT audio-clock
// times (synth.playNote/playChord take an `at`). The same timer then
// maps audio time -> tick for the UI: each scheduled tick carries its
// start time, and the cursor state is set once when that boundary
// passes — never more than once per tick, exactly the old discipline
// (activeChordId / activeLineTick additionally only SET on change).
//
// The schedule, tempo, and voices are read through refs so that editing
// the composition *while it plays* takes effect on the next scheduled
// tick WITHOUT re-arming the clock (which would snap the cursor back to
// tick 0). Only starting/stopping and the loop flag re-arm it. On stop,
// notes already sounding ring out their envelope (as before) but the
// ~100ms of lookahead notes that haven't started yet are cancelled.

import { useEffect, useMemo, useRef, useState } from 'react';
import { synth, type Voice } from '../../../music/synth';
import type { Composition } from './types';
import { buildSchedule, msPerTick } from './playback';
import { spanAt } from './spans';

/** Per-lane voice mapping (melody/chords/bass keep distinct timbres). */
export type LaneVoices = { melody: Voice; chord: Voice; bass: Voice };

/** The original oscillator mapping — the default, byte-identical path. */
export const SYNTH_VOICES: LaneVoices = {
  melody: 'piano',
  chord: 'string',
  bass: 'bass',
};

/** N2 sampled mapping (falls back per-voice while samples load). */
export const SAMPLED_VOICES: LaneVoices = {
  melody: 'piano-sampled',
  chord: 'strings-sampled',
  bass: 'bass-sampled',
};

/** Schedule audio this far ahead of the audio clock (seconds)... */
const LOOKAHEAD_S = 0.1;
/** ...checking for due work this often (ms). Coarser than any tick
 *  (a sixteenth at 180 BPM is ~83ms), so UI updates stay per-tick. */
const TIMER_MS = 25;

export type CompositionPlayback = {
  isPlaying: boolean;
  /** Current tick cursor 0..totalTicks-1, or null when stopped. */
  currentStep: number | null;
  /** Id of the chord span under the playhead, or null (stopped / gap).
   *  Derived from the tick but only SET when it changes, so consumers
   *  (the lyric sheet) re-render per chord, never per tick. */
  activeChordId: string | null;
  /** Anchor tick of the ACTIVE lyric line — the line whose
   *  [anchor, nextAnchor) range contains the playhead — or null (stopped /
   *  before the first line). Anchors are unique per line (see
   *  compositionFromSong), so this identifies exactly one line. Same
   *  discipline as activeChordId: derived per tick but only SET on a line
   *  boundary, never per tick. */
  activeLineTick: number | null;
  play: () => void;
  stop: () => void;
  toggle: () => void;
};

export function useCompositionPlayback(
  comp: Composition,
  opts: {
    loop?: boolean;
    voices?: LaneVoices;
    /** Phase 3 focus mode: confine the transport to [start, end) ticks.
     *  Play starts at `start`; the loop wraps (or playback ends) at `end`.
     *  Changing the range re-arms the clock at the new start. */
    range?: { start: number; end: number } | null;
    /** Mute ONLY the transport's own notes (edit previews stay audible) —
     *  the A/B "hear the original render alone" side. Clock still runs. */
    mute?: boolean;
  } = {},
): CompositionPlayback {
  const loop = opts.loop ?? true;
  // The focus range + transport mute are read LIVE via refs (same discipline
  // as tempo/edits: no re-arm, no cursor reset) — except a range MOVE, which
  // re-arms via rangeKey so play restarts at the new section's start.
  const rangeKey = opts.range ? `${opts.range.start}:${opts.range.end}` : '';
  const rangeRef = useRef(opts.range ?? null);
  rangeRef.current = opts.range ?? null;
  const muteRef = useRef(!!opts.mute);
  muteRef.current = !!opts.mute;
  const [isPlaying, setIsPlaying] = useState(false);
  const [currentStep, setCurrentStep] = useState<number | null>(null);
  const [activeChordId, setActiveChordId] = useState<string | null>(null);
  const [activeLineTick, setActiveLineTick] = useState<number | null>(null);

  const eventsByStep = useMemo(() => {
    const map = new Map<number, ReturnType<typeof buildSchedule>[number]>();
    for (const e of buildSchedule(comp)) map.set(e.step, e);
    return map;
  }, [comp]);

  // Latest schedule + tempo + voices, read live by the running scheduler
  // so edits (and tempo nudges, and the Synth/Sampled picker) apply to
  // the next scheduled tick without resetting position.
  const eventsRef = useRef(eventsByStep);
  eventsRef.current = eventsByStep;
  const tickMsRef = useRef(msPerTick(comp.bpm));
  tickMsRef.current = msPerTick(comp.bpm);
  const voicesRef = useRef(opts.voices ?? SYNTH_VOICES);
  voicesRef.current = opts.voices ?? SYNTH_VOICES;
  // Read live so a key/length change while playing wraps at the right tick.
  const totalTicksRef = useRef(comp.totalTicks);
  totalTicksRef.current = comp.totalTicks;
  // Chord spans, read live for the active-chord derivation.
  const chordsRef = useRef(comp.chords);
  chordsRef.current = comp.chords;
  // Sorted lyric anchors, read live for the active-LINE derivation (line i
  // is active on [anchor_i, anchor_i+1); the last line runs to the end).
  const lyricAnchors = useMemo(
    () => Array.from(new Set(comp.lyrics.map((l) => l.tick))).sort((a, b) => a - b),
    [comp.lyrics],
  );
  const anchorsRef = useRef(lyricAnchors);
  anchorsRef.current = lyricAnchors;

  const stepRef = useRef(0);
  // Last derived chord id / line anchor — state is only set when these
  // change, so the per-tick clock never causes a per-tick re-render
  // downstream.
  const lastChordIdRef = useRef<string | null>(null);
  const lastLineTickRef = useRef<number | null>(null);

  useEffect(() => {
    if (!isPlaying) {
      setCurrentStep(null);
      lastChordIdRef.current = null;
      setActiveChordId(null);
      lastLineTickRef.current = null;
      setActiveLineTick(null);
      return;
    }

    const noteActiveChord = (step: number) => {
      const id = spanAt(chordsRef.current, step)?.id ?? null;
      if (id !== lastChordIdRef.current) {
        lastChordIdRef.current = id;
        setActiveChordId(id);
      }
      // Active line = greatest anchor <= step (binary search; anchors are
      // sorted ascending). Null before the first line.
      const anchors = anchorsRef.current;
      let line: number | null = null;
      let lo = 0;
      let hi = anchors.length - 1;
      while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (anchors[mid] <= step) {
          line = anchors[mid];
          lo = mid + 1;
        } else {
          hi = mid - 1;
        }
      }
      if (line !== lastLineTickRef.current) {
        lastLineTickRef.current = line;
        setActiveLineTick(line);
      }
    };

    // Live clamped focus bounds (whole piece when unfocused). Read per tick
    // so a length edit while playing still wraps at the right place.
    const boundStart = () => {
      const r = rangeRef.current;
      return r ? Math.max(0, Math.min(r.start, totalTicksRef.current - 1)) : 0;
    };
    const boundEnd = () => {
      const r = rangeRef.current;
      return r
        ? Math.max(boundStart() + 1, Math.min(r.end, totalTicksRef.current))
        : totalTicksRef.current;
    };

    // Schedule one tick's notes at exact audio time `when`, remembering a
    // cancel so stop() can drop lookahead notes that haven't started yet.
    const pending: { time: number; cancel: () => void }[] = [];
    const fire = (step: number, when: number) => {
      if (muteRef.current) return; // A/B: original only — clock runs, synth silent
      const e = eventsRef.current.get(step);
      if (!e) return;
      const tickMs = tickMsRef.current;
      const v = voicesRef.current;
      const cancels: (() => void)[] = [];
      // Each layer sustains for its span length and uses its own voice.
      if (e.chord) cancels.push(synth.playChord(e.chord, tickMs * (e.chordTicks ?? 1) * 0.97, v.chord, when));
      if (e.melody != null) cancels.push(synth.playNote(e.melody, tickMs * (e.melodyTicks ?? 1) * 0.95, v.melody, when));
      if (e.bass != null) cancels.push(synth.playNote(e.bass, tickMs * (e.bassTicks ?? 1) * 0.97, v.bass, when));
      if (cancels.length) pending.push({ time: when, cancel: () => cancels.forEach((c) => c()) });
    };

    // ---- lookahead state (local to this run; edits flow in via refs) ----
    let nextStep = boundStart(); // next tick to SCHEDULE (audio side)
    let nextTime = synth.now() + 0.05; // its audio-clock start time
    let endAt: number | null = null; // non-loop: when the last tick finishes
    // UI boundary queue: each scheduled tick with its start time; the UI
    // side pops due entries and moves the cursor once per boundary.
    const uiQueue: { step: number; time: number }[] = [];

    const scheduleAhead = () => {
      const horizon = synth.now() + LOOKAHEAD_S;
      while (endAt == null && nextTime < horizon) {
        fire(nextStep, nextTime);
        uiQueue.push({ step: nextStep, time: nextTime });
        // Tempo is read PER TICK, so a live bpm change stretches from the
        // next scheduled tick onward — the cursor never resets.
        nextTime += tickMsRef.current / 1000;
        const n = nextStep + 1;
        if (n >= boundEnd()) {
          if (loop) nextStep = boundStart();
          else endAt = nextTime; // let the last tick play out, then stop
        } else {
          nextStep = n;
        }
      }
    };

    const updateUi = () => {
      const now = synth.now();
      // Started notes no longer need their stop()-cancel kept around.
      while (pending.length && pending[0].time <= now) pending.shift();
      // Advance the cursor to the LATEST boundary that has passed —
      // at most one state set per timer run, one run per tick boundary.
      let due: { step: number } | null = null;
      while (uiQueue.length && uiQueue[0].time <= now) due = uiQueue.shift()!;
      if (due) {
        stepRef.current = due.step;
        setCurrentStep(due.step);
        noteActiveChord(due.step);
      }
      if (endAt != null && now >= endAt) {
        setIsPlaying(false);
        setCurrentStep(null);
      }
    };

    // Cursor answers immediately on play (the first tick's audio starts
    // ~50ms later on the audio clock; its queued boundary re-set is a no-op).
    stepRef.current = nextStep;
    setCurrentStep(nextStep);
    noteActiveChord(nextStep);
    scheduleAhead();

    const timer = setInterval(() => {
      scheduleAhead();
      updateUi();
    }, TIMER_MS);

    return () => {
      clearInterval(timer);
      // Notes already sounding ring out (old behavior); lookahead notes
      // that haven't started yet are cancelled so stop is immediate.
      const now = synth.now();
      for (const p of pending) if (p.time > now) p.cancel();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- everything else rides refs
  }, [isPlaying, loop, rangeKey]);

  return {
    isPlaying,
    currentStep,
    activeChordId,
    activeLineTick,
    play: () => setIsPlaying(true),
    stop: () => setIsPlaying(false),
    toggle: () => setIsPlaying((p) => !p),
  };
}
