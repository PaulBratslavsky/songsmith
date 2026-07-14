import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, inTauri, savePng, revealFile } from "../ipc/api";
import type { Stage } from "../ipc/generated";
import { pitchClassOf } from "../music/theory";
import { playAlongSvg, downloadPng, pngBytes, diagramSvgShape, pianoVoicedSvg } from "../music/diagrams";
import { guitarCountByName, guitarFretsByName, chordSizeByName, voicedMidisByName } from "../music/engineAdapter";
import { deriveSections } from "./ArrangementBuilder";
import { parseArtifact } from "../lib/artifacts";
import { useSpineSections } from "../lib/sections";
import { extractTags } from "../lib/music/chordpro";

const INV_LABELS = ["root", "1st inv", "2nd inv", "3rd inv", "4th inv"];

function parseVoicings(json: string): Record<string, number> {
  try { return JSON.parse(json || "{}"); } catch { return {}; }
}

export function SongSheet({
  songId, title, subtitle, keyRoot, keyMode, stages, voicings: voicingsJson,
}: {
  songId: string; title: string; subtitle: string; keyRoot: string; keyMode: string; stages: Stage[];
  /** the song's saved voicing/inversion picks (JSON), persisted in the DB */
  voicings: string;
}) {
  const qc = useQueryClient();
  const [instrument, setInstrument] = useState<"guitar" | "piano">("guitar");
  const [voicings, setVoicings] = useState<Record<string, number>>(() => parseVoicings(voicingsJson));
  const chordsStage = stages.find((s) => s.type === "chords");
  const lyricsStage = stages.find((s) => s.type === "lyrics");
  const chords = useQuery({ queryKey: ["stage", chordsStage?.id], queryFn: () => api.getStage(chordsStage!.id), enabled: !!chordsStage });
  const lyrics = useQuery({ queryKey: ["stage", lyricsStage?.id], queryFn: () => api.getStage(lyricsStage!.id), enabled: !!lyricsStage });
  // the section spine owns identity/order (Phase 2); [] = legacy label derivation
  const { sections: spine } = useSpineSections(songId);

  // section model: spine order/labels + per-stage content from the editable
  // Chords + Lyrics artifacts (Builder tab); label-union fallback without spine
  const sections = useMemo(
    () => deriveSections(
      parseArtifact("chords", chords.data?.artifact?.content).data,
      parseArtifact("lyrics", lyrics.data?.artifact?.content).data,
      spine,
    ),
    [chords.data, lyrics.data, spine],
  );

  // every distinct chord shown on the sheet — these get a voicing cycler
  const uniqueChords = useMemo(() => {
    const set: string[] = [];
    for (const sec of sections) {
      const tags = sec.lyrics.flatMap(extractTags);
      for (const c of (tags.length ? tags : sec.chords)) if (c && !set.includes(c)) set.push(c);
    }
    return set;
  }, [sections]);

  // voicing index is stored per instrument (guitar voicing vs piano inversion mean
  // different things), keyed "<instrument>:<chord>". Derive a name→index map for
  // the current instrument to feed the renderer.
  const idxOf = (name: string) => voicings[`${instrument}:${name}`] ?? 0;
  const countOf = (name: string) => Math.max(1, instrument === "guitar" ? guitarCountByName(name) : chordSizeByName(name));
  const curVoicings = useMemo(() => {
    const out: Record<string, number> = {};
    for (const name of uniqueChords) out[name] = voicings[`${instrument}:${name}`] ?? 0;
    return out;
  }, [voicings, instrument, uniqueChords]);

  const sheet = useMemo(
    () => playAlongSvg({ title, subtitle, instrument, rootPc: pitchClassOf(keyRoot) ?? 0, mode: keyMode === "major" ? "major" : "minor", sections, voicings: curVoicings }),
    [sections, instrument, title, subtitle, keyRoot, keyMode, curVoicings],
  );

  const cycle = (name: string, dir: number) => setVoicings((v) => {
    const k = `${instrument}:${name}`;
    const n = countOf(name);
    const next = (((v[k] ?? 0) + dir) % n + n) % n;
    const nv = { ...v, [k]: next };
    const json = JSON.stringify(nv);
    // persist to the song record, and keep the cached song in sync (no refetch)
    api.updateSongVoicings(songId, json).catch(() => { /* ignore */ });
    qc.setQueryData(["song", songId], (old: any) => (old ? { ...old, song: { ...old.song, voicings: json } } : old));
    return nv;
  });

  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 10 }}>
        <div className="row" style={{ gap: 4 }}>
          <button className={"sm" + (instrument === "guitar" ? " primary" : "")} onClick={() => setInstrument("guitar")}>Guitar</button>
          <button className={"sm" + (instrument === "piano" ? " primary" : "")} onClick={() => setInstrument("piano")}>Piano</button>
        </div>
        <button className="primary" onClick={async () => {
          const name = `${(title || "song").replace(/[^\w.-]+/g, "_")}.png`;
          if (inTauri) {
            const bytes = await pngBytes(sheet.svg, sheet.width, sheet.height);
            const path = await savePng(name, bytes);
            if (path) await revealFile(path); // open the containing folder
          } else {
            downloadPng(sheet.svg, sheet.width, sheet.height, name);
          }
        }}>⬇ Export PNG</button>
      </div>

      {uniqueChords.length > 0 && (
        <div className="card" style={{ marginBottom: 10 }}>
          <label>{instrument === "guitar" ? "Voicings" : "Inversions"} — pick the shape shown on the sheet (click ‹ ›)</label>
          <div className="row" style={{ gap: 10, flexWrap: "wrap", marginTop: 6 }}>
            {uniqueChords.map((name) => {
              const n = countOf(name);
              const idx = idxOf(name);
              const sub = instrument === "guitar"
                ? (guitarFretsByName(name, idx)?.label ?? "—")
                : (INV_LABELS[idx] ?? `inv ${idx}`);
              const svg = instrument === "guitar"
                ? diagramSvgShape(guitarFretsByName(name, idx), name)
                : pianoVoicedSvg(voicedMidisByName(name, idx), name);
              return (
                <div key={name} style={{ border: "1px solid var(--line)", borderRadius: 2, padding: "4px 6px", textAlign: "center" }}>
                  <div dangerouslySetInnerHTML={{ __html: svg }} />
                  <div className="row" style={{ justifyContent: "center", gap: 4, alignItems: "center", marginTop: 2 }}>
                    <button className="sm ghost" disabled={n < 2} title="previous" onClick={() => cycle(name, -1)}>‹</button>
                    <span className="faint" style={{ fontSize: 11, minWidth: 72 }}>{sub} ({idx + 1}/{n})</span>
                    <button className="sm ghost" disabled={n < 2} title="next" onClick={() => cycle(name, 1)}>›</button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {chords.isLoading || lyrics.isLoading
        ? <div className="empty">Loading…</div>
        : <div className="card" style={{ overflow: "auto", textAlign: "center" }} dangerouslySetInnerHTML={{ __html: sheet.svg }} />}
    </div>
  );
}
