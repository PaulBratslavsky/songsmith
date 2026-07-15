// Version control for stage artifacts: a timeline of every saved revision
// (newest first) with editable labels, a full/diff preview against the current
// version, whole-revision restore, and per-SECTION cherry-picks for the
// section-based kinds (lyrics / chords / structure). Everything stays
// append-only: restoring — whole or one section — SAVES A NEW REVISION via the
// user's direct save path (api.saveArtifact / api.revertArtifact), so nothing
// is ever lost. Serialization mirrors each stage's editor exactly (chords =
// SectionChordsEditor's `label: names` + {name,beats} data; structure reuses
// the exported structureToMarkdown; lyrics = `[label]\nlines` ChordPro blocks).
//
// Section-spine (docs/SECTION-SPINE-SPEC.md §Snapshots): restores are
// snapshot-aware. A whole-revision restore re-creates deleted spine rows from
// the revision's embedded spine_snapshot core-side (api.revertArtifact). A
// section cherry-pick keeps section_ids through the splice (id-first match);
// if the old section's row was deleted, the row is re-created first (label +
// position from the old revision's snapshot when present) and the spliced
// entry re-keyed to it. Structure cherry-picks on new-shape (notes-only)
// revisions restore the section's FORM straight onto the spine.

import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { diffLines } from "diff";
import { api } from "../ipc/api";
import type { Artifact } from "../ipc/generated";
import { artifactEnvelope, parseArtifact } from "../lib/artifacts";
import { spineSnapshot, useSpineSections } from "../lib/sections";
import { structureToMarkdown } from "./StructureEditor";

/** Kinds whose data is a `sections` array — preview as cards + cherry-pickable. */
const SECTION_KINDS = ["lyrics", "chords", "structure"] as const;
type SectionKind = (typeof SECTION_KINDS)[number];
const isSectionKind = (k: string): k is SectionKind => (SECTION_KINDS as readonly string[]).includes(k);

const normLabel = (s: string) => s.trim().toLowerCase().split(/\s+/).join(" ");

function relTime(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return iso.slice(0, 16).replace("T", " ");
  const s = Math.max(0, (Date.now() - t) / 1000);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 30 * 86400) return `${Math.floor(s / 86400)}d ago`;
  return iso.slice(0, 10);
}

// ---- per-kind text rendering (one canonical text per revision, so diffs of
// two revisions always align, even when a revision's stored `text` drifted) ---

const lyricsBlocks = (sections: { label: string; lines: string[] }[]) =>
  sections.map((s) => `[${s.label || "Section"}]\n${s.lines.join("\n")}`).join("\n\n");
const chordsLines = (sections: { label: string; chords: { name: string }[] }[]) =>
  sections.map((s) => `${s.label || "Section"}: ${s.chords.map((c) => c.name).join(" ")}`).join("\n");

function renderedText(kind: string, content: string): string {
  switch (kind) {
    case "lyrics": {
      const { text, data } = parseArtifact("lyrics", content);
      return data?.sections.length ? lyricsBlocks(data.sections) : text;
    }
    case "chords": {
      const { text, data } = parseArtifact("chords", content);
      return data?.sections.length ? chordsLines(data.sections) : text;
    }
    case "structure": {
      const { text, data } = parseArtifact("structure", content);
      // structureToMarkdown ignores root/mode/bpm (song facts) — dummies are fine
      return data?.sections.length
        ? structureToMarkdown({ root: "", mode: "", bpm: 0, keyNote: data.keyNote, tempoNote: data.tempoNote, sections: data.sections })
        : text;
    }
    case "lyric_spec": {
      const { text, data } = parseArtifact("lyric_spec", content);
      if (!data) return text;
      return [
        `HOOK: ${data.hook}`, `PREMISE: ${data.premise}`, `POV / TENSE: ${data.pov}`,
        `SETTING: ${data.setting}`, `ARC: ${data.arc}`, `DICTION: ${data.diction}`,
        data.referenceVibe ? `REFERENCE VIBE: ${data.referenceVibe}` : "",
        "", "SONG MAP:",
        ...data.beats.map((b) => `- ${b.section}: ${b.beat}`),
        "", `IMAGE BANK: ${data.imageBank.join(" · ")}`, `AVOID: ${data.avoid.join(" · ")}`,
      ].filter((l, i, a) => l !== "" || a[i - 1] !== "").join("\n");
    }
    default:
      return artifactEnvelope(content).text;
  }
}

