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

/** Inner markup for one chord's pad SHAPE — a single compact fingering, not the
 *  whole grid (the fourths layout repeats every shape across the 8×8, so the
 *  full-grid view was pure repetition — user feedback). The root anchors at
 *  row 1 / col 1 of a 5-pad-wide window; each chord tone (closed voicing,
 *  ascending from the root) lands once on its nearest pad. Returns null for
 *  unparseable names (N.C. etc.) — callers show a fallback. */
export function padChordInner(name: string, cell = 12, inversion = 0): { markup: string; width: number; height: number } | null {
  const pcs = chordPcsByName(name);
  if (!pcs.length) return null;
  const rootPc = pcs[0];
  const intervals = [...new Set(pcs.map((pc) => (pc - rootPc + 12) % 12))].sort((a, b) => a - b);
  // closed-voicing inversion, same convention as the piano view: rotate the
  // tone stack, wrapped tones jump an octave, then anchor the LOWEST tone
  const n = intervals.length;
  const k = ((inversion % n) + n) % n;
  const voicing = intervals.map((_, i) => intervals[(k + i) % n] + (k + i >= n ? 12 : 0));
  const lowest = voicing[0];
  const W = 5; // window columns
  const ROOT_T = ROW_INT + 1; // lowest tone's pad at (row 1, col 1) → semitone offset 6 from the window base
  const spots: { row: number; col: number; pc: number; isRoot: boolean }[] = [];
  for (const iv of voicing.map((v) => v - lowest)) {
    const t = ROOT_T + iv;
    let best: { row: number; col: number; score: number } | null = null;
    for (let row = 0; row < 6; row++) {
      const col = t - row * ROW_INT;
      if (col < 0 || col >= W) continue;
      const score = Math.abs(col - 2) + Math.abs(row - 1) * 0.5; // prefer compact, near the root row
      if (!best || score < best.score) best = { row, col, score };
    }
    if (best) {
      const pc = (rootPc + lowest + iv) % 12;
      spots.push({ row: best.row, col: best.col, pc, isRoot: pc === rootPc });
    }
  }
  if (!spots.length) return null;
  const rows = Math.max(...spots.map((s) => s.row)) + 1;
  const gap = Math.max(2, Math.round(cell * 0.12));
  const names = cell >= 18;
  const rx = Math.max(2, Math.round(cell * 0.14));
  const at = new Map(spots.map((s) => [`${s.row}:${s.col}`, s]));
  const parts: string[] = [];
  for (let row = rows - 1; row >= 0; row--) {
    const y = (rows - 1 - row) * (cell + gap);
    for (let col = 0; col < W; col++) {
      const s = at.get(`${row}:${col}`);
      const fill = s ? (s.isRoot ? COL_ROOT : COL_LIT) : COL_OFF;
      const x = col * (cell + gap);
      parts.push(`<rect x="${x}" y="${y}" width="${cell}" height="${cell}" rx="${rx}" fill="${fill}"/>`);
      if (names && s) {
        parts.push(
          `<text x="${x + cell / 2}" y="${y + cell / 2 + cell * 0.16}" fill="${TXT_LIT}" font-size="${Math.round(cell * 0.42)}" font-weight="bold" text-anchor="middle" font-family="monospace">${NOTE_NAMES[s.pc]}</text>`,
        );
      }
    }
  }
  const width = W * cell + (W - 1) * gap;
  const height = rows * cell + (rows - 1) * gap;
  return { markup: parts.join(""), width, height };
}

/** Standalone SVG of one chord's pad shape with the chord name above it. */
export function padChordSvg(name: string, cell = 26, inversion = 0): string | null {
  const g = padChordInner(name, cell, inversion);
  if (!g) return null;
  const label = 16;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${g.width}" height="${g.height + label}" viewBox="0 0 ${g.width} ${g.height + label}">`
    + `<text x="${g.width / 2}" y="11" fill="#c8d0da" font-size="11" font-weight="bold" text-anchor="middle" font-family="monospace">${name.replace(/&/g, "&amp;").replace(/</g, "&lt;")}</text>`
    + `<g transform="translate(0,${label})">${g.markup}</g></svg>`;
}
