import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, pickAudioFile, STAGE_ORDER, STAGE_LABELS, type ParsedLyrics } from "../ipc/api";
import type { Song } from "../ipc/generated";

function StageTrack({ song }: { song: Song }) {
  const idx = STAGE_ORDER.indexOf(song.current_stage as any);
  const complete = song.status === "done";
  return (
    <div className="stage-track" title={STAGE_LABELS[song.current_stage] ?? song.current_stage}>
      {STAGE_ORDER.map((s, i) => (
        <div key={s} className={"seg " + (complete || i < idx ? "done" : i === idx ? "in_progress" : "")} />
      ))}
    </div>
  );
}

function NewSongButton() {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [presetId, setPresetId] = useState("");
  const nav = useNavigate();
  const qc = useQueryClient();
  const presets = useQuery({ queryKey: ["presets"], queryFn: api.listStylePresets });

  const create = useMutation({
    mutationFn: () => api.createSong(presetId || presets.data![0].id, title || "Untitled song"),
    onSuccess: (s) => {
      qc.invalidateQueries({ queryKey: ["songs"] });
      setOpen(false);
      setTitle("");
      nav({ to: "/song/$id", params: { id: s.id } });
    },
  });
  const noPresets = !presets.data || presets.data.length === 0;

  return (
    <>
      <button className="primary" onClick={() => setOpen(true)} disabled={noPresets}>+ New song</button>
      {open && (
        <div className="modal-bg" onClick={() => setOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Start a new song</h2>
            <p className="muted">Seeds the full stage spec with the chosen style preset.</p>
            <label>Style preset</label>
            <select value={presetId || presets.data?.[0]?.id || ""} onChange={(e) => setPresetId(e.target.value)} style={{ width: "100%" }}>
              {presets.data?.map((p) => (<option key={p.id} value={p.id}>{p.name}</option>))}
            </select>
            <label>Working title (optional)</label>
            <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Taillights" style={{ width: "100%" }} />
            <div className="row" style={{ marginTop: 16, justifyContent: "flex-end" }}>
              <button className="ghost" onClick={() => setOpen(false)}>Cancel</button>
              <button className="primary" onClick={() => create.mutate()} disabled={create.isPending}>
                {create.isPending ? "Creating…" : "Create"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

/** "New from lyrics": paste finished lyrics → a new song with Structure +
 *  Lyrics populated from the paste (words kept verbatim). Same preset/title
 *  inputs as the normal create flow; Concept stays blank for the user. */
function NewFromLyricsButton() {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [presetId, setPresetId] = useState("");
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<ParsedLyrics | null>(null);
  const nav = useNavigate();
  const qc = useQueryClient();
  const presets = useQuery({ queryKey: ["presets"], queryFn: api.listStylePresets });

  // live parsed preview (dry-run, debounced)
  useEffect(() => {
    if (!open || !text.trim()) { setPreview(null); return; }
    const t = setTimeout(() => { api.parsePastedLyrics(text).then(setPreview).catch(() => setPreview(null)); }, 400);
    return () => clearTimeout(t);
  }, [text, open]);

  const create = useMutation({
    mutationFn: () => api.createSongFromLyrics(presetId || presets.data![0].id, title || "Untitled song", text),
    onSuccess: (s) => {
      qc.invalidateQueries({ queryKey: ["songs"] });
      setOpen(false); setTitle(""); setText(""); setPreview(null);
      nav({ to: "/song/$id", params: { id: s.id } });
    },
  });
  const noPresets = !presets.data || presets.data.length === 0;

  return (
    <>
      <button onClick={() => setOpen(true)} disabled={noPresets}
        title="Paste finished lyrics → new song with Structure + Lyrics populated (words kept verbatim)">
        📋 New from lyrics
      </button>
      {open && (
        <div className="modal-bg" onClick={() => setOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 720, maxWidth: "92vw" }}>
            <h2>New song from pasted lyrics</h2>
            <p className="muted">
              Your words are kept <b>verbatim</b> — they're only parsed into sections. Structure + Lyrics land
              filled in; add the Concept and run Chords next.
            </p>
            <div className="row" style={{ gap: 10, flexWrap: "wrap" }}>
              <div style={{ flex: 1, minWidth: 200 }}>
                <label>Style preset</label>
                <select value={presetId || presets.data?.[0]?.id || ""} onChange={(e) => setPresetId(e.target.value)} style={{ width: "100%" }}>
                  {presets.data?.map((p) => (<option key={p.id} value={p.id}>{p.name}</option>))}
                </select>
              </div>
              <div style={{ flex: 1, minWidth: 200 }}>
                <label>Working title (optional)</label>
                <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Taillights" style={{ width: "100%" }} />
              </div>
            </div>
            <label style={{ marginTop: 8, display: "block" }}>Lyrics</label>
            <div className="row" style={{ gap: 12, alignItems: "stretch" }}>
              <textarea
                value={text} onChange={(e) => setText(e.target.value)} autoFocus
                placeholder={"[Verse 1]\nCity lights are calling me home…\n\n[Chorus]\n…"}
                style={{ flex: 1, minHeight: 220, fontFamily: "var(--mono)", fontSize: 12 }}
              />
              <div style={{ flex: 1, minHeight: 220, maxHeight: 320, overflowY: "auto", border: "1px solid var(--line)", borderRadius: 2, padding: 8 }}>
                {!preview ? (
                  <span className="faint">The parsed sections appear here.</span>
                ) : (
                  <>
                    <div className="row" style={{ gap: 6, marginBottom: 6, flexWrap: "wrap" }}>
                      <span className="badge">{preview.sections.length} section{preview.sections.length === 1 ? "" : "s"}</span>
                      {preview.used_claude && <span className="badge" title="no headers found — Claude marked the boundaries; every line was validated verbatim">🤖 Claude segmented — words verbatim</span>}
                    </div>
                    {preview.sections.map((s, i) => (
                      <div key={i} style={{ marginBottom: 8 }}>
                        <b style={{ fontSize: 12 }}>[{s.label}]</b>
                        <div style={{ fontFamily: "var(--mono)", fontSize: 11, whiteSpace: "pre-wrap" }}>{s.lines.join("\n")}</div>
                      </div>
                    ))}
                  </>
                )}
              </div>
            </div>
            {create.isError && <div className="banner warn" style={{ marginTop: 10 }}>Create failed: {String((create.error as any)?.message ?? create.error)}</div>}
            <div className="row" style={{ marginTop: 16, justifyContent: "flex-end", gap: 8 }}>
              <button className="ghost" onClick={() => setOpen(false)}>Cancel</button>
              <button className="primary" onClick={() => create.mutate()} disabled={!text.trim() || create.isPending}>
                {create.isPending ? "Creating…" : "Create from lyrics"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export function Library() {
  const nav = useNavigate();
  const qc = useQueryClient();
  const songs = useQuery({ queryKey: ["songs"], queryFn: api.listSongs });
  const presets = useQuery({ queryKey: ["presets"], queryFn: api.listStylePresets });
  const [importMsg, setImportMsg] = useState("");

  const importRef = useMutation({
    mutationFn: async () => {
      const path = await pickAudioFile();
      if (!path) return null;
      setImportMsg(`Analyzing ${path.split("/").pop()} — tempo, key, chords, sections… (local, ~10–30s)`);
      return api.importReference(path);
    },
    onSuccess: (id) => {
      setImportMsg("");
      if (id) { qc.invalidateQueries({ queryKey: ["songs"] }); nav({ to: "/song/$id", params: { id } }); }
    },
    onError: (e: any) => setImportMsg("Import failed: " + String(e?.message ?? e)),
  });

  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Library</h1>
          <span className="muted">Every song mock and the stage it's on.</span>
        </div>
        <div className="row" style={{ gap: 8 }}>
          <button onClick={() => importRef.mutate()} disabled={importRef.isPending}
            title="Import an audio file → analyze locally → new song with Structure + Chords filled in">
            {importRef.isPending ? "Analyzing…" : "⤵ Import reference"}
          </button>
          <NewFromLyricsButton />
          <NewSongButton />
        </div>
      </div>
      {importMsg && <div className="banner" style={{ marginBottom: 12 }}>{importMsg}</div>}

      {presets.data && presets.data.length === 0 && (
        <div className="banner warn">No style presets yet. Create one in <b>Style presets</b> before starting a song.</div>
      )}
      {songs.isLoading && <div className="empty">Loading…</div>}
      {songs.data && songs.data.length === 0 && (
        <div className="empty">No songs yet. Hit “New song” to co-write one with Claude.</div>
      )}

      {songs.data?.map((v) => (
        <div key={v.id} className="list-item" style={{ cursor: "pointer" }} onClick={() => nav({ to: "/song/$id", params: { id: v.id } })}>
          <div className="col" style={{ gap: 6 }}>
            <div className="row" style={{ gap: 10 }}>
              <b>{v.title || "Untitled song"}</b>
              <span className={"badge " + v.status}>{v.status.replace("_", " ")}</span>
              <span className="faint">{v.key_root} {v.key_mode} · {String(v.bpm)} BPM</span>
            </div>
            <div className="row" style={{ gap: 10 }}>
              <StageTrack song={v} />
              <span className="faint">{STAGE_LABELS[v.current_stage] ?? v.current_stage}</span>
            </div>
          </div>
          <span className="faint">open →</span>
        </div>
      ))}
    </div>
  );
}
