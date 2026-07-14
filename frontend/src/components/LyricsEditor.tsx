import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api, type ParsedLyrics } from "../ipc/api";
import type { Section as SpineSection } from "../ipc/generated";
import { FieldChat } from "./FieldChat";
import { parseArtifact, type ChordsData } from "../lib/artifacts";
import { matchBySpineRow, normLabel } from "../lib/sections";
import {
  parseLine as parseChordProLine,
  toLine as lineToChordPro,
  spreadChords as autoPlaceSection,
  type Word,
} from "../lib/music/chordpro";

/** @deprecated compat re-export — import from lib/music/chordpro instead. */
export { parseLine as parseChordProLine } from "../lib/music/chordpro";

// ChordPro model (see lib/music/chordpro): a lyric line is a sequence of words,
// each optionally carrying a chord that lands on its first syllable. Stored
// back as inline "[C]word" text so the chord stays anchored to its word.
// `section_id` links the section to its spine row (docs/SECTION-SPINE-SPEC.md)
// and is carried through every save.
type Section = { section_id?: string; label: string; lines: Word[][]; frozen?: boolean };

/** The words this stage holds, keyed by section label (parsed from inline ChordPro). */
function wordsByLabel(content: string): Record<string, Word[][]> {
  const { text, data } = parseArtifact("lyrics", content);
  const out: Record<string, Word[][]> = {};
  if (data?.sections.length) {
    for (const s of data.sections) {
      out[s.label || "Section"] = s.lines.map(parseChordProLine);
    }
  } else if (text) {
    out["Lyrics"] = text.split("\n").map(parseChordProLine);
  }
  return out;
}

/** Each section's chord progression (from the Chords stage), keyed by label. */
function progressionsByLabel(chordsData: ChordsData | null | undefined): Record<string, string[]> {
  const map: Record<string, string[]> = {};
  for (const s of chordsData?.sections ?? []) {
    if (!s.label) continue;
    map[s.label] = s.chords.map((c) => c.name).filter(Boolean);
  }
  return map;
}

/** Section identity & order: the song's section SPINE when it has rows
 *  (docs/SECTION-SPINE-SPEC.md, Phase 2 — this stage's words attach by
 *  section_id, label fallback), else the Chords stage's labels exactly as
 *  before; this stage only fills in words either way. Sections that exist
 *  only in the artifacts (legacy songs / the Phase-2 write window) are kept,
 *  appended after. A sung section with no saved chord placements gets the
 *  SAME derived placement the Sheet shows (auto-spread the progression), so
 *  the editor never looks empty and matches the export. */
/** Which of this stage's own sections are frozen (locked), keyed by label. */
function frozenByLabel(content: string): Record<string, boolean> {
  const { data } = parseArtifact("lyrics", content);
  const out: Record<string, boolean> = {};
  for (const s of data?.sections ?? []) {
    if (s.frozen === true) out[s.label || "Section"] = true;
  }
  return out;
}

