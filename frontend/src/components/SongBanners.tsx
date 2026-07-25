// The song workspace's top banners, extracted from SongWorkspace (cosmetic
// split, 2026-07-24) — each owns its own state so the route stays readable.
//
//   ImportResumeBanner — an imported song (source:"import" render) with a
//   stage still pending didn't finish its pipeline: one-click ⟳ Resume
//   (rebuilds lyrics from the stashed/re-run transcript, runs missing Claude
//   stages, approves everything) + 📋 paste-from-Suno for lyrics.
//
//   StaleRefreshBanner — out-of-date stages batch refresh; every re-run
//   lands as a DRAFT, so firing the whole batch is safe to review.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, listen, STAGE_LABELS } from "../ipc/api";
import type { Stage } from "../ipc/generated";
import { staleStagesInOrder } from "./StageChecklist";

export function ImportResumeBanner({ songId, stages, onChanged }: {
  songId: string;
  stages: Stage[];
  onChanged: () => void;
}) {
  const [resumeMsg, setResumeMsg] = useState("");
  const [resumeBusy, setResumeBusy] = useState(false);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [pasteText, setPasteText] = useState("");
  const rendersQ = useQuery({ queryKey: ["renders", songId], queryFn: () => api.listRenders(songId) });
  const hasImportRender = !!rendersQ.data?.some((r) => r.source === "import");

  // resume progress arrives on the same import_progress events the Library uses
  useEffect(() => {
    let un = () => {};
    (async () => {
      un = await listen<{ message: string }>("import_progress", (p) => setResumeMsg(p.message));
    })();
    return () => un();
  }, [songId]);

  const resumeImport = async () => {
    if (resumeBusy) return;
    setResumeBusy(true);
    setResumeMsg("Resuming the import…");
    try {
      setResumeMsg(await api.resumeImport(songId));
    } catch (e: any) {
      setResumeMsg(`Resume failed: ${String(e?.message ?? e)} — you can try again.`);
    }
    setResumeBusy(false);
    onChanged();
  };

  // 📋 paste-from-Suno: when transcription can't hear the lyrics, the words
  // usually exist verbatim in Suno — import_lyrics maps them onto the
  // sections (same verbatim parser as the lyrics-first flow)
  const pasteLyrics = async () => {
    if (!pasteText.trim()) return;
    setResumeMsg("Placing the pasted lyrics into sections…");
    try {
      await api.importLyrics(songId, pasteText);
      setResumeMsg("Lyrics placed — review the Lyrics stage.");
      setPasteOpen(false);
      setPasteText("");
    } catch (e: any) {
      setResumeMsg(`Paste failed: ${String(e?.message ?? e)}`);
    }
    onChanged();
  };

  const pending = stages.filter((s) => s.status === "pending");
  if (!hasImportRender || (!pending.length && !resumeMsg)) return null;
  return (
    <div className="banner warn row" style={{ marginBottom: 12, alignItems: "center", gap: 10, flexWrap: "wrap" }}>
      {pending.length > 0 && (
        <span>
          ⟳ <b>This import didn't finish</b> — {pending.map((s) => STAGE_LABELS[s.type] ?? s.type).join(", ")} still empty.
        </span>
      )}
      {pending.length > 0 && (
        <button className="sm primary" disabled={resumeBusy} onClick={() => void resumeImport()} title="retry the missing pieces: re-transcribe lyrics if needed, run the missing Claude stages, approve everything">
          {resumeBusy ? "Resuming…" : "⟳ Resume import"}
        </button>
      )}
      {pending.some((s) => s.type === "lyrics") && (
        <button className="sm" onClick={() => setPasteOpen((o) => !o)}
          title="the surest fix: copy the lyrics from Suno and paste them — they map onto your sections verbatim">
          📋 Paste lyrics
        </button>
      )}
      {resumeMsg && <span className="faint">{resumeMsg}</span>}
      {!resumeBusy && resumeMsg && <button className="sm ghost" title="clear" onClick={() => setResumeMsg("")}>✕</button>}
      {pasteOpen && (
        <div style={{ width: "100%" }}>
          <textarea value={pasteText} onChange={(e) => setPasteText(e.target.value)} spellCheck={false}
            placeholder={"Paste the song's lyrics (Suno's [Verse]/[Chorus] headers welcome — words are kept verbatim)…"}
            style={{ width: "100%", minHeight: 120, fontFamily: "var(--mono)", fontSize: 12 }} />
          <div className="row" style={{ gap: 6, marginTop: 6 }}>
            <button className="sm primary" disabled={!pasteText.trim()} onClick={() => void pasteLyrics()}>Place into sections</button>
            <button className="sm ghost" onClick={() => setPasteOpen(false)}>cancel</button>
          </div>
        </div>
      )}
    </div>
  );
}

export function StaleRefreshBanner({ stages, onChanged }: {
  stages: Stage[];
  onChanged: () => void;
}) {
  const [refreshBusy, setRefreshBusy] = useState(false);
  const [refreshMsg, setRefreshMsg] = useState("");
  const refreshStale = async () => {
    const stale = staleStagesInOrder(stages);
    if (!stale.length || refreshBusy) return;
    setRefreshBusy(true);
    let done = 0;
    try {
      for (const s of stale) {
        setRefreshMsg(`Refreshing ${STAGE_LABELS[s.type] ?? s.type} (${done + 1}/${stale.length})…`);
        await api.runStage(s.id);
        done += 1;
        onChanged();
      }
      setRefreshMsg(`✓ ${done} stage${done === 1 ? "" : "s"} refreshed — review the draft on each stage.`);
    } catch (e: any) {
      setRefreshMsg(`stopped after ${done}: ${String(e?.message ?? e)}`);
    }
    setRefreshBusy(false);
  };

  const stale = staleStagesInOrder(stages);
  if (!stale.length && !refreshMsg) return null;
  return (
    <div className="banner warn row" style={{ marginBottom: 12, alignItems: "center", gap: 10, flexWrap: "wrap" }}>
      {stale.length > 0 && (
        <span>
          ⚠ <b>{stale.length} stage{stale.length === 1 ? " is" : "s are"} out of date</b> — {stale.map((s) => STAGE_LABELS[s.type] ?? s.type).join(", ")}.
        </span>
      )}
      {stale.length > 0 && (
        <button className="sm primary" disabled={refreshBusy} onClick={refreshStale} title="re-run every out-of-date stage in order — each result lands as a draft you accept or discard">
          {refreshBusy ? "Refreshing…" : "🔄 Refresh out-of-date stages"}
        </button>
      )}
      {refreshMsg && <span className="faint">{refreshMsg}</span>}
      {!refreshBusy && refreshMsg && <button className="sm ghost" title="clear" onClick={() => setRefreshMsg("")}>✕</button>}
    </div>
  );
}
