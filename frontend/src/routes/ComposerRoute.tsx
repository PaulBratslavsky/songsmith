import { useMemo } from "react";
import { useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { Sketchpad } from "../components/compose/Sketchpad";
import { compositionFromSong } from "../lib/music/compose/compositionFromSong";
import type { Composition } from "../lib/music/compose/types";
import type { PitchClass } from "../lib/music/types";
import { normalizePitchClass } from "../lib/music/theory/notes";
import { parseArtifact } from "../lib/artifacts";

/** Read the `?song=<id>` search param (no route schema needed). */
function useSongParam(): string | null {
  const search = useRouterState({ select: (s) => s.location.search });
  if (typeof search === "string") {
    return new URLSearchParams(search).get("song");
  }
  // TanStack may parse search into an object
  const v = (search as Record<string, unknown> | undefined)?.song;
  return typeof v === "string" && v ? v : null;
}

/** Fetch a song's chords+lyrics and build a full-song Composition. */
function useSongComposition(songId: string | null): {
  comp: Composition | null;
  title: string | null;
  loading: boolean;
} {
  const song = useQuery({
    queryKey: ["song", songId],
    queryFn: () => api.getSong(songId!),
    enabled: !!songId,
  });

  const chordsStageId = song.data?.stages.find((s) => s.type === "chords")?.id;
  const lyricsStageId = song.data?.stages.find((s) => s.type === "lyrics")?.id;
  const chords = useQuery({ queryKey: ["stage", chordsStageId], queryFn: () => api.getStage(chordsStageId!), enabled: !!chordsStageId });
  const lyrics = useQuery({ queryKey: ["stage", lyricsStageId], queryFn: () => api.getStage(lyricsStageId!), enabled: !!lyricsStageId });
  // the song's section spine (docs/SECTION-SPINE-SPEC.md, Phase 2): owns the
  // Composition's section order/bars when non-empty; [] = legacy chords order
  const sections = useQuery({ queryKey: ["sections", songId], queryFn: () => api.listSections(songId!), enabled: !!songId });

  // Every query must have SETTLED before we build the composition.
  // The stage query keys are shared with SongWorkspace's cache, so chords can
  // resolve instantly from cache while lyrics is still on its first fetch —
  // building `comp` at that moment would seed the Composer lyric-less, and since
  // `comp.id` is stable it would never reload for the session (the late-lyrics
  // race; the spine query joins the same settled gate for the same reason).
  // A stage that doesn't exist counts as settled.
  const stagesSettled =
    !!song.data &&
    (!chordsStageId || chords.isFetched) &&
    (!lyricsStageId || lyrics.isFetched) &&
    sections.isFetched;

  const comp = useMemo<Composition | null>(() => {
    if (!songId || !song.data || !stagesSettled) return null;
    const v = song.data.song;
    const cd = parseArtifact("chords", chords.data?.artifact?.content).data;
    const ld = parseArtifact("lyrics", lyrics.data?.artifact?.content).data;
    if (!cd?.sections.length) return null;
    return compositionFromSong(v.key_root, v.key_mode, cd, ld, {
      id: `song-${songId}`,
      name: v.title || "Imported song",
      bpm: Number(v.bpm) || undefined,
    }, sections.data ?? []);
  }, [songId, song.data, stagesSettled, chords.data, lyrics.data, sections.data]);

  return {
    comp,
    title: song.data?.song.title ?? null,
    loading: !!songId && (song.isLoading || !stagesSettled),
  };
}

export function ComposerRoute() {
  const songId = useSongParam();
  const { comp, title, loading } = useSongComposition(songId);

  // The song's key seeds a blank sketch too (so "New blank" starts in the
  // song's key when you came from a song).
  const initialRoot: PitchClass =
    (comp ? normalizePitchClass(comp.key.root) : null) ?? "C";

  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Composer{songId && title ? ` — ${title}` : ""}</h1>
          <span className="muted">
            {songId
              ? "Full song on one timeline — every section laid end-to-end, the real chords on the chord lane with the lyric line under each, melody + bass empty and editable across the whole song. Change the key and everything transposes."
              : "Hookpad-style 8-bar sketchpad — lay down chords, melody, and bass in scale degrees over a shared grid and loop it through the synth. Change the key and everything transposes."}
          </span>
        </div>
      </div>
      {songId && loading ? (
        <div className="empty">Loading song…</div>
      ) : songId && !comp ? (
        <div className="banner">
          This song has no chords yet — run the <b>Chords</b> stage first, then open it in the Composer.
        </div>
      ) : (
        <Sketchpad key={comp?.id ?? "blank"} initialRoot={initialRoot} initial={comp} songId={comp ? songId : null} />
      )}
    </div>
  );
}