// ---- section previews (cards) -----------------------------------------------

type PreviewSection = { label: string; frozen: boolean; body: string };

function previewSections(kind: string, content: string): PreviewSection[] | null {
  if (kind === "lyrics") {
    const { data } = parseArtifact("lyrics", content);
    return data?.sections.length
      ? data.sections.map((s) => ({ label: s.label || "Section", frozen: s.frozen === true, body: s.lines.join("\n") }))
      : null;
  }
  if (kind === "chords") {
    const { data } = parseArtifact("chords", content);
    return data?.sections.length
      ? data.sections.map((s) => ({
          label: s.label || "Section", frozen: s.frozen === true,
          body: s.chords.map((c) => (c.beats === 4 ? c.name : `${c.name}·${c.beats}`)).join("  "),
        }))
      : null;
  }
  if (kind === "structure") {
    const { data } = parseArtifact("structure", content);
    return data?.sections.length
      ? data.sections.map((s) => ({
          label: s.label || s.type || "Section", frozen: s.frozen === true,
          body: `${s.bars} bars${s.role ? ` — ${s.role}` : ""}`,
        }))
      : null;
  }
  return null;
}

// ---- per-section cherry-pick --------------------------------------------------

/** CURRENT data with ONE section replaced by the old revision's section
 *  (matched by section_id first — rename-proof — then by label, case/space-
 *  insensitive; the old section carried VERBATIM, section_id included; every
 *  other current section — frozen flags included — untouched). A section that
 *  no longer exists is re-inserted at its old slot. Re-serialized exactly the
 *  way the stage's editor saves. Null when either side has no parseable
 *  section data (the UI hides the buttons then — for STRUCTURE the spine-based
 *  restore path takes over instead). */
export function composeSectionRestore(
  kind: string, currentContent: string, oldContent: string, oldIndex: number,
): { text: string; data: unknown } | null {
  const splice = <S extends { label: string; section_id?: string }>(cur: S[], old: S[]): S[] | null => {
    const sec = old[oldIndex];
    if (!sec) return null;
    const out = [...cur];
    let pos = sec.section_id ? out.findIndex((s) => s.section_id === sec.section_id) : -1;
    if (pos < 0) pos = out.findIndex((s) => normLabel(s.label) === normLabel(sec.label));
    if (pos >= 0) out[pos] = sec;
    else out.splice(Math.min(oldIndex, out.length), 0, sec);
    return out;
  };
  const withId = <S extends { section_id?: string }>(s: S) => (s.section_id ? { section_id: s.section_id } : {});
  if (kind === "lyrics") {
    const cur = parseArtifact("lyrics", currentContent).data;
    const old = parseArtifact("lyrics", oldContent).data;
    if (!cur || !old) return null;
    const sections = splice(cur.sections, old.sections);
    if (!sections) return null;
    // mirror LyricsEditor's save: `frozen` persisted only when set
    const data = { sections: sections.map((s) => ({ ...withId(s), label: s.label, lines: s.lines, ...(s.frozen ? { frozen: true } : {}) })) };
    return { text: lyricsBlocks(data.sections), data };
  }
  if (kind === "chords") {
    const cur = parseArtifact("chords", currentContent).data;
    const old = parseArtifact("chords", oldContent).data;
    if (!cur || !old) return null;
    const sections = splice(cur.sections, old.sections);
    if (!sections) return null;
    // mirror SectionChordsEditor.toData: feel kept, frozen only when set
    const data = {
      sections: sections.map((s) => ({
        ...withId(s), label: s.label, feel: s.feel, ...(s.frozen ? { frozen: true } : {}),
        chords: s.chords.map((c) => ({ name: c.name, beats: c.beats })),
      })),
    };
    return { text: chordsLines(data.sections), data };
  }
  if (kind === "structure") {
    // legacy shape only: both sides still carry sections in the artifact data
    const cur = parseArtifact("structure", currentContent).data;
    const old = parseArtifact("structure", oldContent).data;
    if (!cur?.sections.length || !old) return null;
    const sections = splice(cur.sections, old.sections);
    if (!sections) return null;
    // mirror the pre-spine StructureEditor save: notes from the CURRENT data;
    // NO embedded key/bpm (the SONG owns them — docs/SONG-FACTS.md)
    const secs = sections.map((s) => ({ ...withId(s), type: s.type ?? "", label: s.label, bars: s.bars, role: s.role ?? "", ...(s.frozen ? { frozen: true } : {}) }));
    const data = { keyNote: cur.keyNote, tempoNote: cur.tempoNote, sections: secs };
    return { text: structureToMarkdown({ root: "", mode: "", bpm: 0, ...data }), data };
  }
  return null;
}

