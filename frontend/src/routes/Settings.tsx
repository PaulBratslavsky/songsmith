import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery } from "@tanstack/react-query";
import { api, listen, pickFolder } from "../ipc/api";
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

  // ---- Connect Claude account (subscription auth — never an API key) ----
  const auth = useQuery({ queryKey: ["claude-auth"], queryFn: api.claudeAuthStatus });
  const [loginLog, setLoginLog] = useState("");
  const [loginUrl, setLoginUrl] = useState<string | null>(null);
  const [loginStarted, setLoginStarted] = useState(false);
  const [authCode, setAuthCode] = useState("");
  const [testResult, setTestResult] = useState<string | null>(null);
  const logBoxRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    const unLine = listen<{ line?: string; url?: string }>("login_event", (p) => {
      if (p.url) setLoginUrl(p.url);
      if (p.line) setLoginLog((s) => s + p.line + "\n");
    });
    const unDone = listen("login_done", () => {});
    return () => {
      unLine.then((u) => u());
      unDone.then((u) => u());
    };
  }, []);

  useEffect(() => {
    logBoxRef.current?.scrollTo(0, logBoxRef.current.scrollHeight);
  }, [loginLog]);

  const login = useMutation({
    mutationFn: async () => {
      setLoginLog("");
      setLoginUrl(null);
      setAuthCode("");
      setLoginStarted(true);
      return api.claudeLogin();
    },
    onSuccess: (rr) => {
      if (rr.url) setLoginUrl(rr.url);
      if (rr.instructions) setLoginLog((s) => s + rr.instructions + "\n");
    },
    onError: (e: any) => {
      setLoginStarted(false);
      setLoginLog((s) => s + `Error: ${e?.message ?? e}\n`);
    },
  });

  // Paste-back step: write the OAuth code to the held login process's stdin,
  // then auto Re-check status when it completes.
  const submitCode = useMutation({
    mutationFn: () => api.claudeLoginSubmitCode(authCode),
    onSuccess: (rr) => {
      setLoginLog((s) => s + (rr.message ?? "Signed in.") + "\n");
      setLoginStarted(false);
      setAuthCode("");
      setLoginUrl(null);
      auth.refetch();
    },
    onError: (e: any) => setLoginLog((s) => s + `Error: ${e?.message ?? e}\n`),
  });

  const cancelLogin = useMutation({
    mutationFn: api.claudeLoginCancel,
    onSettled: () => {
      setLoginStarted(false);
      setAuthCode("");
      setLoginUrl(null);
    },
  });

  const logout = useMutation({
    mutationFn: api.claudeLogout,
    onSuccess: () => {
      setTestResult(null);
      auth.refetch();
    },
    onError: (e: any) => setTestResult(`❌ ${e?.message ?? e}`),
  });

  const test = useMutation({
    mutationFn: api.testClaude,
    onMutate: () => setTestResult(null),
    onSuccess: (rr) => setTestResult(rr),
    onError: (e: any) => setTestResult(`❌ ${e?.message ?? e}`),
  });

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
        <div>
        <div className="card">
          <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
            <h2 style={{ margin: 0 }}>Connect Claude account</h2>
            {auth.data && (
              <span className={`badge ${auth.data.api_key_set ? "error" : auth.data.logged_in ? "done" : "pending"}`}>
                {auth.data.api_key_set
                  ? "⚠ API key set — connectors off"
                  : auth.data.logged_in
                  ? `● connected${auth.data.account ? ` as ${auth.data.account}` : ""}${auth.data.subscription ? ` (${auth.data.subscription})` : ""}`
                  : "⚠ not signed in"}
              </span>
            )}
          </div>

          <p className="faint" style={{ marginTop: 8 }}>
            Sign in with your <b>claude.ai</b> account so Songsmith drives Claude through your subscription —
            never an API key. Keep <code>ANTHROPIC_API_KEY</code> unset: the app strips it from Claude anyway,
            and that strip is exactly what keeps your connectors loaded.
          </p>

          {auth.data?.connectors_hint && (
            <div className={`banner ${auth.data.api_key_set ? "warn" : auth.data.logged_in ? "ok" : "warn"}`}>
              {auth.data.connectors_hint}
            </div>
          )}

          <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
            <button className="primary" disabled={login.isPending || loginStarted} onClick={() => login.mutate()}>
              {login.isPending ? "Starting…" : loginStarted ? "Waiting for code…" : "Log in"}
            </button>
            {loginStarted && (
              <button disabled={cancelLogin.isPending} onClick={() => cancelLogin.mutate()}>
                Cancel
              </button>
            )}
            {auth.data?.logged_in && !loginStarted && (
              <button
                className="ghost"
                disabled={logout.isPending}
                onClick={() => { if (window.confirm("Log out of your claude.ai account?")) logout.mutate(); }}
              >
                {logout.isPending ? "Logging out…" : "Log out"}
              </button>
            )}
            <button disabled={auth.isFetching} onClick={() => auth.refetch()}>
              {auth.isFetching ? "Checking…" : "Re-check"}
            </button>
            <button onClick={() => api.openUrl(auth.data?.connectors_url ?? "https://claude.ai/settings/connectors")}>
              Connect tools ↗
            </button>
            <button disabled={test.isPending} onClick={() => test.mutate()}>
              {test.isPending ? "Testing…" : "Test"}
            </button>
          </div>

          {loginUrl && (
            <div style={{ marginTop: 10 }}>
              <label>Auth URL — open this if the browser didn't, then click Re-check</label>
              <div className="row" style={{ gap: 8, alignItems: "center" }}>
                <a href={loginUrl} target="_blank" rel="noreferrer" className="faint" style={{ wordBreak: "break-all", flex: 1 }}>
                  {loginUrl}
                </a>
                <button onClick={() => api.openUrl(loginUrl)}>Open</button>
              </div>
            </div>
          )}

          {loginStarted && (
            <div style={{ marginTop: 10 }}>
              <label>Paste authentication code</label>
              <p className="faint" style={{ marginTop: 4 }}>
                Finish signing in in the browser, copy the <b>authentication code</b> claude.ai shows,
                then paste it here.
              </p>
              <div className="row" style={{ gap: 8, alignItems: "center" }}>
                <input
                  value={authCode}
                  onChange={(e) => setAuthCode(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && authCode.trim() && !submitCode.isPending) submitCode.mutate();
                  }}
                  placeholder="Authentication code from claude.ai"
                  style={{ flex: 1 }}
                  autoFocus
                />
                <button className="primary" disabled={!authCode.trim() || submitCode.isPending} onClick={() => submitCode.mutate()}>
                  {submitCode.isPending ? "Submitting…" : "Submit"}
                </button>
              </div>
            </div>
          )}

          {loginLog && (
            <pre ref={logBoxRef} className="login-log">
              {loginLog}
            </pre>
          )}

          {testResult && (
            <div className={`banner ${testResult.startsWith("✅") ? "ok" : "err"}`} style={{ marginTop: 10 }}>
              {testResult}
            </div>
          )}
        </div>

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

          <DoctorCard />

          <div className="card">
            <h2>Music.AI lyrics <span className="badge pending">add-on</span></h2>
            <p className="muted">
              Optional cloud fallback for lyric transcription. Some AI renders (heavily-processed synth vocals) defeat the local
              whisper pipeline — with a <a href="https://music.ai" target="_blank" rel="noreferrer">music.ai</a> API key set, the
              ⟳ Resume import retry offers their transcription instead. <b>This uploads that song's audio to Music.AI</b> — the only
              path where audio leaves your machine, and only when you trigger it. Leave empty to stay fully local.
            </p>
            <label>API key</label>
            <input type="password" value={form.musicai_api_key} onChange={set("musicai_api_key")} spellCheck={false}
              placeholder="paste your Music.AI API key" style={{ width: "100%", fontFamily: "var(--mono)", fontSize: 12 }} />
            <label style={{ marginTop: 8 }}>Workflow slug</label>
            <input value={form.musicai_workflow} onChange={set("musicai_workflow")} spellCheck={false}
              placeholder="e.g. lyric-transcription (create it in the Music.AI console — audio in, lyric transcription JSON out)"
              style={{ width: "100%", fontFamily: "var(--mono)", fontSize: 12 }} />
            <div className="row" style={{ marginTop: 8 }}>
              <button className="primary" onClick={() => save.mutate()}>Save</button>
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

