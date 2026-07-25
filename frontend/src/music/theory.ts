// Thin compatibility layer over the ported music-kb engine (lib/music) — the
// note tables live THERE (single source of truth); this file re-exposes them
// in the simple (string, pc-number) shapes the older components use, plus the
// diatonic-palette convenience.

import { PITCH_CLASSES } from "../lib/music/types";
import { normalizePitchClass } from "../lib/music/theory/notes";

export const NOTE_NAMES: string[] = [...PITCH_CLASSES];

/** Pitch class (0–11) of a note name like "A", "F#", "Bb". */
export function pitchClassOf(root: string): number | null {
  const pc = normalizePitchClass(root.trim());
  return pc == null ? null : PITCH_CLASSES.indexOf(pc);
}

/** The 7 diatonic triads of a key, as chord names — for the palette quick-add. */
export function diatonicChords(rootPc: number, mode: "major" | "minor"): { roman: string; name: string }[] {
  const majSteps = [0, 2, 4, 5, 7, 9, 11];
  const minSteps = [0, 2, 3, 5, 7, 8, 10];
  const majQual = ["", "m", "m", "", "", "m", "dim"];
  const minQual = ["m", "dim", "", "m", "m", "", ""];
  const majRoman = ["I", "ii", "iii", "IV", "V", "vi", "vii°"];
  const minRoman = ["i", "ii°", "III", "iv", "v", "VI", "VII"];
  const steps = mode === "major" ? majSteps : minSteps;
  const quals = mode === "major" ? majQual : minQual;
  const romans = mode === "major" ? majRoman : minRoman;
  return steps.map((st, i) => ({ roman: romans[i], name: NOTE_NAMES[(rootPc + st) % 12] + quals[i] }));
}
