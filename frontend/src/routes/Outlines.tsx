// Outline Builder (user request 2026-07-22): build CUSTOM song outlines —
// named sections with bar counts, a tempo — save them to the library, and lay
// any of them into Ableton as section clips + locators (no chords, no notes).
// Genre templates seed the editor; the workbench persists across reloads.
import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { OUTLINE_TEMPLATES } from "../lib/outlineTemplates";

type Row = { label: string; bars: number };
const BENCH_KEY = "songsmith-outline-workbench";

function loadBench(): { name: string; bpm: number; rows: Row[] } {
  try {
    const v = JSON.parse(localStorage.getItem(BENCH_KEY) || "");
    if (Array.isArray(v?.rows)) return { name: v.name ?? "", bpm: Number(v.bpm) || 120, rows: v.rows };
  } catch {}
  const t = OUTLINE_TEMPLATES[0];
  return { name: "", bpm: t.bpm, rows: t.sections.map(([label, bars]) => ({ label, bars })) };
}

export function Outlines() {
  const qc = useQueryClient();
  const [name, setName] = useState(() => loadBench().name);
  const [bpm, setBpm] = useState(() => loadBench().bpm);
  const [rows, setRows] = useState<Row[]>(() => loadBench().rows);
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    try { localStorage.setItem(BENCH_KEY, JSON.stringify({ name, bpm, rows })); } catch {}
  }, [name, bpm, rows]);

  const saved = useQuery({ queryKey: ["outlines"], queryFn: api.listOutlines });
  const save = useMutation({
    mutationFn: () => api.saveOutline(name.trim() || "Untitled outline", bpm, rows.map((r) => [r.label, r.bars] as [string, number])),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["outlines"] }); setMsg("Saved to library."); },
  });
  const del = useMutation({
    mutationFn: (id: string) => api.deleteOutline(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["outlines"] }),
  });

  const setRow = (i: number, patch: Partial<Row>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const move = (i: number, dir: number) => setRows((rs) => {
    const j = i + dir;
    if (j < 0 || j >= rs.length) return rs;
    const out = [...rs];
    [out[i], out[j]] = [out[j], out[i]];
    return out;
  });
  const totalBars = rows.reduce((a, r) => a + (Number(r.bars) || 0), 0);

  const build = async (b: number, sections: [string, number][]) => {
    setBusy(true);
    setMsg("Laying the outline in Ableton…");
    try { setMsg(await api.abletonBuildOutline(b, sections)); } catch (e: any) { setMsg(String(e?.message ?? e)); }
    setBusy(false);
  };

  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Outlines</h1>
          <span className="muted">Song skeletons — sections and bars only. Build them into Ableton as colored section clips + locators; add the music yourself.</span>
        </div>
      </div>

      <div className="grid2" style={{ alignItems: "start" }}>
        <div className="card">
          <div className="row" style={{ gap: 8, alignItems: "flex-end", flexWrap: "wrap" }}>
            <div style={{ flex: 1, minWidth: 160 }}>
              <label>Name</label>
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. My synthwave arc" style={{ width: "100%" }} />
            </div>
            <div>
              <label>BPM</label>
              <input type="number" value={bpm} onChange={(e) => setBpm(Number(e.target.value) || 120)} style={{ width: 70 }} />
            </div>
            <div>
              <label>Start from</label>
              <select value="" onChange={(e) => {
                const t = OUTLINE_TEMPLATES.find((x) => x.name === e.target.value);
                if (t) { setBpm(t.bpm); setRows(t.sections.map(([label, bars]) => ({ label, bars }))); }
              }}>
                <option value="" disabled>template…</option>
                {OUTLINE_TEMPLATES.map((t) => <option key={t.name} value={t.name}>{t.name}</option>)}
              </select>
            </div>
          </div>

          <label style={{ marginTop: 10 }}>Sections <span className="faint">({rows.length} · {totalBars} bars)</span></label>
          <div className="col" style={{ gap: 6 }}>
            {rows.map((r, i) => (
              <div key={i} className="row" style={{ gap: 6, alignItems: "center" }}>
                <input value={r.label} onChange={(e) => setRow(i, { label: e.target.value })} placeholder="Section" style={{ flex: 1 }} />
                <input type="number" min={1} value={r.bars} onChange={(e) => setRow(i, { bars: Math.max(1, Number(e.target.value) || 1) })} title="bars" style={{ width: 60 }} />
                <button className="sm ghost" disabled={i === 0} title="up" onClick={() => move(i, -1)}>↑</button>
                <button className="sm ghost" disabled={i === rows.length - 1} title="down" onClick={() => move(i, 1)}>↓</button>
                <button className="sm ghost danger" title="remove" onClick={() => setRows((rs) => rs.filter((_, j) => j !== i))}>×</button>
              </div>
            ))}
          </div>
          <div className="row" style={{ gap: 6, marginTop: 8 }}>
            <button className="sm" onClick={() => setRows((rs) => [...rs, { label: "Section", bars: 8 }])}>+ section</button>
          </div>

          <div className="row" style={{ gap: 8, marginTop: 14 }}>
            <button className="primary" disabled={busy || rows.length === 0} onClick={() => build(bpm, rows.map((r) => [r.label, r.bars]))}>
              {busy ? "building…" : "⚡ Build in Ableton"}
            </button>
            <button disabled={save.isPending || rows.length === 0} onClick={() => save.mutate()}>{save.isPending ? "saving…" : "💾 Save to library"}</button>
          </div>
          {msg && <pre className="artifact-text" style={{ whiteSpace: "pre-wrap", maxHeight: 160, marginTop: 10 }}>{msg}</pre>}
          <p className="faint" style={{ fontSize: 11, marginTop: 8 }}>Live must be open with the AbletonMCP control surface on. Re-building relays the Sections track and locators cleanly.</p>
        </div>

        <div className="card">
          <h3>Saved outlines</h3>
          {saved.data?.length === 0 && <p className="faint">Nothing saved yet — build one on the left and 💾 save it.</p>}
          {saved.data?.map((o) => (
            <div key={o.id} className="list-item">
              <div className="col" style={{ gap: 2 }}>
                <b>{o.name}</b>
                <span className="faint" style={{ fontSize: 11 }}>
                  {Number(o.bpm)} BPM · {o.sections.map(([l, b]) => `${l} ${Number(b)}`).join(" · ")}
                </span>
              </div>
              <div className="row" style={{ gap: 6 }}>
                <button className="sm ghost" title="load into the editor" onClick={() => { setName(o.name); setBpm(Number(o.bpm)); setRows(o.sections.map(([label, bars]) => ({ label, bars: Number(bars) }))); }}>load</button>
                <button className="sm" title="lay in Ableton" onClick={() => build(Number(o.bpm), o.sections.map(([l, b]) => [l, Number(b)] as [string, number]))}>⚡</button>
                <button className="sm danger" onClick={() => del.mutate(o.id)}>delete</button>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