/** 🩺 Setup doctor: one-click environment checks (Claude CLI, analyzer
 *  deps, Ableton Remote Script per Live install, music folder) + the
 *  bundled-script installer, so a fresh machine sets itself up from
 *  inside the app instead of hand-patching. */
function DoctorCard() {
  const [checks, setChecks] = useState<{ name: string; status: "ok" | "warn" | "fail"; detail: string }[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const run = async () => {
    setBusy(true);
    setMsg("Checking (the analyzer import test can take a minute)…");
    try {
      setChecks(await api.runDoctor());
      setMsg("");
    } catch (e: any) {
      setMsg(String(e?.message ?? e));
    }
    setBusy(false);
  };
  const install = async () => {
    setBusy(true);
    try {
      setMsg(await api.installAbletonScript());
      setChecks(await api.runDoctor());
    } catch (e: any) {
      setMsg(String(e?.message ?? e));
    }
    setBusy(false);
  };
  const icon = (s: string) => (s === "ok" ? "✅" : s === "warn" ? "⚠️" : "❌");
  const needsScript = checks?.some((c) => c.name.startsWith("Ableton Remote Script") && c.status !== "ok");
  const counts = checks
    ? {
        total: checks.length,
        ok: checks.filter((c) => c.status === "ok").length,
        warn: checks.filter((c) => c.status === "warn").length,
        fail: checks.filter((c) => c.status === "fail").length,
      }
    : null;
  const summary = counts
    ? counts.fail > 0
      ? `${counts.fail} check${counts.fail === 1 ? "" : "s"} failing — fix the ❌ rows below.`
      : counts.warn > 0
        ? `Working, with ${counts.warn} thing${counts.warn === 1 ? "" : "s"} worth a look.`
        : "Everything Songsmith needs is in place."
    : null;
  return (
    <div className="card">
      <h2>🩺 Setup doctor</h2>
      <p className="muted">Checks everything Songsmith needs on this machine: the Claude CLI login, the local analyzer's Python deps, the Ableton Remote Script (per Live install), and the music folder.</p>
      {counts && (
        <div className="row" style={{ gap: 6, margin: "8px 0", flexWrap: "wrap" }}>
          <span className="badge"><b>{counts.total}</b>&nbsp;Checks</span>
          <span className="badge done"><b>{counts.ok}</b>&nbsp;OK</span>
          <span className="badge in_progress"><b>{counts.warn}</b>&nbsp;Warnings</span>
          <span className="badge error"><b>{counts.fail}</b>&nbsp;Failing</span>
        </div>
      )}
      {summary && <p className="faint" style={{ margin: "4px 0 8px" }}>{summary}</p>}
      <div className="row" style={{ gap: 8 }}>
        <button className="primary" disabled={busy} onClick={() => void run()}>{busy ? "checking…" : checks ? "Re-run checks" : "Run checks"}</button>
        {needsScript && (
          <button disabled={busy} onClick={() => void install()} title="write the bundled (patched) AbletonMCP script into every Live install — restart Live afterward">
            🎛 Install Ableton script
          </button>
        )}
      </div>
      {msg && <p className="faint" style={{ marginTop: 8 }}>{msg}</p>}
      {checks && (
        <div className="col" style={{ gap: 4, marginTop: 10 }}>
          {checks.map((c) => (
            <div key={c.name} className="row" style={{ gap: 8, alignItems: "baseline", opacity: c.status === "ok" ? 0.65 : 1 }}>
              <span>{icon(c.status)}</span>
              <b style={{ fontSize: 12, whiteSpace: "nowrap" }}>{c.name}</b>
              <span className="faint" style={{ fontSize: 11 }}>{c.detail}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