function buildSections(content: string, chordsData: ChordsData | null | undefined, spine: readonly SpineSection[] = []): Section[] {
  const byLabel = wordsByLabel(content);
  const frozen = frozenByLabel(content);
  const prog = progressionsByLabel(chordsData);
  const out: Section[] = [];
  const seen = new Set<string>();
  const add = (label: string, lines: Word[][], isFrozen?: boolean, sectionId?: string) => {
    const hasWords = lines.some((l) => l.some((w) => w.text.trim()));
    const hasChords = lines.some((l) => l.some((w) => w.chord));
    if (hasWords && !hasChords && prog[label]?.length) autoPlaceSection(lines, prog[label]);
    out.push({ ...(sectionId ? { section_id: sectionId } : {}), label, lines, ...(isFrozen ?? frozen[label] ? { frozen: true } : {}) });
  };

  if (spine.length) {
    const { data } = parseArtifact("lyrics", content);
    const entries = (data?.sections ?? []).map((s) => ({
      section_id: s.section_id,
      label: s.label || "Section",
      lines: s.lines.map(parseChordProLine),
      frozen: s.frozen === true,
    }));
    // plain-text lyrics (no structured sections) keep their one "Lyrics" block
    if (!entries.length && byLabel["Lyrics"]) {
      entries.push({ section_id: undefined, label: "Lyrics", lines: byLabel["Lyrics"], frozen: false });
    }
    const usedE = new Set<(typeof entries)[number]>();
    const usedC = new Set<NonNullable<ChordsData["sections"]>[number]>();
    for (const row of spine) {
      const e = matchBySpineRow(entries, row, usedE);
      if (e) usedE.add(e);
      // consume the matching chords section too, so leftovers don't re-append it
      const c = matchBySpineRow(chordsData?.sections ?? [], row, usedC);
      if (c) usedC.add(c);
      seen.add(normLabel(row.label));
      add(row.label, e?.lines ?? [], e?.frozen, row.id);
    }
    // Phase-2 window: artifact sections not (yet) in the spine still render —
    // chords-stage sections first (empty words), then lyric-only ones.
    for (const cs of chordsData?.sections ?? []) {
      if (usedC.has(cs) || !cs.label || seen.has(normLabel(cs.label))) continue;
      seen.add(normLabel(cs.label));
      const e = entries.find((x) => !usedE.has(x) && normLabel(x.label) === normLabel(cs.label));
      if (e) usedE.add(e);
      add(cs.label, e?.lines ?? [], e?.frozen, e?.section_id);
    }
    for (const e of entries) {
      if (usedE.has(e) || seen.has(normLabel(e.label))) continue;
      seen.add(normLabel(e.label));
      add(e.label, e.lines, e.frozen, e.section_id);
    }
    return out.length ? out : [{ label: "Lyrics", lines: [[]] }];
  }

  // legacy (no spine): the Chords stage defines identity/order, unchanged
  for (const cs of chordsData?.sections ?? []) {
    const label = cs.label;
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

/** Paste-lyrics modal: paste raw text → live parsed preview (words kept
 *  VERBATIM — deterministic header split; Claude only ever marks boundaries on
 *  unlabeled text, validated line-by-line) → confirm imports it as the Lyrics
 *  artifact and back-fills the Structure stage's section list to match. */
export function PasteLyricsModal({ songId, hasFrozen, onClose, onImported }: {
  songId: string; hasFrozen: boolean; onClose: () => void; onImported: () => void;
}) {
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<ParsedLyrics | null>(null);
  const [parsing, setParsing] = useState(false);

  // live preview — debounce the dry-run parse while the user pastes/types
  useEffect(() => {
    if (!text.trim()) { setPreview(null); setParsing(false); return; }
    setParsing(true);
    const t = setTimeout(() => {
      api.parsePastedLyrics(text)
        .then((p) => { setPreview(p); setParsing(false); })
        .catch(() => { setPreview(null); setParsing(false); });
    }, 400);
    return () => clearTimeout(t);
  }, [text]);

  const doImport = useMutation({
    mutationFn: () => api.importLyrics(songId, text),
    onSuccess: () => { onImported(); onClose(); },
  });

  return (
    <div className="modal-bg" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 720, maxWidth: "92vw" }}>
        <h2>📋 Paste lyrics</h2>
        <p className="muted">
          Drop in finished lyrics — they're parsed into sections but the words are kept <b>verbatim</b>, never rewritten.
          Importing replaces this song's Lyrics and back-fills the Structure section list to match (Concept is untouched).
        </p>
        <div className="row" style={{ gap: 12, alignItems: "stretch" }}>
          <textarea
            value={text} onChange={(e) => setText(e.target.value)} autoFocus
            placeholder={"[Verse 1]\nCity lights are calling me home…\n\n[Chorus]\n…\n\n(headers optional — unlabeled text gets segmented, words untouched)"}
            style={{ flex: 1, minHeight: 260, fontFamily: "var(--mono)", fontSize: 12 }}
          />
          <div style={{ flex: 1, minHeight: 260, maxHeight: 380, overflowY: "auto", border: "1px solid var(--line)", borderRadius: 2, padding: 8 }}>
            {!text.trim() ? (
              <span className="faint">The parsed preview appears here.</span>
            ) : parsing ? (
              <span className="faint">parsing…</span>
            ) : !preview ? (
              <span className="faint">Could not parse — importing would keep everything as one section.</span>
            ) : (
              <>
                <div className="row" style={{ gap: 6, marginBottom: 6, flexWrap: "wrap" }}>
                  <span className="badge">{preview.sections.length} section{preview.sections.length === 1 ? "" : "s"}</span>
                  {preview.used_claude && <span className="badge" title="the text had no section headers, so Claude marked the boundaries — every line was validated verbatim against your paste">🤖 Claude segmented — words verbatim</span>}
                </div>
                {preview.sections.map((s, i) => (
                  <div key={i} style={{ marginBottom: 8 }}>
                    <b style={{ fontSize: 12 }}>[{s.label}]</b>
                    <div style={{ fontFamily: "var(--mono)", fontSize: 11, whiteSpace: "pre-wrap" }}>
                      {s.lines.length ? s.lines.join("\n") : <span className="faint">(instrumental — no lines)</span>}
                    </div>
                  </div>
                ))}
              </>
            )}
          </div>
        </div>
        {hasFrozen && (
          <div className="banner warn" style={{ marginTop: 10 }}>
            ⚠️ This song's Lyrics has 🔒 locked sections — importing replaces <b>everything, including locked sections</b>.
          </div>
        )}
        {doImport.isError && <div className="banner warn" style={{ marginTop: 10 }}>Import failed: {String((doImport.error as any)?.message ?? doImport.error)}</div>}
        <div className="row" style={{ marginTop: 14, justifyContent: "flex-end", gap: 8 }}>
          <button className="ghost" onClick={onClose}>Cancel</button>
          <button className="primary" disabled={!text.trim() || doImport.isPending} onClick={() => doImport.mutate()}>
            {doImport.isPending ? "Importing…" : "Import lyrics"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function LyricsEditor({
  songId, stageId, kind, artifactId, content, onChanged, chordsData, spineSections,
}: {
  songId: string; stageId: string; kind: string; artifactId: string; content: string; onChanged: () => void;
  /** the Chords stage data — drives each section's chord palette */
  chordsData?: ChordsData | null;
  /** the song's section spine (identity/order/labels when non-empty; [] = legacy label derivation) */
  spineSections?: SpineSection[];
}) {
  const spine = spineSections ?? [];
  const [sections, setSections] = useState<Section[]>(() => buildSections(content, chordsData, spine));
  const [dirty, setDirty] = useState(false);
  const [mode, setMode] = useState<"place" | "text">("place");
  const [sel, setSel] = useState<string>(""); // currently-armed chord to place
  const [custom, setCustom] = useState("");
  const [pasteOpen, setPasteOpen] = useState(false);
  // this stage's own frozen sections — the paste modal warns before replacing them
  const hasFrozen = useMemo(() => Object.keys(frozenByLabel(content)).length > 0, [content]);

  // per-section chord palette from the Chords stage (label-matched), plus all-song fallback
  const paletteBySection = useMemo(() => progressionsByLabel(chordsData), [chordsData]);
  // rebuild the section list when a new revision loads, the Chords stage's
  // sections change, or the song's spine rows load/change
  const chordsSig = useMemo(() => (chordsData?.sections ?? []).map((s) => `${s.label}:${s.chords.length}`).join("|"), [chordsData]);
  const spineSig = useMemo(() => spine.map((s) => `${s.id}:${s.label}`).join("|"), [spine]);
  useEffect(() => { if (!dirty) setSections(buildSections(content, chordsData, spine)); }, [artifactId, chordsSig, spineSig]); // eslint-disable-line react-hooks/exhaustive-deps
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
  const setSectionText = (i: number, text: string) => mutate((s) => { s[i].lines = text.split("\n").map(parseChordProLine); return s; });

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
    s[si].lines = text.split("\n").map(parseChordProLine);
    autoPlaceSection(s[si].lines, paletteFor(s[si].label));
    return s;
  });

  const save = useMutation({
    mutationFn: () => {
      // entries carry their spine section_id (docs/SECTION-SPINE-SPEC.md,
      // Phase 3) alongside the label so renames can't detach the words
      const data = { sections: sections.map((s) => ({ ...(s.section_id ? { section_id: s.section_id } : {}), label: s.label, lines: s.lines.map(lineToChordPro), ...(s.frozen ? { frozen: true } : {}) })) };
      const text = sections.map((s) => `[${s.label}]\n${s.lines.map(lineToChordPro).join("\n")}`).join("\n\n");
      return api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text, data }));
    },
    onSuccess: () => { setDirty(false); onChanged(); },
  });

  // self-test + refine: the model critiques its own lyrics (title lands as hook,
  // sections coherent, no clichés) and rewrites them as a new revision
  const selfCheck = useMutation({
    mutationFn: () => api.selfCheckStage(stageId),
    onSuccess: () => { setDirty(false); onChanged(); },
  });

  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 8 }}>
        <div className="row" style={{ gap: 6 }}>
          <button className={"sm" + (mode === "place" ? " primary" : "")} onClick={() => setMode("place")} title="click words to place chords above them">🎵 Place chords</button>
          <button className={"sm" + (mode === "text" ? " primary" : "")} onClick={() => setMode("text")} title="edit raw ChordPro: [C]word">✎ Text</button>
          {mode === "place" && <button className="sm" onClick={autoPlaceAll} title="spread each section's progression across its lyrics as a starting draft — then nudge">⚡ Auto-place</button>}
          <button className="sm" disabled={selfCheck.isPending || dirty} onClick={() => selfCheck.mutate()}
            title={dirty ? "save your edits first" : "Claude self-tests the lyrics (title lands as the hook, sections coherent, no clichés) and rewrites them as a new revision"}>
            {selfCheck.isPending ? "checking…" : "✓ Self-check & refine"}
          </button>
          <button className="sm" onClick={() => setPasteOpen(true)}
            title="paste finished lyrics — parsed into sections with the words kept verbatim; replaces this stage and back-fills Structure to match">
            📋 Paste lyrics
          </button>
        </div>
        <button className="sm primary" disabled={!dirty || save.isPending} onClick={() => save.mutate()}>
          {save.isPending ? "saving…" : dirty ? "save revision" : "saved"}
        </button>
      </div>

      <p className="faint" style={{ fontSize: 11, margin: "0 0 6px" }}>🔒 Locked sections are kept as-is when you regenerate this stage.</p>

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
              {sec.frozen && <span className="badge done" title="locked — kept as-is when you regenerate this stage">locked</span>}
            </div>
            <div className="row" style={{ gap: 6, alignItems: "center" }}>
              <button className={"sm ghost" + (sec.frozen ? " primary" : "")} title={sec.frozen ? "unlock — let regeneration rewrite this section" : "lock — keep this section as-is when you regenerate"} onClick={() => mutate((s) => { s[si].frozen = !s[si].frozen; return s; })}>{sec.frozen ? "🔒" : "🔓"}</button>
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
              <div className="row" style={{ gap: 5, flexWrap: "wrap", marginBottom: 8 }}>
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
        {spine.length ? (
          <>Sections &amp; their order come from the song's <b>section spine</b> — edit them in the <b>Structure</b> stage. This stage just adds the words.</>
        ) : (
          <>Sections &amp; their order come from the <b>Chords</b> stage — add, rename, and drag to reorder them there. This stage just adds the words.</>
        )}
      </p>

      {pasteOpen && (
        <PasteLyricsModal
          songId={songId}
          hasFrozen={hasFrozen}
          onClose={() => setPasteOpen(false)}
          onImported={() => { setDirty(false); onChanged(); }}
        />
      )}
    </div>
  );
}
