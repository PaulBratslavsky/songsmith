import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "../ipc/api";
import type { Section as SpineSection } from "../ipc/generated";
import { FieldChat } from "./FieldChat";
import { stripTags } from "../lib/music/chordpro";
import { normLabel } from "../lib/sections";

export type PromptData = { vocalPrompt: string; stylePrompt: string; taggedLyrics: string; instrumentalTags: string; notes: string };

export function parsePrompt(content: string): PromptData {
  let data: any = null, text = "";
  try { const v = JSON.parse(content); text = v?.text ?? ""; data = v?.data ?? null; } catch { text = content; }
  const block = (re: RegExp) => { const m = text.match(re); return m ? m[1].trim() : ""; };
  return {
    vocalPrompt: data?.vocalPrompt ?? block(/##?\s*VOCAL PROMPT\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    stylePrompt: data?.stylePrompt ?? data?.style_prompt ?? block(/##?\s*STYLE PROMPT\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    taggedLyrics: data?.taggedLyrics ?? data?.tagged_lyrics ?? block(/##?\s*TAGGED LYRICS\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    instrumentalTags: data?.instrumentalTags ?? block(/##?\s*INSTRUMENTAL TAGS\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    notes: data?.notes ?? block(/##?\s*NOTES\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
  };
}

export function promptToMarkdown(d: PromptData): string {
  return [
    ...(d.vocalPrompt ? ["## VOCAL PROMPT", d.vocalPrompt, ""] : []),
    "## STYLE PROMPT", d.stylePrompt, "",
    "## TAGGED LYRICS", d.taggedLyrics, "",
    ...(d.instrumentalTags ? ["## INSTRUMENTAL TAGS", d.instrumentalTags, ""] : []),
    "## NOTES", d.notes,
  ].join("\n");
}

/** The instrumental generator paste: every section as [Label] + its chord run +
 *  an arrangement cue from the spine's role — no sung words. Deterministic from
 *  the Chords stage + spine, so it can always be rebuilt with one click. */
export function buildInstrumentalTags(spine: readonly SpineSection[], chordsData: any): string {
  const chordSecs: any[] = chordsData?.sections ?? [];
  const rows = spine.length
    ? spine.map((r) => ({ id: r.id as string | undefined, label: r.label, role: r.role }))
    : chordSecs.map((s) => ({ id: s.section_id as string | undefined, label: String(s.label ?? "Section"), role: "" }));
  const out: string[] = [];
  for (const r of rows) {
    const cs = chordSecs.find((s) => s.section_id && s.section_id === r.id)
      ?? chordSecs.find((s) => normLabel(String(s.label ?? "")) === normLabel(r.label));
    out.push(`[${r.label}]`);
    const chords: any[] = cs?.chords ?? [];
    if (chords.length) out.push(chords.map((c) => `[${c?.name ?? c}]`).join(" "));
    const cue = (r.role || "").replace(/\s+/g, " ").trim();
    if (cue) out.push(`[${cue}]`);
    out.push("");
  }
  return out.join("\n").trim();
}

// compare the WORDS that get sung — ignore chord-tag spelling/quality differences
// (Bb vs A#, Am vs Am(add9)) so only a real lyric-text mismatch warns.
const wordsOnly = (s: string) => stripTags(s).replace(/\s+/g, " ").trim().toLowerCase();

/** A bracket tag that is a CHORD (not a section header or a delivery cue).
 *  Deliberately strict — quality tokens are spelled out, so [Bridge] /
 *  [Chorus] / [Bass] / [Female Vocal] can never be mistaken for chords even
 *  though they start with note letters. */
const CHORD_TAG = /^[A-G](?:#|b)?(?:maj|min|dim|aug|sus|add|m|M|\+|°|ø)?\d*(?:\((?:[#b]?\d+|add\d+|sus\d+|maj\d+|no\d+)\))*(?:\/[A-G](?:#|b)?)?$/;

/** The tagged lyrics with every CHORD tag removed — section headers and
 *  delivery cues stay. Lines that were only chords (an intro chord run) drop
 *  out entirely. Derived live from the sung tab, so it can never drift. */
export function stripChordTags(text: string): string {
  const out: string[] = [];
  for (const line of text.split("\n")) {
    const tags = [...line.matchAll(/\[([^\]]*)\]/g)];
    const chordTags = tags.filter((m) => CHORD_TAG.test(m[1].trim()));
    if (!chordTags.length) { out.push(line); continue; }
    // a line of ONLY chord tags (+ whitespace) is a chord run — drop it
    const withoutTags = line.replace(/\[[^\]]*\]/g, "").trim();
    if (!withoutTags && chordTags.length === tags.length) continue;
    let stripped = line;
    for (const m of chordTags) stripped = stripped.replace(m[0], "");
    out.push(stripped.replace(/[ \t]{2,}/g, " ").trimEnd());
  }
  // collapse the blank-line runs a dropped chord line can leave behind
  return out.filter((l, i) => l.trim() || (out[i - 1] ?? "").trim()).join("\n").trim();
}

