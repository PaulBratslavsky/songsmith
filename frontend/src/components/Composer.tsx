import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useMutation } from "@tanstack/react-query";
import { DndContext, closestCenter, PointerSensor, KeyboardSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, verticalListSortingStrategy, arrayMove, useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { api } from "../ipc/api";
import { diatonicChords, pitchClassOf, NOTE_NAMES } from "../music/theory";
import { isValidName, chordMidisByName, chordPcsByName, voicedMidisByName, voicedNotesByName, chordSizeByName } from "../music/engineAdapter";
import { pianoVoicedSvg } from "../music/diagrams";
import { playChord, playSequence } from "../music/synth";
import { ImportProgression } from "./ImportProgression";

const INV_LABELS = ["root", "1st inv", "2nd inv", "3rd inv", "4th inv"];

const SCALE_PCS: Record<string, number[]> = { major: [0, 2, 4, 5, 7, 9, 11], minor: [0, 2, 3, 5, 7, 8, 10] };
/** Check the progression against the song key: which chords sit outside the scale,
 *  and whether the key's tonic chord ever appears (a tonal-center sanity check). */
function keyCheck(names: string[], root: string, mode: string): { out: string[]; hasTonic: boolean; tonic: string } {
  const rpc = pitchClassOf(root) ?? 0;
  const allowed = new Set(SCALE_PCS[mode === "major" ? "major" : "minor"].map((i) => (i + rpc) % 12));
  if (mode !== "major") allowed.add((rpc + 11) % 12); // raised 7th → V / vii° read as in-key
  const out: string[] = [];
  for (const n of names) {
    const pcs = chordPcsByName(n);
    if (pcs.length && !pcs.every((p) => allowed.has(p)) && !out.includes(n)) out.push(n);
  }
  const tonic = root + (mode === "major" ? "" : "m");
  const hasTonic = names.some((n) => n === tonic || n.startsWith(tonic + "/") || (n.startsWith(tonic) && !/^[A-G]/.test(n.slice(tonic.length))));
  return { out, hasTonic, tonic };
}

type Chord = { id: string; name: string; beats: number };
type Section = { id: string; label: string; chords: Chord[]; feel?: string };

/** A pointer/keyboard-draggable wrapper (dnd-kit) — works in the Tauri WebView,
 *  unlike HTML5 drag. Hands the drag handle props to its render child. */
function Sortable({ id, children }: { id: string; children: (handle: Record<string, unknown>, dragging: boolean) => ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id });
  const style = { transform: CSS.Transform.toString(transform), transition, opacity: isDragging ? 0.5 : 1, zIndex: isDragging ? 2 : undefined };
  return <div ref={setNodeRef} style={style}>{children({ ...attributes, ...listeners }, isDragging)}</div>;
}

const uid = () => Math.random().toString(36).slice(2, 8);

function fromData(data: any): Section[] {
  const secs = data?.sections;
  if (!Array.isArray(secs)) return [];
  return secs.map((s: any) => ({
    id: uid(),
    label: s.label || s.type || "Section",
    feel: s.feel,
    chords: (Array.isArray(s.chords) ? s.chords : []).map((c: any) => ({
      id: uid(),
      name: typeof c === "string" ? c : c?.name ?? "",
      beats: typeof c === "object" && c?.beats ? c.beats : 4,
    })),
  }));
}
function toData(sections: Section[]) {
  return { sections: sections.map((s) => ({ label: s.label, feel: s.feel, chords: s.chords.map((c) => ({ name: c.name, beats: c.beats })) })) };
}

