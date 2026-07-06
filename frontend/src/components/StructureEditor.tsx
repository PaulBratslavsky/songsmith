import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { NOTE_NAMES, pitchClassOf } from "../music/theory";
import { FieldChat } from "./FieldChat";

export type Section = { type?: string; label: string; bars: number; role: string; frozen?: boolean };
export type StructureData = { root: string; mode: string; bpm: number; keyNote: string; tempoNote: string; sections: Section[] };

export function parseStructure(content: string): StructureData {
  let data: any = null, text = "";
  try { const v = JSON.parse(content); text = v?.text ?? ""; data = v?.data ?? null; } catch { text = content; }
  const km = text.match(/\*\*KEY:\*\*\s*[^\n—-]*[—-]\s*([^\n]+)/i);
  const tm = text.match(/\*\*TEMPO:\*\*\s*[^\n—-]*[—-]\s*([^\n]+)/i);
  return {
    root: NOTE_NAMES[pitchClassOf(data?.key?.root ?? "A") ?? 0],
    mode: data?.key?.mode ?? "minor",
    bpm: Number(data?.bpm ?? 120),
    keyNote: data?.keyNote ?? (km ? km[1].trim() : ""),
    tempoNote: data?.tempoNote ?? (tm ? tm[1].replace(/\([^)]*\)/g, "").trim() : ""),
    sections: Array.isArray(data?.sections)
      ? data.sections.map((s: any) => ({ type: s.type ?? "", label: s.label ?? s.type ?? "", bars: Number(s.bars ?? 8), role: s.role ?? "", frozen: s.frozen === true }))
      : [],
  };
}

export function structureToMarkdown(d: StructureData): string {
  const out: string[] = [
    `**KEY:** ${d.root} ${d.mode}${d.keyNote ? ` — ${d.keyNote}` : ""}`,
    `**TEMPO:** ${d.bpm} BPM${d.tempoNote ? ` — ${d.tempoNote}` : ""}`,
    "",
    "**SECTION MAP**",
    "",
  ];
  d.sections.forEach((s, i) => out.push(`${i + 1}. **${s.label}** (${s.bars} bars)${s.role ? ` — ${s.role}` : ""}`));
  return out.join("\n");
}

export function StructureEditor({
  songId, stageId, kind, content, onChanged,
}: {
  songId: string; stageId: string; kind: string; content: string; onChanged: () => void;
}) {
  const [d, setD] = useState<StructureData>(() => parseStructure(content));
  const [saved, setSaved] = useState("");
  const set = (patch: Partial<StructureData>) => { setD((c) => ({ ...c, ...patch })); setSaved(""); };
  const setSec = (i: number, patch: Partial<Section>) => set({ sections: d.sections.map((s, j) => (j === i ? { ...s, ...patch } : s)) });
  const addSec = () => set({ sections: [...d.sections, { label: "Section", bars: 8, role: "" }] });
  const rmSec = (i: number) => set({ sections: d.sections.filter((_, j) => j !== i) });
  const move = (i: number, dir: number) => {
    const j = i + dir; if (j < 0 || j >= d.sections.length) return;
    const s = [...d.sections]; [s[i], s[j]] = [s[j], s[i]]; set({ sections: s });
  };

  const save = useMutation({
    mutationFn: async () => {
      // persist `frozen` only when set, so unfrozen sections stay as before
      const sections = d.sections.map((s) => ({ type: s.type, label: s.label, bars: s.bars, role: s.role, ...(s.frozen ? { frozen: true } : {}) }));
      await api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text: structureToMarkdown(d), data: { key: { root: d.root, mode: d.mode }, bpm: d.bpm, keyNote: d.keyNote, tempoNote: d.tempoNote, sections } }));
      // Structure is the source of truth for key/tempo — sync it to the song so
      // the Chords palette, Sheet, and Ableton all infer from one place
      await api.updateSongKey(songId, d.root, d.mode, d.bpm);
    },
    onSuccess: () => { setSaved("Saved — key/tempo synced to the song."); onChanged(); },
  });

  return (
    <div className="col" style={{ gap: 12 }}>
      <div className="row" style={{ gap: 10, flexWrap: "wrap", alignItems: "flex-end" }}>
        <div><label>Key</label><select value={d.root} onChange={(e) => set({ root: e.target.value })}>{NOTE_NAMES.map((n) => <option key={n} value={n}>{n}</option>)}</select></div>
        <div><label>Mode</label><select value={d.mode} onChange={(e) => set({ mode: e.target.value })}><option value="minor">minor</option><option value="major">major</option></select></div>
        <div><label>BPM</label><input type="number" value={d.bpm} onChange={(e) => set({ bpm: Number(e.target.value) })} style={{ width: 70 }} /></div>
      </div>
      <div><label>Key note <span className="faint">(why this key)</span></label><input value={d.keyNote} onChange={(e) => set({ keyNote: e.target.value })} style={{ width: "100%" }} /></div>
      <div><label>Tempo note <span className="faint">(why this tempo)</span></label><input value={d.tempoNote} onChange={(e) => set({ tempoNote: e.target.value })} style={{ width: "100%" }} /></div>

      <div>
        <label>Sections</label>
        <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>🔒 Locked sections are kept as-is when you regenerate this stage.</p>
        <div className="col" style={{ gap: 8 }}>
          {d.sections.map((s, i) => (
            <div key={i} className={s.frozen ? "frozen" : ""} style={{ border: "1px solid var(--line)", borderRadius: 2, padding: 8 }}>
              <div className="row" style={{ gap: 6, alignItems: "center" }}>
                <button className={"sm ghost" + (s.frozen ? " primary" : "")} title={s.frozen ? "unlock — let regeneration rewrite this section" : "lock — keep this section as-is when you regenerate"} onClick={() => setSec(i, { frozen: !s.frozen })}>{s.frozen ? "🔒" : "🔓"}</button>
                <input value={s.label} onChange={(e) => setSec(i, { label: e.target.value })} placeholder="Verse 1" style={{ flex: 1 }} />
                {s.frozen && <span className="badge done" title="locked — kept as-is when you regenerate">locked</span>}
                <input type="number" value={s.bars} onChange={(e) => setSec(i, { bars: Number(e.target.value) })} title="bars" style={{ width: 60 }} />
                <button className="sm ghost" title="up" disabled={i === 0} onClick={() => move(i, -1)}>↑</button>
                <button className="sm ghost" title="down" disabled={i === d.sections.length - 1} onClick={() => move(i, 1)}>↓</button>
                <FieldChat stageLabel="Structure" fieldLabel={`${s.label || "section"} — role`} current={s.role} onResult={(v) => setSec(i, { role: v })} />
                <button className="sm ghost" title="remove" onClick={() => rmSec(i)}>×</button>
              </div>
              <textarea value={s.role} onChange={(e) => setSec(i, { role: e.target.value })} placeholder="energy / role of this section…" style={{ width: "100%", minHeight: 40, marginTop: 6 }} />
            </div>
          ))}
        </div>
        <button className="sm" style={{ marginTop: 6 }} onClick={addSec}>+ section</button>
      </div>

      <div className="row" style={{ gap: 8 }}>
        <button className="primary sm" disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? "saving…" : "Save structure"}</button>
        {saved && <span className="faint">{saved}</span>}
      </div>
    </div>
  );
}
