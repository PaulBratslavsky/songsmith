// Minimal Web Audio synth — plays absolute chords so you can hear a progression.
// Preview only (not full-song audio generation). Voiced from real chord tones.
//
// The Composer (lib/music/compose/*) needs distinct timbres for melody,
// chords, and bass, so the synth gained selectable *voices* (oscillator
// type + envelope + optional lowpass) and a small `synth` object exposing
// playNote / playChord / setMuted with a trailing `voice` arg. The original
// playChord / playChordAt / playSequence exports are unchanged so existing
// call sites (Chord Builder, Circle of Fifths) keep working.

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

export type Voice = "default" | "piano" | "string" | "bass";

type VoiceParams = {
  type: OscillatorType;
  /** Attack time in seconds. */
  attack: number;
  /** Peak gain for one voice (kept low for layered voices like chords). */
  peak: number;
  /** Lowpass cutoff in Hz, or null for no filter. Tames buzzy sawtooths. */
  cutoff: number | null;
};

const VOICES: Record<Voice, VoiceParams> = {
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

  /** Play one MIDI note with the given voice for `durationMs`. */
  playNote(midi: number, durationMs = 600, voice: Voice = "default"): void {
    if (this.muted) return;
    const { ac, master } = this.ensure();
    const v = VOICES[voice];
    const freq = midiToHz(midi);
    const osc = ac.createOscillator();
    const gain = ac.createGain();
    osc.type = v.type;
    osc.frequency.setValueAtTime(freq, ac.currentTime);

    const t = ac.currentTime;
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
  }

  /** Play multiple notes at once (chord). */
  playChord(midis: number[], durationMs = 900, voice: Voice = "default"): void {
    midis.forEach((m) => this.playNote(m, durationMs, voice));
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