export function Composer({
  songId, stageId, kind, artifactId, keyRoot, keyMode, initialData, onChanged,
}: {
  songId: string; stageId: string; kind: string; artifactId: string;
  keyRoot: string; keyMode: string; initialData: any; onChanged: () => void;
}) {
  const [sections, setSections] = useState<Section[]>(() => fromData(initialData));
  const [selected, setSelected] = useState<{ s: number; c: number } | null>(null);
  const [playingIdx, setPlayingIdx] = useState<{ s: number; c: number } | null>(null);
  const [dirty, setDirty] = useState(false);

  // reload from a newer revision (e.g. after Claude edits via the stage chat),
  // unless the user has unsaved manual edits in progress
  useEffect(() => {
    if (!dirty) setSections(fromData(initialData));
  }, [artifactId]);

  // palette scale — defaults to the song's key, selectable to explore other scales
  const songRoot = NOTE_NAMES[pitchClassOf(keyRoot) ?? 0];
  const songMode: "major" | "minor" = keyMode === "major" ? "major" : "minor";
  const [scaleRoot, setScaleRoot] = useState(songRoot);
  const [scaleMode, setScaleMode] = useState<"major" | "minor">(songMode);
  const palette = useMemo(() => diatonicChords(pitchClassOf(scaleRoot) ?? 0, scaleMode), [scaleRoot, scaleMode]);
  const offKey = scaleRoot !== songRoot || scaleMode !== songMode;

  const mutate = (fn: (s: Section[]) => Section[]) => { setSections((cur) => fn(structuredClone(cur))); setDirty(true); };
  const setChordName = (si: number, ci: number, name: string) => mutate((s) => { s[si].chords[ci].name = name; return s; });
  const setChordBeats = (si: number, ci: number, beats: number) => mutate((s) => { s[si].chords[ci].beats = Math.max(1, beats); return s; });
  const addChord = (si: number, name = "") => mutate((s) => { s[si].chords.push({ id: uid(), name, beats: 4 }); return s; });
  const importToSection = (si: number, names: string[]) => mutate((s) => { s[si].chords = names.map((n) => ({ id: uid(), name: n, beats: 4 })); return s; });
  const removeChord = (si: number, ci: number) => mutate((s) => { s[si].chords.splice(ci, 1); return s; });
  const setLabel = (si: number, label: string) => mutate((s) => { s[si].label = label; return s; });
  const addSection = (label = "Section") => mutate((s) => { s.push({ id: uid(), label, chords: [] }); return s; });
  const removeSection = (si: number) => mutate((s) => { s.splice(si, 1); return s; });
  const moveSection = (si: number, dir: -1 | 1) => mutate((s) => {
    const j = si + dir; if (j < 0 || j >= s.length) return s;
    [s[si], s[j]] = [s[j], s[si]]; return s;
  });
  // pointer/keyboard drag-and-drop reorder via dnd-kit (HTML5 drag is unreliable
  // in the Tauri WebView — its native handler intercepts it). A small activation
  // distance lets the handle still receive plain clicks.
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const onDragEnd = (e: DragEndEvent) => {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    mutate((s) => {
      const from = s.findIndex((x) => x.id === active.id);
      const to = s.findIndex((x) => x.id === over.id);
      return from < 0 || to < 0 ? s : arrayMove(s, from, to);
    });
  };

  const selChord = selected ? sections[selected.s]?.chords[selected.c] : null;
  // inversion preview for the selected chord (cycle the bass note up the chord tones)
  const [inv, setInv] = useState(0);
  useEffect(() => { setInv(0); }, [selected?.s, selected?.c, selChord?.name]);
  const playOne = (name: string) => { const m = chordMidisByName(name); if (m.length) playChord(m); };
  const playSection = (si: number) => {
    const steps = sections[si].chords.map((c) => ({ notes: chordMidisByName(c.name), beats: c.beats })).filter((s) => s.notes.length);
    playSequence(steps, 120, (i) => setPlayingIdx(i < 0 ? null : { s: si, c: i }));
  };

  const save = useMutation({
    mutationFn: () => {
      const text = sections.map((s) => `${s.label}: ${s.chords.map((c) => c.name).join(" ")}`).join("\n");
      return api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text, data: toData(sections) }));
    },
    onSuccess: () => { setDirty(false); onChanged(); },
  });

  if (sections.length === 0) {
    return (
      <div className="banner">
        No sections yet. Run the <b>Structure</b> stage, then <b>Chords</b> — the progression opens here to edit, play, and (via the stage chat below) ask Claude to change.
        <div className="row" style={{ gap: 6, marginTop: 8 }}>
          <button className="sm" onClick={() => addSection()}>+ section</button>
          <button className="sm" onClick={() => addSection("Instrumental")}>+ instrumental</button>
        </div>
      </div>
    );
  }

  const check = useMemo(() => keyCheck(sections.flatMap((s) => s.chords.map((c) => c.name)), keyRoot, keyMode), [sections, keyRoot, keyMode]);
  const inKey = check.out.length === 0 && check.hasTonic;
  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 8 }}>
        <span className="faint">{keyRoot} {keyMode} · click a chord to voice it · 7ths/sus/borrowed all OK</span>
        <div className="row" style={{ gap: 8, alignItems: "center" }}>
          <span className={"badge " + (inKey ? "done" : "pending")} title={inKey ? `every chord fits ${keyRoot} ${keyMode}` : [check.out.length ? `outside ${keyRoot} ${keyMode}: ${check.out.join(", ")}` : "", !check.hasTonic ? `tonic ${check.tonic} never appears — the progression may be centered on another key` : ""].filter(Boolean).join(" · ")}>
            {inKey ? `✓ in ${keyRoot} ${keyMode}` : `⚠ key check: ${check.out.length ? `${check.out.length} outside scale` : `no ${check.tonic} tonic`}`}
          </span>
          <button className="sm primary" disabled={!dirty || save.isPending} onClick={() => save.mutate()}>{save.isPending ? "saving…" : dirty ? "save revision" : "saved"}</button>
        </div>
      </div>

      <ImportProgression sectionLabels={sections.map((s) => s.label)} onImport={importToSection} />

      <div className="card" style={{ marginBottom: 10 }}>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
          <label style={{ margin: 0 }}>Palette — chords in scale <span className="faint">(click to add to selected section)</span></label>
          <div className="row" style={{ gap: 4, alignItems: "center" }}>
            <select value={scaleRoot} onChange={(e) => setScaleRoot(e.target.value)} title="scale root">
              {NOTE_NAMES.map((n) => <option key={n} value={n}>{n}</option>)}
            </select>
            <select value={scaleMode} onChange={(e) => setScaleMode(e.target.value as "major" | "minor")} title="scale type">
              <option value="major">major</option>
              <option value="minor">minor</option>
            </select>
            {offKey && <button className="sm ghost" title="back to the song's key" onClick={() => { setScaleRoot(songRoot); setScaleMode(songMode); }}>↺ song key</button>}
          </div>
        </div>
        {offKey && <div className="faint" style={{ fontSize: 11, marginBottom: 4 }}>exploring {scaleRoot} {scaleMode} — the song's key is {songRoot} {songMode} (set in Structure)</div>}
        <div className="row" style={{ flexWrap: "wrap", gap: 5 }}>
          {palette.map((p) => (
            <button key={p.roman} className="sm" title={p.roman} onClick={() => { playOne(p.name); if (selected) addChord(selected.s, p.name); }}>
              {p.name} <span className="faint">{p.roman}</span>
            </button>
          ))}
        </div>
      </div>

      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
        <SortableContext items={sections.map((s) => s.id)} strategy={verticalListSortingStrategy}>
          {sections.map((sec, si) => (
            <Sortable key={sec.id} id={sec.id}>
              {(handle, dragging) => (
                <div className={"card section-card" + (dragging ? " dragging" : "")} style={{ marginBottom: 8 }}>
                  <div className="row" style={{ justifyContent: "space-between" }}>
                    <div className="row" style={{ gap: 6, alignItems: "center" }}>
                      <span className="drag-handle" title="drag to reorder" {...handle}>⠿</span>
                      <input value={sec.label} onChange={(e) => setLabel(si, e.target.value)} title="section name (Intro, Solo, Drop…)" style={{ width: 180, fontWeight: 600 }} />
                    </div>
                    <div className="row" style={{ gap: 6 }}>
                      <button className="sm ghost" title="move up" disabled={si === 0} onClick={() => moveSection(si, -1)}>↑</button>
                      <button className="sm ghost" title="move down" disabled={si === sections.length - 1} onClick={() => moveSection(si, 1)}>↓</button>
                      <button className="sm" onClick={() => playSection(si)}>▶ play</button>
                      <button className="sm" onClick={() => { addChord(si); setSelected({ s: si, c: sections[si].chords.length }); }}>+ chord</button>
                      <button className="sm ghost danger" title="remove section" onClick={() => removeSection(si)}>remove</button>
                    </div>
                  </div>
                  {sec.feel && <div className="faint" style={{ marginBottom: 6 }}>{sec.feel}</div>}
                  <div className="row" style={{ flexWrap: "wrap", gap: 6, marginTop: 4 }}>
                    {sec.chords.map((c, ci) => {
                      const active = selected?.s === si && selected?.c === ci;
                      const playing = playingIdx?.s === si && playingIdx?.c === ci;
                      const ok = isValidName(c.name);
                      return (
                        <div key={c.id} className="chord-cell" style={{ borderColor: playing ? "var(--accent)" : active ? "var(--accent-dim)" : ok ? "var(--line)" : "var(--danger)" }} onClick={() => setSelected({ s: si, c: ci })}>
                          <input value={c.name} onChange={(e) => setChordName(si, ci, e.target.value)} placeholder="Am" style={{ width: 56, padding: "2px 4px", textAlign: "center", border: "none", background: "transparent" }} />
                          <div className="row" style={{ gap: 4, justifyContent: "center" }}>
                            <button className="sm ghost" title="play" onClick={(e) => { e.stopPropagation(); playOne(c.name); }}>♪</button>
                            <input type="number" min={1} value={c.beats} onChange={(e) => setChordBeats(si, ci, Number(e.target.value))} title="beats" style={{ width: 34, padding: "1px 3px" }} />
                            <button className="sm ghost danger" title="remove" onClick={(e) => { e.stopPropagation(); removeChord(si, ci); }}>×</button>
                          </div>
                        </div>
                      );
                    })}
                    {sec.chords.length === 0 && <span className="faint">no chords — use the palette or “+ chord”. A section with chords but no lyrics plays as an instrumental.</span>}
                  </div>
                </div>
              )}
            </Sortable>
          ))}
        </SortableContext>
      </DndContext>

      <div className="row" style={{ gap: 6, marginBottom: 10 }}>
        <button className="sm" onClick={() => addSection()}>+ section</button>
        <button className="sm" onClick={() => addSection("Instrumental")} title="add a wordless section (Intro / Solo / Break / Drop)">+ instrumental</button>
      </div>

      {selChord && (() => {
        const count = Math.max(1, chordSizeByName(selChord.name));
        const i = Math.min(inv, count - 1);
        const midis = voicedMidisByName(selChord.name, i);
        if (!midis.length) return null;
        return (
          <div className="card">
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
              <h3 style={{ margin: 0 }}>Voicing — {selChord.name}</h3>
              <div className="row" style={{ gap: 6, alignItems: "center" }}>
                <button className="sm ghost" title="previous inversion" disabled={count < 2} onClick={() => setInv((v) => ((v - 1) % count + count) % count)}>‹</button>
                <span className="faint" style={{ fontSize: 11, minWidth: 78, textAlign: "center" }}>{INV_LABELS[i] ?? `inv ${i}`} ({i + 1}/{count})</span>
                <button className="sm ghost" title="next inversion" disabled={count < 2} onClick={() => setInv((v) => (v + 1) % count)}>›</button>
                <button className="sm" onClick={() => playChord(midis)}>♪ play</button>
              </div>
            </div>
            <div className="fit-svg" dangerouslySetInnerHTML={{ __html: pianoVoicedSvg(midis, selChord.name) }} />
            <div className="faint" style={{ marginTop: 6 }}>notes (low→high): {voicedNotesByName(selChord.name, i).join(" · ")}</div>
          </div>
        );
      })()}
    </div>
  );
}