// ---- widgets -----------------------------------------------------------------

/** Editable revision label — saves on Enter / blur, Escape reverts. */
function LabelInput({ rv, onSave }: { rv: Artifact; onSave: (label: string | null) => void }) {
  const [val, setVal] = useState(rv.label ?? "");
  const commit = () => { const t = val.trim(); if (t !== (rv.label ?? "")) onSave(t || null); };
  return (
    <input
      className="hist-label" value={val} placeholder="add a label…"
      title="name this revision (e.g. “pre-chorus rewrite”) — saves on Enter / blur"
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setVal(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") setVal(rv.label ?? "");
      }}
    />
  );
}

/** The `🕘 History (vN)` opener + modal. Drop it anywhere a stage artifact shows. */
export function HistoryButton({ songId, stageId, kind, current, onChanged }: {
  songId: string; stageId: string; kind: string; current: Artifact; onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        className="sm" onClick={() => setOpen(true)}
        title="every saved version of this stage — compare, label, and restore (whole or per section); restoring creates a NEW version, nothing is lost"
      >
        🕘 History (v{String(current.version)})
      </button>
      {open && (
        <RevisionHistory songId={songId} stageId={stageId} kind={kind} current={current} onClose={() => setOpen(false)} onChanged={onChanged} />
      )}
    </>
  );
}

