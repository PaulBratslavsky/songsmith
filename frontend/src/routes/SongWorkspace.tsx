import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, listen, STAGE_LABELS } from "../ipc/api";
import type { Stage } from "../ipc/generated";
import { StageChecklist, staleStageIds, staleCauseLabels, staleStagesInOrder } from "../components/StageChecklist";
import { DraftBar } from "../components/DraftBar";
import { ArtifactPanel } from "../components/ArtifactPanel";
import { HistoryButton } from "../components/RevisionHistory";
import { AIRunPanel } from "../components/AIRunPanel";
import { SectionChordsEditor } from "../components/SectionChordsEditor";
import { LyricsEditor } from "../components/LyricsEditor";
import { ConceptEditor } from "../components/ConceptEditor";
import { StructureEditor } from "../components/StructureEditor";
import { PromptEditor } from "../components/PromptEditor";
import { LyricSpecEditor } from "../components/LyricSpecEditor";
import { FieldDrawer, useFieldDrawer } from "../components/FieldDrawer";
import { FinalRenders } from "../components/FinalRenders";
import { SongSheet } from "../components/SongSheet";
import { SongNotation } from "../components/SongNotation";
import { ArrangementBuilder } from "../components/ArrangementBuilder";
import { parseArtifact } from "../lib/artifacts";
import { NOTE_NAMES } from "../music/theory";
import { useSpineSections } from "../lib/sections";

