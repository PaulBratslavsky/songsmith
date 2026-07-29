// Build a full-song Composition from a generated song — the import
// direction of the Composer's flexibility goal (COMPOSER-SPEC.md, DECISION
// 2026-06-29). Lays every section's chords end-to-end on one long,
// variable-length timeline, attaches each section's lyric lines to the
// chord starts they sit under, and keeps melody/bass empty (editable).
//
// Lyric sheet v3 (COMPOSER-SPEC): a section WITH tagged lyric placements
// lays its chord spans FROM those placements — the ground truth of the
// sung song. Each ChordPro [chord] tag becomes ONE span (beats cycle the
// section's Chords-stage progression BY POSITION), so a chorus that sings
// the progression twice honestly shows twice the blocks, and the timeline's
// spans equal the sheet's chord occurrences 1:1 in order (sync by
// construction — selection/play-along can never light a "twin"). Sections
// with no tagged placements (instrumentals) keep the progression-once
// layout.
//
// Pure + deterministic (no React/audio): given the song key, the Chords
// stage data, and the Lyrics stage data, it returns a Composition that's
// run through parseStoredComposition so the span invariants hold.
//
// Reuse, not duplication:
//   - lyric/chord section pairing reuses deriveSections from the Sheet
//     preview / Builder (components/ArrangementBuilder) — same alignment
//     the existing sheet uses, so the Composer's lyric row matches it.
//   - chord-name parsing reuses the theory engine (parse-chord); the
//     diatonic degree is computed against the song's scale so changing the
//     key still transposes the degree-based playback.

import type {
  Composition,
  ChordSpan,
  Degree,
  KeyMode,
  LyricLine,
  LyricWord,
  NoteSpan,
  Section,
} from './types';
import {
  SCHEMA_VERSION,
  DEFAULT_BPM,
  TICKS_PER_BEAT,
  TICKS_PER_BAR,
} from './types';
import { parseStoredComposition } from './schema';
import { parseChordSymbol } from '../theory/parse-chord';
import { getScalePitchClasses } from '../theory/scales';
import { normalizePitchClass } from '../theory/notes';
import type { PitchClass, ScaleType } from '../types';
import { PITCH_CLASSES } from '../types';
import { deriveSections } from '../../../components/ArrangementBuilder';
import { parseLine } from '../chordpro';
import type { ArtifactChord, ChordsData, ChordsSection, LyricsData } from '../../artifacts';
import type { Section as SpineSection } from '../../../ipc/generated';
import { matchBySpineRow } from '../../sections';
import { noteSpansFromTranscription, type TranscribedNote } from './transcription';

const uid = (() => {
  let n = 0;
  return (p: string) => `${p}-imp-${(n += 1)}`;
})();

function modeToScaleType(mode: KeyMode): ScaleType {
  return mode === 'major' ? 'major' : 'minor';
}

/**
 * Diatonic degree (1..7) for an absolute chord in the given key. The
 * chord's root PC is matched to the scale; in-scale roots map directly,
 * out-of-key roots pick the nearest scale degree by semitone distance
 * (display still comes from the chord `name`, so the exact quality isn't
 * lost — this only drives degree-based playback when no name parses).
 */
export function degreeForChordName(
  name: string,
  root: PitchClass,
  mode: KeyMode,
): Degree {
  const scalePcs = getScalePitchClasses({ root, type: modeToScaleType(mode) });
  const parsed = parseChordSymbol(name);
  const chordRoot = parsed?.root ?? normalizePitchClass(name) ?? root;
  const exact = scalePcs.indexOf(chordRoot);
  if (exact >= 0) return (Math.min(exact, 6) + 1) as Degree;
  // nearest scale degree by chromatic distance
  const ci = PITCH_CLASSES.indexOf(chordRoot);
  let best = 0;
  let bestDist = 99;
  scalePcs.forEach((pc, i) => {
    const pi = PITCH_CLASSES.indexOf(pc);
    const d = Math.min((ci - pi + 12) % 12, (pi - ci + 12) % 12);
    if (d < bestDist) {
      bestDist = d;
      best = i;
    }
  });
  return (Math.min(best, 6) + 1) as Degree;
}

