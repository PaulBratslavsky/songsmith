import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { FieldChat } from "./FieldChat";

// ChordPro model: a lyric line is a sequence of words, each optionally carrying
// a chord that lands on its first syllable. Stored back as inline "[C]word" text
// so the chord is anchored to the word and stays aligned when lyrics are edited.
type Word = { text: string; chord?: string };
type Section = { label: string; lines: Word[][] };

// Preserve chord names exactly as authored — do NOT auto-sharpen. Flats are often
// the musically-correct spelling (e.g. Bb = bII Neapolitan in A minor), and the
// AI's qualities (Am(add9), "Bb (ghost)") carry intent. Mangling them also caused
// false "doesn't match Lyrics" warnings in the Generation Prompt.
const TOKEN_RE = /\[([^\]]+)\]|(\S+)/g;
/** Parse a ChordPro line ("[Dm]The dashboard [Bb]glows") into words+chords. */
function parseLine(s: string): Word[] {
  const words: Word[] = [];
  let pending: string | undefined;
  let m: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((m = TOKEN_RE.exec(s))) {
    if (m[1] != null) pending = m[1].trim();
    else { words.push({ text: m[2], chord: pending }); pending = undefined; }
  }
  if (pending) words.push({ text: "", chord: pending }); // trailing chord, no word
  return words;
}
/** Serialise words back to a ChordPro line. */
function lineToChordPro(words: Word[]): string {
  return words.map((w) => (w.chord ? `[${w.chord}]` : "") + w.text).join(" ").trim();
}

/** Spread a section's chord progression across its lyric lines as a first draft:
 *  chords are split across non-empty lines proportionally, then dropped on
 *  evenly-spaced words within each line. Mutates the lines in place. */
function autoPlaceSection(lines: Word[][], progression: string[]): void {
  for (const line of lines) for (const w of line) w.chord = undefined; // start clean
  if (!progression.length) return;
  const neIdx = lines.map((l, i) => (l.some((w) => w.text) ? i : -1)).filter((i) => i >= 0);
  const neCount = neIdx.length;
  if (!neCount) return;
  const perLine: string[][] = Array.from({ length: neCount }, () => []);
  progression.forEach((name, j) => { perLine[Math.min(neCount - 1, Math.floor((j * neCount) / progression.length))].push(name); });
  neIdx.forEach((li, k) => {
    const line = lines[li];
    const my = perLine[k];
    const wordIdx = line.map((w, i) => (w.text ? i : -1)).filter((i) => i >= 0);
    if (!wordIdx.length) return;
    my.forEach((name, m) => {
      const at = wordIdx[my.length === 1 ? 0 : Math.min(wordIdx.length - 1, Math.round((m * (wordIdx.length - 1)) / (my.length - 1)))];
      line[at].chord = name;
    });
  });
}

function parse(content: string): { text: string; data: any } {
  try {
    const v = JSON.parse(content);
    if (v && typeof v === "object" && "text" in v) return { text: v.text ?? "", data: v.data ?? null };
    return { text: content, data: v };
  } catch {
    return { text: content, data: null };
  }
}

/** The words this stage holds, keyed by section label (parsed from inline ChordPro). */
function wordsByLabel(content: string): Record<string, Word[][]> {
  const { text, data } = parse(content);
  const out: Record<string, Word[][]> = {};
  const secs = data?.sections;
  if (Array.isArray(secs) && secs.length) {
    for (const s of secs) {
      const label = s.label || s.type || "Section";
      const raw: string[] = Array.isArray(s.lines) ? s.lines : typeof s.text === "string" ? s.text.split("\n") : [];
      out[label] = raw.map(parseLine);
    }
  } else if (text) {
    out["Lyrics"] = text.split("\n").map(parseLine);
  }
  return out;
}

/** Each section's chord progression (from the Chords stage), keyed by label. */
function progressionsByLabel(chordsData: any): Record<string, string[]> {
  const map: Record<string, string[]> = {};
  for (const s of chordsData?.sections ?? []) {
    const lbl = s.label || s.type; if (!lbl) continue;
    map[lbl] = (s.chords ?? []).map((c: any) => (typeof c === "string" ? c : c?.name)).filter(Boolean);
  }
  return map;
}

/** Section spine = the Chords stage (labels + order); this stage only fills in words.
 *  Sections that exist only in the lyrics (legacy songs) are kept, appended after.
 *  A sung section with no saved chord placements gets the SAME derived placement the
 *  Sheet shows (auto-spread the progression), so the editor never looks empty and
 *  matches the export — without needing a manual "auto-place + save". */
