// Single source of truth for validating a Composition. Kept for Phase 3
// persistence (libSQL `composition` table via the mcp-shim CRUD pattern) —
// NOT wired to any network/Strapi code yet. The lenient read path
// (parseStoredComposition) + reidentify are the seams the persistence
// layer will load through.

import { z } from 'zod';
import type { Composition } from './types';
import { SCHEMA_VERSION, TOTAL_TICKS } from './types';

const DegreeSchema = z.number().int().min(1).max(7);

const SpanBase = {
  id: z.string().min(1).max(64),
  degree: DegreeSchema,
  start: z.number().int().min(0).max(TOTAL_TICKS - 1),
  length: z.number().int().min(1).max(TOTAL_TICKS),
};

const ChordSpanSchema = z.object({
  ...SpanBase,
  seventh: z.boolean(),
});
const NoteSpanSchema = z.object({
  ...SpanBase,
  octave: z.union([z.literal(0), z.literal(1)]),
});

// Lenient chord schema for stored rows: v1 rows have no `seventh`, so
// default it to false (triad). See SCHEMA_VERSION history in types.ts.
const StoredChordSpanSchema = z.object({
  ...SpanBase,
  seventh: z.boolean().default(false),
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
  chords: z.array(ChordSpanSchema).max(64),
  melody: z.array(NoteSpanSchema).max(512),
  bass: z.array(NoteSpanSchema).max(512),
});

// Lenient variant for reading stored rows: pre-versioning rows (saved
// before `version` existed) are missing the field, so default it.
const StoredCompositionSchema = CompositionSchema.extend({
  version: z.number().int().default(SCHEMA_VERSION),
  chords: z.array(StoredChordSpanSchema).max(64),
});

/**
 * Validate + migrate a Composition read from storage. Returns null when
 * the blob can't be coerced into the current shape (so callers can drop
 * the row instead of crashing the editor). Add migration branches here
 * keyed on the parsed `version` as the shape evolves.
 */
export function parseStoredComposition(raw: unknown): Composition | null {
  const result = StoredCompositionSchema.safeParse(raw);
  if (!result.success) return null;
  const parsed = result.data;
  // Future: if (parsed.version < SCHEMA_VERSION) migrate step-by-step.
  return { ...parsed, version: SCHEMA_VERSION } as Composition;
}

let loadIdCounter = 0;

/**
 * Fresh span/note ids for a composition loaded from storage, so they
 * can't collide with ids minted later in an editing session. Phase 3's
 * load path runs stored rows through parseStoredComposition then this.
 */
export function reidentify(comp: Composition): Composition {
  const mint = (prefix: string) => `${prefix}-load-${(loadIdCounter += 1)}`;
  return {
    ...comp,
    chords: comp.chords.map((s) => ({ ...s, id: mint('span') })),
    melody: comp.melody.map((n) => ({ ...n, id: mint('note') })),
    bass: comp.bass.map((n) => ({ ...n, id: mint('note') })),
  };
}
