// Phase 2 (stem transcription → Composer lanes): fold basic-pitch note
// events (absolute MIDI, seconds) into the Composer's diatonic NoteSpan
// lanes. The lanes are degree-based (1..7) with a narrow octave band
// (melody: base+0/+1, bass: single band), so the fold is lossy on purpose —
// the goal is an EDITABLE sketch of what the AI sang/played, in the same
// vocabulary the user composes in, not a piano-roll clone.
//
// Mapping: quantize to 16th-note ticks (via the summary's bpm), shift the
// whole lane by whole octaves so its median lands inside the band, then per
// note pick the (degree, octave) whose resolved playback MIDI is nearest —
// preferring an exact pitch-class match (right note name) over raw distance,
// so chromatic passing notes snap to the nearest scale degree but in-scale
// notes never land on a neighbor. Overlaps from quantization are trimmed to
// keep the lanes monophonic (the lane invariant).
import type { Composition, Degree, NoteSpan } from './types';
import { TICKS_PER_BEAT } from './types';
import { resolveBassMidi, resolveMelodyMidi } from './playback';

export type TranscribedNote = { start: number; end: number; midi: number; amp?: number };

const DEGREES: Degree[] = [1, 2, 3, 4, 5, 6, 7];

const uid = (() => {
  let n = 0;
  return (p: string) => `${p}-tr-${(n += 1)}`;
})();

export function noteSpansFromTranscription(
  comp: Composition,
  events: TranscribedNote[],
  lane: 'melody' | 'bass',
  bpm: number,
  /** The analysis' first downbeat (seconds). Event times are ABSOLUTE, but
   *  bar 1 of the grid is this instant — and the render audio is nudged by
   *  exactly this much — so notes must be rebased or they land late against
   *  both the chords and the audio (audit 2026-07-28). */
  offsetSec = 0,
): NoteSpan[] {
  if (!events.length || !(bpm > 0)) return [];
  // every playable (degree, octave) with its resolved MIDI
  const cands: { degree: Degree; octave: 0 | 1; midi: number }[] = [];
  for (const degree of DEGREES) {
    for (const octave of lane === 'bass' ? ([0] as const) : ([0, 1] as const)) {
      const midi =
        lane === 'bass'
          ? resolveBassMidi(comp, { degree, octave })
          : resolveMelodyMidi(comp, { degree, octave });
      if (midi != null) cands.push({ degree, octave, midi });
    }
  }
  if (!cands.length) return [];

  // whole-octave shift centering the lane's median inside the band, so a
  // tenor verse and a soprano chorus both land ON the lane (contour kept)
  const sorted = events.map((e) => e.midi).sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  const center = (cands[0].midi + cands[cands.length - 1].midi) / 2;
  const shift = Math.round((median - center) / 12) * 12;

  const toTick = (sec: number) => Math.round(((sec - offsetSec) * bpm * TICKS_PER_BEAT) / 60);
  const spans: NoteSpan[] = [];
  for (const e of [...events].sort((a, b) => a.start - b.start)) {
    const start = toTick(e.start);
    if (start < 0 || start >= comp.totalTicks) continue; // pre-downbeat pickup notes drop
    const length = Math.max(1, Math.min(toTick(e.end) - start, comp.totalTicks - start));
    const target = e.midi - shift;
    let best = cands[0];
    let bestScore = Infinity;
    for (const c of cands) {
      const pcMatch = ((c.midi % 12) + 12) % 12 === ((e.midi % 12) + 12) % 12;
      const score = Math.abs(c.midi - target) + (pcMatch ? 0 : 3);
      if (score < bestScore) {
        bestScore = score;
        best = c;
      }
    }
    // monophonic: a new onset trims (or evicts) the still-ringing note
    while (spans.length) {
      const prev = spans[spans.length - 1];
      if (start >= prev.start + prev.length) break;
      if (start > prev.start) {
        prev.length = start - prev.start;
        break;
      }
      spans.pop(); // same quantized onset — keep the later (louder-sorted) one out; last wins
    }
    spans.push({ id: uid(lane), start, length, degree: best.degree, octave: best.octave });
  }
  return spans;
}