/** A chords-stage section's playable chords (already normalized to
 *  {name, beats} by lib/artifacts); entries without a name are skipped. */
function readChords(sec: { chords: ArtifactChord[] }): ArtifactChord[] {
  return sec.chords.filter((c) => c.name);
}

/**
 * Build a Composition from a RENDER ANALYSIS (Phase 1 round-trip: the
 * Reference Analyst's real tempo/key + section map with per-section chords).
 * Reuses the full compositionFromSong layout (bar padding, spine-shaped
 * sections) with fabricated section rows — no song is involved.
 */
export function compositionFromAnalysis(
  a: {
    bpm: number; key_root: string; key_mode: string;
    sections: { label: string; bars: number; chords: { name: string; beats: number }[] }[];
    /** Phase 2: transcribed note events (seconds) from the vocals/bass stems. */
    melody?: TranscribedNote[]; bass?: TranscribedNote[];
    /** analyzer v2: bar 1 of the grid — the notes rebase onto it */
    first_downbeat_sec?: number;
  },
  name: string,
  /** The owning song's SPINE (iteration loop): when an analysis section's
   *  label matches a spine row, the Composition section carries that row's
   *  REAL id — so "export back to song" maps the rebuild onto the song
   *  instead of orphaned `an-N` ids. Imported songs match 1:1 (spine and
   *  analysis come from the same Reference Analyst run); unmatched sections
   *  keep fabricated ids and export-back falls back to label matching. */
  spine: readonly SpineSection[] = [],
): Composition | null {
  if (!a.sections.length) return null;
  const norm = (s: string) => s.trim().toLowerCase();
  const used = new Set<string>();
  const rows = a.sections.map((s, i) => {
    const match = spine.find((r) => !used.has(r.id) && norm(r.label) === norm(s.label));
    if (match) used.add(match.id);
    return {
      id: match?.id ?? `an-${i}`,
      song_id: "",
      position: i,
      label: s.label,
      type: "",
      bars: Math.max(1, Number(s.bars) || 8),
      role: "",
      created_at: "",
      updated_at: "",
    };
  }) as unknown as SpineSection[];
  const chordsData = {
    sections: a.sections.map((s, i) => ({
      section_id: `an-${i}`,
      label: s.label,
      chords: (s.chords ?? []).map((c) => ({ name: c.name, beats: Math.max(1, Number(c.beats) || 4) })),
    })),
  } as unknown as ChordsData;
  const comp = compositionFromSong(a.key_root, a.key_mode, chordsData, null, {
    // the schema caps ids at 64 chars — a long render filename used to push
    // it over, parseStoredComposition returned null, and Analyze → Composer
    // failed with "the analysis found no sections" (audit 2026-07-28)
    id: `analysis-${name}`.slice(0, 64),
    name,
    bpm: Number(a.bpm) || undefined,
  }, rows);
  // Phase 2: the transcribed melody/bass fold into the diatonic lanes — an
  // editable sketch of what the AI sang/played, in the Composer's vocabulary.
  if (a.melody?.length || a.bass?.length) {
    const downbeat = Number(a.first_downbeat_sec) || 0;
    const withNotes: Composition = {
      ...comp,
      melody: noteSpansFromTranscription(comp, a.melody ?? [], 'melody', comp.bpm, downbeat),
      bass: noteSpansFromTranscription(comp, a.bass ?? [], 'bass', comp.bpm, downbeat),
    };
    return parseStoredComposition(withNotes) ?? withNotes;
  }
  return comp;
}

/** A stored Melodist/Arranger take: per-section degree-based note events —
 *  ALREADY the Composer's vocabulary (degree 1–7, octave 0|1, 16th ticks). */
export type PartTake = {
  sections: { label: string; notes: { degree: number; octave: number; start: number; length: number }[] }[];
};

/**
 * Fill the Composition's melody/bass lanes from stored takes (Melodist lead →
 * melody, Arranger bass → bass): sections match by label (first-unused), note
 * starts offset by the section's startTick and clip to its length. The takes
 * become EDITABLE lanes — tweak a generated take by hand instead of only
 * re-rolling it. Lanes with no take stay as they were.
 */
