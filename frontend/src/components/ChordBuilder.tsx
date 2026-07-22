import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, inTauri, savePng, revealFile } from "../ipc/api";
import { NOTE_NAMES, diatonicChords } from "../music/theory";
import { playChord } from "../music/synth";
import { chartSvg, downloadPng, pngBytes, diagramSvgShape, pianoVoicedSvg } from "../music/diagrams";
import { padChordSvg } from "../music/pads";
import { QUALITY_OPTIONS, guitarFrets, guitarCount, chordPcsIdx, voicedMidis, voicedNotes, chordMidisByName, guitarFretsByName, guitarCountByName, chordSizeByName, voicedMidisByName } from "../music/engineAdapter";
import type { ChordQuality } from "../lib/music/types";
import { CircleOfFifths } from "./CircleOfFifths";
import { GuitarView } from "./GuitarView";

const labelFor = (q: ChordQuality) => QUALITY_OPTIONS.find(([, e]) => e === q)?.[0] ?? q;
const suffix = (q: ChordQuality) => { const l = labelFor(q); return l === "maj" ? "" : l; };

export function ChordBuilder() {
  const [root, setRoot] = useState(0);
  const [quality, setQuality] = useState<ChordQuality>("maj");
  // the working progression + per-chord shape picks survive reloads (they
  // used to live in bare component state — a hot reload or page hop wiped
  // freshly picked inversions right before an export; user-hit 2026-07-16)
  const WORKBENCH_KEY = "songsmith-builder-workbench";
  const loadBench = (): { prog: string[]; picks: { g: number; p: number; a: number }[] } => {
    try {
      const v = JSON.parse(localStorage.getItem(WORKBENCH_KEY) || "");
      if (Array.isArray(v?.prog) && Array.isArray(v?.picks)) return { prog: v.prog, picks: v.picks };
    } catch {}
    return { prog: [], picks: [] };
  };
  const [prog, setProg] = useState<string[]>(() => loadBench().prog);
  // per-entry shape picks for the progression cards (view-only — the library
  // stores chord NAMES): guitar voicing / piano inversion / pad inversion
  const [progView, setProgView] = useState<"guitar" | "piano" | "ableton">("guitar");
  const [abMsg, setAbMsg] = useState("");
  const nav = useNavigate();
  // hand the progression to the Composer: chord lane seeded (one bar each),
  // melody/bass empty; the scale lens (when set) fixes the key
  const openInComposer = (chords: string[]) => {
    const search: Record<string, string> = { prog: chords.join(",") };
    if (scale) { search.prog_root = NOTE_NAMES[scale.root]; search.prog_mode = scale.mode; }
    nav({ to: "/composer", search: search as never });
  };
  // the MIDI voicing honors the picked inversions — Push picks in Push view,
  // else the piano picks (guitar voicings don't map to closed inversions)
  const buildInAbleton = async (chords: string[], inversions?: number[]) => {
    setAbMsg("Stubbing the progression in Ableton…");
    try { setAbMsg(await api.abletonBuildProgression(chords, inversions)); } catch (e: any) { setAbMsg(String(e?.message ?? e)); }
  };
  const [picks, setPicks] = useState<{ g: number; p: number; a: number }[]>(() => loadBench().picks);
  useEffect(() => {
    try { localStorage.setItem(WORKBENCH_KEY, JSON.stringify({ prog, picks })); } catch {}
  }, [prog, picks]);
  const addProg = (c: string, pick?: { g: number; p: number; a: number }) => {
    setProg((p) => [...p, c]);
    setPicks((p) => [...p, pick ?? { g: 0, p: 0, a: 0 }]);
  };
  const removeProg = (i: number) => { setProg((pr) => pr.filter((_, j) => j !== i)); setPicks((pr) => pr.filter((_, j) => j !== i)); };
  // swap a card for the chord currently built above (picks reset — new chord,
  // new shape); plays it so the change is heard in place
  const replaceProg = (i: number, c: string, pick?: { g: number; p: number; a: number }) => {
    setProg((pr) => pr.map((x, j) => (j === i ? c : x)));
    setPicks((pr) => pr.map((e, j) => (j === i ? pick ?? { g: 0, p: 0, a: 0 } : e)));
    const m = chordMidisByName(c);
    if (m.length) playChord(m);
  };
  const clearProg = () => { setProg([]); setPicks([]); };
  const loadProg = (chords: string[], picksJson?: string) => {
    setProg(chords);
    // restore the saved per-chord shapes when the row carries them
    let saved: { g: number; p: number; a: number }[] = [];
    try { const v = JSON.parse(picksJson || ""); if (Array.isArray(v)) saved = v; } catch {}
    setPicks(chords.map((_, i) => ({ g: saved[i]?.g ?? 0, p: saved[i]?.p ?? 0, a: saved[i]?.a ?? 0 })));
  };
  const [name, setName] = useState("");
  const [view, setView] = useState<"guitar" | "piano" | "ableton">("guitar");
  // scale lens: highlights the key's diatonic chords on the circle and lists
  // them as one-click chips (root + quality land in the builder)
  const [scale, setScale] = useState<{ root: number; mode: "major" | "minor" } | null>(null);
  const diatonic = scale ? diatonicChords(scale.root, scale.mode) : [];
  const scalePcs = new Set(diatonic.map((d) => {
    const m = d.name.match(/^([A-G]#?)/); return m ? NOTE_NAMES.indexOf(m[1]) : -1;
  }));
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
  const save = useMutation({ mutationFn: () => api.saveProgression(name.trim() || "Untitled progression", prog, JSON.stringify(picks)), onSuccess: () => { qc.invalidateQueries({ queryKey: ["progressions"] }); setName(""); } });
  const del = useMutation({ mutationFn: (id: string) => api.deleteProgression(id), onSuccess: () => qc.invalidateQueries({ queryKey: ["progressions"] }) });

  return (
    <div className="grid2" style={{ alignItems: "start" }}>
      <div>
        <div className="card">
          <h3>Build a chord</h3>
          <label>Root</label>
          <div className="row" style={{ flexWrap: "wrap", gap: 4 }}>
            {NOTE_NAMES.map((n, i) => (<button key={n} className={"sm" + (i === root ? " primary" : "")} style={scale && scalePcs.has(i) && i !== root ? { borderColor: "var(--accent)" } : undefined} title={scale && scalePcs.has(i) ? "in the selected scale" : undefined} onClick={() => setRoot(i)}>{n}</button>))}
          </div>
          <label>Quality</label>
          <div className="row" style={{ flexWrap: "wrap", gap: 4 }}>
            {QUALITY_OPTIONS.map(([lbl, q]) => (<button key={q} className={"sm" + (q === quality ? " primary" : "")} onClick={() => setQuality(q)}>{lbl}</button>))}
          </div>

          <div className="row" style={{ justifyContent: "space-between", marginTop: 12, alignItems: "center" }}>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <b style={{ fontSize: 18 }}>{built}{inversion > 0 ? ` (inv ${inversion})` : ""}</b>
              <button className="sm" onClick={() => playChord(voicedMidis(root, quality, inversion))}>♪ play</button>
              <button className="sm primary" title="add with the voicing/inversion picked above" onClick={() => addProg(built, { g: v, p: inversion, a: inversion })}>+ add</button>
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
          <div className="row" style={{ gap: 8, alignItems: "flex-end", marginBottom: 6 }}>
            <div>
              <label>Scale</label>
              <select
                value={scale ? String(scale.root) : ""}
                onChange={(e) => setScale(e.target.value === "" ? null : { root: Number(e.target.value), mode: scale?.mode ?? "minor" })}
              >
                <option value="">(none)</option>
                {NOTE_NAMES.map((n, pc) => <option key={n} value={pc}>{n}</option>)}
              </select>
            </div>
            <select disabled={!scale} value={scale?.mode ?? "minor"} onChange={(e) => scale && setScale({ ...scale, mode: e.target.value as "major" | "minor" })}>
              <option value="minor">minor</option>
              <option value="major">major</option>
            </select>
            {scale && <span className="faint" style={{ fontSize: 11 }}>diatonic chords highlighted below — click a chip to build it</span>}
          </div>
          {scale && (
            <div className="row" style={{ flexWrap: "wrap", gap: 4, marginBottom: 6 }}>
              {diatonic.map((d) => (
                <button
                  key={d.roman}
                  className="sm"
                  onClick={() => {
                    const m = d.name.match(/^([A-G]#?)(.*)$/);
                    if (!m) return;
                    setRoot(NOTE_NAMES.indexOf(m[1]));
                    setQuality(m[2] === "dim" ? "dim" : m[2] === "m" ? "min" : "maj");
                    const midis = chordMidisByName(d.name);
                    if (midis.length) playChord(midis);
                  }}
                >
                  {d.name} <span className="faint">{d.roman}</span>
                </button>
              ))}
            </div>
          )}
          <div style={{ display: "flex", justifyContent: "center" }}>
            <CircleOfFifths rootPc={root} quality={quality === "min" ? "m" : ""} onPick={(pc, q) => { setRoot(pc); setQuality(q === "m" ? "min" : "maj"); }} highlightNames={scale ? new Set(diatonic.map((d) => d.name)) : undefined} />
          </div>
          <p className="faint">Outer ring = major, inner = relative minor. Click to pick &amp; hear.{scale ? " Ringed = in the selected scale." : ""}</p>
        </div>
      </div>

      <div>
        <div className="card">
          <div className="row" style={{ justifyContent: "space-between" }}>
            <h3 style={{ margin: 0 }}>Progression</h3>
            <div className="row" style={{ gap: 4 }}>
              {prog.length > 0 && (["guitar", "piano", "ableton"] as const).map((vw) => (
                <button key={vw} className={"sm" + (progView === vw ? " primary" : "")} onClick={() => setProgView(vw)}>{vw === "guitar" ? "Guitar" : vw === "piano" ? "Piano" : "Push"}</button>
              ))}
              {prog.length > 0 && <button className="sm" title="open this progression in the Composer — chord lane seeded, melody/bass yours" onClick={() => openInComposer(prog)}>🎹 Composer</button>}
              {prog.length > 0 && <button className="sm" title="stub this progression in Ableton Live (AbletonMCP must be on)" onClick={() => buildInAbleton(prog, picks.map((k) => (progView === "ableton" ? k.a : k.p)))}>⚡ Ableton</button>}
              {prog.length > 0 && <button className="sm" onClick={async () => {
                // the chart follows the ACTIVE view — guitar voicings, piano
                // inversions, or pad shapes, exactly as picked per chord
                const chart = chartSvg(prog, name || "Chord chart", picks.map((k) => (progView === "guitar" ? k.g : progView === "piano" ? k.p : k.a)), progView);
                const fname = `${(name || "chord-chart").replace(/[^\w.-]+/g, "_")}.png`;
                if (inTauri) {
                  const bytes = await pngBytes(chart.svg, chart.width, chart.height);
                  const path = await savePng(fname, bytes);
                  if (path) await revealFile(path); // open the containing folder
                } else {
                  downloadPng(chart.svg, chart.width, chart.height, fname);
                }
              }}>⬇ Export chart</button>}
            </div>
          </div>
          {abMsg && <div className="banner" style={{ marginTop: 8, whiteSpace: "pre-wrap" }}>{abMsg}<button className="sm ghost" style={{ marginLeft: 8 }} onClick={() => setAbMsg("")}>×</button></div>}
          {prog.length === 0 ? (
            <p className="faint">No chords yet — build a chord and “+ add”.</p>
          ) : (
            <div style={{ display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 8, marginTop: 8, justifyItems: "center" }}>
              {prog.map((c, i) => {
                const k = picks[i] ?? { g: 0, p: 0, a: 0 };
                const idx = progView === "guitar" ? k.g : progView === "piano" ? k.p : k.a;
                const count = Math.max(1, progView === "guitar" ? guitarCountByName(c) : chordSizeByName(c));
                const svg = progView === "guitar"
                  ? diagramSvgShape(guitarFretsByName(c, idx), c)
                  : progView === "piano"
                  ? pianoVoicedSvg(voicedMidisByName(c, idx), c)
                  : (padChordSvg(c, 20, idx) ?? `<svg xmlns="http://www.w3.org/2000/svg" width="90" height="60"></svg>`);
                const sub = progView === "guitar" ? (guitarFretsByName(c, idx)?.label ?? "—") : ["root", "1st inv", "2nd inv", "3rd inv", "4th inv"][idx] ?? `inv ${idx}`;
                const cycle = (dir: number) => setPicks((pr) => pr.map((e, j) => {
                  if (j !== i) return e;
                  const next = ((idx + dir) % count + count) % count;
                  return progView === "guitar" ? { ...e, g: next } : progView === "piano" ? { ...e, p: next } : { ...e, a: next };
                }));
                const play = () => {
                  const m = progView === "guitar" ? chordMidisByName(c) : voicedMidisByName(c, idx);
                  if (m.length) playChord(m);
                };
                // fixed-size SVGs (esp. the piano's keyboard span) must fit the
                // 4-col grid cell — scale oversized ones down with EXPLICIT
                // width/height (the WebView renders style-only svg at height 0)
                const fitted = svg.replace(/<svg width="([\d.]+)" height="([\d.]+)"/, (m, w, h) => {
                  const W = parseFloat(w); const H = parseFloat(h); const MAX = 150;
                  if (W <= MAX) return m;
                  const s = MAX / W;
                  // the original viewBox attribute follows and keeps the aspect
                  return `<svg width="${Math.round(W * s)}" height="${Math.round(H * s)}"`;
                });
                return (
                  <div key={i} className="col" style={{ alignItems: "center", gap: 2, minWidth: 0, width: "100%" }}>
                    <div dangerouslySetInnerHTML={{ __html: fitted }} onClick={play} style={{ cursor: "pointer", maxWidth: "100%", display: "flex", justifyContent: "center" }} />
                    <div className="row" style={{ gap: 4, alignItems: "center" }}>
                      <button className="sm ghost" disabled={count < 2} onClick={() => cycle(-1)}>‹</button>
                      <span className="faint" style={{ fontSize: 10, minWidth: 64, textAlign: "center" }}>{sub} ({idx + 1}/{count})</span>
                      <button className="sm ghost" disabled={count < 2} onClick={() => cycle(1)}>›</button>
                    </div>
                    <div className="row" style={{ gap: 4 }}>
                      <button className="sm ghost" title={`replace with ${built} (the chord built above, at its picked voicing/inversion)`} onClick={() => replaceProg(i, built, { g: v, p: inversion, a: inversion })}>replace</button>
                      <button className="sm ghost danger" onClick={() => removeProg(i)}>remove</button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
          {prog.length > 0 && (
            <div className="row" style={{ gap: 8, marginTop: 10, flexWrap: "wrap" }}>
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Progression name" style={{ width: 200 }} />
              <button className="primary" onClick={() => save.mutate()} disabled={save.isPending}>Save to library</button>
              <button className="ghost" onClick={clearProg}>clear</button>
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
                  <button className="sm ghost" onClick={() => loadProg(p.chords, p.picks)}>load</button>
                  <button className="sm" title="open in the Composer" onClick={() => openInComposer(p.chords)}>🎹</button>
                  <button className="sm" title="stub in Ableton" onClick={() => buildInAbleton(p.chords)}>⚡</button>
                  <button className="sm" onClick={async () => {
                    const chart = chartSvg(p.chords, p.name);
                    const fname = `${p.name.replace(/[^\w.-]+/g, "_")}.png`;
                    if (inTauri) {
                      const bytes = await pngBytes(chart.svg, chart.width, chart.height);
                      const path = await savePng(fname, bytes);
                      if (path) await revealFile(path);
                    } else {
                      downloadPng(chart.svg, chart.width, chart.height, fname);
                    }
                  }}>export</button>
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