export function PromptEditor({
  songId, stageId, kind, content, onChanged, lyricsTagged, spineSections, chordsData,
}: {
  songId: string; stageId: string; kind: string; content: string; onChanged: () => void;
  /** the Lyrics stage's tagged text — the source of truth for the tagged lyrics field */
  lyricsTagged?: string;
  /** the song's section spine + Chords stage data — sources for the Instrumental tab */
  spineSections?: SpineSection[];
  chordsData?: any;
}) {
  const [d, setD] = useState<PromptData>(() => parsePrompt(content));
  const [saved, setSaved] = useState("");
  const [tagTab, setTagTab] = useState<"sung" | "words" | "instrumental">("sung");
  const buildInstr = () => buildInstrumentalTags(spineSections ?? [], chordsData);
  // the Words-only tab is DERIVED from the sung tab — never stored, so it
  // can't drift out of sync with the lyrics
  const wordsOnlyText = useMemo(() => stripChordTags(d.taggedLyrics), [d.taggedLyrics]);
  // first visit to the Instrumental tab seeds it from Chords + Structure
  const openInstrumental = () => {
    setTagTab("instrumental");
    if (!d.instrumentalTags.trim()) set({ instrumentalTags: buildInstr() });
  };
  const set = (patch: Partial<PromptData>) => { setD((c) => ({ ...c, ...patch })); setSaved(""); };
  // the tagged lyrics should mirror the Lyrics stage verbatim; flag drift + offer a one-click pull
  const lyr = (lyricsTagged ?? "").trim();
  const lyricsDiffer = !!lyr && wordsOnly(d.taggedLyrics) !== wordsOnly(lyr);
  const pullFromLyrics = () => set({ taggedLyrics: lyr });
  // auto-mirror: when the Lyrics stage loads and the words differ, sync once so the
  // generator always sings the real lyrics (the prompt can never silently drift).
  const mirrored = useRef(false);
  useEffect(() => {
    if (!mirrored.current && lyr && wordsOnly(d.taggedLyrics) !== wordsOnly(lyr)) { mirrored.current = true; set({ taggedLyrics: lyr }); }
  }, [lyr]); // eslint-disable-line react-hooks/exhaustive-deps
  const save = useMutation({
    mutationFn: () => api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text: promptToMarkdown(d), data: d })),
    onSuccess: () => { setSaved("Saved."); onChanged(); },
  });
  return (
    <div className="col" style={{ gap: 12 }}>
      <div>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
          <label style={{ margin: 0 }}>Vocal prompt <span className="faint">the singer — texture, gender/register, delivery, mic/production, imperfections · pasted FIRST (the generator front-loads it)</span></label>
          <FieldChat stageLabel="Generation Prompt" fieldLabel="vocal prompt" current={d.vocalPrompt} onResult={(v) => set({ vocalPrompt: v })} />
        </div>
        <textarea value={d.vocalPrompt} onChange={(e) => set({ vocalPrompt: e.target.value })} placeholder="e.g. breathy female alto, whisper-sung intimate delivery, dry close-mic, occasional voice cracks, no autotune" style={{ width: "100%", minHeight: 40 }} />
      </div>
      <div>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
          <label style={{ margin: 0 }}>Style prompt <span className="faint">the sonic world — genre, mood, instrumentation, mix, key, tempo · no chords, no vocal (that's above)</span></label>
          <div className="row" style={{ gap: 6, alignItems: "center" }}>
            {(() => {
              const combined = [d.vocalPrompt, d.stylePrompt].map((s) => s.trim()).filter(Boolean).join(", ");
              const over = combined.length > 1000;
              return (
                <button
                  className={"sm" + (over ? "" : " ghost")}
                  title={over ? "over the 1,000-character style-box limit (v4.5+) — trim before pasting" : "copy vocal + style combined, vocal first — the exact STYLE box paste"}
                  onClick={() => navigator.clipboard.writeText(combined)}
                >
                  ⧉ Copy STYLE box {over ? `(${combined.length}/1000 ⚠)` : `(${combined.length}/1000)`}
                </button>
              );
            })()}
            <FieldChat stageLabel="Generation Prompt" fieldLabel="style prompt" current={d.stylePrompt} onResult={(v) => set({ stylePrompt: v })} />
          </div>
        </div>
        <textarea value={d.stylePrompt} onChange={(e) => set({ stylePrompt: e.target.value })} style={{ width: "100%", minHeight: 50 }} />
      </div>
      <div>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
          <div className="row" style={{ gap: 8, alignItems: "flex-end" }}>
            <div className="row" style={{ gap: 4 }}>
              <button className={"sm" + (tagTab === "sung" ? " primary" : "")} onClick={() => setTagTab("sung")} title="the sung song: lyrics with [Section] headers and inline [Chord] tags">Song + chords</button>
              <button className={"sm" + (tagTab === "words" ? " primary" : "")} onClick={() => setTagTab("words")} title="the same lyrics with the chord tags stripped — for generators that sing the chord names, or when you want the melody left free">Words only</button>
              <button className={"sm" + (tagTab === "instrumental" ? " primary" : "")} onClick={openInstrumental}>Instrumental</button>
            </div>
            <label style={{ margin: 0 }}>
              {tagTab === "sung"
                ? <>Tagged lyrics <span className="faint">→ paste into the generator's LYRICS box ([Section] + inline [Chord] tags, from the Lyrics stage)</span></>
                : tagTab === "words"
                ? <>Words only <span className="faint">→ the sung lyrics with every [Chord] tag stripped ([Section] headers + delivery cues stay) · a live view of the Sung tab, nothing to save</span></>
                : <>Instrumental tags <span className="faint">→ paste into the generator's LYRICS box for an instrumental take ([Section] + [Chord] runs + arrangement cues, no words)</span></>}
            </label>
          </div>
          <div className="row" style={{ gap: 6, alignItems: "center" }}>
            {tagTab === "sung" && lyr && <button className={"sm" + (lyricsDiffer ? " primary" : " ghost")} title="replace with the exact lyrics from the Lyrics stage" onClick={pullFromLyrics}>↺ Pull from Lyrics</button>}
            {tagTab === "words" && <button className="sm primary" title="copy the chord-free lyrics — paste straight into the generator's LYRICS box" onClick={() => navigator.clipboard.writeText(wordsOnlyText)}>⧉ Copy words</button>}
            {tagTab === "instrumental" && <button className="sm ghost" title="rebuild from the Chords stage + section roles" onClick={() => set({ instrumentalTags: buildInstr() })}>↺ Build from Chords + Structure</button>}
            {tagTab !== "words" && (
              <FieldChat
                stageLabel="Generation Prompt"
                fieldLabel={tagTab === "sung" ? "tagged lyrics" : "instrumental tags"}
                current={tagTab === "sung" ? d.taggedLyrics : d.instrumentalTags}
                onResult={(v) => set(tagTab === "sung" ? { taggedLyrics: v } : { instrumentalTags: v })}
              />
            )}
          </div>
        </div>
        {tagTab === "sung" && lyricsDiffer && (
          <div className="banner warn" style={{ marginBottom: 6 }}>
            ⚠ These don't match your <b>Lyrics</b> stage. The generator should sing your actual lyrics — click <b>↺ Pull from Lyrics</b> to sync, then Save.
          </div>
        )}
        {tagTab === "sung" ? (
          <textarea value={d.taggedLyrics} onChange={(e) => set({ taggedLyrics: e.target.value })} spellCheck={false} style={{ width: "100%", minHeight: 220, fontFamily: "var(--mono)", fontSize: 12 }} />
        ) : tagTab === "words" ? (
          <textarea value={wordsOnlyText} readOnly spellCheck={false} title="a live view of the Sung tab with chords stripped — edit the words on the Sung tab" style={{ width: "100%", minHeight: 220, fontFamily: "var(--mono)", fontSize: 12, opacity: 0.92 }} />
        ) : (
          <textarea value={d.instrumentalTags} onChange={(e) => set({ instrumentalTags: e.target.value })} spellCheck={false} style={{ width: "100%", minHeight: 220, fontFamily: "var(--mono)", fontSize: 12 }} />
        )}
      </div>
      <div>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
          <label style={{ margin: 0 }}>Notes <span className="faint">for YOU, not the generator — key/tempo/energy arc reference while producing</span></label>
          <FieldChat stageLabel="Generation Prompt" fieldLabel="notes" current={d.notes} onResult={(v) => set({ notes: v })} />
        </div>
        <textarea value={d.notes} onChange={(e) => set({ notes: e.target.value })} style={{ width: "100%", minHeight: 50 }} />
      </div>
      <div className="row" style={{ gap: 8 }}>
        <button className="primary sm" disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? "saving…" : "Save prompt"}</button>
        {saved && <span className="faint">{saved}</span>}
      </div>
    </div>
  );
}