export function applyTakesToComposition(
  comp: Composition,
  melody: PartTake | null | undefined,
  bass: PartTake | null | undefined,
): Composition {
  const mk = (take: PartTake | null | undefined): NoteSpan[] => {
    if (!take?.sections?.length) return [];
    const used = new Set<number>();
    const spans: NoteSpan[] = [];
    for (const sec of comp.sections) {
      const idx = take.sections.findIndex(
        (s, i) => !used.has(i) && s.label.trim().toLowerCase() === sec.name.trim().toLowerCase(),
      );
      if (idx < 0) continue;
      used.add(idx);
      for (const n of take.sections[idx].notes ?? []) {
        const start = sec.startTick + Math.max(0, Math.round(n.start));
        if (start >= sec.startTick + sec.lengthTicks) continue;
        const length = Math.min(
          Math.max(1, Math.round(n.length)),
          sec.startTick + sec.lengthTicks - start,
        );
        spans.push({
          id: uid('take'),
          degree: Math.min(7, Math.max(1, Math.round(n.degree))) as Degree,
          octave: n.octave ? 1 : 0,
          start,
          length,
        });
      }
    }
    return spans.sort((a, b) => a.start - b.start);
  };
  const m = mk(melody);
  const b = mk(bass);
  if (!m.length && !b.length) return comp;
  const next: Composition = {
    ...comp,
    melody: m.length ? m : comp.melody,
    bass: b.length ? b : comp.bass,
  };
  return parseStoredComposition(next) ?? next;
}

/**
 * ARRANGE IS THE SOURCE OF TRUTH (user decision 2026-07-15): a spine row's bar
 * count owns its Composer section length 1:1. When the laid chord spans come
 * up short (e.g. 4 placements over an 8-bar verse whose loop plays twice),
 * the progression keeps cycling as instrumental fill until the section is
 * full; a final span is truncated to land exactly on the bar line. Sung spans
 * are never cut — a section whose placements overflow its bar count keeps its
 * real length.
 */
function padSectionToBars(
  chords: ChordSpan[],
  raw: ArtifactChord[],
  root: PitchClass,
  mode: KeyMode,
  laidCount: number,
  tick: number,
  targetEnd: number,
): number {
  if (!raw.length) return Math.max(tick, targetEnd);
  let i = laidCount;
  while (tick < targetEnd) {
    const rc = raw[i % raw.length];
    const length = Math.min(
      Math.max(1, Math.round(rc.beats * TICKS_PER_BEAT)),
      targetEnd - tick,
    );
    chords.push({
      id: uid('span'),
      degree: degreeForChordName(rc.name, root, mode),
      seventh: false,
      name: rc.name,
      start: tick,
      length,
    });
    tick += length;
    i += 1;
  }
  return tick;
}

/**
 * Build a Composition from a bare chord progression (the Chord Builder's
 * "→ Composer" export): one span per chord, one bar each, a single
 * "Progression" section, melody/bass empty and editable. The key defaults to
 * the first chord's root (+ its quality's mode) when none is given.
 */
export function compositionFromProgression(
  names: string[],
  keyRoot?: string,
  keyMode?: string,
): Composition | null {
  const clean = names.map((n) => n.trim()).filter(Boolean);
  if (!clean.length) return null;
  const first = parseChordSymbol(clean[0]);
  const root: PitchClass = normalizePitchClass(keyRoot ?? '') ?? first?.root ?? 'C';
  const mode: KeyMode = keyMode === 'major' || keyMode === 'minor'
    ? keyMode
    : /m(?!aj)/.test(clean[0].slice(1)) ? 'minor' : 'major';
  const chords: ChordSpan[] = clean.map((name, i) => ({
    id: uid('span'),
    degree: degreeForChordName(name, root, mode),
    seventh: false,
    name,
    start: i * TICKS_PER_BAR,
    length: TICKS_PER_BAR,
  }));
  const totalTicks = clean.length * TICKS_PER_BAR;
  const draft: Composition = {
    id: `prog-${clean.join('-')}`,
    version: SCHEMA_VERSION,
    name: 'Progression sketch',
    key: { root, mode },
    bpm: DEFAULT_BPM,
    bars: clean.length,
    totalTicks,
    chords,
    melody: [],
    bass: [],
    sections: [{ id: uid('sec'), name: 'Progression', startTick: 0, lengthTicks: totalTicks }],
    lyrics: [],
  };
  return parseStoredComposition(draft) ?? draft;
}

