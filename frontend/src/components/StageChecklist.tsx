import { STAGE_LABELS } from "../ipc/api";
import type { Stage } from "../ipc/generated";

/** Stages whose current artifact is older than a later-edited upstream stage —
 *  i.e. out of date and worth re-running. Timestamps are ISO, so string-comparable. */
export function staleStageIds(stages: Stage[]): Set<string> {
  const sorted = [...stages].sort((a, b) => Number(a.ordinal) - Number(b.ordinal));
  const out = new Set<string>();
  let newestUpstream = "";
  for (const s of sorted) {
    if (s.artifact_at && newestUpstream && s.artifact_at < newestUpstream) out.add(s.id);
    if (s.artifact_at && s.artifact_at > newestUpstream) newestUpstream = s.artifact_at;
  }
  return out;
}

export function StageChecklist({
  stages,
  currentType,
  selectedId,
  onSelect,
  bare,
}: {
  stages: Stage[];
  currentType: string;
  selectedId: string | null;
  onSelect: (s: Stage) => void;
  /** rail mode: drop the card chrome, render compact for the sidebar */
  bare?: boolean;
}) {
  const stale = staleStageIds(stages);
  return (
    <div className={bare ? "checklist rail" : "pane checklist"}>
      {bare ? <div className="rail-head">SONG SPEC</div> : <h3>Song spec</h3>}
      {stages.map((s) => {
        const active = selectedId ? s.id === selectedId : s.type === currentType;
        const isStale = stale.has(s.id);
        return (
          <div key={s.id} className={"step" + (active ? " active" : "")} onClick={() => onSelect(s)} title={isStale ? "out of date — an earlier stage changed; re-run to refresh" : undefined}>
            <span className="ord">{Number(s.ordinal) + 1}</span>
            <span className={"dot " + s.status} />
            <span className="label">{STAGE_LABELS[s.type] ?? s.type}</span>
            {isStale ? <span className="stale-flag" title="out of date — re-run">⚠</span> : s.status === "done" && <span className="faint">✓</span>}
          </div>
        );
      })}
    </div>
  );
}
