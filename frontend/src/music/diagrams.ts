// SVG chord-diagram builders — for inline display and for exporting a printable
// chord chart you can play from.

import { NOTE_NAMES } from "./theory";
import { guitarFretsByName, chordPcsByName, voicedMidisByName, type GuitarShape } from "./engineAdapter";

const INK = "#e8e6e0";
const LINE = "#5f666c";
const DOT = "#c8ff3d";

/** Inner SVG markup for one guitar voicing at offset (ox,oy). ~78x96. */
export function guitarInner(sh: GuitarShape | null, ox = 0, oy = 0): string {
  const sp = 12, left = ox + 12, top = oy + 22, rows = 5;
  if (!sh) return `<text x="${ox + 39}" y="${oy + 60}" fill="${LINE}" font-size="9" text-anchor="middle" font-family="monospace">(see piano)</text>`;
  let s = "";
  for (let i = 0; i < 6; i++) s += `<line x1="${left + i * sp}" y1="${top}" x2="${left + i * sp}" y2="${top + rows * 12}" stroke="${LINE}"/>`;
  for (let r = 0; r <= rows; r++) s += `<line x1="${left}" y1="${top + r * 12}" x2="${left + 5 * sp}" y2="${top + r * 12}" stroke="${LINE}" stroke-width="${r === 0 && sh.baseFret === 1 ? 2.5 : 1}"/>`;
  if (sh.baseFret > 1) s += `<text x="${left - 6}" y="${top + 10}" fill="${INK}" font-size="8" text-anchor="end" font-family="monospace">${sh.baseFret}</text>`;
  sh.frets.forEach((f, i) => {
    const x = left + i * sp;
    if (f === -1) s += `<text x="${x}" y="${top - 4}" fill="${LINE}" font-size="9" text-anchor="middle" font-family="monospace">×</text>`;
    else if (f === 0) s += `<circle cx="${x}" cy="${top - 6}" r="3" fill="none" stroke="${INK}"/>`;
    else { const row = f - sh.baseFret; s += `<circle cx="${x}" cy="${top + row * 12 + 6}" r="4.5" fill="${DOT}"/>`; }
  });
  return s;
}

/** Standalone SVG for a specific voicing (name + diagram + voicing label). */
export function diagramSvgShape(sh: GuitarShape | null, name: string): string {
  return `<svg width="86" height="110" viewBox="0 0 86 110" xmlns="http://www.w3.org/2000/svg">` +
    `<text x="43" y="12" fill="${INK}" font-size="12" font-weight="bold" text-anchor="middle" font-family="monospace">${name}</text>` +
    guitarInner(sh, 0, 0) +
    (sh ? `<text x="43" y="106" fill="${LINE}" font-size="8" text-anchor="middle" font-family="monospace">${sh.label}</text>` : "") +
    `</svg>`;
}

/** Standalone SVG for a chord's best voicing (chips, etc.). */
export function diagramSvg(name: string): string {
  return diagramSvgShape(guitarFretsByName(name), name);
}

/** A printable chart for a progression (grid of best-voicing diagrams + notes). */
export function chartSvg(names: string[], title = "Chord chart"): string {
  const cols = Math.min(4, Math.max(1, names.length));
  const cw = 100, ch = 130, pad = 16, top = 40;
  const rows = Math.ceil(names.length / cols);
  const W = pad * 2 + cols * cw, H = top + pad + rows * ch;
  let body = `<rect width="${W}" height="${H}" fill="#0e0f10"/>` +
    `<text x="${pad}" y="26" fill="${DOT}" font-size="16" font-weight="bold" font-family="monospace">${title}</text>`;
  names.forEach((n, i) => {
    const cx = pad + (i % cols) * cw, cy = top + Math.floor(i / cols) * ch;
    const notes = chordPcsByName(n).map((pc) => NOTE_NAMES[pc]).join(" ");
    body += `<text x="${cx + 43}" y="${cy + 12}" fill="${INK}" font-size="13" font-weight="bold" text-anchor="middle" font-family="monospace">${n}</text>`;
    body += guitarInner(guitarFretsByName(n), cx, cy);
    body += `<text x="${cx + 43}" y="${cy + 122}" fill="${LINE}" font-size="9" text-anchor="middle" font-family="monospace">${notes}</text>`;
  });
  return `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg">${body}</svg>`;
}

