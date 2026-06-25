import { useEffect, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { api, pickFolder } from "../ipc/api";
import type { Settings } from "../ipc/generated";

export function SettingsPage() {
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const tools = useQuery({ queryKey: ["tools"], queryFn: api.listTools });
  const mcp = useQuery({ queryKey: ["mcp"], queryFn: api.mcpConfig });
  const status = useQuery({ queryKey: ["claude"], queryFn: api.claudeStatus });
  const [form, setForm] = useState<Settings | null>(null);
  const [saved, setSaved] = useState("");
  const [detectMsg, setDetectMsg] = useState("");
  const [abletonMsg, setAbletonMsg] = useState("");

  // pin a uv-managed Python so cryptography uses a prebuilt arm64 wheel (the
  // default x86_64 framework Python forces a source build that fails on Apple Silicon)
  const fillAbleton = () => setForm((f) => (f ? { ...f, ableton_mcp: JSON.stringify({ command: "uvx", args: ["--python", "3.12", "ableton-mcp"] }, null, 2) } : f));
  const testAbleton = async () => { setAbletonMsg("Testing 127.0.0.1:9877…"); setAbletonMsg(await api.testAbleton()); };
  const resetAbleton = async () => { setAbletonMsg("Freeing connection…"); setAbletonMsg(await api.resetAbleton()); };

  const chooseMusicFolder = async () => { const f = await pickFolder(); if (f) { setForm((c) => (c ? { ...c, music_folder: f } : c)); setSaved(""); } };

  const detectAbleton = async () => {
    const r = await api.detectAbletonMcp();
    if (r.found && r.entry) {
      setForm((f) => (f ? { ...f, ableton_mcp: JSON.stringify(r.entry, null, 2) } : f));
      setDetectMsg(`Found "${r.name}" in Claude Desktop — review and Save.`);
    } else {
      setDetectMsg("No Ableton server found in Claude Desktop config — paste its { command, args } below.");
    }
  };

  useEffect(() => { if (settings.data && !form) setForm(settings.data); }, [settings.data]);
  const save = useMutation({ mutationFn: () => api.setSettings(form!), onSuccess: () => setSaved("Saved.") });

  if (!form) return <div className="empty">Loading…</div>;
  const set = (k: keyof Settings) => (e: any) => setForm({ ...form, [k]: e.target.value });

  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Settings</h1>
          <span className="muted">The engine is Claude — your Claude Code subscription, via MCP.</span>
        </div>
      </div>

      <div className="grid2" style={{ alignItems: "start" }}>
        <div className="card">
          <h2>Claude engine</h2>
          <p className="muted">A harness around Claude for songwriters. No local model — Claude works through the <code>claude</code> CLI and this app's MCP server.</p>
          <div className="kv" style={{ margin: "10px 0" }}>
            <span className="k">CLI</span>
            <span>
              {status.data?.found ? <span className="badge done">found</span> : <span className="badge" style={{ color: "var(--danger)" }}>not found</span>}{" "}
              {status.data?.version && <span className="faint">{status.data.version}</span>}
            </span>
            <span className="k">Path</span>
            <span className="faint">{status.data?.bin || "claude (on PATH)"}</span>
          </div>
          {!status.data?.found && (
            <div className="banner err">The <code>claude</code> CLI wasn't found. Install Claude Code and run <code>claude</code> once to sign in.</div>
          )}
          <label>Model override (optional)</label>
          <input value={form.claude_model} onChange={set("claude_model")} placeholder="empty = default · or opus / sonnet" style={{ width: "100%" }} />
          <div className="row" style={{ marginTop: 12, gap: 8 }}>
            <button className="primary" onClick={() => save.mutate()}>Save</button>
            {saved && <span className="faint">{saved}</span>}
          </div>
        </div>

        <div>
          <div className="card">
            <h2>MCP connection</h2>
            <p className="muted">The Chat tab wires this automatically. To drive the harness from your own terminal:</p>
            <label>claude mcp add</label>
            <div className="artifact-text" style={{ maxHeight: "none" }}>{mcp.data?.command_hint}</div>
            <label>App database</label>
            <div className="artifact-text" style={{ maxHeight: "none" }}>{mcp.data?.db_path}</div>
          </div>
          <div className="card">
            <h2>Ableton MCP</h2>
            <p className="muted">
              Connect your Ableton MCP server so the in-app Claude can build the song in Ableton.
              Once connected, ask in any chat: <i>"set up this song's structure in Ableton."</i>
            </p>
            <div className="row" style={{ gap: 8, marginBottom: 6 }}>
              <span className="k">Status</span>
              {form.ableton_mcp.trim() ? <span className="badge done">connected</span> : <span className="badge pending">not connected</span>}
            </div>
            <div className="row" style={{ gap: 8, marginBottom: 6, flexWrap: "wrap" }}>
              <button onClick={fillAbleton}>Use uvx ableton-mcp</button>
              <button onClick={detectAbleton}>Detect from Claude Desktop</button>
              {form.ableton_mcp.trim() && <button className="ghost" onClick={() => setForm({ ...form, ableton_mcp: "" })}>disconnect</button>}
            </div>
            <div className="row" style={{ gap: 8, marginBottom: 6, flexWrap: "wrap" }}>
              <button onClick={testAbleton} title="probe Ableton's Remote Script on port 9877 directly">Test connection</button>
              <button className="ghost" onClick={resetAbleton} title="stop stray ableton-mcp processes holding the socket">Free connection (reset)</button>
            </div>
            {abletonMsg && <pre className="artifact-text" style={{ whiteSpace: "pre-wrap", maxHeight: 180, marginBottom: 6 }}>{abletonMsg}</pre>}
            {detectMsg && <div className="faint" style={{ marginBottom: 6 }}>{detectMsg}</div>}
            <label>Server config (JSON)</label>
            <textarea value={form.ableton_mcp} onChange={set("ableton_mcp")} style={{ minHeight: 80, fontSize: 12 }}
              placeholder={'{ "command": "uvx", "args": ["ableton-mcp"] }'} />
            <div className="row" style={{ marginTop: 8 }}>
              <button className="primary" onClick={() => save.mutate()}>Save</button>
            </div>
          </div>

          <div className="card">
            <h2>Music folder</h2>
            <p className="muted">Where all your song renders live. The Final renders <b>+ Add version</b> button opens this folder so you drop the song here — all music in one place.</p>
            <div className="row" style={{ gap: 8, marginBottom: 6 }}>
              <button onClick={chooseMusicFolder}>Choose folder…</button>
              {form.music_folder.trim() && <button className="ghost" onClick={() => setForm({ ...form, music_folder: "" })}>clear</button>}
            </div>
            <div className="artifact-text" style={{ maxHeight: "none" }}>{form.music_folder || "— no folder set —"}</div>
            <div className="row" style={{ marginTop: 8 }}>
              <button className="primary" onClick={() => save.mutate()}>Save</button>
            </div>
          </div>

          <div className="card">
            <h2>Reference analyzer <span className="badge pending">spike</span></h2>
            <p className="muted">Local command that analyzes an imported reference track (tempo, key, chords, sections). The audio path is appended automatically — your audio never leaves the machine. Empty = reference import off.</p>
            <textarea value={form.analyzer_cmd} onChange={set("analyzer_cmd")} spellCheck={false}
              placeholder="/path/to/analysis/.venv/bin/python /path/to/analysis/analyze.py"
              style={{ width: "100%", minHeight: 50, fontFamily: "var(--mono)", fontSize: 12 }} />
            <div className="row" style={{ gap: 8, marginTop: 8 }}>
              <button className="primary" onClick={() => save.mutate()}>Save</button>
              <button className="ghost" onClick={() => setForm({ ...form, analyzer_cmd: "/Users/paul/programing/songsmith-studio/analysis/.venv/bin/python /Users/paul/programing/songsmith-studio/analysis/analyze.py" })}>use dev default</button>
            </div>
          </div>

          <div className="card">
            <h2>Tool registry</h2>
            <p className="muted">{tools.data?.length ?? 0} tools — one registry for the UI, the agent, and Claude over MCP.</p>
            <div style={{ maxHeight: 220, overflow: "auto" }}>
              {tools.data?.map((t) => (
                <div key={t.name} className="row" style={{ justifyContent: "space-between", padding: "3px 0" }}>
                  <code>{t.name}</code>
                  {t.destructive && <span className="badge" style={{ color: "var(--danger)" }}>destructive</span>}
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
