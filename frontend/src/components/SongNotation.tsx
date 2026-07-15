// The song workspace's "Notation" tab (user request 2026-07-15): the real
// staff-notation rendering of the song — same VexFlow view the Composer uses,
// built read-only from the Arrange source of truth (spine sections + Chords
// stage + lyric placements) via compositionFromSong. The Composer stays the
// place to EDIT melody/bass; this tab is for reading the sheet in context.
import { lazy, Suspense, useMemo } from "react";
import type { Section as SpineSection } from "../ipc/generated";
import { compositionFromSong } from "../lib/music/compose/compositionFromSong";
import { keyToScaleSelection } from "../lib/music/compose/playback";
import { degreeLabel, type DegreeLabel } from "../lib/music/compose/labels";
import { getDiatonicChords } from "../lib/music/theory/diatonic";
import type { ChordsData, LyricsData } from "../lib/artifacts";

// same lazy chunk discipline as the Sketchpad — VexFlow loads on first open
const NotationView = lazy(() =>
  import("./compose/NotationView").then((m) => ({ default: m.NotationView })),
);

export function SongNotation({
  songId, title, keyRoot, keyMode, bpm, chordsData, lyricsData, spine,
}: {
  songId: string; title: string; keyRoot: string; keyMode: string; bpm: number;
  chordsData: ChordsData | null; lyricsData: LyricsData | null;
  spine: SpineSection[];
}) {
  const comp = useMemo(
    () =>
      chordsData?.sections.length
        ? compositionFromSong(keyRoot, keyMode, chordsData, lyricsData, {
            id: `sheet-${songId}`,
            name: title || "Song",
            bpm: Number(bpm) || undefined,
          }, spine)
        : null,
    [songId, title, keyRoot, keyMode, bpm, chordsData, lyricsData, spine],
  );
  const labels = useMemo(() => {
    const m: Record<number, DegreeLabel> = {};
    if (comp) for (const c of getDiatonicChords(keyToScaleSelection(comp))) m[c.degree] = degreeLabel(c);
    return m;
  }, [comp]);

  if (!comp) {
    return (
      <div className="banner">
        No chords yet — run the <b>Chords</b> stage first, then the notation renders every section's staves here.
      </div>
    );
  }
  return (
    <div className="card">
      <p className="faint" style={{ margin: "0 0 8px", fontSize: 11 }}>
        Read-only staff view of the arrangement — sections and bars match the Arrange tab 1:1. Write melody/bass in the Composer.
      </p>
      <Suspense fallback={<div className="empty">Loading notation…</div>}>
        <NotationView comp={comp} labels={labels} activeBar={null} />
      </Suspense>
    </div>
  );
}
