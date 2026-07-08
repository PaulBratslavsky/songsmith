import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "../ipc/api";
import { FieldChat } from "./FieldChat";

export type PromptData = { stylePrompt: string; taggedLyrics: string; notes: string };

export function parsePrompt(content: string): PromptData {
  let data: any = null, text = "";
  try { const v = JSON.parse(content); text = v?.text ?? ""; data = v?.data ?? null; } catch { text = content; }
  const block = (re: RegExp) => { const m = text.match(re); return m ? m[1].trim() : ""; };
  return {
    stylePrompt: data?.stylePrompt ?? data?.style_prompt ?? block(/##?\s*STYLE PROMPT\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    taggedLyrics: data?.taggedLyrics ?? data?.tagged_lyrics ?? block(/##?\s*TAGGED LYRICS\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
    notes: data?.notes ?? block(/##?\s*NOTES\s*\n([\s\S]*?)(?:\n##?\s|\n*$)/i),
  };
}

export function promptToMarkdown(d: PromptData): string {
  return [
    "## STYLE PROMPT", d.stylePrompt, "",
    "## TAGGED LYRICS", d.taggedLyrics, "",
    "## NOTES", d.notes,
  ].join("\n");
}

// compare the WORDS that get sung — ignore chord-tag spelling/quality differences
// (Bb vs A#, Am vs Am(add9)) so only a real lyric-text mismatch warns.
const wordsOnly = (s: string) => s.replace(/\[[^\]]*\]/g, "").replace(/\s+/g, " ").trim().toLowerCase();

export function PromptEditor({
  songId, stageId, kind, content, onChanged, lyricsTagged,
}: {
  songId: string; stageId: string; kind: string; content: string; onChanged: () => void;
  /** the Lyrics stage's tagged text — the source of truth for the tagged lyrics field */
  lyricsTagged?: string;
}) {
  const [d, setD] = useState<PromptData>(() => parsePrompt(content));
  const [saved, setSaved] = useState("");
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
          <label style={{ margin: 0 }}>Style prompt <span className="faint">→ paste into the generator's STYLE box (genre, mood, instrumentation, vocal, mix, key, tempo · no chords)</span></label>
          <FieldChat stageLabel="Generation Prompt" fieldLabel="style prompt" current={d.stylePrompt} onResult={(v) => set({ stylePrompt: v })} />
        </div>
        <textarea value={d.stylePrompt} onChange={(e) => set({ stylePrompt: e.target.value })} style={{ width: "100%", minHeight: 50 }} />
      </div>
      <div>
        <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
          <label style={{ margin: 0 }}>Tagged lyrics <span className="faint">→ paste into the generator's LYRICS box ([Section] + inline [Chord] tags, from the Lyrics stage)</span></label>
          <div className="row" style={{ gap: 6, alignItems: "center" }}>
            {lyr && <button className={"sm" + (lyricsDiffer ? " primary" : " ghost")} title="replace with the exact lyrics from the Lyrics stage" onClick={pullFromLyrics}>↺ Pull from Lyrics</button>}
            <FieldChat stageLabel="Generation Prompt" fieldLabel="tagged lyrics" current={d.taggedLyrics} onResult={(v) => set({ taggedLyrics: v })} />
          </div>
        </div>
        {lyricsDiffer && (
          <div className="banner warn" style={{ marginBottom: 6 }}>
            ⚠ These don't match your <b>Lyrics</b> stage. The generator should sing your actual lyrics — click <b>↺ Pull from Lyrics</b> to sync, then Save.
          </div>
        )}
        <textarea value={d.taggedLyrics} onChange={(e) => set({ taggedLyrics: e.target.value })} spellCheck={false} style={{ width: "100%", minHeight: 220, fontFamily: "var(--mono)", fontSize: 12 }} />
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
