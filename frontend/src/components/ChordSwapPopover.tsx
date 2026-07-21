// Inline chord swap (user-designed 2026-07-21): click a chord in the Arrange
// view → this popover. Chips-first (diatonic chords of the song key with roman
// numerals, then common variations of the picked root), with a Wheel tab
// (circle of fifths, scale ringed) for visual picking and borrowed/out-of-key
// chords. Every tap swaps IMMEDIATELY and plays — tap the old chord to undo,
// close when happy. Persistence stays with the editor's normal save flow.
import { useState } from "react";
import { NOTE_NAMES, diatonicChords, pitchClassOf } from "../music/theory";
import { CircleOfFifths } from "./CircleOfFifths";
import { chordMidisByName } from "../music/engineAdapter";
import { playChord } from "../music/synth";

// the full variation set the user picked: triads, 7ths, sus, 9s, 6ths, dim/aug
const VARIATION_QUALITIES = ["", "m", "7", "maj7", "m7", "sus2", "sus4", "add9", "9", "m9", "6", "m6", "dim", "dim7", "aug"];

function rootOf(name: string): string | null {
  const m = name.trim().match(/^([A-G](?:#|b)?)/);
  return m ? m[1] : null;
}

export function ChordSwapPopover({
  current, keyRoot, keyMode, onPick, onClose,
}: {
  current: string;
  keyRoot: string;
  keyMode: string;
  /** swap the chord in place (already validated + played) */
  onPick: (name: string) => void;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<"chips" | "wheel">("chips");
  const mode = keyMode === "major" ? "major" : "minor";
  const rootPc = pitchClassOf(keyRoot) ?? 0;
  const diatonic = diatonicChords(rootPc, mode);
  const curRoot = rootOf(current) ?? keyRoot;
  const variations = VARIATION_QUALITIES
    .map((q) => curRoot + q)
    .filter((n) => chordMidisByName(n).length > 0);
  const pick = (name: string) => {
    const m = chordMidisByName(name);
    if (!m.length) return;
    playChord(m);
    onPick(name);
  };
  const curWheel = (() => {
    const r = rootOf(current);
    const minor = /^[A-G](?:#|b)?m(?!aj)/.test(current.trim());
    return { pc: r ? pitchClassOf(r) ?? 0 : 0, q: minor ? "m" : "" };
  })();

  return (
    <>
      <div onClick={onClose} style={{ position: "fixed", inset: 0, zIndex: 40 }} />
      <div className="card" style={{ position: "absolute", zIndex: 41, top: "100%", left: 0, marginTop: 4, width: 320, padding: 10, boxShadow: "0 10px 28px rgba(0,0,0,.55)" }}>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 6 }}>
          <b>{current || "pick a chord"}</b>
          <div className="row" style={{ gap: 4 }}>
            <button className={"sm" + (tab === "chips" ? " primary" : "")} onClick={() => setTab("chips")}>Chips</button>
            <button className={"sm" + (tab === "wheel" ? " primary" : "")} onClick={() => setTab("wheel")}>Wheel</button>
            <button className="sm ghost" title="close" onClick={onClose}>✕</button>
          </div>
        </div>

        {tab === "chips" ? (
          <>
            <label style={{ fontSize: 10 }}>In {keyRoot} {mode}</label>
            <div className="row" style={{ flexWrap: "wrap", gap: 4, marginBottom: 8 }}>
              {diatonic.map((d) => (
                <button key={d.roman} className={"sm" + (d.name === current ? " primary" : "")} title={d.roman} onClick={() => pick(d.name)}>
                  {d.name} <span className="faint">{d.roman}</span>
                </button>
              ))}
            </div>
            <label style={{ fontSize: 10 }}>Variations of {curRoot}</label>
            <div className="row" style={{ flexWrap: "wrap", gap: 4 }}>
              {variations.map((n) => (
                <button key={n} className={"sm" + (n === current ? " primary" : "")} onClick={() => pick(n)}>{n}</button>
              ))}
            </div>
            <p className="faint" style={{ fontSize: 10, margin: "8px 0 0" }}>tap = swap in place + hear it · tap the old chord to undo · borrowed chords live on the Wheel</p>
          </>
        ) : (
          <>
            <div style={{ display: "flex", justifyContent: "center" }}>
              <CircleOfFifths
                rootPc={curWheel.pc}
                quality={curWheel.q}
                onPick={(pc, q) => pick(NOTE_NAMES[pc] + q)}
                highlightNames={new Set(diatonic.map((d) => d.name))}
              />
            </div>
            <p className="faint" style={{ fontSize: 10, margin: "6px 0 0" }}>ringed = in {keyRoot} {mode} · anything else is a borrowed pick · refine with the Chips tab's variations</p>
          </>
        )}
      </div>
    </>
  );
}
