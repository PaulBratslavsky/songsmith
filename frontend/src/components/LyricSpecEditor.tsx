import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { api } from "../ipc/api";
import type { Section as SpineSection } from "../ipc/generated";
import { FieldChat } from "./FieldChat";
import { normLabel } from "../lib/sections";

type Beat = { section_id?: string; section: string; beat: string; frozen?: boolean };
export type Diction = "plain-spoken" | "balanced" | "literary";
export type SpecData = { hook: string; premise: string; pov: string; setting: string; arc: string; diction: Diction; referenceVibe: string; beats: Beat[]; imageBank: string[]; avoid: string[] };

const splitList = (s: string) => s.split(/\s*·\s*|\n/).map((x) => x.trim()).filter(Boolean);
const DICTIONS: Diction[] = ["plain-spoken", "balanced", "literary"];

export function parseSpec(content: string): SpecData {
  let data: any = null;
  try { const v = JSON.parse(content); data = v?.data ?? (typeof v === "object" && "hook" in v ? v : null); } catch { /* raw */ }
  return {
    hook: data?.hook ?? "",
    premise: data?.premise ?? "",
    pov: data?.pov ?? "",
    setting: data?.setting ?? "",
    arc: data?.arc ?? "",
    diction: DICTIONS.includes(data?.diction) ? data.diction : "balanced",
    referenceVibe: data?.referenceVibe ?? "",
    beats: Array.isArray(data?.beats)
      ? data.beats.map((b: any) => ({
          // spine link (docs/SECTION-SPINE-SPEC.md) — kept through saves
          ...(typeof b.section_id === "string" && b.section_id ? { section_id: b.section_id } : {}),
          section: b.section ?? b.label ?? "",
          beat: b.beat ?? b.text ?? "",
        }))
      : [],
    imageBank: Array.isArray(data?.imageBank) ? data.imageBank : [],
    avoid: Array.isArray(data?.avoid) ? data.avoid : [],
  };
}

export function specToMarkdown(d: SpecData): string {
  const out = [
    `**HOOK:** ${d.hook}`,
    `**PREMISE:** ${d.premise}`,
    `**POV / TENSE:** ${d.pov}`,
    `**SETTING:** ${d.setting}`,
    `**ARC:** ${d.arc}`,
    `**DICTION:** ${d.diction} (${d.diction === "plain-spoken" ? "conversational, almost no metaphor" : d.diction === "literary" ? "dense, poetic, image-rich" : "mostly plain with a few sharp images"})`,
    d.referenceVibe ? `**REFERENCE VIBE:** ${d.referenceVibe}` : "",
    "",
    "**SONG MAP (beat sheet)**",
    ...d.beats.map((b) => `- ${b.section}: ${b.beat}`),
    "",
    `**IMAGE BANK:** ${d.imageBank.join(" · ")}`,
    `**AVOID:** ${d.avoid.join(" · ")}`,
  ];
  return out.join("\n");
}

/** Phase 2 (docs/SECTION-SPINE-SPEC.md): show the beat sheet in SPINE order
 *  when the song has spine rows — each beat matched by section_id first, then
 *  by normalized label; unmatched beats keep their original relative order,
 *  appended after. [] (legacy / no spine) leaves the order untouched. */
export function orderBeatsBySpine(beats: Beat[], spine: readonly SpineSection[]): Beat[] {
  if (!spine.length) return beats;
  const used = new Set<number>();
  const out: Beat[] = [];
  for (const row of spine) {
    const i = beats.findIndex((b, j) => !used.has(j) && (b.section_id ? b.section_id === row.id : normLabel(b.section) === normLabel(row.label)));
    if (i >= 0) { used.add(i); out.push(beats[i]); }
  }
  beats.forEach((b, j) => { if (!used.has(j)) out.push(b); });
  return out;
}

