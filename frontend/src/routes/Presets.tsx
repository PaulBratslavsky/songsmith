import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, listen } from "../ipc/api";
import type { StylePreset, StyleInput } from "../ipc/generated";
import { PresetChat } from "../components/PresetChat";

const EMPTY: StyleInput = {
  name: "", genre: "", mood: "", influences: "", key_tempo_feel: "", vocal_range: "", themes: "", lyric_exemplars: "",
};

function PresetForm({ initial, editingId, onDone }: { initial: StyleInput; editingId: string | null; onDone: () => void }) {
  const [form, setForm] = useState<StyleInput>(initial);
  const [generating, setGenerating] = useState(false);
  const [genErr, setGenErr] = useState<string | null>(null);
  const [stream, setStream] = useState("");
  const streamRef = useRef("");
  const qc = useQueryClient();
  useEffect(() => setForm(initial), [editingId]);

  useEffect(() => {
    let un = () => {};
    (async () => {
      un = await listen<{ token: string }>("preset_token", (p) => {
        streamRef.current += p.token;
        setStream(streamRef.current.slice(-500));
      });
    })();
    return () => un();
  }, []);

  const autofill = async () => {
    if (!form.name.trim()) { setGenErr("Enter a name/seed first."); return; }
    setGenerating(true); setGenErr(null); setStream(""); streamRef.current = "";
    try {
      const g = await api.generateStylePreset(form.name.trim(), form.genre || undefined);
      setForm((f) => ({ ...f, genre: g.genre || f.genre, mood: g.mood || f.mood, influences: g.influences || f.influences,
        key_tempo_feel: g.key_tempo_feel || f.key_tempo_feel, vocal_range: g.vocal_range || f.vocal_range, themes: g.themes || f.themes }));
    } catch (e: any) { setGenErr(String(e?.message ?? e)); }
    setGenerating(false);
  };

  const save = useMutation({
    mutationFn: () => (editingId ? api.updateStylePreset(editingId, form) : api.createStylePreset(form)),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["presets"] }); onDone(); },
  });
  const set = (k: keyof StyleInput) => (e: any) => setForm({ ...form, [k]: e.target.value });

  return (
    <div className="card">
      <h2>{editingId ? "Edit style preset" : "New style preset"}</h2>
      <p className="muted">Persistent context Claude reads on every stage of every song.</p>
      <label>Name</label>
      <input value={form.name} onChange={set("name")} placeholder="e.g. Night Drive" style={{ width: "100%" }} />
      <div className="row" style={{ marginTop: 8, gap: 8 }}>
        <button onClick={autofill} disabled={generating || !form.name.trim()}>
          {generating ? (<><span className="spin">▮</span> Generating…</>) : "✨ Auto-fill with AI"}
        </button>
        <span className="faint">Drafts the preset from the name using the style skill.</span>
      </div>
      {genErr && <div className="banner err" style={{ marginTop: 8 }}>{genErr}</div>}
      {generating && stream && <div className="stream" style={{ marginTop: 8, maxHeight: "16vh" }}>{stream}</div>}

      <label>Genre / sub-genre</label>
      <textarea value={form.genre} onChange={set("genre")} placeholder="e.g. synthwave / darksynth" />
      <label>Mood & energy</label>
      <textarea value={form.mood} onChange={set("mood")} placeholder="moody, propulsive, cinematic" />
      <label>Influences (describe the sound)</label>
      <textarea value={form.influences} onChange={set("influences")} placeholder="80s film scores, neon-noir" />
      <label>Key / tempo feel</label>
      <textarea value={form.key_tempo_feel} onChange={set("key_tempo_feel")} placeholder="A minor, ~120 BPM, four-on-the-floor" />
      <label>Vocal range</label>
      <textarea value={form.vocal_range} onChange={set("vocal_range")} placeholder="mid baritone" />
      <label>Recurring themes</label>
      <textarea value={form.themes} onChange={set("themes")} placeholder="motion, loneliness, the open road" />
      <label>Lyric exemplars</label>
      <textarea value={form.lyric_exemplars} onChange={set("lyric_exemplars")} rows={4}
        placeholder={"one line per lyric, e.g.\nI left the porch light on again\nNobody's coming home"} />
      <p className="faint" style={{ marginTop: 2 }}>a few lines that sound like what you want — calibrates the Lyricist's voice; never copied</p>
      <div className="row" style={{ marginTop: 14, justifyContent: "flex-end" }}>
        {editingId && <button className="ghost" onClick={onDone}>Cancel</button>}
        <button className="primary" disabled={!form.name || save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? "Saving…" : editingId ? "Save changes" : "Create preset"}
        </button>
      </div>
      {editingId && <PresetChat presetId={editingId} current={form} onApplied={(p) => setForm({ name: p.name, genre: p.genre, mood: p.mood, influences: p.influences, key_tempo_feel: p.key_tempo_feel, vocal_range: p.vocal_range, themes: p.themes, lyric_exemplars: p.lyric_exemplars })} />}
    </div>
  );
}

export function Presets() {
  const [editing, setEditing] = useState<StylePreset | null>(null);
  const [creating, setCreating] = useState(false);
  const presets = useQuery({ queryKey: ["presets"], queryFn: api.listStylePresets });

  return (
    <div>
      <div className="topbar">
        <div>
          <h1>Style presets</h1>
          <span className="muted">Reusable artist/style context — set up once, revised occasionally.</span>
        </div>
        {!creating && !editing && <button className="primary" onClick={() => setCreating(true)}>+ New preset</button>}
      </div>

      <div className="grid2" style={{ alignItems: "start" }}>
        <div>
          {presets.data?.length === 0 && !creating && <div className="empty">No presets yet.</div>}
          {presets.data?.map((p) => (
            <div key={p.id} className="list-item">
              <div className="col" style={{ gap: 4 }}>
                <b>{p.name}</b>
                <span className="faint" style={{ maxWidth: 360 }}>{p.genre || "—"}</span>
              </div>
              <button className="sm" onClick={() => { setEditing(p); setCreating(false); }}>edit</button>
            </div>
          ))}
        </div>
        <div>
          {(creating || editing) && (
            <PresetForm
              key={editing?.id ?? "new"}
              editingId={editing?.id ?? null}
              initial={editing ? {
                name: editing.name, genre: editing.genre, mood: editing.mood, influences: editing.influences,
                key_tempo_feel: editing.key_tempo_feel, vocal_range: editing.vocal_range, themes: editing.themes,
                lyric_exemplars: editing.lyric_exemplars,
              } : EMPTY}
              onDone={() => { setEditing(null); setCreating(false); }}
            />
          )}
        </div>
      </div>
    </div>
  );
}
