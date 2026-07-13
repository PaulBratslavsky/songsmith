// Minimal Web Audio synth — plays absolute chords so you can hear a progression.
// Preview only (not full-song audio generation). Voiced from real chord tones.
//
// The Composer (lib/music/compose/*) needs distinct timbres for melody,
// chords, and bass, so the synth gained selectable *voices* (oscillator
// type + envelope + optional lowpass) and a small `synth` object exposing
// playNote / playChord / setMuted with a trailing `voice` arg. The original
// playChord / playChordAt / playSequence exports are unchanged so existing
// call sites (Chord Builder, Circle of Fifths) keep working.
//
// N2 adds SAMPLED voices ('piano-sampled' / 'strings-sampled' /
// 'bass-sampled') backed by vendored FluidR3_GM notes (music/soundfonts).
// They ride the same playNote/playChord API and the same master gain
// (mute works); while the sample chunks load — or if decoding fails —
// each sampled voice gracefully falls back to its oscillator sibling.
// playNote/playChord also grew an optional `at` (audio-clock start time,
// for the lookahead scheduler) and return a cancel fn; existing callers
// that ignore both are unaffected.

let ctx: AudioContext | null = null;
function audio(): AudioContext {
  if (!ctx) ctx = new (window.AudioContext || (window as any).webkitAudioContext)();
  return ctx;
}

const midiToHz = (m: number) => 440 * Math.pow(2, (m - 69) / 12);

/** Play a chord (MIDI notes) for `dur` seconds starting at audio-time `at`. */
export function playChordAt(notes: number[], at: number, dur: number) {
  const ac = audio();
  const master = ac.createGain();
  master.gain.value = 0.0001;
  master.connect(ac.destination);
  master.gain.setValueAtTime(0.0001, at);
  master.gain.exponentialRampToValueAtTime(0.18, at + 0.02);
  master.gain.exponentialRampToValueAtTime(0.0001, at + dur);

  for (const n of notes) {
    const osc = ac.createOscillator();
    osc.type = "triangle";
    osc.frequency.value = midiToHz(n);
    osc.connect(master);
    osc.start(at);
    osc.stop(at + dur + 0.05);
  }
}

export function playChord(notes: number[], dur = 1.0) {
  const ac = audio();
  if (ac.state === "suspended") ac.resume();
  playChordAt(notes, ac.currentTime + 0.02, dur);
}

export type Step = { notes: number[]; beats: number };

/** Play a sequence of chords at `bpm`. Calls `onStep(index)` as each plays.
 *  Returns a stop() handle. */
export function playSequence(steps: Step[], bpm: number, onStep?: (i: number) => void): () => void {
  const ac = audio();
  if (ac.state === "suspended") ac.resume();
  const secPerBeat = 60 / bpm;
  let t = ac.currentTime + 0.06;
  const timers: number[] = [];
  steps.forEach((s, i) => {
    const dur = s.beats * secPerBeat;
    playChordAt(s.notes, t, dur * 0.95);
    const fireAt = (t - ac.currentTime) * 1000;
    timers.push(window.setTimeout(() => onStep?.(i), fireAt));
    t += dur;
  });
  timers.push(window.setTimeout(() => onStep?.(-1), (t - ac.currentTime) * 1000));
  return () => timers.forEach(clearTimeout);
}

// ---- Per-voice synth (Composer) ----

import { getSample, loadSamples, samplesReady, type SampledInstrument } from "./soundfonts";

/** The original oscillator voices (VOICES holds their params). */
type OscVoice = "default" | "piano" | "string" | "bass";

export type Voice = OscVoice | "piano-sampled" | "strings-sampled" | "bass-sampled";

/** No-op cancel for paths with nothing to stop. */
const NOOP = () => {};

type SampledParams = {
  instrument: SampledInstrument;
  /** Oscillator voice used while samples load / if decoding failed. */
  fallback: OscVoice;
  /** Per-note gain (samples are hotter than the oscillator peaks). */
  peak: number;
};

const SAMPLED: Partial<Record<Voice, SampledParams>> = {
  "piano-sampled": { instrument: "piano", fallback: "piano", peak: 0.9 },
  "strings-sampled": { instrument: "strings", fallback: "string", peak: 0.5 },
  "bass-sampled": { instrument: "bass", fallback: "bass", peak: 0.9 },
};

type VoiceParams = {
  type: OscillatorType;
  /** Attack time in seconds. */
  attack: number;
  /** Peak gain for one voice (kept low for layered voices like chords). */
  peak: number;
  /** Lowpass cutoff in Hz, or null for no filter. Tames buzzy sawtooths. */
  cutoff: number | null;
};

const VOICES: Record<OscVoice, VoiceParams> = {
  default: { type: "triangle", attack: 0.005, peak: 0.5, cutoff: null },
  // Melody — bright and percussive.
  piano: { type: "triangle", attack: 0.003, peak: 0.5, cutoff: 4500 },
  // Chords — softer sawtooth pad, filtered so stacked notes don't buzz.
  string: { type: "sawtooth", attack: 0.05, peak: 0.26, cutoff: 2200 },
  // Bass — smooth sine with a touch more level.
  bass: { type: "sine", attack: 0.006, peak: 0.6, cutoff: 800 },
};

class VoiceSynth {
  private masterGain: GainNode | null = null;
  private masterCtx: AudioContext | null = null;
  private muted = false;