const ROOT_COL = "#c8ff3d", SCALE_COL = "#5cff9d";
function scaleSet(rootPc: number, mode: "major" | "minor"): Set<number> {
  const steps = mode === "major" ? [0, 2, 4, 5, 7, 9, 11] : [0, 2, 3, 5, 7, 8, 10];
  return new Set(steps.map((st) => (rootPc + st) % 12));
}

/** Full guitar neck (6×15) with the whole key scale marked (root highlighted). */
function scaleNeckInner(rootPc: number, mode: "major" | "minor", ox: number, oy: number): { markup: string; height: number } {
  const TUNE = [4, 9, 2, 7, 11, 4]; // low E→high e pitch classes
  const FRETS = 15, nutX = ox + 24, fw = 46, sg = 20, top = oy + 12;
  const scale = scaleSet(rootPc, mode);
  const yOf = (s: number) => top + (5 - s) * sg; // low E at bottom
  let m = "";
  for (let s = 0; s < 6; s++) m += `<line x1="${nutX}" y1="${yOf(s)}" x2="${nutX + FRETS * fw}" y2="${yOf(s)}" stroke="${LINE}"/>`;
  m += `<line x1="${nutX}" y1="${yOf(5)}" x2="${nutX}" y2="${yOf(0)}" stroke="${INK}" stroke-width="3"/>`;
  for (let f = 1; f <= FRETS; f++) {
    m += `<line x1="${nutX + f * fw}" y1="${yOf(5)}" x2="${nutX + f * fw}" y2="${yOf(0)}" stroke="${LINE}"/>`;
    m += `<text x="${nutX + (f - 0.5) * fw}" y="${yOf(0) + 16}" fill="${LINE}" font-size="9" text-anchor="middle" font-family="monospace">${f}</text>`;
  }
  for (let s = 0; s < 6; s++) {
    for (let f = 0; f <= FRETS; f++) {
      const pc = (TUNE[s] + f) % 12;
      if (!scale.has(pc)) continue;
      const cx = f === 0 ? nutX - 12 : nutX + (f - 0.5) * fw;
      const cy = yOf(s);
      const root = pc === rootPc;
      m += `<circle cx="${cx}" cy="${cy}" r="8.5" fill="${root ? ROOT_COL : SCALE_COL}"/>`;
      m += `<text x="${cx}" y="${cy + 3}" fill="#11140a" font-size="8" font-weight="bold" text-anchor="middle" font-family="monospace">${NOTE_NAMES[pc]}</text>`;
    }
  }
  return { markup: m, height: 5 * sg + 30 };
}

/** Full piano (multi-octave) with the whole key scale highlighted (root brighter). */
function scalePianoInner(rootPc: number, mode: "major" | "minor", ox: number, oy: number): { markup: string; height: number } {
  const scale = scaleSet(rootPc, mode);
  const octaves = 2;
  const w = 26, whites = [0, 2, 4, 5, 7, 9, 11];
  const blackOff: Record<number, number> = { 1: 0, 3: 1, 6: 3, 8: 4, 10: 5 };
  let m = "";
  for (let o = 0; o < octaves; o++) {
    whites.forEach((pc, i) => {
      const on = scale.has(pc);
      const x = ox + (o * 7 + i) * w;
      m += `<rect x="${x}" y="${oy}" width="${w}" height="84" fill="${pc === rootPc ? ROOT_COL : on ? SCALE_COL : "#16181a"}" stroke="${LINE}"/>`;
      if (on) m += `<text x="${x + w / 2}" y="${oy + 74}" fill="#11140a" font-size="10" text-anchor="middle" font-family="monospace">${NOTE_NAMES[pc]}</text>`;
    });
  }
  for (let o = 0; o < octaves; o++) {
    Object.keys(blackOff).forEach((k) => {
      const pc = Number(k); const on = scale.has(pc);
      const x = ox + (o * 7 + blackOff[pc] + 1) * w - 8;
      m += `<rect x="${x}" y="${oy}" width="16" height="52" fill="${pc === rootPc ? ROOT_COL : on ? "#8aa92c" : "#000"}" stroke="${LINE}"/>`;
    });
  }
  return { markup: m, height: 94 };
}

