import { playChord } from "../music/synth";
import { NOTE_NAMES } from "../music/theory";
import { chordMidisByName } from "../music/engineAdapter";

// clockwise from top: C G D A E B F# Db Ab Eb Bb F
const MAJ_PC = [0, 7, 2, 9, 4, 11, 6, 1, 8, 3, 10, 5];
const MAJ_LBL = ["C", "G", "D", "A", "E", "B", "F#", "C#", "G#", "D#", "A#", "F"];
const MIN_LBL = ["Am", "Em", "Bm", "F#m", "C#m", "G#m", "D#m", "A#m", "Fm", "Cm", "Gm", "Dm"];
const minPc = (majPc: number) => (majPc + 9) % 12; // relative minor root

export function CircleOfFifths({
  rootPc,
  quality,
  onPick,
  highlightNames,
}: {
  rootPc: number;
  quality: string;
  onPick: (pc: number, quality: string) => void;
  /** chord names (e.g. "F#m", "D") to ring-highlight — the selected scale's diatonic chords */
  highlightNames?: Set<string>;
}) {
  const size = 220, c = size / 2, rOut = 92, rIn = 60;
  const pick = (pc: number, q: string) => {
    const m = chordMidisByName(NOTE_NAMES[pc] + q);
    if (m.length) playChord(m);
    onPick(pc, q);
  };
  const node = (i: number, r: number, lbl: string, pc: number, q: string) => {
    const ang = (i * 30 - 90) * (Math.PI / 180);
    const x = c + r * Math.cos(ang), y = c + r * Math.sin(ang);
    const sel = rootPc === pc && (quality === q || (q === "" && quality === "") );
    const hl = !sel && (highlightNames?.has(NOTE_NAMES[pc] + q) ?? false);
    return (
      <g key={lbl + r} style={{ cursor: "pointer" }} onClick={() => pick(pc, q)}>
        <circle cx={x} cy={y} r={15} fill={sel ? "var(--accent)" : "var(--paper-3)"} stroke={hl ? "var(--accent)" : "var(--line)"} strokeWidth={hl ? 2 : 1} />
        <text x={x} y={y + 3.5} textAnchor="middle" fontSize={q ? 9 : 10} fontFamily="monospace"
          fill={sel ? "#11140a" : "var(--ink)"}>{lbl}</text>
      </g>
    );
  };
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`}>
      <circle cx={c} cy={c} r={rOut + 16} fill="none" stroke="var(--line)" />
      <circle cx={c} cy={c} r={rIn - 16} fill="none" stroke="var(--line)" />
      {MAJ_PC.map((pc, i) => node(i, rOut, MAJ_LBL[i], pc, ""))}
      {MAJ_PC.map((pc, i) => node(i, rIn, MIN_LBL[i], minPc(pc), "m"))}
    </svg>
  );
}
