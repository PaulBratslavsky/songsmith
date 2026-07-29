// A/B render comparison (the iteration loop: import v1 → rebuild → re-render
// in Suno → import v2 → COMPARE). Both takes decode once and play in perfect
// sync through their own gain nodes; the A/B switch is a ~20ms crossfade at
// the SAME position, so differences in arrangement/mix are instantly audible
// — no re-seeking, no losing your place.
import { useEffect, useRef, useState } from "react";
import { api } from "../ipc/api";

type R = { id: string; label: string; file_path: string };

export function RenderAB({ renders }: { renders: R[] }) {
  const [aId, setAId] = useState(renders[0]?.id ?? "");
  const [bId, setBId] = useState(renders[1]?.id ?? "");
  const [active, setActive] = useState<"A" | "B">("A");
  const [playing, setPlaying] = useState(false);
  const [pos, setPos] = useState(0);
  const [dur, setDur] = useState(0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const ctxRef = useRef<AudioContext | null>(null);
  const bufCache = useRef(new Map<string, AudioBuffer>());
  const nodes = useRef<{ src: AudioBufferSourceNode; gain: GainNode }[]>([]);
  const startRef = useRef({ at: 0, offset: 0 });
  const activeRef = useRef(active);
  activeRef.current = active;

  // keep selections valid as renders come and go
  useEffect(() => {
    if (!renders.find((r) => r.id === aId)) setAId(renders[0]?.id ?? "");
    if (!renders.find((r) => r.id === bId)) setBId(renders[1]?.id ?? renders[0]?.id ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reconcile on list change only
  }, [renders]);

  const load = async (path: string) => {
    const hit = bufCache.current.get(path);
    if (hit) return hit;
    const b64 = await api.readAudioB64(path);
    const bin = atob(b64);
    const arr = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
    const ctx = (ctxRef.current ??= new AudioContext());
    const buf = await ctx.decodeAudioData(arr.buffer);
    bufCache.current.set(path, buf);
    return buf;
  };

  const stop = () => {
    nodes.current.forEach((n) => { try { n.src.stop(); } catch { /* done */ } });
    nodes.current = [];
    setPlaying(false);
  };

  const startAt = async (offset: number) => {
    const ra = renders.find((r) => r.id === aId);
    const rb = renders.find((r) => r.id === bId);
    if (!ra || !rb || busy) return;
    setBusy(true);
    setErr("");
    try {
      const [ba, bb] = await Promise.all([load(ra.file_path), load(rb.file_path)]);
      const ctx = ctxRef.current!;
      stop();
      const mk = (buf: AudioBuffer, on: boolean) => {
        const gain = ctx.createGain();
        gain.gain.value = on ? 1 : 0;
        gain.connect(ctx.destination);
        const src = ctx.createBufferSource();
        src.buffer = buf;
        src.connect(gain);
        src.start(0, Math.max(0, Math.min(offset, buf.duration - 0.05)));
        return { src, gain };
      };
      nodes.current = [mk(ba, activeRef.current === "A"), mk(bb, activeRef.current === "B")];
      startRef.current = { at: ctx.currentTime, offset };
      setDur(Math.max(ba.duration, bb.duration));
      setPos(offset);
      setPlaying(true);
      void ctx.resume?.();
    } catch (e: any) {
      setErr(String(e?.message ?? e));
    }
    setBusy(false);
  };

  const switchTo = (side: "A" | "B") => {
    setActive(side);
    const [na, nb] = nodes.current;
    if (na && nb && ctxRef.current) {
      const t = ctxRef.current.currentTime;
      na.gain.gain.setTargetAtTime(side === "A" ? 1 : 0, t, 0.02);
      nb.gain.gain.setTargetAtTime(side === "B" ? 1 : 0, t, 0.02);
    }
  };

  // position ticker + end-of-audio stop
  useEffect(() => {
    if (!playing) return;
    const t = setInterval(() => {
      const ctx = ctxRef.current;
      if (!ctx) return;
      const p = ctx.currentTime - startRef.current.at + startRef.current.offset;
      setPos(p);
      // rewind at the end — leaving pos AT the duration made the next ▶ a
      // silent blip until you dragged the slider back (audit 2026-07-28)
      if (dur && p >= dur) { stop(); setPos(0); }
    }, 200);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- stop is stable enough
  }, [playing, dur]);
  // picking different takes stops playback (fresh buffers on next play)
  useEffect(() => { stop(); setPos(0); }, [aId, bId]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => () => stop(), []); // eslint-disable-line react-hooks/exhaustive-deps

  if (renders.length < 2) return null;
  const fmt = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(Math.max(0, s) % 60)).padStart(2, "0")}`;
  const sel = (v: string, set: (x: string) => void, exclude: string) => (
    <select value={v} onChange={(e) => set(e.target.value)} style={{ maxWidth: 170 }}>
      {renders.map((r) => (
        <option key={r.id} value={r.id} disabled={r.id === exclude}>{r.label}</option>
      ))}
    </select>
  );

  return (
    <div className="card" style={{ marginTop: 10, padding: 10 }}>
      <div className="row" style={{ alignItems: "center", gap: 8, flexWrap: "wrap" }}>
        <span className="cmp-cap" title="play two takes in sync and flip between them — same position, instant switch">A/B compare</span>
        <b style={{ fontSize: 12 }}>A</b>
        {sel(aId, setAId, bId)}
        <b style={{ fontSize: 12 }}>B</b>
        {sel(bId, setBId, aId)}
        <button className="sm primary" disabled={busy || aId === bId} onClick={() => (playing ? stop() : void startAt(pos))}>
          {busy ? "loading…" : playing ? "⏹ stop" : "▶ play"}
        </button>
        <div className="row" style={{ gap: 2 }}>
          {(["A", "B"] as const).map((s) => (
            <button key={s} className={"sm" + (active === s ? " primary" : "")} disabled={aId === bId}
              onClick={() => switchTo(s)} title={`hear take ${s} (instant, same position)`}>
              {s}
            </button>
          ))}
        </div>
        <input type="range" min={0} max={Math.max(1, dur)} step={0.5} value={Math.min(pos, dur || 0)}
          onChange={(e) => { const v = Number(e.target.value); setPos(v); if (playing) void startAt(v); }}
          style={{ flex: 1, minWidth: 120 }} />
        <span className="faint" style={{ fontSize: 11 }}>{fmt(pos)} / {dur ? fmt(dur) : "–:––"}</span>
        {err && <span className="faint" style={{ color: "var(--danger)" }}>{err}</span>}
      </div>
    </div>
  );
}