/** Multi-octave piano showing the actual voiced MIDI notes, so inversions are
 *  visible (the bass note shifts and is highlighted brighter). */
export function pianoVoicedInner(midis: number[], ox = 0, oy = 0, w = 11): { markup: string; width: number; height: number } {
  if (!midis.length) return { markup: "", width: 0, height: 0 };
  const set = new Set(midis);
  const lo = Math.min(...midis), hi = Math.max(...midis);
  const startMidi = Math.floor(lo / 12) * 12;
  const octs = Math.max(2, Math.ceil((hi - startMidi + 1) / 12));
  const whites = [0, 2, 4, 5, 7, 9, 11];
  const blackOff: Record<number, number> = { 1: 0, 3: 1, 6: 3, 8: 4, 10: 5 };
  const h = Math.round(w * 4);
  let m = "", wi = 0;
  for (let o = 0; o < octs; o++) for (const off of whites) {
    const midi = startMidi + o * 12 + off;
    const on = set.has(midi), bass = on && midi === lo;
    m += `<rect x="${ox + wi * w}" y="${oy}" width="${w}" height="${h}" fill="${bass ? ROOT_COL : on ? DOT : "#16181a"}" stroke="${LINE}"/>`;
    wi++;
  }
  for (let o = 0; o < octs; o++) for (const k of Object.keys(blackOff)) {
    const pc = Number(k), midi = startMidi + o * 12 + pc;
    const on = set.has(midi), bass = on && midi === lo;
    const x = ox + (o * 7 + blackOff[pc] + 1) * w - w * 0.3;
    m += `<rect x="${x.toFixed(1)}" y="${oy}" width="${(w * 0.6).toFixed(1)}" height="${Math.round(h * 0.62)}" fill="${bass ? ROOT_COL : on ? "#8aa92c" : "#000"}" stroke="${LINE}"/>`;
  }
  return { markup: m, width: octs * 7 * w, height: h };
}

/** Standalone card SVG (name + voiced piano) for the inversion picker. */
export function pianoVoicedSvg(midis: number[], name: string, sub = ""): string {
  const inner = pianoVoicedInner(midis, 6, 18, 11);
  const W = Math.max(86, inner.width + 12), H = 110;
  return `<svg width="${W}" height="${H}" viewBox="0 0 ${W} ${H}" xmlns="http://www.w3.org/2000/svg">` +
    `<text x="${W / 2}" y="12" fill="${INK}" font-size="12" font-weight="bold" text-anchor="middle" font-family="monospace">${name}</text>` +
    inner.markup +
    (sub ? `<text x="${W / 2}" y="106" fill="${LINE}" font-size="8" text-anchor="middle" font-family="monospace">${sub}</text>` : "") +
    `</svg>`;
}

type PASection = { label: string; chords: string[]; lyrics: string[] };

/** One-page lead sheet (Letter portrait): title, a deduped strip of chord shapes
 *  at the chosen voicing/inversion, then a two-column chord-over-lyric body that
 *  auto-fits to a single standard page. Renders inline and exports to PNG. */