function buildSections(content: string, chordsData: any): Section[] {
  const byLabel = wordsByLabel(content);
  const prog = progressionsByLabel(chordsData);
  const out: Section[] = [];
  const seen = new Set<string>();
  const add = (label: string, lines: Word[][]) => {
    const hasWords = lines.some((l) => l.some((w) => w.text.trim()));
    const hasChords = lines.some((l) => l.some((w) => w.chord));
    if (hasWords && !hasChords && prog[label]?.length) autoPlaceSection(lines, prog[label]);
    out.push({ label, lines });
  };
  for (const cs of chordsData?.sections ?? []) {
    const label = cs.label || cs.type;
    if (!label || seen.has(label)) continue;
    seen.add(label);
    add(label, byLabel[label] ?? []);
  }
  for (const label of Object.keys(byLabel)) {
    if (seen.has(label)) continue;
    seen.add(label);
    add(label, byLabel[label]);
  }
  return out.length ? out : [{ label: "Lyrics", lines: [[]] }];
}

export function LyricsEditor({
  songId, stageId, kind, artifactId, content, onChanged, chordsData,
}: {
  songId: string; stageId: string; kind: string; artifactId: string; content: string; onChanged: () => void;
  /** the Chords stage data — drives each section's chord palette */
  chordsData?: any;
}) {
  const [sections, setSections] = useState<Section[]>(() => buildSections(content, chordsData));
  const [dirty, setDirty] = useState(false);
  const [mode, setMode] = useState<"place" | "text">("place");
  const [sel, setSel] = useState<string>(""); // currently-armed chord to place
  const [custom, setCustom] = useState("");

  // per-section chord palette from the Chords stage (label-matched), plus all-song fallback
  const paletteBySection = useMemo(() => {
    const map: Record<string, string[]> = {};
    for (const s of chordsData?.sections ?? []) {
      const lbl = s.label || s.type; if (!lbl) continue;
      map[lbl] = (s.chords ?? []).map((c: any) => (typeof c === "string" ? c : c?.name)).filter(Boolean);
    }
    return map;
  }, [chordsData]);
  // rebuild the spine when a new revision loads or the Chords stage's sections change
  const chordsSig = useMemo(() => (chordsData?.sections ?? []).map((s: any) => `${s.label || s.type}:${(s.chords ?? []).length}`).join("|"), [chordsData]);
  useEffect(() => { if (!dirty) setSections(buildSections(content, chordsData)); }, [artifactId, chordsSig]); // eslint-disable-line react-hooks/exhaustive-deps
  const allChords = useMemo(() => {
    const set: string[] = [];
    Object.values(paletteBySection).flat().forEach((c) => { if (!set.includes(c)) set.push(c); });
    return set;
  }, [paletteBySection]);
  const paletteFor = (label: string) => (paletteBySection[label]?.length ? paletteBySection[label] : allChords);

  const mutate = (fn: (s: Section[]) => Section[]) => { setSections((cur) => fn(structuredClone(cur))); setDirty(true); };

  // a section is instrumental when it carries no sung words (chords-only / empty)
  const isInstrumental = (sec: Section) => sec.lines.every((line) => line.every((w) => !w.text.trim()));
  // its chord bars: prefer chords already in the section, else the Chords-stage progression
  const barsFor = (sec: Section) => {
    const own = sec.lines.flat().map((w) => w.chord).filter(Boolean) as string[];
    return own.length ? own : paletteFor(sec.label);
  };
  const setSectionText = (i: number, text: string) => mutate((s) => { s[i].lines = text.split("\n").map(parseLine); return s; });

  // place/clear the armed chord on a word (click word → set; click its chord → clear)
  const toggleAt = (si: number, li: number, wi: number) => mutate((s) => {
    const w = s[si].lines[li][wi];
    if (w.chord) w.chord = w.chord === sel || !sel ? undefined : sel; // has chord → replace with armed, or clear
    else if (sel) w.chord = sel;
    return s;
  });
  const clearAt = (si: number, li: number, wi: number) => mutate((s) => { s[si].lines[li][wi].chord = undefined; return s; });
  const autoPlace = (si: number) => mutate((s) => { autoPlaceSection(s[si].lines, paletteFor(s[si].label)); return s; });
  const autoPlaceAll = () => mutate((s) => { s.forEach((sec) => autoPlaceSection(sec.lines, paletteFor(sec.label))); return s; });

  // a section's words as plain text (no chord tags) — what the 💬 refine edits
  const sectionPlain = (sec: Section) => sec.lines.map((line) => line.map((w) => w.text).join(" ").replace(/\s+/g, " ").trim()).join("\n");
  // apply a refined/edited block of words back to the section, then re-place chords
  const applyRefine = (si: number, text: string) => mutate((s) => {
    s[si].lines = text.split("\n").map(parseLine);
    autoPlaceSection(s[si].lines, paletteFor(s[si].label));
    return s;
  });

  const save = useMutation({
    mutationFn: () => {
      const data = { sections: sections.map((s) => ({ label: s.label, lines: s.lines.map(lineToChordPro) })) };
      const text = sections.map((s) => `[${s.label}]\n${s.lines.map(lineToChordPro).join("\n")}`).join("\n\n");
      return api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text, data }));
    },
    onSuccess: () => { setDirty(false); onChanged(); },
  });

  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 8 }}>
        <div className="row" style={{ gap: 6 }}>
          <button className={"sm" + (mode === "place" ? " primary" : "")} onClick={() => setMode("place")} title="click words to place chords above them">🎵 Place chords</button>
          <button className={"sm" + (mode === "text" ? " primary" : "")} onClick={() => setMode("text")} title="edit raw ChordPro: [C]word">✎ Text</button>
          {mode === "place" && <button className="sm" onClick={autoPlaceAll} title="spread each section's progression across its lyrics as a starting draft — then nudge">⚡ Auto-place</button>}
        </div>
        <button className="sm primary" disabled={!dirty || save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? "saving…" : dirty ? "save revision" : "saved"}
        </button>
      </div>

      {mode === "place" && (
        <p className="faint" style={{ margin: "0 0 8px" }}>
          Pick a chord, then click the word it lands on — it pins above that word and stays aligned on the Sheet. Click a placed chord to remove it.
        </p>
      )}

      {sections.map((sec, si) => (
        <div key={si} className="card" style={{ marginBottom: 8 }}>
          <div className="row" style={{ justifyContent: "space-between", marginBottom: 6 }}>
            <div className="row" style={{ gap: 8, alignItems: "center" }}>
              <b>{sec.label}</b>
              {isInstrumental(sec) && <span className="badge" title="no sung words — plays as an instrumental">🎸 instrumental</span>}
            </div>
            <div className="row" style={{ gap: 6, alignItems: "center" }}>
              {mode === "place" && !isInstrumental(sec) && <button className="sm ghost" onClick={() => autoPlace(si)} title="spread this section's progression across its lyrics">⚡ auto-place</button>}
              <FieldChat stageLabel="Lyrics" fieldLabel={isInstrumental(sec) ? `${sec.label} (write lyrics)` : sec.label} current={sectionPlain(sec)} onResult={(t) => applyRefine(si, t)} />
            </div>
          </div>

          {mode === "place" && isInstrumental(sec) ? (
            <div className="cp-instrumental">
              <div className="cp-bars">{barsFor(sec).length ? "| " + barsFor(sec).join(" | ") + " |" : "— no chords yet · add them in the Chords stage"}</div>
              <span className="faint" style={{ fontSize: 11 }}>
                No vocal — plays as a solo / break / drop. <b>Edit its chords in the Chords stage.</b> To make it sung, switch to <b>✎ Text</b> and type words.
              </span>
            </div>
          ) : mode === "text" ? (
            <textarea
              value={sec.lines.map(lineToChordPro).join("\n")}
              onChange={(e) => setSectionText(si, e.target.value)}
              style={{ minHeight: 110, fontFamily: "var(--mono)", fontSize: 12 }}
              placeholder="[Dm]The dashboard [Bb]glows…  (inline [chord] tags, one line per line)"
            />
          ) : (
            <>
              <div className="row cp-palette" style={{ gap: 5, flexWrap: "wrap", marginBottom: 8 }}>
                {paletteFor(sec.label).map((c) => (
                  <button key={c} className={"sm" + (sel === c ? " primary" : "")} onClick={() => setSel((p) => (p === c ? "" : c))}>{c}</button>
                ))}
                {paletteFor(sec.label).length === 0 && <span className="faint" style={{ fontSize: 11 }}>no chords yet — run the Chords stage, or type one →</span>}
                <input
                  value={custom} onChange={(e) => setCustom(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter" && custom.trim()) { setSel(custom.trim()); setCustom(""); } }}
                  placeholder="+chord ⏎" style={{ width: 78, fontSize: 11 }}
                />
                {sel && <span className="faint" style={{ fontSize: 11 }}>armed: <b>{sel}</b> — click a word</span>}
              </div>
              <div className="cp-lyrics">
                {sec.lines.map((line, li) => (
                  <div key={li} className="cp-line">
                    {line.length === 0 ? (
                      <span className="faint" style={{ fontSize: 12 }}>·</span>
                    ) : line.map((w, wi) => (
                      <span key={wi} className="cp-word">
                        <button
                          className={"cp-chord" + (w.chord ? " set" : "") + (sel ? " armed" : "")}
                          title={w.chord ? "click to remove" : sel ? `place ${sel}` : "arm a chord first"}
                          onClick={() => (w.chord ? clearAt(si, li, wi) : toggleAt(si, li, wi))}
                        >{w.chord || ""}</button>
                        <span className="cp-text" onClick={() => toggleAt(si, li, wi)}>{w.text || "—"}</span>
                      </span>
                    ))}
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      ))}

      <p className="faint" style={{ fontSize: 11, marginTop: 4 }}>
        Sections &amp; their order come from the <b>Chords</b> stage — add, rename, and drag to reorder them there. This stage just adds the words.
      </p>
    </div>
  );
}
