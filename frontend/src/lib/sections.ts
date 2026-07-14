// The song's SECTION SPINE, read-side (docs/SECTION-SPINE-SPEC.md, Phase 2).
// One React Query per song (key ["sections", songId]) shared by every reader
// (SongSheet, ArrangementBuilder, editors, Composer import). An EMPTY array
// means "no spine" — every consumer falls back to its legacy artifact-label
// derivation, so unmigrated songs/mock data behave exactly as before.

import { useQuery } from "@tanstack/react-query";
import { api } from "../ipc/api";
import type { Section } from "../ipc/generated";

/** The spine rows for a song, position-ordered. `[]` while loading or when
 *  the song has no spine (legacy) — callers must keep their label fallback. */
export function useSpineSections(songId: string | null | undefined): { sections: Section[]; isFetched: boolean } {
  const q = useQuery({
    queryKey: ["sections", songId],
    queryFn: () => api.listSections(songId!),
    enabled: !!songId,
  });
  return { sections: q.data ?? [], isFetched: q.isFetched };
}

/** Case/whitespace-insensitive label key (mirror of core freeze::norm_label). */
export function normLabel(s: string): string {
  return s.trim().toLowerCase().split(/\s+/).join(" ");
}

/** Find the artifact-section entry that carries a spine row's CONTENT:
 *  match by `section_id` when the entry has one, else by normalized label.
 *  `used` lets ordered walks consume each entry at most once. */
export function matchBySpineRow<T extends { section_id?: string; label: string }>(
  entries: readonly T[],
  row: { id: string; label: string },
  used?: Set<T>,
): T | undefined {
  const free = (e: T) => !used?.has(e);
  return (
    entries.find((e) => free(e) && e.section_id === row.id) ??
    entries.find((e) => free(e) && !e.section_id && normLabel(e.label) === normLabel(row.label)) ??
    entries.find((e) => free(e) && normLabel(e.label) === normLabel(row.label))
  );
}
