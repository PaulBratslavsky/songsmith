import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { compositionFromAnalysis } from "../lib/music/compose/compositionFromSong";
import { CompositionSchema } from "../lib/music/compose/schema";
import { api, pickAudioFile, openFile, revealFile, inTauri } from "../ipc/api";
import { useSpineSections } from "../lib/sections";
import { RenderAB } from "./RenderAB";

const baseName = (p: string) => p.split("/").pop() ?? p;

export function FinalRenders({ songId }: { songId: string }) {
  const qc = useQueryClient();
  const nav = useNavigate();
  // Phase 1 round-trip: analyze THIS render (real tempo/key/sections/chords),
  // save it as a linked composition, open the Composer against the audio
  const [analyzing, setAnalyzing] = useState<string | null>(null);
  const [anMsg, setAnMsg] = useState("");
  // the song's spine links analyzed sections to REAL section ids, so a
  // Composer rebuild of this render can export back onto the song
  const { sections: spineSections } = useSpineSections(songId);
  const analyzeToComposer = async (rd: { id: string; label: string; file_path: string; analysis: string }) => {
    setAnalyzing(rd.id);
    setAnMsg(`Analyzing "${rd.label}" — stems, tempo, key, chords, melody… (local, a few minutes, then Claude cleans it up)`);
    try {
      // imports stash their analysis on the render — instant open, no re-run
      const stored = (() => { try { const v = JSON.parse(rd.analysis || ""); return v?.sections ? v : null; } catch { return null; } })();
      const a = stored ?? (await api.analyzeForComposer(rd.file_path));
      const comp = compositionFromAnalysis(a, `${rd.label} (analyzed)`, spineSections);
      if (!comp) throw new Error("the analysis found no sections");
      const saved = await api.saveComposition(null, `${rd.label} (analyzed)`, songId, JSON.stringify(CompositionSchema.parse(comp)));
      setAnMsg("");
      // the analysis' first section start = the first downbeat — the audio
      // arrives pre-aligned to bar 1 (the nudge stays hand-tunable)
      const nudge = Math.round(((a.sections?.[0] as { start_sec?: number } | undefined)?.start_sec ?? 0) * 1000);
      nav({ to: "/composer", search: { comp: saved.id, audio: rd.file_path, ...(nudge ? { nudge } : {}) } as never });
    } catch (e: any) {
      setAnMsg(`Analysis failed: ${String(e?.message ?? e)}`);
    }
    setAnalyzing(null);
  };
  const renders = useQuery({ queryKey: ["renders", songId], queryFn: () => api.listRenders(songId) });
  const settings = useQuery({ queryKey: ["settings"], queryFn: api.getSettings });
  const musicFolder = settings.data?.music_folder?.trim() || "";
  const invalidate = () => qc.invalidateQueries({ queryKey: ["renders", songId] });

  const add = useMutation({
    mutationFn: async () => {
      // open the music folder so the user can drop the song in (all music in one place),
      // then pick the file — the picker defaults to that folder
      if (musicFolder) await openFile(musicFolder);
      const path = await pickAudioFile(musicFolder || undefined);
      if (!path) return null;
      return api.addRender(songId, baseName(path), path, "", "");
    },
    onSuccess: (v) => v && invalidate(),
  });
  const pick = useMutation({ mutationFn: ({ id, on }: { id: string; on: boolean }) => api.setRenderPick(id, on), onSuccess: invalidate });
  const del = useMutation({ mutationFn: (id: string) => api.deleteRender(id), onSuccess: invalidate });

  return (
    <div className="card" style={{ marginTop: 14 }}>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <div>
          <h2 style={{ margin: 0 }}>🎧 Final renders</h2>
          <span className="muted">
            {musicFolder
              ? <>+ Add opens <code>{baseName(musicFolder)}</code> — drop your song there, then pick it. All music lives in one place.</>
              : <>Set a music folder in <b>Settings</b> so all songs save to one place. For now, + Add just picks a file.</>}
          </span>
        </div>
        <button className="primary" onClick={() => add.mutate()} disabled={!inTauri || add.isPending}>
          {add.isPending ? "…" : "+ Add version"}
        </button>
      </div>
      {!inTauri && <div className="banner warn" style={{ marginTop: 10 }}>File picking works in the desktop app.</div>}

      {anMsg && <div className="banner" style={{ marginTop: 8 }}>{anMsg}</div>}
      {renders.data && renders.data.length === 0 && (
        <div className="empty">No renders yet — generate audio from the prompt, then add the file here.</div>
      )}

      {/* iteration loop: v1 vs v2 in sync — flip at the same position */}
      {inTauri && renders.data && renders.data.length >= 2 && <RenderAB renders={renders.data} />}

      {renders.data?.map((rd) => (
        <div key={rd.id} className="list-item" style={{ marginTop: 8, borderColor: rd.is_pick ? "var(--accent-dim)" : undefined }}>
          <div className="col" style={{ gap: 3 }}>
            <div className="row" style={{ gap: 8 }}>
              <b>{rd.label}</b>
              {rd.is_pick && <span className="badge done">★ pick</span>}
              {rd.source && <span className="faint">{rd.source}</span>}
            </div>
            <span className="faint" style={{ maxWidth: 520, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{rd.file_path}</span>
          </div>
          <div className="row" style={{ gap: 6 }}>
            <button className="sm primary" disabled={analyzing != null} title="analyze this take (real tempo/key/sections/chords) and open it in the Composer with the audio — rebuild the AI song by hand" onClick={() => analyzeToComposer(rd)}>
              {analyzing === rd.id ? "analyzing…" : "🎼 Analyze → Composer"}
            </button>
            <button className="sm" onClick={() => openFile(rd.file_path)}>▶ play</button>
            <button className="sm" onClick={() => revealFile(rd.file_path)}>reveal</button>
            <button className="sm" onClick={() => pick.mutate({ id: rd.id, on: !rd.is_pick })}>{rd.is_pick ? "unpick" : "★ pick"}</button>
            <button className="sm danger" onClick={() => del.mutate(rd.id)}>remove</button>
          </div>
        </div>
      ))}
    </div>
  );
}