  private ensure(): { ac: AudioContext; master: GainNode } {
    const ac = audio();
    if (ac.state === "suspended") void ac.resume();
    if (!this.masterGain || this.masterCtx !== ac) {
      this.masterGain = ac.createGain();
      this.masterGain.gain.value = this.muted ? 0 : 0.3;
      this.masterGain.connect(ac.destination);
      this.masterCtx = ac;
    }
    return { ac, master: this.masterGain };
  }

  setMuted(muted: boolean) {
    this.muted = muted;
    if (this.masterGain) this.masterGain.gain.value = muted ? 0 : 0.3;
  }

  /** Current audio-clock time (creates/resumes the context) — the
   *  timebase the lookahead scheduler passes back as `at`. */
  now(): number {
    return this.ensure().ac.currentTime;
  }

  /** Kick off the lazy soundfont load (idempotent; resolves false on
   *  failure — sampled voices then keep falling back to oscillators). */
  preloadSampled(): Promise<boolean> {
    return loadSamples(this.ensure().ac);
  }

  /**
   * Play one MIDI note with the given voice for `durationMs`, starting
   * now or at audio-clock time `at`. Returns a cancel fn that stops the
   * note immediately (used by the scheduler to drop not-yet-started
   * lookahead notes on stop); existing callers ignore it.
   */
  playNote(midi: number, durationMs = 600, voice: Voice = "default", at?: number): () => void {
    if (this.muted) return NOOP;
    const sampled = SAMPLED[voice];
    if (sampled) {
      if (!samplesReady()) {
        // First sampled request triggers the load; sound the oscillator
        // sibling in the meantime so playback never goes silent.
        void this.preloadSampled();
        return this.playNote(midi, durationMs, sampled.fallback, at);
      }
      const played = this.playSampled(midi, durationMs, sampled, at);
      // Missing sample (shouldn't happen in-range) → oscillator fallback.
      return played ?? this.playNote(midi, durationMs, sampled.fallback, at);
    }

    const { ac, master } = this.ensure();
    // Every sampled voice returned above, so only oscillator voices reach here.
    const v = VOICES[voice as OscVoice];
    const freq = midiToHz(midi);
    const osc = ac.createOscillator();
    const gain = ac.createGain();
    osc.type = v.type;

    const t = at ?? ac.currentTime;
    osc.frequency.setValueAtTime(freq, t);
    const attack = v.attack;
    const release = durationMs / 1000;
    gain.gain.setValueAtTime(0, t);
    gain.gain.linearRampToValueAtTime(v.peak, t + attack);
    gain.gain.exponentialRampToValueAtTime(0.001, t + attack + release);

    let tail: AudioNode = osc;
    let filter: BiquadFilterNode | null = null;
    if (v.cutoff != null) {
      filter = ac.createBiquadFilter();
      filter.type = "lowpass";
      filter.frequency.setValueAtTime(v.cutoff, t);
      osc.connect(filter);
      tail = filter;
    }
    tail.connect(gain);
    gain.connect(master);
    osc.start(t);
    osc.stop(t + attack + release + 0.05);
    osc.onended = () => {
      try {
        gain.disconnect();
        filter?.disconnect();
      } catch {
        /* already disconnected */
      }
    };
    return () => {
      try {
        osc.stop();
      } catch {
        /* already stopped */
      }
    };
  }

  /** Sampled playback: nearest vendored note, repitched via playbackRate,
   *  short release ramp at note end. Null when the bank lacks the note. */
  private playSampled(
    midi: number,
    durationMs: number,
    params: SampledParams,
    at?: number,
  ): (() => void) | null {
    const sample = getSample(params.instrument, midi);
    if (!sample) return null;
    const { ac, master } = this.ensure();
    const t = at ?? ac.currentTime;

    const src = ac.createBufferSource();
    src.buffer = sample.buffer;
    src.playbackRate.value = sample.rate;

    // Sustain for the note length (capped by the sample's own tail),
    // then a short release so cut-offs don't click. The samples carry
    // their natural decay, so no synthetic envelope beyond that.
    const dur = Math.min(durationMs / 1000, sample.buffer.duration / sample.rate);
    const release = 0.12;
    const gain = ac.createGain();
    gain.gain.setValueAtTime(params.peak, t);
    gain.gain.setValueAtTime(params.peak, t + dur);
    gain.gain.exponentialRampToValueAtTime(0.001, t + dur + release);

    src.connect(gain);
    gain.connect(master);
    src.start(t);
    src.stop(t + dur + release + 0.05);
    src.onended = () => {
      try {
        gain.disconnect();
      } catch {
        /* already disconnected */
      }
    };
    return () => {
      try {
        src.stop();
      } catch {
        /* already stopped */
      }
    };
  }

  /** Play multiple notes at once (chord). Returns a cancel for all. */
  playChord(midis: number[], durationMs = 900, voice: Voice = "default", at?: number): () => void {
    const cancels = midis.map((m) => this.playNote(m, durationMs, voice, at));
    return () => cancels.forEach((c) => c());
  }

  /** Play notes in sequence (arpeggio / scale ascending). */
  playSequence(midis: number[], noteDurationMs = 220, voice: Voice = "default"): void {
    if (this.muted) return;
    midis.forEach((m, i) => {
      setTimeout(() => this.playNote(m, noteDurationMs * 1.5, voice), i * noteDurationMs);
    });
  }
}

export const synth = new VoiceSynth();