export function playAlongSvg(o: { title: string; subtitle: string; instrument: "guitar" | "piano"; rootPc: number; mode: "major" | "minor"; sections: PASection[]; voicings?: Record<string, number> }): { svg: string; width: number; height: number } {
  const esc = (t: string) => t.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  const guitar = o.instrument === "guitar";
  const voi = o.voicings ?? {};
  const PAGE_W = 816, PAGE_H = 1056, margin = 30; // US Letter @ ~96dpi
  const rows: string[] = [];

  // ---- header
  let y = margin + 16;
  rows.push(`<text x="${margin}" y="${y}" fill="${DOT}" font-size="22" font-weight="bold" font-family="monospace">${esc(o.title)}</text>`); y += 18;
  rows.push(`<text x="${margin}" y="${y}" fill="${LINE}" font-size="12" font-family="monospace">${esc(o.subtitle)} · ${NOTE_NAMES[o.rootPc]} ${o.mode}</text>`); y += 18;

  // ---- unique chords across the song (prefer the ones tagged over the lyrics)
  const uniq: string[] = [];
  for (const sec of o.sections) {
    const tags: string[] = [];
    for (const line of sec.lyrics) { const re = /\[([^\]]+)\]/g; let m: RegExpExecArray | null; while ((m = re.exec(line))) tags.push(m[1]); }
    for (const c of (tags.length ? tags : sec.chords)) if (c && !uniq.includes(c)) uniq.push(c);
  }

  // ---- chord-shape strip (each chord once, at its chosen voicing/inversion)
  const stripScale = 0.82;
  rows.push(`<text x="${margin}" y="${y}" fill="${INK}" font-size="10" font-weight="bold" font-family="monospace">CHORDS — ${guitar ? "guitar voicings" : "piano (inversions)"}</text>`); y += 6;
  const stripH = Math.round(104 * stripScale);
  let sx = margin, sy = y;
  for (const name of uniq) {
    let inner: string, natW: number;
    if (guitar) {
      natW = 80;
      inner = `<text x="40" y="10" fill="${INK}" font-size="11" font-weight="bold" text-anchor="middle" font-family="monospace">${esc(name)}</text>`
        + guitarInner(guitarFretsByName(name, voi[name] ?? 0), 0, 4);
    } else {
      const pv = pianoVoicedInner(voicedMidisByName(name, voi[name] ?? 0), 0, 16, 10);
      natW = Math.max(70, pv.width);
      inner = `<text x="${(natW / 2).toFixed(0)}" y="10" fill="${INK}" font-size="11" font-weight="bold" text-anchor="middle" font-family="monospace">${esc(name)}</text>` + pv.markup;
    }
    const dispW = natW * stripScale;
    if (sx + dispW > PAGE_W - margin) { sx = margin; sy += stripH + 8; }
    rows.push(`<g transform="translate(${sx.toFixed(1)},${sy}) scale(${stripScale})">${inner}</g>`);
    sx += dispW + 10;
  }
  const headerBottom = sy + stripH + 16;
  rows.push(`<line x1="${margin}" y1="${headerBottom - 8}" x2="${PAGE_W - margin}" y2="${headerBottom - 8}" stroke="${LINE}"/>`);

  // ---- two-column chord-over-lyric body, font sized so the longest line fits a column
  const gutter = 26;
  const colW = (PAGE_W - 2 * margin - gutter) / 2;
  const hasWords = (l: string) => l.replace(/\[[^\]]+\]/g, "").replace(/\([^)]*\)/g, "").replace(/[^A-Za-z]/g, "").length > 0;
  let maxChars = 0;
  for (const sec of o.sections) for (const line of sec.lyrics) if (hasWords(line)) maxChars = Math.max(maxChars, line.replace(/\[[^\]]+\]/g, "").length);
  const charW = Math.max(4.2, Math.min(7.0, (colW - 6) / Math.max(maxChars, 30)));
  const fs = +(charW / 0.585).toFixed(1);
  const chFs = Math.max(8, fs - 1);
  const lyrH = fs + 5, chordH = chFs + 2, secGap = fs + 6;

  type L = { type: "label" | "bar" | "cl" | "plain"; text: string; marks?: { c: string; pos: number }[] };
  type Block = { lines: L[]; h: number };
  const blocks: Block[] = [];
  for (const sec of o.sections) {
    const lines: L[] = [{ type: "label", text: sec.label }];
    let h = lyrH + 4;
    const sung = sec.lyrics.filter(hasWords);
    if (!sung.length && sec.chords.length) { lines.push({ type: "bar", text: "| " + sec.chords.join(" | ") + " |" }); h += lyrH; }
    for (const line of sung) {
      if (/\[[^\]]+\]/.test(line)) {
        let plain = "", last = 0; const marks: { c: string; pos: number }[] = [];
        const re = /\[([^\]]+)\]/g; let mm: RegExpExecArray | null;
        while ((mm = re.exec(line))) { plain += line.slice(last, mm.index); marks.push({ c: mm[1], pos: plain.length }); last = mm.index + mm[0].length; }
        plain += line.slice(last);
        lines.push({ type: "cl", text: plain, marks }); h += chordH + lyrH;
      } else { lines.push({ type: "plain", text: line }); h += lyrH; }
    }
    h += secGap;
    blocks.push({ lines, h });
  }

  // pack blocks into two balanced columns (keep each section whole)
  const total = blocks.reduce((a, b) => a + b.h, 0);
  const col: Block[][] = [[], []];
  let acc = 0;
  for (const b of blocks) { col[acc < total / 2 ? 0 : 1].push(b); acc += b.h; }
  const colH = col.map((cb) => cb.reduce((a, b) => a + b.h, 0));
  const bodyNaturalH = Math.max(colH[0], colH[1], 1);

  const body: string[] = [];
  const renderCol = (cb: Block[], cx: number) => {
    let ly = 0;
    for (const b of cb) {
      for (const l of b.lines) {
        if (l.type === "label") { ly += lyrH; body.push(`<text x="${cx}" y="${ly}" fill="${INK}" font-size="${fs + 1}" font-weight="bold" font-family="monospace">${esc(l.text)}</text>`); ly += 4; }
        else if (l.type === "bar") { ly += lyrH; body.push(`<text x="${cx}" y="${ly}" fill="${DOT}" font-size="${fs}" font-weight="bold" font-family="monospace">${esc(l.text)}</text>`); }
        else if (l.type === "cl") {
          ly += chordH;
          for (const mk of l.marks!) body.push(`<text x="${(cx + mk.pos * charW).toFixed(1)}" y="${ly}" fill="${DOT}" font-size="${chFs}" font-weight="bold" font-family="monospace">${esc(mk.c)}</text>`);
          ly += lyrH;
          body.push(`<text x="${cx}" y="${ly}" fill="${INK}" font-size="${fs}" font-family="monospace" xml:space="preserve">${esc(l.text)}</text>`);
        } else { ly += lyrH; body.push(`<text x="${cx}" y="${ly}" fill="${INK}" font-size="${fs}" font-family="monospace">${esc(l.text)}</text>`); }
      }
      ly += secGap;
    }
  };
  renderCol(col[0], 0);
  renderCol(col[1], colW + gutter);

  // scale the body to fit the remaining page height (a long song shrinks a touch)
  const availH = PAGE_H - headerBottom - margin;
  const scale = Math.min(1, availH / bodyNaturalH);
  rows.push(`<g transform="translate(${margin},${headerBottom}) scale(${scale.toFixed(4)})">${body.join("")}</g>`);

  const svg = `<svg width="${PAGE_W}" height="${PAGE_H}" viewBox="0 0 ${PAGE_W} ${PAGE_H}" xmlns="http://www.w3.org/2000/svg"><rect width="${PAGE_W}" height="${PAGE_H}" fill="#0e0f10"/>${rows.join("")}</svg>`;
  return { svg, width: PAGE_W, height: PAGE_H };
}

