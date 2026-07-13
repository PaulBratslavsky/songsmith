// Sampled-instrument store (Composer N2 — realistic playback).
//
// Decodes the vendored FluidR3_GM note samples (assets/soundfonts/*.json,
// MIT — source + regeneration notes in the README next to the assets)
// into AudioBuffers. The JSON assets are DYNAMICALLY imported, so — like
// NotationView does with vexflow — they become their own lazy Vite
// chunks: the main bundle carries none of the sample data, and nothing
// loads until a sampled voice is first requested (or preloaded from the
// Sound picker). Local assets only; zero network fetches at runtime.
//
// Each bank keeps one velocity layer at every 3rd semitone; getSample
// returns the nearest sample plus the playbackRate that repitches it to
// the requested note (≤ 1 semitone away).

export type SampledInstrument = 'piano' | 'strings' | 'bass';

type Bank = {
  /** Sampled MIDI numbers, ascending, for nearest-note lookup. */
  midis: number[];
  buffers: Map<number, AudioBuffer>;
};

const banks = new Map<SampledInstrument, Bank>();
let loadPromise: Promise<boolean> | null = null;
let ready = false;

function base64ToArrayBuffer(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const buf = new ArrayBuffer(bin.length);
  const bytes = new Uint8Array(buf);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return buf;
}

async function decodeBank(
  ac: AudioContext,
  data: Record<string, string>,
): Promise<Bank> {
  const midis: number[] = [];
  const buffers = new Map<number, AudioBuffer>();
  const entries = Object.entries(data);
  await Promise.all(
    entries.map(async ([midiStr, b64]) => {
      const midi = Number(midiStr);
      const buf = await ac.decodeAudioData(base64ToArrayBuffer(b64));
      buffers.set(midi, buf);
    }),
  );
  for (const [midiStr] of entries) midis.push(Number(midiStr));
  midis.sort((a, b) => a - b);
  return { midis, buffers };
}

/** True once all three banks are decoded and playable. */
export function samplesReady(): boolean {
  return ready;
}

/**
 * Idempotent lazy load: dynamic-import the three JSON chunks and decode
 * them. Resolves true on success, false on failure (import or decode) —
 * callers fall back to the oscillator voices either way, so this never
 * throws.
 */
export function loadSamples(ac: AudioContext): Promise<boolean> {
  if (!loadPromise) {
    loadPromise = (async () => {
      try {
        const [piano, strings, bass] = await Promise.all([
          import('../assets/soundfonts/piano.json'),
          import('../assets/soundfonts/strings.json'),
          import('../assets/soundfonts/bass.json'),
        ]);
        const [pb, sb, bb] = await Promise.all([
          decodeBank(ac, piano.default as Record<string, string>),
          decodeBank(ac, strings.default as Record<string, string>),
          decodeBank(ac, bass.default as Record<string, string>),
        ]);
        banks.set('piano', pb);
        banks.set('strings', sb);
        banks.set('bass', bb);
        ready = true;
        return true;
      } catch (err) {
        console.warn('soundfont load failed — using oscillator voices', err);
        // Leave loadPromise set: a failed decode won't succeed on retry
        // (the assets are bundled, not fetched), so don't loop.
        return false;
      }
    })();
  }
  return loadPromise;
}

/**
 * Nearest sample for `midi` in the instrument's bank, with the
 * playbackRate that repitches it to `midi`. Null while samples aren't
 * loaded (caller falls back to the oscillator voice).
 */
export function getSample(
  instrument: SampledInstrument,
  midi: number,
): { buffer: AudioBuffer; rate: number } | null {
  const bank = banks.get(instrument);
  if (!bank || bank.midis.length === 0) return null;
  let best = bank.midis[0];
  for (const m of bank.midis) {
    if (Math.abs(m - midi) < Math.abs(best - midi)) best = m;
  }
  const buffer = bank.buffers.get(best);
  if (!buffer) return null;
  return { buffer, rate: Math.pow(2, (midi - best) / 12) };
}
