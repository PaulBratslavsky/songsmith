// Typed, lenient parsing for stage-artifact `content` blobs — the ONE place
// that knows each artifact kind's data shape (audit Tier-2 #8). Replaces the
// five ad-hoc `any` parsers (SongWorkspace.artifactData, dataOf ×3, LyricsEditor
// .parse) and the four copies of chord-shape coercion (string chords → {name,
// beats: 4}; missing/invalid beats → 4).
//
// Leniency contract (matches what those parsers tolerated):
//   - content is `{kind, text, data}` JSON; a bare JSON object counts as data,
//     non-JSON content counts as text (data: null).
//   - chords: `{name, beats}` objects OR legacy plain strings.
//   - lyric section lines: `lines: string[]` OR legacy `text: "a\nb"`.
//   - a section's `label` falls back to its `type`; consumers keep their own
//     final fallback ("" is preserved so "skip unlabeled" call sites still can).
//   - garbage (data not an object / not schema-shaped) degrades to data: null,
//     exactly like the old `?? null` parsers.

import { z } from "zod";

const str = (v: unknown): string => (typeof v === "string" ? v : "");
const asObj = (v: unknown): Record<string, unknown> =>
  v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {};

// ---- chords ----------------------------------------------------------------

/** One chord, normalized: legacy `"Am"` → `{ name: "Am", beats: 4 }`. */
const ChordSchema = z.preprocess((c) => {
  if (typeof c === "string") return { name: c, beats: 4 };
  const o = asObj(c);
  const beats = o.beats ? Number(o.beats) : NaN;
  return {
    name: str(o.name),
    beats: Number.isFinite(beats) && beats !== 0 ? beats : 4,
  };
}, z.object({ name: z.string(), beats: z.number() }));
export type ArtifactChord = z.infer<typeof ChordSchema>;

const ChordsSectionSchema = z.preprocess(
  (raw) => {
    const s = asObj(raw);
    return {
      // spine link (docs/SECTION-SPINE-SPEC.md) — attached by the migration;
      // Phase-2 readers match content by section_id first, label fallback
      section_id: str(s.section_id) || undefined,
      label: str(s.label) || str(s.type),
      feel: str(s.feel) || undefined,
      frozen: s.frozen === true || undefined,
      chords: Array.isArray(s.chords) ? s.chords : [],
    };
  },
  z.object({
    section_id: z.string().optional(),
    label: z.string(),
    feel: z.string().optional(),
    frozen: z.literal(true).optional(),
    chords: z.array(ChordSchema),
  }),
);
export type ChordsSection = z.infer<typeof ChordsSectionSchema>;

export const ChordsDataSchema = z.preprocess(
  (raw) => ({ sections: Array.isArray(asObj(raw).sections) ? asObj(raw).sections : [] }),
  z.object({ sections: z.array(ChordsSectionSchema) }),
);
export type ChordsData = z.infer<typeof ChordsDataSchema>;

// ---- lyrics ----------------------------------------------------------------

const LyricsSectionSchema = z.preprocess(
  (raw) => {
    const s = asObj(raw);
    const lines = Array.isArray(s.lines)
      ? s.lines.map((l) => (typeof l === "string" ? l : String(l ?? "")))
      : typeof s.text === "string"
        ? s.text.split("\n")
        : [];
    return { section_id: str(s.section_id) || undefined, label: str(s.label) || str(s.type), frozen: s.frozen === true || undefined, lines };
  },
  z.object({
    /** spine link — see ChordsSectionSchema */
    section_id: z.string().optional(),
    label: z.string(),
    frozen: z.literal(true).optional(),
    lines: z.array(z.string()),
  }),
);
export type LyricsSection = z.infer<typeof LyricsSectionSchema>;

export const LyricsDataSchema = z.preprocess(
  (raw) => ({ sections: Array.isArray(asObj(raw).sections) ? asObj(raw).sections : [] }),
  z.object({ sections: z.array(LyricsSectionSchema) }),
);
export type LyricsData = z.infer<typeof LyricsDataSchema>;

// ---- structure -------------------------------------------------------------

const StructureSectionSchema = z.preprocess(
  (raw) => {
    const s = asObj(raw);
    const bars = Number(s.bars ?? 8);
    return {
      section_id: str(s.section_id) || undefined,
      type: str(s.type),
      label: str(s.label) || str(s.type),
      bars: Number.isFinite(bars) ? bars : 8,
      role: str(s.role),
      frozen: s.frozen === true || undefined,
    };
  },
  z.object({
    /** spine link — see ChordsSectionSchema */
    section_id: z.string().optional(),
    type: z.string(),
    label: z.string(),
    bars: z.number(),
    role: z.string(),
    frozen: z.literal(true).optional(),
  }),
);
export type StructureSection = z.infer<typeof StructureSectionSchema>;

