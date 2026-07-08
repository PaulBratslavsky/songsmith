import { useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../ipc/api";
import type { Stage } from "../ipc/generated";
import { pitchClassOf } from "../music/theory";
import { playAlongSvg } from "../music/diagrams";
import { SectionChordsEditor } from "./SectionChordsEditor";
import { LyricsEditor } from "./LyricsEditor";
import { parseArtifact, type ChordsData, type LyricsData } from "../lib/artifacts";
import { hasTags, parseLine, spreadChords, toLine } from "../lib/music/chordpro";

/** Build the sheet's sections from the (editable) Chords + Lyrics artifacts.
 *  The section's chord sequence is spread across its lyric lines (proportionally),
 *  with multiple chords per line placed over evenly-spaced words when needed. */
export function deriveSections(chordsData: ChordsData | null, lyricsData: LyricsData | null): { label: string; chords: string[]; lyrics: string[] }[] {
  const cSecs = chordsData?.sections ?? [];
  const lSecs = lyricsData?.sections ?? [];
  const labels: string[] = [];
  [...cSecs, ...lSecs].forEach((s) => { const l = s.label; if (l && !labels.includes(l)) labels.push(l); });
  return labels.map((label) => {
    const c = cSecs.find((s) => s.label === label);
    const l = lSecs.find((s) => s.label === label);
    const names: string[] = c ? c.chords.map((ch) => ch.name).filter(Boolean) : [];
    const lines: string[] = l?.lines ?? [];

    // ChordPro: if the lyrics already carry inline [chord] tags, they hold the
    // exact, user-placed positions — render them as-is (no lossy spreading).
    if (lines.some(hasTags)) {
      return { label, chords: names, lyrics: lines };
    }
    if (!names.length) return { label, chords: names, lyrics: lines };

    // legacy fallback: the ONE auto-place (same spread the Lyrics editor's ⚡ uses)
    const words = lines.map(parseLine);
    spreadChords(words, names);
    return { label, chords: names, lyrics: words.map(toLine) };
  });
}

export function ArrangementBuilder({
  songId, title, subtitle, keyRoot, keyMode, stages,
}: {
  songId: string; title: string; subtitle: string; keyRoot: string; keyMode: string; stages: Stage[];
}) {
  const qc = useQueryClient();
  const chordsStage = stages.find((s) => s.type === "chords");
  const lyricsStage = stages.find((s) => s.type === "lyrics");
  const chords = useQuery({ queryKey: ["stage", chordsStage?.id], queryFn: () => api.getStage(chordsStage!.id), enabled: !!chordsStage });
  const lyrics = useQuery({ queryKey: ["stage", lyricsStage?.id], queryFn: () => api.getStage(lyricsStage!.id), enabled: !!lyricsStage });
  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["stage", chordsStage?.id] });
    qc.invalidateQueries({ queryKey: ["stage", lyricsStage?.id] });
    qc.invalidateQueries({ queryKey: ["song", songId] });
  };

  const cd = parseArtifact("chords", chords.data?.artifact?.content).data;
  const ld = parseArtifact("lyrics", lyrics.data?.artifact?.content).data;
  const preview = useMemo(
    () => playAlongSvg({ title, subtitle, instrument: "guitar", rootPc: pitchClassOf(keyRoot) ?? 0, mode: keyMode === "major" ? "major" : "minor", sections: deriveSections(cd, ld) }),
    [cd, ld, title, subtitle, keyRoot, keyMode],
  );

  const cArt = chords.data?.artifact, lArt = lyrics.data?.artifact;

  return (
    <div className="row" style={{ gap: 14, alignItems: "flex-start" }}>
      <div style={{ flex: "1 1 50%", minWidth: 340 }}>
        <div className="col" style={{ gap: 16 }}>
          <div>
            <h3 style={{ marginBottom: 6 }}>Chords</h3>
            <p className="faint" style={{ margin: "0 0 8px" }}>Edit per section to match what Suno produced. One chord lands on the start of each lyric line.</p>
            {cArt ? (
              <SectionChordsEditor songId={songId} stageId={chordsStage!.id} kind={cArt.kind} artifactId={cArt.id} keyRoot={keyRoot} keyMode={keyMode} initialData={cd} onChanged={invalidate} />
            ) : <div className="banner">Run the <b>Chords</b> stage in Workspace first.</div>}
          </div>
          <div>
            <h3 style={{ marginBottom: 6 }}>Lyrics</h3>
            {lArt ? (
              <LyricsEditor songId={songId} stageId={lyricsStage!.id} kind={lArt.kind} artifactId={lArt.id} content={lArt.content} onChanged={invalidate} chordsData={cd} />
            ) : <div className="banner">Run the <b>Lyrics</b> stage in Workspace first.</div>}
          </div>
        </div>
      </div>
      <div className="card fit-svg" style={{ flex: "1 1 50%", minWidth: 360, overflow: "auto", maxHeight: "82vh", position: "sticky", top: 8 }}>
        <div className="faint" style={{ fontSize: 11, marginBottom: 6 }}>Live preview — saves to the Chords/Lyrics above update this. Export from the Sheet preview tab.</div>
        <div dangerouslySetInnerHTML={{ __html: preview.svg }} />
      </div>
    </div>
  );
}