/**
 * Build a Composition spanning the whole song.
 *
 * @param keyRoot  song key root (e.g. "A"); accepts flat/sharp spellings
 * @param keyMode  "major" | "minor"
 * @param chordsData  the Chords stage artifact's `data`
 *                    ({ sections: [{ label, feel?, chords:[{name,beats}] }] })
 * @param lyricsData  the Lyrics stage artifact's `data`
 *                    ({ sections: [{ label, lines:[] }] })
 * @param opts.name  composition name (song title)
 * @param opts.bpm   tempo
 * @param spine  the song's section SPINE rows (docs/SECTION-SPINE-SPEC.md,
 *               Phase 2). When non-empty the spine owns section order/labels/
 *               bars; per-stage content attaches by section_id (label fallback
 *               for legacy entries) and every Composition section carries a
 *               `section_id` so Phase 3's export can map back losslessly.
 *               `[]` (legacy / unmigrated) keeps the chords-artifact order
 *               exactly as before.
 */
export function compositionFromSong(
  keyRoot: string,
  keyMode: string,
  chordsData: ChordsData | null,
  lyricsData: LyricsData | null,
  opts: { id?: string; name?: string; bpm?: number } = {},
  spine: readonly SpineSection[] = [],
): Composition {
  const root: PitchClass = normalizePitchClass(keyRoot) ?? 'C';
  const mode: KeyMode = keyMode === 'major' ? 'major' : 'minor';

  const cSecs = chordsData?.sections ?? [];

  // The ordered section walk. With a spine, the spine rows own order/labels/
  // bars and each row picks up its Chords-stage content by section_id (label
  // fallback); chords-artifact sections not yet in the spine (the Phase-2
  // write window) are appended. Without a spine it is the chords artifact's
  // own order, unchanged.
  type OrderedSec = { label: string; section_id?: string; bars?: number; cSec?: ChordsSection };
  let ordered: OrderedSec[];
  if (spine.length) {
    const usedC = new Set<ChordsSection>();
    ordered = spine.map((row) => {
      const c = matchBySpineRow(cSecs, row, usedC);
      if (c) usedC.add(c);
      return { label: row.label || 'Section', section_id: row.id, bars: Math.max(1, Number(row.bars) || 8), cSec: c };
    });
    for (const s of cSecs) {
      if (!usedC.has(s)) ordered.push({ label: s.label || 'Section', cSec: s });
    }
  } else {
    ordered = cSecs.map((s) => ({ label: s.label || 'Section', cSec: s }));
  }

  // Section label → lyric lines, from the Sheet-preview's own alignment
  // (deriveSections, spine-aware), so the Composer lyric sheet matches the
  // Sheet. Each line keeps its word-level ChordPro breakdown (the [chord]-tag
  // anchors), which the lyric sheet uses to print chord names above the
  // exact words.
  const derived = deriveSections(chordsData, lyricsData, spine);
  const lyricLinesByLabel = new Map<string, LyricWord[][]>();
  for (const d of derived) {
    const lines = d.lyrics
      .map((l) => parseLine(l).filter((w) => w.text || w.chord))
      .filter((words) => words.some((w) => w.text.trim().length > 0));
    lyricLinesByLabel.set(d.label, lines);
  }

  const chords: ChordSpan[] = [];
  const sections: Section[] = [];
  const lyrics: LyricLine[] = [];
  let tick = 0;

  // Last anchor pushed overall — defensively keeps every lyric anchor
  // unique + strictly increasing across the whole song (the sheet keys
  // lines by anchor; the playback hook identifies the active line by it).
  let lastAnchor = -1;
  const pushLine = (anchor: number, words: LyricWord[]) => {
    const a = Math.max(anchor, lastAnchor + 1);
    lastAnchor = a;
    const text = words.map((w) => w.text).filter(Boolean).join(' ');
    lyrics.push({ tick: a, text, words });
  };

  for (const os of ordered) {
    const label: string = os.label;
    const idTag = os.section_id ? { section_id: os.section_id } : {};
    const raw = os.cSec ? readChords(os.cSec) : [];
    const lines = lyricLinesByLabel.get(label) ?? [];

    // Flatten the section's tagged ChordPro placements in sung order, and
    // remember each line's FIRST placement (its anchor chord).
    const placements: string[] = [];
    const firstPlacementOfLine: (number | null)[] = lines.map((words) => {
      let first: number | null = null;
      for (const w of words) {
        if (!w.chord) continue;
        if (first == null) first = placements.length;
        placements.push(w.chord);
      }
      return first;
    });

    const sectionStart = tick;

    if (placements.length) {
      // Lyric-tagged section (lyric-sheet v3): ONE span per placement —
      // the timeline lays the progression as many times as the lyrics
      // actually cycle it. Beats cycle the Chords-stage progression BY
      // POSITION (placement i → progression[i % n].beats; 4 when the
      // progression is empty) — positional, never a name lookup. Section
      // length = the sum of its spans. Invariant: this section's spans
      // == its sheet chord occurrences, 1:1 in order.
      const spanStarts: number[] = [];
      placements.forEach((name, i) => {
        const beats = raw.length ? raw[i % raw.length].beats : 4;
        const length = Math.max(1, Math.round(beats * TICKS_PER_BEAT));
        spanStarts.push(tick);
        chords.push({
          id: uid('span'),
          degree: degreeForChordName(name, root, mode),
          seventh: false,
          name,
          start: tick,
          length,
        });
        tick += length;
      });
      sections.push({
        id: uid('sec'),
        name: label,
        startTick: sectionStart,
        lengthTicks: tick - sectionStart,
        ...idTag,
      });

      // Anchors: a tagged line sits EXACTLY at its first placement's span
      // start; runs of untagged lines spread midway between their tagged
      // neighbours' anchors (section start/end as the outer bounds).
      const sectionEnd = tick;
      const anchors: number[] = lines.map((_w, i) => {
        const fp = firstPlacementOfLine[i];
        return fp != null ? spanStarts[fp] : -1;
      });
      let i = 0;
      while (i < lines.length) {
        if (anchors[i] >= 0) {
          i += 1;
          continue;
        }
        let j = i;
        while (j < lines.length && anchors[j] < 0) j += 1; // untagged run [i, j)
        const lo = i > 0 ? anchors[i - 1] : sectionStart;
        const hi = j < lines.length ? anchors[j] : sectionEnd;
        const k = j - i;
        for (let m = 0; m < k; m += 1) {
          // A leading run starts at the section start; inner/trailing
          // runs sit strictly between the bounds (gap permitting —
          // pushLine keeps anchors unique either way).
          anchors[i + m] =
            i > 0
              ? lo + Math.round(((m + 1) * (hi - lo)) / (k + 1))
              : lo + Math.round((m * (hi - lo)) / (k + 1));
        }
        i = j;
      }
      lines.forEach((words, li) => pushLine(anchors[li], words));

      // spine bars own the section length — cycle the progression as
      // instrumental fill up to the bar target (sung spans stay untouched)
      if (os.bars) {
        tick = padSectionToBars(chords, raw, root, mode, placements.length, tick, sectionStart + os.bars * TICKS_PER_BAR);
        sections[sections.length - 1].lengthTicks = tick - sectionStart;
      }
      continue;
    }

    // ---- No tagged placements: keep the progression-once layout. ----

    if (!raw.length) {
      if (!os.section_id) {
        // legacy: a chordless section (e.g. a bare Intro) — give it one bar
        // so the band still shows it (byte-identical without a spine).
        sections.push({ id: uid('sec'), name: label, startTick: tick, lengthTicks: TICKS_PER_BAR });
        tick += TICKS_PER_BAR;
        continue;
      }
      // spine row with no chords content: the SPINE's bar count sizes the
      // section, and its lyric lines (if any) spread evenly across it —
      // lyric-only sections are spine rows now, so they land here in order.
      const lengthTicks = Math.max(1, os.bars ?? 1) * TICKS_PER_BAR;
      const start = tick;
      sections.push({ id: uid('sec'), name: label, startTick: start, lengthTicks, ...idTag });
      tick += lengthTicks;
      lines.forEach((words, i) => {
        pushLine(start + Math.round((i * lengthTicks) / lines.length), words);
      });
      continue;
    }
    const chordStarts: number[] = [];
    for (const rc of raw) {
      const length = Math.max(1, Math.round(rc.beats * TICKS_PER_BEAT));
      chordStarts.push(tick);
      chords.push({
        id: uid('span'),
        degree: degreeForChordName(rc.name, root, mode),
        seventh: false,
        name: rc.name,
        start: tick,
        length,
      });
      tick += length;
    }
    // spine bars own the section length — keep cycling the progression to
    // fill the row's bar count (Arrange is the source of truth)
    if (os.bars) {
      tick = padSectionToBars(chords, raw, root, mode, raw.length, tick, sectionStart + os.bars * TICKS_PER_BAR);
    }
    sections.push({
      id: uid('sec'),
      name: label,
      startTick: sectionStart,
      lengthTicks: tick - sectionStart,
      ...idTag,
    });

    // Anchor this section's (untagged) lyric lines (lyric-sheet-v2 fix).
    // When the CHORDS outnumber the lines, pair line i with chord start i
    // (the song lands one chord at the start of each lyric line). When the
    // LINES outnumber (or equal) the chords, distribute them evenly across
    // the section's tick span instead — the old code pinned every overflow
    // line to the LAST chord's tick, so play-along tracking skipped the
    // back half of the section. Either way every line gets a unique,
    // strictly increasing anchor. `words` carries the word-level chord
    // anchors for the lyric sheet.
    const sectionLength = tick - sectionStart;
    lines.forEach((words, i) => {
      const anchor =
        lines.length >= chordStarts.length
          ? sectionStart + Math.round((i * sectionLength) / lines.length)
          : (chordStarts[i] ?? sectionStart);
      pushLine(anchor, words);
    });
  }

  // ---- Lyric-only sections (they exist in the Lyrics stage but the walk
  // above — spine rows + chords sections — never got them, e.g. a Bridge
  // written only in the lyrics on a pre-spine song). BACKLOG decision
  // (2026-07-07): INCLUDE them — a section band with NO chord spans, one bar
  // per lyric line (min one bar), the lines anchored evenly across it.
  // Playback simply has no chords there; the sheet still shows the section
  // chip and its (chordless) lines. (With a spine, lyric-only sections are
  // spine rows and were already laid out in order above.)
  const coveredLabels = new Set(ordered.map((os) => os.label));
  for (const d of derived) {
    if (coveredLabels.has(d.label)) continue;
    const lines = lyricLinesByLabel.get(d.label) ?? [];
    const sectionStart = tick;
    const lengthTicks = Math.max(1, lines.length) * TICKS_PER_BAR;
    sections.push({ id: uid('sec'), name: d.label, startTick: sectionStart, lengthTicks, ...(d.section_id ? { section_id: d.section_id } : {}) });
    tick += lengthTicks;
    lines.forEach((words, i) => {
      pushLine(sectionStart + Math.round((i * lengthTicks) / lines.length), words);
    });
  }

  // Round the total length up to whole bars (so the ruler ends clean).
  const totalTicks = Math.max(TICKS_PER_BAR, Math.ceil(tick / TICKS_PER_BAR) * TICKS_PER_BAR);
  const bars = totalTicks / TICKS_PER_BAR;

  const draft: Composition = {
    id: opts.id ?? uid('comp'),
    version: SCHEMA_VERSION,
    name: opts.name || 'Imported song',
    key: { root, mode },
    bpm: opts.bpm ?? DEFAULT_BPM,
    bars,
    totalTicks,
    chords,
    melody: [],
    bass: [],
    sections,
    lyrics,
  };

  // Run through the validator so invariants hold (sorted/in-range). If it
  // somehow fails to parse, fall back to the unvalidated draft rather than
  // crashing the editor.
  return parseStoredComposition(draft) ?? draft;
}
