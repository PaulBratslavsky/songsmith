import { useEffect, useRef, useState } from "react";
import { api, listen } from "../ipc/api";
import type { StyleInput, StylePreset } from "../ipc/generated";

type Msg = { role: "you" | "claude"; text: string; tools: string[] };

/** Collapsible chat to edit a style preset over MCP. Claude calls
 *  update_style_preset; on completion the form reloads with the change. */
export function PresetChat({
  presetId,
  current,
  onApplied,
}: {
  presetId: string;
  current: StyleInput;
  onApplied: (p: StylePreset) => void;
}) {
  const [open, setOpen] = useState(false);
  const [chat, setChat] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const sessionRef = useRef<string | null>(null);
  const currentRef = useRef(current);
  currentRef.current = current;
  const patchLast = (fn: (m: Msg) => Msg) => setChat((c) => c.map((m, i) => (i === c.length - 1 ? fn(m) : m)));

  useEffect(() => { sessionRef.current = null; setChat([]); }, [presetId]);

  useEffect(() => {
    let un1 = () => {}, un2 = () => {}, un3 = () => {};
    (async () => {
      un1 = await listen<{ event: any }>("chat_event", ({ event }) => {
        const t = event?.type;
        if (t === "stream_event" && event.event?.type === "content_block_delta" && event.event.delta?.type === "text_delta") {
          patchLast((m) => ({ ...m, text: m.text + (event.event.delta.text ?? "") }));
        } else if (t === "assistant") {
          const tools = (event.message?.content ?? []).filter((b: any) => b.type === "tool_use").map((b: any) => b.name);
          if (tools.length) patchLast((m) => ({ ...m, tools: [...m.tools, ...tools] }));
        } else if (t === "result") {
          patchLast((m) => (m.text.trim() ? m : { ...m, text: event.result ?? "" }));
        }
      });
      un2 = await listen<{}>("chat_done", async () => {
        setBusy(false);
        const p = await api.getStylePreset(presetId);
        if (p) onApplied(p);
      });
      un3 = await listen<{ error: string }>("chat_error", (p) => { patchLast((m) => ({ ...m, text: "⚠️ " + p.error })); setBusy(false); });
    })();
    return () => { un1(); un2(); un3(); };
  }, [presetId, onApplied]);

  const send = async () => {
    const ask = input.trim();
    if (!ask || busy) return;
    setInput("");
    setChat((c) => [...c, { role: "you", text: ask, tools: [] }, { role: "claude", text: "", tools: [] }]);
    setBusy(true);
    const ctx =
      `[Songsmith — style preset]\n` +
      `Edit the style preset id="${presetId}". Current fields (JSON): ${JSON.stringify(currentRef.current)}\n` +
      `To APPLY, call the songsmith MCP tool update_style_preset with id="${presetId}" and the full field set ` +
      `{name, genre, mood, influences, key_tempo_feel, vocal_range, themes, lyric_exemplars} — preserve unchanged fields, change only what's asked. ` +
      `Then reply with one short line.\n\nRequest: ${ask}`;
    try {
      sessionRef.current = await api.chatSend(ctx, sessionRef.current ?? undefined);
    } catch (e: any) {
      patchLast((m) => ({ ...m, text: "⚠️ " + String(e?.message ?? e) }));
      setBusy(false);
    }
  };

  return (
    <div className="card" style={{ marginTop: 12 }}>
      <div className="row" style={{ justifyContent: "space-between", cursor: "pointer" }} onClick={() => setOpen((o) => !o)}>
        <h3 style={{ margin: 0 }}>💬 Edit this preset with Claude</h3>
        <button className="sm ghost">{open ? "hide ▾" : "show ▸"}</button>
      </div>
      {open && (
        <div style={{ marginTop: 10 }}>
          {chat.length > 0 && (
            <div className="chat-log" style={{ maxHeight: "26vh", marginBottom: 8 }}>
              {chat.map((m, i) => (
                <div key={i} className={"bubble " + (m.role === "you" ? "user" : "")}>
                  <div className="who">{m.role}</div>
                  {m.text ? <div className="bubble-text">{m.text}</div> : (m.role === "claude" && busy && <span className="faint spin">▮ working…</span>)}
                  {m.tools.length > 0 && <div className="tools-row">{m.tools.map((t, j) => <span key={j} className="tag">🔧 {t.replace("mcp__songsmith__", "")}</span>)}</div>}
                </div>
              ))}
            </div>
          )}
          <div className="row" style={{ gap: 8 }}>
            <input className="grow" value={input} onChange={(e) => setInput(e.target.value)} onKeyDown={(e) => e.key === "Enter" && send()}
              placeholder='e.g. "make it darker / more aggressive" · "shift to F# minor, 90 BPM" · "add lo-fi tape warmth"' />
            <button className="primary" onClick={send} disabled={busy || !input.trim()}>{busy ? "…" : "Send"}</button>
          </div>
          <p className="faint" style={{ marginTop: 6 }}>Claude updates the preset over MCP; the form reloads. Review and Save.</p>
        </div>
      )}
    </div>
  );
}