export const StructureDataSchema = z.preprocess(
  (raw) => {
    const d = asObj(raw);
    const key = asObj(d.key);
    const bpm = Number(d.bpm);
    return {
      // LEGACY ONLY (docs/SONG-FACTS.md): old rows embedded the song's key/bpm;
      // the SONG owns them now. Tolerated when present, absent on new saves —
      // readers must take key/tempo from the song, never from here.
      key: str(key.root) ? { root: str(key.root), mode: str(key.mode) || "minor" } : undefined,
      bpm: d.bpm != null && Number.isFinite(bpm) ? bpm : undefined,
      keyNote: str(d.keyNote),
      tempoNote: str(d.tempoNote),
      sections: Array.isArray(d.sections) ? d.sections : [],
    };
  },
  z.object({
    /** @deprecated legacy embedded song fact — read the song's key instead */
    key: z.object({ root: z.string(), mode: z.string() }).optional(),
    /** @deprecated legacy embedded song fact — read the song's bpm instead */
    bpm: z.number().optional(),
    keyNote: z.string(),
    tempoNote: z.string(),
    sections: z.array(StructureSectionSchema),
  }),
);
export type StructureData = z.infer<typeof StructureDataSchema>;

// ---- lyric spec ------------------------------------------------------------

const DICTIONS = ["plain-spoken", "balanced", "literary"] as const;

export const LyricSpecDataSchema = z.preprocess(
  (raw) => {
    const d = asObj(raw);
    const strings = (v: unknown) =>
      Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
    return {
      hook: str(d.hook),
      premise: str(d.premise),
      pov: str(d.pov),
      setting: str(d.setting),
      arc: str(d.arc),
      diction: (DICTIONS as readonly string[]).includes(str(d.diction)) ? d.diction : "balanced",
      referenceVibe: str(d.referenceVibe),
      beats: Array.isArray(d.beats)
        ? d.beats.map((b) => {
            const o = asObj(b);
            return {
              section_id: str(o.section_id) || undefined,
              section: str(o.section) || str(o.label),
              beat: str(o.beat) || str(o.text),
            };
          })
        : [],
      imageBank: strings(d.imageBank),
      avoid: strings(d.avoid),
    };
  },
  z.object({
    hook: z.string(),
    premise: z.string(),
    pov: z.string(),
    setting: z.string(),
    arc: z.string(),
    diction: z.enum(DICTIONS),
    referenceVibe: z.string(),
    beats: z.array(z.object({ section_id: z.string().optional(), section: z.string(), beat: z.string() })),
    imageBank: z.array(z.string()),
    avoid: z.array(z.string()),
  }),
);
export type LyricSpecData = z.infer<typeof LyricSpecDataSchema>;

// ---- generation prompt -----------------------------------------------------

export const PromptDataSchema = z.preprocess(
  (raw) => {
    const d = asObj(raw);
    return {
      stylePrompt: str(d.stylePrompt) || str(d.style_prompt),
      taggedLyrics: str(d.taggedLyrics) || str(d.tagged_lyrics),
      notes: str(d.notes),
    };
  },
  z.object({ stylePrompt: z.string(), taggedLyrics: z.string(), notes: z.string() }),
);
export type PromptData = z.infer<typeof PromptDataSchema>;

// ---- the one entry point ----------------------------------------------------

const SCHEMAS = {
  chords: ChordsDataSchema,
  lyrics: LyricsDataSchema,
  structure: StructureDataSchema,
  lyric_spec: LyricSpecDataSchema,
  generation_prompt: PromptDataSchema,
} as const;

export type ArtifactKind = keyof typeof SCHEMAS;
export type ArtifactDataOf<K extends ArtifactKind> = z.infer<(typeof SCHEMAS)[K]>;

/** Split artifact content into its `{text, data}` envelope without validating
 *  data (the LyricsEditor-style lenient read: a bare object counts as data,
 *  non-JSON counts as text). */
export function artifactEnvelope(content: string | null | undefined): { text: string; data: unknown } {
  if (!content) return { text: "", data: null };
  try {
    const v: unknown = JSON.parse(content);
    if (v && typeof v === "object" && "text" in v) {
      const o = v as Record<string, unknown>;
      return { text: o.text == null ? "" : String(o.text), data: o.data ?? null };
    }
    return { text: content, data: v };
  } catch {
    return { text: content, data: null };
  }
}

/**
 * Parse an artifact's `content` for the given kind. Lenient: coerces the
 * legacy shapes the old ad-hoc parsers handled (string chords, `text` lines,
 * `type`-as-label, snake_case prompt fields); returns `data: null` on garbage.
 */
export function parseArtifact<K extends ArtifactKind>(
  kind: K,
  content: string | null | undefined,
): { text: string; data: ArtifactDataOf<K> | null } {
  const { text, data } = artifactEnvelope(content);
  if (data == null || typeof data !== "object") return { text, data: null };
  const res = SCHEMAS[kind].safeParse(data);
  return { text, data: res.success ? (res.data as ArtifactDataOf<K>) : null };
}
