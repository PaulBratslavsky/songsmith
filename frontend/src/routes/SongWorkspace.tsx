import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api, STAGE_LABELS } from "../ipc/api";
import type { Stage } from "../ipc/generated";
import { StageChecklist, staleStageIds } from "../components/StageChecklist";
import { ArtifactPanel } from "../components/ArtifactPanel";
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
import { ArrangementBuilder } from "../components/ArrangementBuilder";
import { parseArtifact } from "../lib/artifacts";

export function SongWorkspace() {
  const { id } = useParams({ from: "/song/$id" });
  const nav = useNavigate();
  const qc = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [tab, setTab] = useState<"workspace" | "builder" | "sheet" | "renders">("workspace");
  const [abMsg, setAbMsg] = useState("");
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

  const song = useQuery({ queryKey: ["song", id], queryFn: () => api.getSong(id) });
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
  };
  const setStatus = useMutation({ mutationFn: (s: string) => api.updateSongStatus(id, s), onSuccess: invalidate });
  const setTitle = useMutation({ mutationFn: (t: string) => api.updateSongTitle(id, t), onSuccess: invalidate });
  const [editTitle, setEditTitle] = useState<string | null>(null);
  // 🎯 the producer's one-line brief — the north star every stage honors (title-edit pattern)
  const setIntent = useMutation({ mutationFn: (t: string) => api.updateSongIntent(id, t), onSuccess: invalidate });
  const [editIntent, setEditIntent] = useState<string | null>(null);
  const del = useMutation({
    mutationFn: () => api.deleteSong(id),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["songs"] }); nav({ to: "/" }); },
  });

  if (song.isLoading) return <div className="empty">Loading…</div>;
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
            <span className="faint">{preset.name} · {v.key_root} {v.key_mode} · {String(v.bpm)} BPM</span>
          </div>
        </div>
        <div className="row" style={{ gap: 6 }}>
          {/* Done state makes the Composer the obvious next step (spec: queued CTA) */}
          <button
            className={v.status === "done" ? "primary" : ""}
            onClick={() => nav({ to: "/composer", search: { song: id } })}
            title={v.status === "done"
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

      <div className="row" style={{ gap: 6, marginBottom: 12 }}>
        <button className={"sm" + (tab === "workspace" ? " primary" : "")} onClick={() => setTab("workspace")}>Workspace</button>
        <button className={"sm" + (tab === "builder" ? " primary" : "")} onClick={() => setTab("builder")}>Arrange</button>
        <button className={"sm" + (tab === "sheet" ? " primary" : "")} onClick={() => setTab("sheet")}>Sheet preview</button>
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
            {sd?.skill && <span className="faint">skill: {sd.skill.name}</span>}
          </div>
          {isStale && (
            <div className="banner warn" style={{ marginBottom: 8 }}>
              ⚠ <b>Out of date.</b> An earlier stage changed after this was generated. Re-run this stage to rebuild it from the current upstream content.
            </div>
          )}
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
            />
          ) : sd?.artifact && sd.stage.type === "lyric_spec" ? (
            <LyricSpecEditor
              key={sd.artifact.id}
              songId={id}
              stageId={sd.stage.id}
              kind={sd.artifact.kind}
              content={sd.artifact.content}
              onChanged={invalidate}
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
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 8 }}>
              <h3 style={{ margin: 0 }}>Style context</h3>
              <button className="sm ghost" title="close" onClick={() => setShowStyle(false)}>✕</button>
            </div>
            <div className="col" style={{ gap: 10 }}>
              {[["Genre", preset.genre], ["Mood", preset.mood], ["Influences", preset.influences], ["Key / tempo", preset.key_tempo_feel], ["Vocal range", preset.vocal_range], ["Themes", preset.themes]].map(([k, val]) => (
                <div key={k}><label>{k}</label><div className="faint">{val || "—"}</div></div>
              ))}
            </div>
            <hr />
            <p className="faint">Approved outputs from earlier stages carry forward as context to the stage you run. Click a field's 💬 to refine it inline.</p>
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
