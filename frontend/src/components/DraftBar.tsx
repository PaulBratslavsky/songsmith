// Regenerate-as-draft (user decision 2026-07-15): a re-run parks its output
// here — the CURRENT version below stays untouched until the producer accepts.
// One draft per stage; a newer re-run replaces an unreviewed one.
import { useState } from "react";
import { api } from "../ipc/api";
import type { StageDraft } from "../ipc/generated";

function draftText(d: StageDraft): string {
  try {
    const v = JSON.parse(d.content);
    return String(v?.text ?? d.content);
  } catch {
    return d.content;
  }
}

export function DraftBar({ draft, onChanged }: { draft: StageDraft; onChanged: () => void }) {
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(true);
  const act = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    try { await fn(); } catch { /* surfaced by refetch */ }
    setBusy(false);
    onChanged();
  };
  return (
    <div className="banner" style={{ borderColor: "var(--accent)", marginBottom: 10 }}>
      <div className="row" style={{ justifyContent: "space-between", alignItems: "center", gap: 8 }}>
        <span>
          ✨ <b>New draft from the last run.</b> Your current version below is untouched — accept to replace it (locked sections stay locked), or discard.
        </span>
        <div className="row" style={{ gap: 6 }}>
          <button className="sm ghost" onClick={() => setOpen((o) => !o)}>{open ? "hide preview" : "preview"}</button>
          <button className="sm primary" disabled={busy} onClick={() => act(() => api.acceptStageDraft(draft.stage_id))}>Accept draft</button>
          <button className="sm ghost danger" disabled={busy} onClick={() => act(() => api.discardStageDraft(draft.stage_id))}>Discard</button>
        </div>
      </div>
      {open && (
        <div className="artifact-text" style={{ maxHeight: 260, overflow: "auto", marginTop: 8, whiteSpace: "pre-wrap", fontFamily: "var(--mono)", fontSize: 12 }}>
          {draftText(draft)}
        </div>
      )}
    </div>
  );
}
