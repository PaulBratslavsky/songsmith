import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { NOTE_NAMES } from "../music/theory";
import { playChord } from "../music/synth";
import { diagramSvg, chartSvg, downloadSvg, pianoVoicedSvg } from "../music/diagrams";
import { padChordSvg } from "../music/pads";
import { QUALITY_OPTIONS, guitarFrets, guitarCount, chordPcsIdx, voicedMidis, voicedNotes, chordMidisByName } from "../music/engineAdapter";
import type { ChordQuality } from "../lib/music/types";
import { CircleOfFifths } from "./CircleOfFifths";
import { GuitarView } from "./GuitarView";

const labelFor = (q: ChordQuality) => QUALITY_OPTIONS.find(([, e]) => e === q)?.[0] ?? q;
const suffix = (q: ChordQuality) => { const l = labelFor(q); return l === "maj" ? "" : l; };

export function ChordBuilder() {
  const [root, setRoot] = useState(0);
  const [quality, setQuality] = useState<ChordQuality>("maj");
  const [prog, setProg] = useState<string[]>([]);
  const [name, setName] = useState("");
  const [view, setView] = useState<"guitar" | "piano" | "ableton">("guitar");
  const [vIdx, setVIdx] = useState(0);
  const [inversion, setInversion] = useState(0);
  const qc = useQueryClient();

  const built = NOTE_NAMES[root] + suffix(quality);
  const pcs = chordPcsIdx(root, quality);
  const maxInv = Math.max(0, pcs.length - 1);
  const count = guitarCount(root, quality);
  const v = count ? ((vIdx % count) + count) % count : 0;
  const shape = count ? guitarFrets(root, quality, v) : null;
  useEffect(() => { setVIdx(0); setInversion(0); }, [root, quality]);

  const saved = useQuery({ queryKey: ["progressions"], queryFn: api.listProgressions });
  const save = useMutation({ mutationFn: () => api.saveProgression(name.trim() || "Untitled progression", prog), onSuccess: () => { qc.invalidateQueries({ queryKey: ["progressions"] }); setName(""); } });
  const del = useMutation({ mutationFn: (id: string) => api.deleteProgression(id), onSuccess: () => qc.invalidateQueries({ queryKey: ["progressions"] }) });

  return (
    <div className="grid2" style={{ alignItems: "start" }}>
      <div>
        <div className="card">
          <h3>Build a chord</h3>
          <label>Root</label>
          <div className="row" style={{ flexWrap: "wrap", gap: 4 }}>
            {NOTE_NAMES.map((n, i) => (<button key={n} className={"sm" + (i === root ? " primary" : "")} onClick={() => setRoot(i)}>{n}</button>))}
          </div>
          <label>Quality</label>
          <div className="row" style={{ flexWrap: "wrap", gap: 4 }}>
            {QUALITY_OPTIONS.map(([lbl, q]) => (<button key={q} className={"sm" + (q === quality ? " primary" : "")} onClick={() => setQuality(q)}>{lbl}</button>))}
          </div>

          <div className="row" style={{ justifyContent: "space-between", marginTop: 12, alignItems: "center" }}>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <b style={{ fontSize: 18 }}>{built}{inversion > 0 ? ` (inv ${inversion})` : ""}</b>
              <button className="sm" onClick={() => playChord(voicedMidis(root, quality, inversion))}>♪ play</button>
              <button className="sm primary" onClick={() => setProg((p) => [...p, built])}>+ add</button>
            </div>
            <div className="row" style={{ gap: 4 }}>
              <button className={"sm" + (view === "guitar" ? " primary" : "")} onClick={() => setView("guitar")}>Guitar</button>
              <button className={"sm" + (view === "piano" ? " primary" : "")} onClick={() => setView("piano")}>Piano</button>
              <button className={"sm" + (view === "ableton" ? " primary" : "")} onClick={() => setView("ableton")}>Ableton</button>
            </div>
          </div>

          <div className="row" style={{ gap: 8, marginTop: 10, alignItems: "center" }}>
            <span className="faint">inversion</span>
            <button className="sm" onClick={() => setInversion((i) => Math.max(0, i - 1))} disabled={inversion <= 0}>‹</button>
            <span>{inversion}</span>
            <button className="sm" onClick={() => setInversion((i) => Math.min(maxInv, i + 1))} disabled={inversion >= maxInv}>›</button>
            <span className="faint">voiced (low→high): {voicedNotes(root, quality, inversion).join(" · ")}</span>
          </div>

          <div style={{ marginTop: 10, minHeight: 70 }}>
            {view === "guitar" ? (
              shape ? (
                <>
                  <div className="row" style={{ justifyContent: "space-between", alignItems: "center", maxWidth: 240 }}>
                    <button className="sm" onClick={() => setVIdx((i) => (i - 1 + count) % count)} disabled={count < 2}>‹ Prev</button>
                    <span className="faint">{shape.label} · {v + 1} of {count}</span>
                    <button className="sm" onClick={() => setVIdx((i) => (i + 1) % count)} disabled={count < 2}>Next ›</button>
                  </div>
                  <GuitarView frets={shape.frets} />
                </>
              ) : <span className="faint">(no guitar shape for this chord — see Piano)</span>
            ) : view === "piano" ? (
              // the VOICED piano — the drawn keys follow the inversion (the old
              // MiniPiano drew bare pitch classes, so ‹ › changed nothing)
              <div dangerouslySetInnerHTML={{ __html: pianoVoicedSvg(voicedMidis(root, quality, inversion), built) }} />
            ) : (
              padChordSvg(built, 26, inversion)
                ? <div dangerouslySetInnerHTML={{ __html: padChordSvg(built, 26, inversion)! }} />
                : <span className="faint">(no pad shape for this chord)</span>
            )}
          </div>
          <div className="faint">{pcs.map((pc) => NOTE_NAMES[pc]).join(" · ")}</div>
        </div>

        <div className="card">
          <h3>Circle of fifths</h3>
          <div style={{ display: "flex", justifyContent: "center" }}>
            <CircleOfFifths rootPc={root} quality={quality === "min" ? "m" : ""} onPick={(pc, q) => { setRoot(pc); setQuality(q === "m" ? "min" : "maj"); }} />
          </div>
          <p className="faint">Outer ring = major, inner = relative minor. Click to pick &amp; hear.</p>
        </div>
      </div>

      <div>
        <div className="card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <h3 style={{ margin: 0 }}>Progression</h3>
            {prog.length > 0 && <button className="sm" onClick={() => downloadSvg(chartSvg(prog, name || "Chord chart"), "chord-chart.svg")}>⬇ Export chart</button>}
          </div>
          {prog.length === 0 ? (
            <p className="faint">No chords yet — build a chord and “+ add”.</p>
          ) : (
            <div className="row" style={{ flexWrap: "wrap", gap: 8, marginTop: 8 }}>
              {prog.map((c, i) => (
                <div key={i} className="col" style={{ alignItems: "center", gap: 2 }}>
                  <div dangerouslySetInnerHTML={{ __html: diagramSvg(c) }} onClick={() => { const m = chordMidisByName(c); if (m.length) playChord(m); }} style={{ cursor: "pointer" }} />
                  <button className="sm ghost danger" onClick={() => setProg((pr) => pr.filter((_, j) => j !== i))}>remove</button>
                </div>
              ))}
            </div>
          )}
          {prog.length > 0 && (
            <div className="row" style={{ gap: 8, marginTop: 10, flexWrap: "wrap" }}>
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Progression name" style={{ width: 200 }} />
              <button className="primary" onClick={() => save.mutate()} disabled={save.isPending}>Save to library</button>
              <button className="ghost" onClick={() => setProg([])}>clear</button>
            </div>
          )}
        </div>

        {saved.data && saved.data.length > 0 && (
          <div className="card">
            <h3>Saved progressions</h3>
            {saved.data.map((p) => (
              <div key={p.id} className="list-item" style={{ marginBottom: 6 }}>
                <div className="col" style={{ gap: 2 }}>
                  <b>{p.name}</b>
                  <span className="faint">{p.chords.join(" · ")}</span>
                </div>
                <div className="row" style={{ gap: 6 }}>
                  <button className="sm ghost" onClick={() => setProg(p.chords)}>load</button>
                  <button className="sm" onClick={() => downloadSvg(chartSvg(p.chords, p.name), `${p.name}.svg`)}>export</button>
                  <button className="sm danger" onClick={() => del.mutate(p.id)}>delete</button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
