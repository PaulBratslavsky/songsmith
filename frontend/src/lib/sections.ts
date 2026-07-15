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

/** One `spine_snapshot` entry (docs/SECTION-SPINE-SPEC.md §Snapshots). */
export type SpineSnapshotEntry = { section_id: string; label: string; position: number };

/** The light `spine_snapshot` block an editor save embeds beside `data` —
 *  mirror of core spine::snapshot_of, built from an ordered row list (spine
 *  rows or `{section_id,label}` pairs already in save order). Restores use it
 *  to re-create deleted rows, keeping the journal self-contained. */
export function spineSnapshot(rows: readonly { section_id?: string; id?: string; label: string }[]): SpineSnapshotEntry[] {
  return rows.flatMap((r, position) => {
    const section_id = r.section_id ?? r.id;
    return section_id ? [{ section_id, label: r.label, position }] : [];
  });
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