export function SongWorkspace() {
  const { id } = useParams({ from: "/song/$id" });
  const nav = useNavigate();
  const qc = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [tab, setTab] = useState<"workspace" | "builder" | "sheet" | "notation" | "renders">("workspace");
  const [abMsg, setAbMsg] = useState("");
  // batch "Refresh out-of-date stages": walks the stale stages in run order;
  // every re-run lands as a DRAFT, so firing the whole batch is safe to review
  const [refreshBusy, setRefreshBusy] = useState(false);
  const [refreshMsg, setRefreshMsg] = useState("");
  const refreshStale = async () => {
    const stale = staleStagesInOrder(song.data?.stages ?? []);
    if (!stale.length || refreshBusy) return;
    setRefreshBusy(true);
    let done = 0;
    try {
      for (const s of stale) {
        setRefreshMsg(`Refreshing ${STAGE_LABELS[s.type] ?? s.type} (${done + 1}/${stale.length})…`);
        await api.runStage(s.id);
        done += 1;
        invalidate();
      }
      setRefreshMsg(`✓ ${done} stage${done === 1 ? "" : "s"} refreshed — review the draft on each stage.`);
    } catch (e: any) {
      setRefreshMsg(`stopped after ${done}: ${String(e?.message ?? e)}`);
    }
    setRefreshBusy(false);
  };
  // ⟳ Resume import: an imported song (has a source:"import" render) with a
  // stage still pending didn't finish its pipeline — offer a one-click retry
  // (rebuilds lyrics from the stashed/re-run transcript, runs missing stages,
  // approves everything). Progress arrives on the same import_progress events.
  const [resumeMsg, setResumeMsg] = useState("");
  const [resumeBusy, setResumeBusy] = useState(false);
  const rendersQ = useQuery({ queryKey: ["renders", id], queryFn: () => api.listRenders(id) });
  const hasImportRender = !!rendersQ.data?.some((r) => r.source === "import");
  const resumeImport = async () => {
    if (resumeBusy) return;
    setResumeBusy(true);
    setResumeMsg("Resuming the import…");
    try {
      setResumeMsg(await api.resumeImport(id));
    } catch (e: any) {
      setResumeMsg(`Resume failed: ${String(e?.message ?? e)} — you can try again.`);
    }
    setResumeBusy(false);
    invalidate();
  };
  useEffect(() => {
    let un = () => {};
    (async () => {
      un = await listen<{ message: string }>("import_progress", (p) => setResumeMsg(p.message));
    })();
    return () => un();
  }, [id]);
  const [showStyle, setShowStyle] = useState(false);
  const fd = useFieldDrawer();
  // the right inspector flyout is open when a field is focused or Style is toggled
  const inspectorOpen = !!fd?.active || showStyle;
  // a focused field takes precedence over the style panel
  useEffect(() => { if (fd?.active) setShowStyle(false); }, [fd?.active]);
  // portal target for the sidebar's per-song nav (rendered by the app shell)
  const [navSlot, setNavSlot] = useState<HTMLElement | null>(null);
  useEffect(() => { setNavSlot(document.getElementById("song-nav-slot")); }, []);
  // navigating to a different song clears the stage selection (avoids stale highlight)
  useEffect(() => { setSelectedId(null); fd?.close?.(); }, [id]); // eslint-disable-line react-hooks/exhaustive-deps
  const buildAbleton = async () => { setAbMsg("Stubbing the song in Ableton — Sections + Bass / Chords / Melody / Filler / Arp…"); try { setAbMsg(await api.abletonBuildSong(id)); } catch (e: any) { setAbMsg(String(e?.message ?? e)); } };
  // live per-step progress while the Ableton build runs (backend emits one
  // event per connect/clear/track-create/section step)
  useEffect(() => {
    let un = () => {};
    (async () => {
      un = await listen<{ song_id: string; message: string }>("ableton_progress", (p) => {
        if (p.song_id === id) setAbMsg(`⚡ ${p.message}`);
      });
    })();
    return () => un();
  }, [id]);

  const song = useQuery({ queryKey: ["song", id], queryFn: () => api.getSong(id) });
  // the song's section spine (docs/SECTION-SPINE-SPEC.md, Phase 2) — section
  // identity/order for the editors below; [] = legacy artifact-label fallback
  const { sections: spineSections, isFetched: spineFetched } = useSpineSections(id);
  const currentType = song.data?.song.current_stage ?? "concept";
  const activeStageId = useMemo(() => {
    if (!song.data) return null;
    if (selectedId) return selectedId;
    return song.data.stages.find((s) => s.type === currentType)?.id ?? song.data.stages[0]?.id ?? null;
  }, [song.data, selectedId, currentType]);

  const stage = useQuery({
    queryKey: ["stage", activeStageId],
    queryFn: () => api.getStage(activeStageId!),
    enabled: !!activeStageId,
  });

  // chords-stage data feeds the Lyrics editor's per-section chord palette (ChordPro)
  const chordsStageId = song.data?.stages.find((s) => s.type === "chords")?.id;
  const chordsStage = useQuery({ queryKey: ["stage", chordsStageId], queryFn: () => api.getStage(chordsStageId!), enabled: !!chordsStageId });
  const chordsData = parseArtifact("chords", chordsStage.data?.artifact?.content).data;

  // lyrics-stage tagged text feeds the Generation Prompt's tagged-lyrics field (verbatim).
  // build it from the lyrics data sections (robust) so it reflects the current words+chords.
  const lyricsStageId = song.data?.stages.find((s) => s.type === "lyrics")?.id;
  const lyricsStageQ = useQuery({ queryKey: ["stage", lyricsStageId], queryFn: () => api.getStage(lyricsStageId!), enabled: !!lyricsStageId });
  const lyricsTagged = (() => {
    const { text, data: ld } = parseArtifact("lyrics", lyricsStageQ.data?.artifact?.content);
    if (ld?.sections.length) {
      return ld.sections.map((s) => `[${s.label || "Section"}]\n${s.lines.join("\n")}`).join("\n\n").trim();
    }
    return text.trim();
  })();

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["song", id] });
    qc.invalidateQueries({ queryKey: ["stage", activeStageId] });
    qc.invalidateQueries({ queryKey: ["songs"] });
    // Phase 3: editors/imports/runs WRITE the spine — refetch it on any change
    qc.invalidateQueries({ queryKey: ["sections", id] });
  };
  const setStatus = useMutation({ mutationFn: (s: string) => api.updateSongStatus(id, s), onSuccess: invalidate });
  const setTitle = useMutation({ mutationFn: (t: string) => api.updateSongTitle(id, t), onSuccess: invalidate });
  const [editTitle, setEditTitle] = useState<string | null>(null);
  // 🎯 the producer's one-line brief — the north star every stage honors (title-edit pattern)
  const setIntent = useMutation({ mutationFn: (t: string) => api.updateSongIntent(id, t), onSuccess: invalidate });
  const [editIntent, setEditIntent] = useState<string | null>(null);
  // header key/BPM click-to-edit — live views of the SONG facts (write-through
  // via update_song_key, same policy as every other picker; docs/SONG-FACTS.md)
  const [editKey, setEditKey] = useState(false);
  const setSongKey = async (root: string, mode: string, bpm: number) => {
    await api.updateSongKey(id, root, mode, bpm);
    invalidate();
  };
  const del = useMutation({
    mutationFn: () => api.deleteSong(id),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["songs"] }); nav({ to: "/" }); },
  });

  // wait for the spine too: the Structure/LyricSpec/Lyrics editors seed their
  // state on mount, so the section list must be known before they render
  if (song.isLoading || !spineFetched) return <div className="empty">Loading…</div>;
  if (!song.data) return <div className="empty">Song not found.</div>;
  const v = song.data.song;
  const preset = song.data.preset;
  const sd = stage.data;
  const allStagesDone = song.data.stages.length > 0 && song.data.stages.every((s) => s.status === "done");
  const isStale = !!sd?.stage && staleStageIds(song.data.stages).has(sd.stage.id);

  return (
    <div>
      <div className="topbar">
        <div>
          {editTitle === null ? (
            <h1 onDoubleClick={() => setEditTitle(v.title)} title="double-click to rename" style={{ cursor: "text" }}>{v.title || "Untitled song"}</h1>
          ) : (
            <input
              autoFocus
              value={editTitle}
              onChange={(e) => setEditTitle(e.target.value)}
              onBlur={() => { if (editTitle.trim() && editTitle !== v.title) setTitle.mutate(editTitle.trim()); setEditTitle(null); }}
              onKeyDown={(e) => {
                if (e.key === "Enter") { if (editTitle.trim() && editTitle !== v.title) setTitle.mutate(editTitle.trim()); setEditTitle(null); }
                if (e.key === "Escape") setEditTitle(null);
              }}
              style={{ fontSize: 22, fontWeight: 700, width: "min(480px, 60vw)" }}
            />
          )}
          {editIntent === null ? (
            <div
              className={v.intent ? "muted" : "faint"}
              onDoubleClick={() => setEditIntent(v.intent)}
              title="double-click to edit — every stage follows this brief"
              style={{ cursor: "text", margin: "2px 0" }}
            >
              🎯 {v.intent || "set the song's intent — every stage follows it"}
            </div>
          ) : (
            <input
              autoFocus
              value={editIntent}
              onChange={(e) => setEditIntent(e.target.value)}
              placeholder="what's this song about? (one line — the north star)"
              onBlur={() => { if (editIntent.trim() !== v.intent) setIntent.mutate(editIntent.trim()); setEditIntent(null); }}
              onKeyDown={(e) => {
                if (e.key === "Enter") { if (editIntent.trim() !== v.intent) setIntent.mutate(editIntent.trim()); setEditIntent(null); }
                if (e.key === "Escape") setEditIntent(null);
              }}
              style={{ width: "min(480px, 60vw)", margin: "2px 0" }}
            />
          )}
          <div className="row" style={{ gap: 8 }}>
            <span className={"badge " + v.status}>{v.status.replace("_", " ")}</span>
            <span className="faint">
              {preset.name} ·{" "}
              {editKey ? (
                <span className="row" style={{ display: "inline-flex", gap: 4, alignItems: "center" }}>
                  <select value={v.key_root} onChange={(e) => setSongKey(e.target.value, v.key_mode, Number(v.bpm))}>
                    {NOTE_NAMES.map((n) => <option key={n} value={n}>{n}</option>)}
                  </select>
                  <select value={v.key_mode} onChange={(e) => setSongKey(v.key_root, e.target.value, Number(v.bpm))}>
                    <option value="minor">minor</option>
                    <option value="major">major</option>
                  </select>
                  <input type="number" value={Number(v.bpm)} onChange={(e) => setSongKey(v.key_root, v.key_mode, Number(e.target.value) || 120)} style={{ width: 62 }} /> BPM
                  <button className="sm ghost" title="done" onClick={() => setEditKey(false)}>✓</button>
                </span>
              ) : (
                <span title="click to edit the song's key/BPM — applies instantly, every stage follows it" style={{ cursor: "pointer", textDecoration: "underline dotted" }} onClick={() => setEditKey(true)}>
                  {v.key_root} {v.key_mode} · {String(v.bpm)} BPM
                </span>
              )}
            </span>
          </div>
        </div>
        <div className="row" style={{ gap: 6 }}>
          {/* Done state makes the Composer the obvious next step (spec: queued CTA).
              Gated like Build in Ableton: a half-built song imports a half-built
              (or stale) timeline — every stage must be done first. */}
          <button
            className={v.status === "done" ? "primary" : ""}
            onClick={() => nav({ to: "/composer", search: { song: id } })}
            disabled={!allStagesDone}
            title={!allStagesDone
              ? "Complete every song-spec stage first (Concept → Generation Prompt)"
              : v.status === "done"
              ? "The song is done — open the finished song on the full Composer timeline (every section end-to-end, chords synced to the lyrics, melody + bass editable)"
              : "Open the whole song in the visual Composer — every section on one timeline, chords synced to the lyrics, melody + bass editable across the song"}
          >
            🎹 Open in Composer
          </button>
          <button onClick={buildAbleton} disabled={!allStagesDone} title={allStagesDone ? "Stub the whole song in Ableton — a named Sections clip track + Bass/Chords/Melody/Filler/Arp MIDI parts from your progression (direct, no chat)" : "Complete every song-spec stage first (Concept → Generation Prompt)"}>⚡ Build in Ableton</button>
          {v.status !== "done" ? (
            <button className="primary" onClick={() => setStatus.mutate("done")}>Mark done</button>
          ) : (
            <button onClick={() => setStatus.mutate("in_progress")}>Reopen</button>
          )}
          <button className="danger" onClick={() => setConfirmDelete(true)}>Delete</button>
        </div>
      </div>
      {abMsg && (
        <div style={{ position: "relative", marginBottom: 12 }}>
          <button className="sm ghost" title="clear" onClick={() => setAbMsg("")} style={{ position: "absolute", top: 4, right: 4, zIndex: 1 }}>✕</button>
          <pre className="artifact-text" style={{ whiteSpace: "pre-wrap", maxHeight: 200, margin: 0, paddingRight: 32 }}>{abMsg}</pre>
        </div>
      )}

      {(() => {
        // an imported song with a stage still pending = the import pipeline
        // didn't finish (empty transcript, a failed Claude call, app quit…)
        const pending = song.data.stages.filter((s) => s.status === "pending");
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
            {resumeMsg && <span className="faint">{resumeMsg}</span>}
            {!resumeBusy && resumeMsg && <button className="sm ghost" title="clear" onClick={() => setResumeMsg("")}>✕</button>}
          </div>
        );
      })()}

      {(() => {
        const stale = staleStagesInOrder(song.data.stages);
        if ((!stale.length && !refreshMsg) || tab !== "workspace") return null;
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
      })()}

      <div className="row" style={{ gap: 6, marginBottom: 12 }}>
        <button className={"sm" + (tab === "workspace" ? " primary" : "")} onClick={() => setTab("workspace")}>Workspace</button>
        <button className={"sm" + (tab === "builder" ? " primary" : "")} onClick={() => setTab("builder")}>Arrange</button>
        <button className={"sm" + (tab === "sheet" ? " primary" : "")} onClick={() => setTab("sheet")}>Sheet preview</button>
        <button className={"sm" + (tab === "notation" ? " primary" : "")} onClick={() => setTab("notation")}>𝄞 Notation</button>
        <button className={"sm" + (tab === "renders" ? " primary" : "")} onClick={() => setTab("renders")}>🎧 Renders</button>
        <div style={{ flex: 1 }} />
        {tab === "workspace" && (
          <button className={"sm" + (showStyle && !fd?.active ? " primary" : "")} title="show the style context the AI uses"
            onClick={() => { fd?.close?.(); setShowStyle((s) => !s); }}>🎨 Style context</button>
        )}
      </div>

      {/* per-song nav lives in the sidebar's empty space (portaled out of the workspace) */}
      {navSlot && createPortal(
        <>
          <StageChecklist bare stages={song.data.stages} currentType={currentType} selectedId={activeStageId} onSelect={(s: Stage) => { setSelectedId(s.id); setTab("workspace"); }} />
          <button className={"rail-btn" + (tab === "renders" ? " active" : "")} onClick={() => setTab("renders")}>
            <span>🎧 Final renders</span><span className="faint">›</span>
          </button>
        </>,
        navSlot,
      )}

      {tab === "workspace" && (
      <>
      <div className="workspace solo">
        <div className="pane">
          <div className="row" style={{ justifyContent: "space-between", marginBottom: 8 }}>
            <h2>{sd ? STAGE_LABELS[sd.stage.type] : "Stage"}</h2>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              {sd?.skill && <span className="faint">skill: {sd.skill.name}</span>}
              {/* version control for THIS stage's artifact — compare, label, restore (whole or per section) */}
              {sd?.artifact && (
                <HistoryButton songId={id} stageId={sd.stage.id} kind={sd.artifact.kind} current={sd.artifact} onChanged={invalidate} />
              )}
            </div>
          </div>
          {isStale && sd?.stage && (
            <div className="banner warn" style={{ marginBottom: 8 }}>
              ⚠ <b>Out of date.</b>{" "}
              {(() => {
                // name WHAT changed instead of "an earlier stage": the newer
                // upstream stages, plus section-spine drift vs this artifact's
                // snapshot (the lyrics-import back-fill case that confused)
                const causes = staleCauseLabels(song.data.stages, sd.stage.id);
                const snap: { label?: string }[] | null = (() => {
                  try { const v = JSON.parse(sd.artifact?.content ?? ""); return Array.isArray(v?.spine_snapshot) ? v.spine_snapshot : null; } catch { return null; }
                })();
                const snapLabels = snap?.map((r) => r.label ?? "");
                const liveLabels = spineSections.map((r) => r.label);
                const drifted = !!snapLabels && (snapLabels.length !== liveLabels.length || snapLabels.some((l, i) => l !== liveLabels[i]));
                const parts: string[] = [];
                if (causes.length) parts.push(`${causes.join(" and ")} changed after this was generated`);
                if (drifted) parts.push(`the section list changed (${snapLabels!.length} → ${liveLabels.length} sections)`);
                if (!parts.length) parts.push("an earlier stage changed after this was generated");
                return <>{parts.join(", and ")}. </>;
              })()}
              Re-run this stage to rebuild it — the result lands as a draft you accept or discard.
            </div>
          )}
          {sd?.draft && <DraftBar draft={sd.draft} onChanged={invalidate} />}
          {stage.isLoading && <div className="empty">Loading stage…</div>}
          {sd && !sd.artifact && (
            <div className="banner">
              No artifact yet. Run this stage to co-write the <b>{STAGE_LABELS[sd.stage.type]}</b> using the style preset and prior approved stages as context.
            </div>
          )}
          {sd?.artifact && sd.stage.type === "concept" ? (
            <ConceptEditor
              key={sd.artifact.id}
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              content={sd.artifact.content}
              onChanged={invalidate}
            />
          ) : sd?.artifact && sd.stage.type === "structure" ? (
            <StructureEditor
              key={sd.artifact.id}
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              content={sd.artifact.content}
              keyRoot={v.key_root}
              keyMode={v.key_mode}
              bpm={Number(v.bpm)}
              onChanged={invalidate}
              spineSections={spineSections}
            />
          ) : sd?.artifact && sd.stage.type === "chords" ? (
            <SectionChordsEditor
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              artifactId={sd.artifact.id}
              keyRoot={v.key_root}
              keyMode={v.key_mode}
              initialData={parseArtifact("chords", sd.artifact.content).data}
              onChanged={invalidate}
              spineSections={spineSections}
            />
          ) : sd?.artifact && sd.stage.type === "lyric_spec" ? (
            <LyricSpecEditor
              key={sd.artifact.id}
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              content={sd.artifact.content}
              onChanged={invalidate}
              spineSections={spineSections}
            />
          ) : sd?.artifact && sd.stage.type === "lyrics" ? (
            <LyricsEditor
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              artifactId={sd.artifact.id}
              content={sd.artifact.content}
              onChanged={invalidate}
              chordsData={chordsData}
              spineSections={spineSections}
            />
          ) : sd?.artifact && sd.stage.type === "prompt" ? (
            <PromptEditor
              key={sd.artifact.id}
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              content={sd.artifact.content}
              onChanged={invalidate}
              lyricsTagged={lyricsTagged}
              spineSections={spineSections}
              chordsData={chordsData}
            />
          ) : (
            sd?.artifact && <ArtifactPanel artifact={sd.artifact} songId={id} stageId={sd.stage.id} onChanged={invalidate} />
          )}
          {sd && (
            <AIRunPanel stageId={sd.stage.id} stageType={sd.stage.type} hasArtifact={!!sd.artifact} approved={!!sd.artifact?.approved} onChanged={invalidate} songId={id} keyRoot={v.key_root} keyMode={v.key_mode} bpm={Number(v.bpm)} />
          )}
        </div>

      </div>

      <aside className={"inspector-flyout" + (inspectorOpen ? " open" : "")}>
        {fd?.active ? (
          <FieldDrawer context={[
            `Key / tempo: ${v.key_root} ${v.key_mode} · ${String(v.bpm)} BPM`,
            `Genre: ${preset.genre}`,
            `Mood: ${preset.mood}`,
            preset.themes ? `Themes: ${preset.themes}` : "",
          ].filter(Boolean)} />
        ) : showStyle ? (
          <>
            {/* Mirrors core/src/agent.rs::build_system_prompt — the EFFECTIVE context every stage
                receives: THE SONG (the brief) outranks the preset's themes on story; the preset
                owns the SOUND; lyric exemplars go to the lyrics stage only. */}
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 2 }}>
              <h3 style={{ margin: 0 }}>Style context</h3>
              <button className="sm ghost" title="close" onClick={() => setShowStyle(false)}>✕</button>
            </div>
            <p className="faint" style={{ fontSize: 11, marginBottom: 12 }}>what every stage actually receives, in precedence order</p>

            <h3 style={{ color: "var(--accent)", marginBottom: 4 }}>The song — the brief</h3>
            <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>outranks everything below on story — no stage may drift from it</p>
            <div className="col" style={{ gap: 10, marginBottom: 14 }}>
              <div><label style={{ margin: 0 }}>Title</label><div className="faint">{v.title || "—"}</div></div>
              <div>
                <label style={{ margin: 0 }}>🎯 Intent</label>
                <div className="faint">{v.intent.trim() || "none stated — stages honor the title's plain meaning"}</div>
              </div>
              <div><label style={{ margin: 0 }}>Key / tempo (song fields)</label><div className="faint">{v.key_root} {v.key_mode} · {String(v.bpm)} BPM</div></div>
            </div>

            <h3 style={{ marginBottom: 4 }}>Style preset — the sound</h3>
            <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>{preset.name} — genre, mood, instrumentation, tempo feel, vocal</p>
            <div className="col" style={{ gap: 10, marginBottom: 14 }}>
              {[["Genre", preset.genre], ["Mood", preset.mood], ["Influences", preset.influences], ["Key / tempo feel", preset.key_tempo_feel], ["Vocal range", preset.vocal_range]].map(([k, val]) => (
                <div key={k}><label style={{ margin: 0 }}>{k}</label><div className="faint">{val || "—"}</div></div>
              ))}
              <div>
                <label style={{ margin: 0 }}>Themes</label>
                <div className="faint">{preset.themes || "—"}</div>
                <div style={{ fontSize: 11, fontStyle: "italic", color: "var(--warn)" }}>defaults only — the song's title/intent win on conflict</div>
              </div>
            </div>

            {preset.lyric_exemplars.trim() && (
              <>
                <h3 style={{ marginBottom: 4 }}>Lyric exemplars</h3>
                <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>lyrics stage only — voice calibration, never copied</p>
                <pre className="faint" style={{ whiteSpace: "pre-wrap", margin: "0 0 14px", fontSize: 12 }}>{preset.lyric_exemplars.trim()}</pre>
              </>
            )}

            <hr />
            <p className="faint">Approved outputs from earlier stages carry forward as context to the stage you run. The Lyrics stage also receives a computed technical brief (bars / chords / tempo budgets). Click a field's 💬 to refine it inline.</p>
          </>
        ) : null}
      </aside>
      </>
      )}
      {tab === "builder" && (
        <ArrangementBuilder
          songId={id}
          title={v.title || "Untitled song"}
          subtitle={`${preset.name} · ${v.key_root} ${v.key_mode} · ${String(v.bpm)} BPM`}
          keyRoot={v.key_root}
          keyMode={v.key_mode}
          stages={song.data.stages}
        />
      )}
      {tab === "sheet" && (
        <SongSheet
          songId={id}
          title={v.title || "Untitled song"}
          subtitle={`${preset.name} · ${v.key_root} ${v.key_mode} · ${String(v.bpm)} BPM`}
          keyRoot={v.key_root}
          keyMode={v.key_mode}
          stages={song.data.stages}
          voicings={v.voicings}
        />
      )}
      {tab === "notation" && (
        <SongNotation
          songId={id}
          title={v.title || "Untitled song"}
          keyRoot={v.key_root}
          keyMode={v.key_mode}
          bpm={Number(v.bpm)}
          chordsData={chordsData}
          lyricsData={parseArtifact("lyrics", lyricsStageQ.data?.artifact?.content).data}
          spine={spineSections}
        />
      )}
      {tab === "renders" && <FinalRenders songId={id} />}

      {confirmDelete && (
        <div className="modal-bg" onClick={() => setConfirmDelete(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h2>Delete this song?</h2>
            <p className="muted">This permanently removes the song and all its stages and artifacts.</p>
            <div className="row" style={{ justifyContent: "flex-end", marginTop: 14 }}>
              <button className="ghost" onClick={() => setConfirmDelete(false)}>Cancel</button>
              <button className="danger" onClick={() => del.mutate()}>Delete permanently</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
