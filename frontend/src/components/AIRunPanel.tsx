import { useEffect, useRef, useState } from "react";
import { api, listen, STAGE_LABELS } from "../ipc/api";
import { NOTE_NAMES } from "../music/theory";

const SEED_HINT: Record<string, string> = {
  concept: "Optional: a title, a line, or a feeling to build the concept around.",
  structure: "Optional: a structure you have in mind (e.g. verse-chorus-verse-bridge-chorus).",
  chords: "Optional: a chord loop you already have (e.g. Am F C G).",
  lyrics: "Optional: a chorus line or hook you want to keep.",
  prompt: "Optional: extra direction for the generator (energy, instrumentation).",
};

export function AIRunPanel({
  stageId,
  stageType,
  hasArtifact,
  approved,
  onChanged,
  songId,
  keyRoot,
  keyMode,
  bpm,
}: {
  stageId: string;
  stageType: string;
  hasArtifact: boolean;
  approved: boolean;
  onChanged: () => void;
  songId?: string;
  keyRoot?: string;
  keyMode?: string;
  bpm?: number;
}) {
  const [seed, setSeed] = useState("");
  const [stream, setStream] = useState("");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const streamRef = useRef("");
  const cancelledRef = useRef(false);

  useEffect(() => {
    setStream("");
    setError(null);
    streamRef.current = "";
    setSeed("");
  }, [stageId]);

  useEffect(() => {
    let un1 = () => {};
    let un2 = () => {};
    (async () => {
      un1 = await listen<{ stage_id: string; token: string }>("stage_token", (p) => {
        if (p.stage_id !== stageId) return;
        streamRef.current += p.token;
        setStream(streamRef.current);
      });
      un2 = await listen<{ stage_id: string }>("stage_done", (p) => {
        if (p.stage_id !== stageId) return;
        setRunning(false);
        onChanged();
      });
    })();
    return () => { un1(); un2(); };
  }, [stageId]);

  const run = async () => {
    setRunning(true);
    setError(null);
    setStream("");
    streamRef.current = "";
    cancelledRef.current = false;
    try {
      // force the picked key to win over a stale key in an earlier stage's context
      const keyDirective = stageType === "chords" && keyRoot
        ? `Write the progression strictly in ${keyRoot} ${keyMode ?? "minor"}: treat ${keyRoot} as the TONIC / home chord and resolve to it, using that scale's diatonic chords (plus tasteful borrowed ones). If any earlier stage names a different key, it is STALE — this key wins.\n\n`
        : "";
      const userInput = (keyDirective + (seed || "")).trim();
      await api.runStage(stageId, userInput || undefined);
      setRunning(false);
      onChanged();
    } catch (e: any) {
      // a user-initiated cancel is not an error — end quietly
      if (!cancelledRef.current) setError(String(e?.message ?? e));
      setRunning(false);
      onChanged();
    }
  };
  const cancel = async () => {
    cancelledRef.current = true;
    try { await api.cancelStage(stageId); } catch {}
    setRunning(false);
  };
  const approve = async () => { await api.approveStage(stageId); onChanged(); };
  // key/scale/BPM are SONG facts (docs/SONG-FACTS.md) — editable BEFORE the
  // first run so the generation is grounded in the user's choice instead of
  // the preset-seeded default (user-reported: no way to pick the key pre-run,
  // so the first Structure locked in A minor / 138).
  const setKey = async (root: string, mode: string, newBpm?: number) => {
    if (songId) { await api.updateSongKey(songId, root, mode, newBpm ?? bpm ?? 120); onChanged(); }
  };

  return (
    <div className="card" style={{ marginTop: 12 }}>
      <h3>Co-write with Claude</h3>
      {(stageType === "chords" || stageType === "structure") && songId && keyRoot && (
        <div className="row" style={{ gap: 10, alignItems: "flex-end", marginBottom: 8 }}>
          <div><label>Key</label><select value={keyRoot} onChange={(e) => setKey(e.target.value, keyMode ?? "minor")} disabled={running}>{NOTE_NAMES.map((n) => <option key={n} value={n}>{n}</option>)}</select></div>
          <div><label>Scale</label><select value={keyMode ?? "minor"} onChange={(e) => setKey(keyRoot, e.target.value)} disabled={running}><option value="minor">minor</option><option value="major">major</option></select></div>
          <div><label>BPM</label><input type="number" value={bpm ?? 120} onChange={(e) => setKey(keyRoot, keyMode ?? "minor", Number(e.target.value) || 120)} disabled={running} style={{ width: 70 }} /></div>
          <span className="faint" style={{ fontSize: 11 }}>the song's key/tempo — every stage follows it</span>
        </div>
      )}
      <label>Your seed ({STAGE_LABELS[stageType]})</label>
      <textarea value={seed} onChange={(e) => setSeed(e.target.value)} placeholder={SEED_HINT[stageType]} />

      <div className="row" style={{ marginTop: 10, gap: 8 }}>
        <button className="primary" onClick={run} disabled={running}>
          {running ? (<><span className="spin">▮</span> Running…</>) : hasArtifact ? "Re-run / refine" : "Run stage"}
        </button>
        {running && <button onClick={cancel}>Cancel</button>}
        {hasArtifact && (
          <button onClick={approve} disabled={approved}>{approved ? "Approved ✓" : "Approve & advance"}</button>
        )}
      </div>

      {error && <div className="banner err" style={{ marginTop: 10 }}>{error}</div>}
      {(running || stream) && (
        <>
          <label>Live output</label>
          <div className="stream">{stream || <span className="faint">waiting for Claude…</span>}</div>
        </>
      )}
    </div>
  );
}