export function LyricSpecEditor({
  songId, stageId, kind, content, onChanged, spineSections,
}: {
  songId: string; stageId: string; kind: string; content: string; onChanged: () => void;
  /** the song's section spine — orders the beat sheet when non-empty */
  spineSections?: SpineSection[];
}) {
  const [d, setD] = useState<SpecData>(() => {
    const parsed = parseSpec(content);
    return { ...parsed, beats: orderBeatsBySpine(parsed.beats, spineSections ?? []) };
  });
  const [saved, setSaved] = useState("");
  const set = (patch: Partial<SpecData>) => { setD((c) => ({ ...c, ...patch })); setSaved(""); };
  const setBeat = (i: number, patch: Partial<Beat>) => set({ beats: d.beats.map((b, j) => (j === i ? { ...b, ...patch } : b)) });
  const addBeat = () => set({ beats: [...d.beats, { section: "Section", beat: "" }] });
  const rmBeat = (i: number) => set({ beats: d.beats.filter((_, j) => j !== i) });

  const save = useMutation({
    mutationFn: () => api.saveArtifact(songId, stageId, kind, JSON.stringify({ kind, text: specToMarkdown(d), data: d })),
    onSuccess: () => { setSaved("Saved — the Lyricist writes from this plan."); onChanged(); },
  });

  const field = (label: string, key: keyof SpecData, hint?: string, area = false) => (
    <div>
      <div className="row" style={{ justifyContent: "space-between", alignItems: "flex-end" }}>
        <label style={{ margin: 0 }}>{label} {hint && <span className="faint">{hint}</span>}</label>
        <FieldChat stageLabel="Lyric Spec" fieldLabel={label} current={d[key] as string} onResult={(v) => set({ [key]: v } as any)} />
      </div>
      {area
        ? <textarea value={d[key] as string} onChange={(e) => set({ [key]: e.target.value } as any)} style={{ width: "100%", minHeight: 44 }} />
        : <input value={d[key] as string} onChange={(e) => set({ [key]: e.target.value } as any)} style={{ width: "100%" }} />}
    </div>
  );

  return (
    <div className="col" style={{ gap: 12 }}>
      <p className="faint" style={{ margin: 0 }}>The songwriter's plan — lock the hook, POV, arc, and a beat per section before writing. The Lyricist writes from this.</p>
      {field("Hook", "hook", "(the central repeated line)")}
      {field("Premise", "premise", "(what it's really about, one sentence)", true)}
      {field("POV / tense", "pov", "(who sings, to whom, what tense)")}
      {field("Setting", "setting", "(where & when)")}
      {field("Arc", "arc", "(starts → lands)")}

      <div>
        <label>Diction <span className="faint">(how plain or poetic the words are)</span></label>
        <div className="row" style={{ gap: 4 }}>
          {DICTIONS.map((dc) => (
            <button key={dc} className={"sm" + (d.diction === dc ? " primary" : "")} onClick={() => set({ diction: dc })}>{dc}</button>
          ))}
          <span className="faint" style={{ fontSize: 11, alignSelf: "center", marginLeft: 4 }}>
            {d.diction === "plain-spoken" ? "conversational — almost no metaphor" : d.diction === "literary" ? "dense, poetic, image-rich" : "mostly plain with a few sharp images"}
          </span>
        </div>
      </div>
      {field("Reference vibe", "referenceVibe", "(describe the FEEL to aim for — not another song's words)", true)}

      <div>
        <label>Song map <span className="faint">(one beat per section — what it says, how it moves)</span></label>
        <div className="col" style={{ gap: 6 }}>
          {d.beats.map((b, i) => (
            <div key={i} className="row" style={{ gap: 6, alignItems: "flex-start" }}>
              <input value={b.section} onChange={(e) => setBeat(i, { section: e.target.value })} placeholder="Verse 1" style={{ width: 130, fontWeight: 600 }} />
              <textarea value={b.beat} onChange={(e) => setBeat(i, { beat: e.target.value })} placeholder="what this section says…" style={{ flex: 1, minHeight: 38 }} />
              <button className="sm ghost" title="remove" onClick={() => rmBeat(i)}>×</button>
            </div>
          ))}
        </div>
        <button className="sm" style={{ marginTop: 6 }} onClick={addBeat}>+ beat</button>
      </div>

      <div>
        <label>Image bank <span className="faint">(concrete details to pull from · separate with ·)</span></label>
        <textarea value={d.imageBank.join(" · ")} onChange={(e) => set({ imageBank: splitList(e.target.value) })} style={{ width: "100%", minHeight: 44 }} />
      </div>
      <div>
        <label>Avoid <span className="faint">(clichés / words to steer clear of · separate with ·)</span></label>
        <textarea value={d.avoid.join(" · ")} onChange={(e) => set({ avoid: splitList(e.target.value) })} style={{ width: "100%", minHeight: 38 }} />
      </div>

      <div className="row" style={{ gap: 8 }}>
        <button className="primary sm" disabled={save.isPending} onClick={() => save.mutate()}>{save.isPending ? "saving…" : "Save spec"}</button>
        {saved && <span className="faint">{saved}</span>}
      </div>
    </div>
  );
}
