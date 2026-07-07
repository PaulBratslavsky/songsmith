// Single source of truth for validating a Composition. Phase 3 persistence
// (the libSQL `composition` table) saves through CompositionSchema and loads
// through the lenient read path (parseStoredComposition) + reidentify — the
// Composer's Save/Open buttons and the MCP composition tools all move the
// same blob. compositionFromSong (the song→Composition builder) also runs
// its output through parseStoredComposition so the invariants hold for
// imported songs too.

import { z } from 'zod';
import type { Composition } from './types';
import {
  DEFAULT_BARS,
  DEFAULT_TOTAL_TICKS,
  MAX_TOTAL_TICKS,
  SCHEMA_VERSION,
  TICKS_PER_BAR,
} from './types';

const DegreeSchema = z.number().int().min(1).max(7);

const SpanBase = {
  id: z.string().min(1).max(64),
  degree: DegreeSchema,
  start: z.number().int().min(0).max(MAX_TOTAL_TICKS - 1),
  length: z.number().int().min(1).max(MAX_TOTAL_TICKS),
};

const ChordSpanSchema = z.object({
  ...SpanBase,
  seventh: z.boolean(),
  name: z.string().min(1).max(32).optional(),
});
const NoteSpanSchema = z.object({
  ...SpanBase,
  octave: z.union([z.literal(0), z.literal(1)]),
});

const SectionSchema = z.object({
  id: z.string().min(1).max(64),
  name: z.string().min(1).max(120),
  startTick: z.number().int().min(0).max(MAX_TOTAL_TICKS),
  lengthTicks: z.number().int().min(1).max(MAX_TOTAL_TICKS),
});
const LyricWordSchema = z.object({
  text: z.string().max(80),
  chord: z.string().min(1).max(32).optional(),
});
const LyricLineSchema = z.object({
  tick: z.number().int().min(0).max(MAX_TOTAL_TICKS),
  text: z.string().max(400),
  /** Word-level ChordPro breakdown (chord names above words); optional. */
  words: z.array(LyricWordSchema).max(120).optional(),
});

// Lenient chord schema for stored rows: v1 rows have no `seventh`, so
// default it to false (triad). See SCHEMA_VERSION history in types.ts.
const StoredChordSpanSchema = z.object({
  ...SpanBase,
  seventh: z.boolean().default(false),
  name: z.string().min(1).max(32).optional(),
});

/**
 * Strict schema for a Composition. `version` is required on purpose so the
 * save path must receive a fully-formed composition.
 */
export const CompositionSchema = z.object({
  id: z.string().min(1).max(64),
  version: z.number().int().min(1).max(1_000_000),
  name: z.string().min(1).max(200),
  key: z.object({
    root: z.string().min(1).max(3),
    mode: z.enum(['major', 'minor']),
  }),
  bpm: z.number().int().min(20).max(400),
  bars: z.number().int().min(1).max(MAX_TOTAL_TICKS / TICKS_PER_BAR),
  totalTicks: z.number().int().min(TICKS_PER_BAR).max(MAX_TOTAL_TICKS),
  chords: z.array(ChordSpanSchema).max(2048),
  melody: z.array(NoteSpanSchema).max(8192),
  bass: z.array(NoteSpanSchema).max(8192),
  sections: z.array(SectionSchema).max(256),
  lyrics: z.array(LyricLineSchema).max(2048),
});

// Lenient variant for reading stored rows: pre-versioning rows (saved
// before `version` existed) are missing the field, so default it. v1/v2
// rows have no length/sections/lyrics, so default to an 8-bar sketch with
// no sections/lyrics.
const StoredCompositionSchema = CompositionSchema.extend({
  version: z.number().int().default(SCHEMA_VERSION),
  bars: z.number().int().min(1).max(MAX_TOTAL_TICKS / TICKS_PER_BAR).default(DEFAULT_BARS),
  totalTicks: z.number().int().min(TICKS_PER_BAR).max(MAX_TOTAL_TICKS).default(DEFAULT_TOTAL_TICKS),
  chords: z.array(StoredChordSpanSchema).max(2048),
  sections: z.array(SectionSchema).max(256).default([]),
  lyrics: z.array(LyricLineSchema).max(2048).default([]),
});

/**
 * Validate + migrate a Composition read from storage (or built by
 * compositionFromSong). Returns null when the blob can't be coerced into
 * the current shape (so callers can drop the row instead of crashing the
 * editor). Add migration branches here keyed on the parsed `version`.
 */
export function parseStoredComposition(raw: unknown): Composition | null {
  const result = StoredCompositionSchema.safeParse(raw);
  if (!result.success) return null;
  const parsed = result.data;
  // Keep totalTicks consistent with bars (older rows or hand-built blobs
  // might disagree); bars is the source of truth.
  const totalTicks = parsed.bars * TICKS_PER_BAR;
  return { ...parsed, totalTicks, version: SCHEMA_VERSION } as Composition;
}

let loadIdCounter = 0;

/**
 * Fresh span/note ids for a composition loaded from storage, so they
 * can't collide with ids minted later in an editing session. Phase 3's
 * load path runs stored rows through parseStoredComposition then this.
 * Sections/lyrics carry no editable ids that collide, but section ids are
 * reminted too for tidiness.
 */
export function reidentify(comp: Composition): Composition {
  const mint = (prefix: string) => `${prefix}-load-${(loadIdCounter += 1)}`;
  return {
    ...comp,
    chords: comp.chords.map((s) => ({ ...s, id: mint('span') })),
    melody: comp.melody.map((n) => ({ ...n, id: mint('note') })),
    bass: comp.bass.map((n) => ({ ...n, id: mint('note') })),
    sections: comp.sections.map((sec) => ({ ...sec, id: mint('sec') })),
  };
}
