// Ableton Push chromatic-mode pad grids (user request 2026-07-15): an 8×8 grid
// where RIGHT = +1 semitone and UP = +5 semitones (the fourths layout Push uses
// in chromatic mode), bottom-left = C. Scale members light up green, the root
// orange, everything else stays dark — exactly the reference screenshot. Chord
// "shapes" reuse the same fixed grid so a shape is a real pad pattern you can
// finger on the hardware, comparable across chords.
import { NOTE_NAMES } from "./theory";
import { chordPcsByName } from "./engineAdapter";

const ROWS = 8;
const COLS = 8;
const ROW_INT = 5; // semitones per row upward (fourths layout)
const BASE_PC = 0; // bottom-left pad = C (Push chromatic default)

const MAJOR = [0, 2, 4, 5, 7, 9, 11];
const MINOR = [0, 2, 3, 5, 7, 8, 10];

// Ableton-ish palette (matches the user's reference capture)
const COL_ROOT = "#e87a52";
const COL_LIT = "#77d1a2";
const COL_OFF = "#252a33";
const TXT_LIT = "#10151c";
const TXT_OFF = "#5b6472";

type PadKind = "root" | "lit" | "off";

function grid(kindOf: (pc: number) => PadKind, cell: number): { markup: string; width: number; height: number } {
  const gap = Math.max(2, Math.round(cell * 0.12));
  const names = cell >= 18;
  const r = Math.max(2, Math.round(cell * 0.14));
  const parts: string[] = [];
  for (let row = ROWS - 1; row >= 0; row--) {
    const y = (ROWS - 1 - row) * (cell + gap);
    for (let col = 0; col < COLS; col++) {
      const pc = (BASE_PC + row * ROW_INT + col) % 12;
      const kind = kindOf(pc);
      const fill = kind === "root" ? COL_ROOT : kind === "lit" ? COL_LIT : COL_OFF;
      const x = col * (cell + gap);
      parts.push(`<rect x="${x}" y="${y}" width="${cell}" height="${cell}" rx="${r}" fill="${fill}"/>`);
      if (names) {
        const color = kind === "off" ? TXT_OFF : TXT_LIT;
        parts.push(
          `<text x="${x + cell / 2}" y="${y + cell / 2 + cell * 0.16}" fill="${color}" font-size="${Math.round(cell * 0.42)}" font-weight="bold" text-anchor="middle" font-family="monospace">${NOTE_NAMES[pc]}</text>`,
        );
      }
    }
  }
  const width = COLS * cell + (COLS - 1) * gap;
  const height = ROWS * cell + (ROWS - 1) * gap;
  return { markup: parts.join(""), width, height };
}

function scalePcs(rootPc: number, mode: "major" | "minor"): Set<number> {
  const ivs = mode === "major" ? MAJOR : MINOR;
  return new Set(ivs.map((i) => (rootPc + i) % 12));
}

/** The full-size scale grid: root orange, scale members green, note names on every pad. */
export function padScaleSvg(rootPc: number, mode: "major" | "minor", cell = 40): string {
  const inScale = scalePcs(rootPc, mode);
  const g = grid((pc) => (pc === rootPc ? "root" : inScale.has(pc) ? "lit" : "off"), cell);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${g.width}" height="${g.height}" viewBox="0 0 ${g.width} ${g.height}">${g.markup}</svg>`;
}

/** Inner markup for one chord's pad shape (chord tones lit, chord root orange).
 *  Returns null for unparseable names (N.C. etc.) — callers show a fallback. */
export function padChordInner(name: string, cell = 12): { markup: string; width: number; height: number } | null {
  const pcs = chordPcsByName(name);
  if (!pcs.length) return null;
  const rootPc = pcs[0];
  const members = new Set(pcs);
  return grid((pc) => (pc === rootPc ? "root" : members.has(pc) ? "lit" : "off"), cell);
}

/** Standalone SVG of one chord's pad shape with the chord name above it. */
export function padChordSvg(name: string, cell = 20): string | null {
  const g = padChordInner(name, cell);
  if (!g) return null;
  const label = 16;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${g.width}" height="${g.height + label}" viewBox="0 0 ${g.width} ${g.height + label}">`
    + `<text x="${g.width / 2}" y="11" fill="#c8d0da" font-size="11" font-weight="bold" text-anchor="middle" font-family="monospace">${name.replace(/&/g, "&amp;").replace(/</g, "&lt;")}</text>`
    + `<g transform="translate(0,${label})">${g.markup}</g></svg>`;
}
