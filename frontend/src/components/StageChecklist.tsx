import { STAGE_LABELS } from "../ipc/api";
import type { Stage } from "../ipc/generated";

/** WHY a stage is stale: the earlier stages whose current artifact is NEWER
 *  than this stage's — the provenance the banner names ("Structure changed
 *  after this was generated"), ordinal order. Empty when not stale. */
export function staleCauseLabels(stages: Stage[], stageId: string): string[] {
  const sorted = [...stages].sort((a, b) => Number(a.ordinal) - Number(b.ordinal));
  const me = sorted.find((s) => s.id === stageId);
  if (!me?.artifact_at) return [];
  return sorted
    .filter((s) => Number(s.ordinal) < Number(me.ordinal) && s.artifact_at && s.artifact_at > me.artifact_at!)
    .map((s) => STAGE_LABELS[s.type] ?? s.type);
}

/** The stale stages in run order — what "Refresh out-of-date stages" walks. */
export function staleStagesInOrder(stages: Stage[]): Stage[] {
  const stale = staleStageIds(stages);
  return [...stages]
    .sort((a, b) => Number(a.ordinal) - Number(b.ordinal))
    .filter((s) => stale.has(s.id));
}

/** Stages whose current artifact is older than a later-edited upstream stage —
 *  i.e. out of date and worth re-running. Timestamps are ISO, so string-comparable.
 *
 *  A `verbatim` stage is NEVER stale. The rule below assumes the pipeline ran
 *  forward (concept → … → lyrics), but the paste flows run it BACKWARD: the
 *  user's words come first and everything else is derived from them, so writing
 *  a Concept afterwards made its own sources look out of date. Worse, the batch
 *  "Refresh out-of-date stages" would then re-run the Lyricist over pasted
 *  lyrics and rewrite them — the exact thing the verbatim contract forbids
 *  (user-hit, 2026-08-12). Nothing upstream can invalidate words the user
 *  wrote, so provenance beats ordinal here.
 *
 *  They still count as upstream for genuinely-derived stages below them. */
export function staleStageIds(stages: Stage[]): Set<string> {
  const sorted = [...stages].sort((a, b) => Number(a.ordinal) - Number(b.ordinal));
  const out = new Set<string>();
  let newestUpstream = "";
  for (const s of sorted) {
    if (s.artifact_at && newestUpstream && s.artifact_at < newestUpstream && !s.verbatim) out.add(s.id);
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
