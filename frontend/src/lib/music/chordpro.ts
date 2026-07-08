// ChordPro utilities — the ONE home for inline "[C]word" chord-tag handling
// (audit Tier-3 #16). A lyric line is a sequence of words, each optionally
// carrying a chord that lands on its first syllable; stored back as inline
// "[C]word" text so the chord is anchored to the word and stays aligned when
// lyrics are edited.
//
// Chord names are preserved exactly as authored — do NOT auto-sharpen. Flats
// are often the musically-correct spelling (e.g. Bb = bII Neapolitan in A
// minor), and the AI's qualities (Am(add9), "Bb (ghost)") carry intent.
// Mangling them also caused false "doesn't match Lyrics" warnings in the
// Generation Prompt.

export type Word = { text: string; chord?: string };

const TOKEN_RE = /\[([^\]]+)\]|(\S+)/g;
const TAG_RE = /\[([^\]]+)\]/g;

/** Parse a ChordPro line ("[Dm]The dashboard [Bb]glows") into words+chords. */
export function parseLine(s: string): Word[] {
  const words: Word[] = [];
  let pending: string | undefined;
  let m: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((m = TOKEN_RE.exec(s))) {
    if (m[1] != null) pending = m[1].trim();
    else { words.push({ text: m[2], chord: pending }); pending = undefined; }
  }
  if (pending) words.push({ text: "", chord: pending }); // trailing chord, no word
  return words;
}

/** Serialise words back to a ChordPro line. */
export function toLine(words: Word[]): string {
  return words.map((w) => (w.chord ? `[${w.chord}]` : "") + w.text).join(" ").trim();
}

/** Does the line carry any inline [chord] tag? */
export function hasTags(s: string): boolean {
  return /\[[^\]]+\]/.test(s);
}

/** The line's inline [chord] tags, in order. */
export function extractTags(s: string): string[] {
  const tags: string[] = [];
  let m: RegExpExecArray | null;
  TAG_RE.lastIndex = 0;
  while ((m = TAG_RE.exec(s))) tags.push(m[1]);
  return tags;
}

/** The line with every [tag] removed (empty "[]" included). */
export function stripTags(s: string): string {
  return s.replace(/\[[^\]]*\]/g, "");
}

/** Spread a section's chord progression across its lyric lines as a first
 *  draft — the ONE auto-place implementation (the Lyrics editor's ⚡ behavior
 *  is canonical): chords are split across non-empty lines proportionally,
 *  then dropped on evenly-spaced words within each line (one chord per word,
 *  later chords win). Mutates the lines in place. */
export function spreadChords(lines: Word[][], progression: string[]): void {
  for (const line of lines) for (const w of line) w.chord = undefined; // start clean
  if (!progression.length) return;
  const neIdx = lines.map((l, i) => (l.some((w) => w.text) ? i : -1)).filter((i) => i >= 0);
  const neCount = neIdx.length;
  if (!neCount) return;
  const perLine: string[][] = Array.from({ length: neCount }, () => []);
  progression.forEach((name, j) => { perLine[Math.min(neCount - 1, Math.floor((j * neCount) / progression.length))].push(name); });
  neIdx.forEach((li, k) => {
    const line = lines[li];
    const my = perLine[k];
    const wordIdx = line.map((w, i) => (w.text ? i : -1)).filter((i) => i >= 0);
    if (!wordIdx.length) return;
    my.forEach((name, m) => {
      const at = wordIdx[my.length === 1 ? 0 : Math.min(wordIdx.length - 1, Math.round((m * (wordIdx.length - 1)) / (my.length - 1)))];
      line[at].chord = name;
    });
  });
}
