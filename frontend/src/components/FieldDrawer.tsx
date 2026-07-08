import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { api } from "../ipc/api";

export type FieldRef = { id: string; stageLabel: string; fieldLabel: string; value: string; onChange: (v: string) => void };

/** The song whose page we're on — field refinement sends it so Claude gets the
 *  song's key/BPM + style preset as context (💬 edits stay on-style). The 💬
 *  drawer only exists on /song/<id> pages, so the URL is the source of truth. */
function currentSongId(): string | undefined {
  return window.location.pathname.match(/\/song\/([^/?#]+)/)?.[1];
}
type Ctx = { active: FieldRef | null; open: (r: FieldRef) => void; sync: (r: FieldRef) => void; close: () => void };

const FieldDrawerContext = createContext<Ctx | null>(null);
export const useFieldDrawer = () => useContext(FieldDrawerContext);

export function FieldDrawerProvider({ children }: { children: ReactNode }) {
  const [active, setActive] = useState<FieldRef | null>(null);
  return (
    <FieldDrawerContext.Provider value={{
      active,
      open: (r) => setActive(r),
      sync: (r) => setActive((cur) => (cur && cur.id === r.id ? r : cur)),
      close: () => setActive(null),
    }}>{children}</FieldDrawerContext.Provider>
  );
}

/** The inspector panel — renders for the currently-focused field. */
export function FieldDrawer({ context }: { context: string[] }) {
  const d = useFieldDrawer();
  if (!d?.active) return null;
  const f = d.active;
  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
        <h3 style={{ margin: 0 }}>{f.stageLabel} › {f.fieldLabel}</h3>
        <button className="sm ghost" title="close" onClick={d.close}>✕</button>
      </div>
      <textarea value={f.value} onChange={(e) => f.onChange(e.target.value)} spellCheck={false}
        style={{ width: "100%", minHeight: 140, fontSize: 12, fontFamily: "var(--mono)" }} />
      <Variants f={f} />
      {context.length > 0 && (
        <div style={{ marginTop: 12 }}>
          <label>What the AI sees</label>
          <div className="faint" style={{ fontSize: 11, lineHeight: 1.5 }}>{context.map((c, i) => <div key={i}>{c}</div>)}</div>
        </div>
      )}
      <hr />
      <FieldThread key={f.id} f={f} />
    </div>
  );
}

function Variants({ f }: { f: FieldRef }) {
  const [opts, setOpts] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const gen = async () => {
    setBusy(true); setOpts([]);
    try {
      const out = await api.refineField(f.stageLabel, f.fieldLabel, f.value, "Give exactly 3 DISTINCT alternative versions of this field. Separate the three with a line containing only ~~~ . No numbering, no commentary.", currentSongId());
      const parts = out.split(/\n?~~~\n?/).map((s) => s.trim()).filter(Boolean);
      setOpts(parts.length >= 2 ? parts : [out.trim()]);
    } catch (e: any) { setOpts([`Error: ${String(e?.message ?? e)}`]); }
    finally { setBusy(false); }
  };
  return (
    <div style={{ marginTop: 8 }}>
      <div className="row" style={{ gap: 6 }}>
        <button className="sm" disabled={busy} onClick={gen}>{busy ? "…" : "✦ Variants"}</button>
        {opts.length > 0 && <button className="sm ghost" onClick={() => setOpts([])}>clear</button>}
      </div>
      {opts.map((o, i) => (
        <div key={i} className="row" style={{ gap: 6, alignItems: "flex-start", marginTop: 6, border: "1px solid var(--line)", borderRadius: 2, padding: 6 }}>
          <div className="faint" style={{ flex: 1, fontSize: 11, whiteSpace: "pre-wrap" }}>{o}</div>
          <button className="sm ghost" title="use this" onClick={() => { f.onChange(o); setOpts([]); }}>use</button>
        </div>
      ))}
    </div>
  );
}

function FieldThread({ f }: { f: FieldRef }) {
  const [msgs, setMsgs] = useState<{ role: "you" | "claude"; text: string }[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const send = async () => {
    const t = input.trim(); if (!t || busy) return;
    setInput(""); setMsgs((m) => [...m, { role: "you", text: t }]); setBusy(true);
    try {
      const out = await api.refineField(f.stageLabel, f.fieldLabel, f.value, t, currentSongId());
      f.onChange(out);
      setMsgs((m) => [...m, { role: "claude", text: "✓ updated the field" }]);
    } catch (e: any) { setMsgs((m) => [...m, { role: "claude", text: "error: " + String(e?.message ?? e) }]); }
    finally { setBusy(false); }
  };
  return (
    <div>
      <label>Ask Claude to change this field</label>
      {msgs.map((m, i) => (
        <div key={i} className="faint" style={{ fontSize: 11, marginBottom: 3 }}><b>{m.role}</b> {m.text}</div>
      ))}
      <textarea value={input} onChange={(e) => setInput(e.target.value)} rows={2}
        placeholder={`e.g. "more dread", "fold in phonk"  (⌘/Ctrl+Enter)`}
        onKeyDown={(e) => { if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) send(); }}
        style={{ width: "100%", fontSize: 12 }} />
      <button className="sm primary" disabled={busy || !input.trim()} onClick={send} style={{ marginTop: 4 }}>{busy ? "asking…" : "Update field"}</button>
    </div>
  );
}
