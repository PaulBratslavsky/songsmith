// Aligned reference-audio playback for the Composer (Phase 1 render
// round-trip): load a local render (base64 over IPC, once), decode it, expose
// waveform peaks, and FOLLOW the composition transport — start with Play,
// stop with Stop, re-seek on loop wrap. A nudge (ms) lines the audio's first
// downbeat up with bar 1; mute/volume ride a gain node live.
import { useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../../../ipc/api';
import { TICKS_PER_BEAT } from './types';

export function useRenderAudio(
  path: string | null,
  bpm: number,
  isPlaying: boolean,
  currentStep: number | null,
) {
  const [buffer, setBuffer] = useState<AudioBuffer | null>(null);
  const [enabled, setEnabled] = useState(true);
  const [nudgeMs, setNudgeMs] = useState(0);
  const [gain, setGain] = useState(0.9);
  const [err, setErr] = useState('');
  const ctxRef = useRef<AudioContext | null>(null);
  const srcRef = useRef<AudioBufferSourceNode | null>(null);
  const gainRef = useRef<GainNode | null>(null);
  const prevStep = useRef<number | null>(null);
  const nudgeRef = useRef(0);
  nudgeRef.current = nudgeMs;

  useEffect(() => {
    let on = true;
    (async () => {
      if (!path) return;
      try {
        const b64 = await api.readAudioB64(path);
        const bin = atob(b64);
        const arr = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
        const ctx = (ctxRef.current ??= new AudioContext());
        const buf = await ctx.decodeAudioData(arr.buffer);
        if (on) setBuffer(buf);
      } catch (e) {
        if (on) setErr(String((e as Error)?.message ?? e));
      }
    })();
    return () => { on = false; };
  }, [path]);

  const stopSrc = () => {
    try { srcRef.current?.stop(); } catch { /* already stopped */ }
    srcRef.current = null;
  };
  const startAt = (tick: number) => {
    const ctx = ctxRef.current;
    if (!buffer || !ctx) return;
    stopSrc();
    const g = (gainRef.current ??= (() => {
      const n = ctx.createGain();
      n.connect(ctx.destination);
      return n;
    })());
    g.gain.value = enabled ? gain : 0;
    const src = ctx.createBufferSource();
    src.buffer = buffer;
    src.connect(g);
    const offset = (tick / TICKS_PER_BEAT) * (60 / bpm) + nudgeRef.current / 1000;
    if (offset >= 0 && offset < buffer.duration) src.start(0, offset);
    else if (offset < 0) src.start(ctxRef.current!.currentTime - offset, 0);
    srcRef.current = src;
    void ctx.resume?.();
  };

  // transport follow: start on play, stop on stop, re-seek on any jump —
  // backward (loop wrap) OR a forward leap (focus moved to a later section;
  // normal advance is +1 per tick, timer batching can skip a few)
  useEffect(() => {
    if (!buffer) return;
    if (isPlaying && currentStep != null) {
      const jumped =
        prevStep.current != null &&
        (currentStep < prevStep.current || currentStep - prevStep.current > 8);
      if (srcRef.current == null || jumped) startAt(currentStep);
    } else if (!isPlaying) {
      stopSrc();
    }
    prevStep.current = currentStep;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- startAt reads refs
  }, [isPlaying, currentStep, buffer]);

  // mute/volume ride the gain node without restarting
  useEffect(() => {
    if (gainRef.current) gainRef.current.gain.value = enabled ? gain : 0;
  }, [gain, enabled]);
  useEffect(() => () => stopSrc(), []);

  // min/max-ish peaks for the waveform strip (600 columns, coarse scan)
  const peaks = useMemo(() => {
    if (!buffer) return null;
    const ch = buffer.getChannelData(0);
    const cols = 600;
    const per = Math.max(1, Math.floor(ch.length / cols));
    const out: number[] = [];
    for (let c = 0; c < cols; c++) {
      let m = 0;
      for (let i = c * per; i < (c + 1) * per && i < ch.length; i += 32) {
        const v = Math.abs(ch[i]);
        if (v > m) m = v;
      }
      out.push(m);
    }
    return out;
  }, [buffer]);

  return { buffer, peaks, enabled, setEnabled, nudgeMs, setNudgeMs, gain, setGain, err };
}
