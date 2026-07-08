import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, listen, inTauri, STAGE_LABELS } from "../ipc/api";

type Msg = { role: "user" | "assistant"; text: string; tools: string[] };

/**
 * Chat with Claude (your Claude Code subscription) running headless with this
 * app's MCP server attached — so Claude can read and update the songwriting
 * workflow (style presets, songs, stages, artifacts) as you talk. This is the
 * harness: Claude is the engine, the app is the cockpit.
 */
export function ChatPanel({ songId }: { songId?: string }) {
  const [messages, setMessages] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sessionRef = useRef<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const qc = useQueryClient();
  // did Claude call any tool this turn? if so, its writes (presets, songs,
  // stages, renders, settings…) won't show in the UI until we refetch.
  const usedToolRef = useRef(false);
  const song = useQuery({ queryKey: ["song", songId], queryFn: () => api.getSong(songId!), enabled: !!songId });
  const v = song.data?.song;

  // mutate the last (assistant) message
  const patchLast = (fn: (m: Msg) => Msg) =>
    setMessages((ms) => ms.map((m, i) => (i === ms.length - 1 ? fn(m) : m)));

  useEffect(() => {
    if (!inTauri) return;
    let unEvent = () => {};
    let unDone = () => {};
    let unErr = () => {};
    (async () => {
      unEvent = await listen<{ event: any }>("chat_event", ({ event }) => {
        const t = event?.type;
        if (t === "stream_event") {
          const d = event.event?.delta;
          if (event.event?.type === "content_block_delta" && d?.type === "text_delta") {
            patchLast((m) => ({ ...m, text: m.text + (d.text ?? "") }));
          }
        } else if (t === "assistant") {
          const blocks = event.message?.content ?? [];
          const tools = blocks.filter((b: any) => b.type === "tool_use").map((b: any) => b.name);
          if (tools.length) {
            usedToolRef.current = true;
            patchLast((m) => ({ ...m, tools: [...m.tools, ...tools] }));
          }
        } else if (t === "result") {
          patchLast((m) => (m.text.trim() ? m : { ...m, text: event.result ?? "" }));
        }
      });
      unDone = await listen<{}>("chat_done", () => {
        setSending(false);
        // Claude may have created/edited data through the MCP server — refetch
        // so new presets, songs, stages, renders, etc. show up immediately.
        if (usedToolRef.current) qc.invalidateQueries();
        usedToolRef.current = false;
      });
      unErr = await listen<{ error: string }>("chat_error", (p) => {
        setError(p.error);
        setSending(false);
      });
    })();
    return () => {
      unEvent();
      unDone();
      unErr();
    };
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo(0, scrollRef.current.scrollHeight);
  }, [messages]);

  const sendText = async (text: string) => {
    if (!text || sending) return;
    setError(null);
    setMessages((ms) => [...ms, { role: "user", text, tools: [] }, { role: "assistant", text: "", tools: [] }]);
    setSending(true);
    try {
      const sid = await api.chatSend(text, sessionRef.current ?? undefined, songId);
      sessionRef.current = sid;
    } catch (e: any) {
      setError(String(e?.message ?? e));
      setSending(false);
    }
  };
  const send = () => { const t = input.trim(); if (!t) return; setInput(""); sendText(t); };

  const ABLETON_CMD = "Structure this song in Ableton using the connected ableton MCP. Note: this server (ableton_mcp 1.2.0) has NO locator tool, so use named arrangement clips instead. Steps: (1) set_tempo to the song's BPM; (2) switch_to_arrangement_view; (3) create_midi_track if needed; (4) for EACH section in order, create_clip, set_clip_name to the section name, and duplicate_to_arrangement at the running bar offset computed from the section bar counts — so the timeline reads Intro, Verse 1, Pre-Chorus, … as named clips. First list the mcp__ableton tools to confirm names. Report exactly what you created. If the ableton tools aren't reachable, say so plainly — don't pretend.";

  // direct, deterministic Ableton builds — bypass the MCP/LLM, talk to the socket
  const buildAbleton = async (kind: "locators" | "clips") => {
    if (sending || !songId) return;
    setMessages((ms) => [...ms, { role: "user", text: `⚡ Build ${kind} in Ableton (direct)`, tools: [] }, { role: "assistant", text: `Building ${kind} via the Remote Script…`, tools: [] }]);
    try {
      const out = kind === "locators" ? await api.abletonBuild(songId) : await api.abletonBuildClips(songId);
      patchLast((m) => ({ ...m, text: out }));
    } catch (e: any) {
      patchLast((m) => ({ ...m, text: "Error: " + String(e?.message ?? e) }));
    }
  };

  if (!inTauri) {
    return (
      <div className="banner warn">
        The AI chat runs only in the desktop app (it spawns the <code>claude</code> CLI with the
        harness MCP server). Launch with <code>npx tauri dev</code>.
      </div>
    );
  }

  return (
    <div className="chat">
      {v && (
        <div className="chat-context" title="Claude can see this song, its stage, and its section map">
          <span className="faint">context ▸</span> <b>{v.title || "Untitled"}</b> · {STAGE_LABELS[v.current_stage] ?? v.current_stage} · {v.key_root} {v.key_mode} · {String(v.bpm)} BPM
        </div>
      )}
      {v && (
        <div className="row" style={{ gap: 6, flexWrap: "wrap", marginBottom: 6 }}>
          <button className="sm primary" disabled={sending} title="DIRECT: real Arrangement locators per section (no LLM)" onClick={() => buildAbleton("locators")}>⚡ Locators (direct)</button>
          <button className="sm primary" disabled={sending} title="DIRECT: named, color-coded clips per section on a Sections track" onClick={() => buildAbleton("clips")}>⊞ Clips (direct)</button>
        </div>
      )}
      <div className="chat-log" ref={scrollRef}>
        {messages.length === 0 && (
          <div className="empty">
            Ask Claude to drive your songwriting — e.g. “list my songs”, “run the Concept stage for
            Night Drive”, or “rewrite the chorus with a darker image and save it”.
          </div>
        )}
        {messages.map((m, i) => (
          <div key={i} className={"bubble " + m.role}>
            <div className="who">{m.role === "user" ? "you" : "claude"}</div>
            {m.text ? (
              <div className="bubble-text">{m.text}</div>
            ) : (
              m.role === "assistant" && sending && <span className="faint spin">▮ thinking…</span>
            )}
            {m.tools.length > 0 && (
              <div className="tools-row">
                {m.tools.map((t, j) => (
                  <span key={j} className="tag">🔧 {t.replace("mcp__songsmith__", "")}</span>
                ))}
              </div>
            )}
          </div>
        ))}
      </div>

      {error && <div className="banner err">{error}</div>}

      <div className="chat-input">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) send();
          }}
          placeholder="Message Claude…  (⌘/Ctrl+Enter to send)"
          rows={2}
        />
        <button className="primary" onClick={send} disabled={sending || !input.trim()}>
          {sending ? "…" : "Send"}
        </button>
      </div>
    </div>
  );
}