/** Rasterize an SVG to PNG and download it. */
export function downloadPng(svg: string, width: number, height: number, filename: string, scale = 2) {
  const img = new Image();
  img.onload = () => {
    const canvas = document.createElement("canvas");
    canvas.width = width * scale;
    canvas.height = height * scale;
    const ctx = canvas.getContext("2d")!;
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0);
    canvas.toBlob((blob) => {
      if (!blob) return;
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = filename;
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
    }, "image/png");
  };
  img.src = "data:image/svg+xml;base64," + btoa(unescape(encodeURIComponent(svg)));
}

/** Rasterize an SVG to PNG and resolve the raw bytes (for saving via Tauri). */
export function pngBytes(svg: string, width: number, height: number, scale = 2): Promise<number[]> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = width * scale;
      canvas.height = height * scale;
      const ctx = canvas.getContext("2d")!;
      ctx.scale(scale, scale);
      ctx.drawImage(img, 0, 0);
      canvas.toBlob(async (blob) => {
        if (!blob) return reject(new Error("rasterize failed"));
        resolve(Array.from(new Uint8Array(await blob.arrayBuffer())));
      }, "image/png");
    };
    img.onerror = reject;
    img.src = "data:image/svg+xml;base64," + btoa(unescape(encodeURIComponent(svg)));
  });
}

export function downloadSvg(svg: string, filename: string) {
  const blob = new Blob([svg], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