export function RevisionHistory({ songId, stageId, kind, current, onClose, onChanged }: {
  songId: string; stageId: string; kind: string; current: Artifact;
  onClose: () => void; onChanged: () => void;
}) {
  const qc = useQueryClient();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<"diff" | "full">("diff");
  const { sections: spine } = useSpineSections(songId);

  const revisions = useQuery({ queryKey: ["revisions", stageId], queryFn: () => api.listArtifactRevisions(stageId) });
  const revs = revisions.data ?? [];
  const head = revs[0] ?? current; // newest revision = the current version
  const selected = revs.find((r) => r.id === selectedId) ?? head;
  const isCurrent = selected.id === head.id;

  const refreshRevisions = () => qc.invalidateQueries({ queryKey: ["revisions", stageId] });
  const refreshAll = () => { refreshRevisions(); onChanged(); };

  const setLabel = useMutation({
    mutationFn: (a: { id: string; label: string | null }) => api.setArtifactLabel(a.id, a.label),
    onSuccess: refreshRevisions,
  });
  const restoreAll = useMutation({
    mutationFn: () => api.revertArtifact(selected.id),
    onSuccess: () => { setSelectedId(null); refreshAll(); },
  });
  /** The old revision's snapshot entry for a section id (label + position), if any. */
  const snapshotEntryOf = (content: string, sectionId: string): { label?: string; position?: number } | undefined => {
    try {
      const snap = (JSON.parse(content) as { spine_snapshot?: { section_id?: string; label?: string; position?: number }[] }).spine_snapshot;
      return snap?.find((e) => e?.section_id === sectionId);
    } catch {
      return undefined;
    }
  };

  const restoreSection = useMutation({
    mutationFn: async (oldIndex: number) => {
      // STRUCTURE on a spine song (new shape — the current revision carries no
      // section copy): restoring a section restores its FORM onto the SPINE
      // (update the matching row, or re-create it), then a notes-only save
      // records the action as a new revision.
      const curStructure = kind === "structure" ? parseArtifact("structure", head.content).data : null;
      if (kind === "structure" && !curStructure?.sections.length) {
        const old = parseArtifact("structure", selected.content).data;
        const sec = old?.sections[oldIndex];
        if (!sec) throw new Error("couldn't read the old section");
        const row =
          (sec.section_id ? spine.find((r) => r.id === sec.section_id) : undefined) ??
          spine.find((r) => normLabel(r.label) === normLabel(sec.label));
        if (row) await api.updateSection(row.id, sec.label, sec.type ?? "", sec.bars, sec.role ?? "");
        else {
          const snap = sec.section_id ? snapshotEntryOf(selected.content, sec.section_id) : undefined;
          await api.createSection(songId, sec.label, sec.type ?? "", sec.bars, sec.role ?? "",
            snap?.position != null ? Number(snap.position) : Math.min(oldIndex, spine.length));
        }
        const fresh = await api.listSections(songId);
        const data = { keyNote: curStructure?.keyNote ?? "", tempoNote: curStructure?.tempoNote ?? "" };
        const text = structureToMarkdown({
          root: "", mode: "", bpm: 0, ...data,
          sections: fresh.map((r) => ({ label: r.label, bars: Number(r.bars), role: r.role })),
        });
        return api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text, data, spine_snapshot: spineSnapshot(fresh) }));
      }

      const composed = composeSectionRestore(kind, head.content, selected.content, oldIndex);
      if (!composed) throw new Error("couldn't compose the section restore");
      // the old section may reference a DELETED spine row — re-create it (label
      // + position from the old revision's snapshot when it has one) and re-key
      // the spliced entry to the fresh row before saving (§Snapshots)
      const oldSec = (parseArtifact(kind as "lyrics" | "chords" | "structure", selected.content).data?.sections as
        | { section_id?: string; label: string }[]
        | undefined)?.[oldIndex];
      if (oldSec?.section_id && spine.length && !spine.some((r) => r.id === oldSec.section_id)) {
        const snap = snapshotEntryOf(selected.content, oldSec.section_id);
        const created = await api.createSection(songId, String(snap?.label ?? oldSec.label), "", 8, "",
          snap?.position != null ? Number(snap.position) : undefined);
        for (const s of (composed.data as { sections?: { section_id?: string }[] }).sections ?? []) {
          if (s.section_id === oldSec.section_id) s.section_id = created.id;
        }
      }
      const fresh = spine.length ? await api.listSections(songId) : [];
      return api.saveArtifact(songId, stageId, kind, JSON.stringify({
        kind, ...composed,
        ...(fresh.length ? { spine_snapshot: spineSnapshot(fresh) } : {}),
      }));
    },
    onSuccess: refreshAll,
  });

  const currentText = useMemo(() => renderedText(kind, head.content), [kind, head.content]);
  const selectedText = useMemo(() => renderedText(kind, selected.content), [kind, selected.content]);
  const diff = useMemo(() => (isCurrent ? [] : diffLines(currentText, selectedText)), [isCurrent, currentText, selectedText]);
  const changed = diff.some((p) => p.added || p.removed);

  const selSections = previewSections(kind, selected.content);
  // per-section restore needs the old side parseable (and an older revision
  // picked); lyrics/chords also need the CURRENT side to splice into, while
  // STRUCTURE without current sections restores via the SPINE instead
  const canCherryPick =
    isSectionKind(kind) && !isCurrent && !!selSections &&
    (kind === "structure" ? spine.length > 0 || previewSections(kind, head.content) != null : previewSections(kind, head.content) != null);
  const busy = restoreAll.isPending || restoreSection.isPending;
  const err = restoreAll.error ?? restoreSection.error;

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal hist-modal" onClick={(e) => e.stopPropagation()}>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline" }}>
          <h2 style={{ margin: 0 }}>🕘 Revision history <span className="faint">— {kind}</span></h2>
          <button className="sm ghost" title="close" onClick={onClose}>✕</button>
        </div>
        <p className="faint" style={{ margin: "4px 0 10px", fontSize: 11 }}>
          Every save is kept. Restoring — the whole version or a single section — creates a <b>new</b> version; nothing is lost.
        </p>

        <div className="hist-body">
          {/* ---- timeline (newest first) ---- */}
          <div className="hist-timeline">
            {revisions.isLoading && <span className="faint">loading…</span>}
            {revs.map((rv, i) => (
              <div
                key={rv.id}
                className={"hist-item" + (rv.id === selected.id ? " active" : "")}
                onClick={() => { setSelectedId(rv.id); setView(i === 0 ? "full" : "diff"); }}
              >
                <div className="row" style={{ justifyContent: "space-between", alignItems: "center", gap: 6 }}>
                  <b>v{String(rv.version)}</b>
                  <span className="row" style={{ gap: 6, alignItems: "center" }}>
                    {i === 0 && <span className="badge published">current</span>}
                    {rv.approved && <span className="badge done" title="this revision was approved">✓</span>}
                    <span className="faint" style={{ fontSize: 11 }} title={rv.created_at.slice(0, 19).replace("T", " ")}>{relTime(rv.created_at)}</span>
                  </span>
                </div>
                <LabelInput key={rv.id + " " + (rv.label ?? "")} rv={rv} onSave={(label) => setLabel.mutate({ id: rv.id, label })} />
              </div>
            ))}
          </div>

          {/* ---- preview / diff ---- */}
          <div className="hist-preview">
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 8, gap: 8 }}>
              <div className="row" style={{ gap: 6, alignItems: "center" }}>
                {!isCurrent && (
                  <>
                    <button className={"sm" + (view === "diff" ? " primary" : "")} onClick={() => setView("diff")} title="what restoring this version would change">± diff vs current</button>
                    <button className={"sm" + (view === "full" ? " primary" : "")} onClick={() => setView("full")}>full</button>
                  </>
                )}
                {isCurrent && <span className="faint" style={{ fontSize: 11 }}>this is the current version</span>}
              </div>
              {!isCurrent && (
                <button
                  className="sm primary" disabled={busy}
                  title="bring this whole version back — saved as a NEW version on top (nothing is lost)"
                  onClick={() => restoreAll.mutate()}
                >
                  {restoreAll.isPending ? "restoring…" : `⟲ Restore all of v${String(selected.version)}`}
                </button>
              )}
            </div>
            {err != null && <div className="banner err">{String((err as any)?.message ?? err)}</div>}

            {view === "diff" && !isCurrent ? (
              !changed ? (
                <div className="banner">No differences — v{String(selected.version)} matches the current version.</div>
              ) : (
                <pre className="artifact-text hist-diff" style={{ margin: 0 }}>
                  {diff.map((p, i) => (
                    <span key={i} className={p.added ? "diff-add" : p.removed ? "diff-del" : ""}>{p.value}</span>
                  ))}
                </pre>
              )
            ) : selSections ? (
              <>
                {canCherryPick && (
                  <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>
                    Cherry-pick: restore just one section from v{String(selected.version)} — the rest of the current version (🔒 flags included) stays put.
                  </p>
                )}
                {selSections.map((s, i) => (
                  <div key={i} className="card" style={{ marginBottom: 8 }}>
                    <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
                      <div className="row" style={{ gap: 8, alignItems: "center" }}>
                        <b>{s.label}</b>
                        {s.frozen && <span className="badge done" title="this section was locked in this revision">🔒</span>}
                      </div>
                      {canCherryPick && (
                        <button
                          className="sm" disabled={busy}
                          title={`replace the current “${s.label}” with this revision's — saved as a new version`}
                          onClick={() => restoreSection.mutate(i)}
                        >
                          {restoreSection.isPending ? "…" : "⟲ restore this section"}
                        </button>
                      )}
                    </div>
                    <pre style={{ margin: 0, whiteSpace: "pre-wrap", fontSize: 12, fontFamily: "var(--mono)" }}>
                      {s.body || <span className="faint">(empty — instrumental)</span>}
                    </pre>
                  </div>
                ))}
              </>
            ) : (
              <pre className="artifact-text" style={{ margin: 0 }}>{selectedText || <span className="faint">(empty)</span>}</pre>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
