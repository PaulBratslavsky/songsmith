// In-memory mock of the Rust tool registry, for running the UI in a plain
// browser (no Tauri). Every registry command must exist here (mock-parity).

import { STAGE_ORDER, type CommandArgs, type CommandMap, type CommandResult } from "./api";
import type { Artifact, CompositionMeta, Section, Song, Stage } from "./generated";

type Any = Record<string, any>;
const uid = () => Math.random().toString(36).slice(2, 10);
const now = () => new Date().toISOString();
const KEY = "songsmith-mock-v1";

function load(): Any {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) return JSON.parse(raw);
  } catch {}
  return seed();
}
function save(db: Any) {
  try {
    localStorage.setItem(KEY, JSON.stringify(db));
  } catch {}
}

function seed(): Any {
  const ts = now();
  const skills = [
    ["songsmith-concept", "Song Concept", "concept"],
    ["songsmith-structure", "Song Structure", "structure"],
    ["songsmith-chords", "Chord Progressions", "chords"],
    ["songsmith-lyrics", "Lyricist", "lyrics"],
    ["songsmith-prompt", "Generation Prompt", "prompt"],
    ["songsmith-style", "Style Builder", "style"],
  ].map(([key, name, stage_type]) => ({
    id: uid(), key, name, stage_type,
    instructions: `(${name}) — built-in skill. Edit me in the Skills page.`,
    source: "builtin", enabled: true, created_at: ts, updated_at: ts,
  }));
  const presetId = uid();
  // a demo song so the Sheet / play-along view has real data to render
  const songId = uid();
  const song = {
    id: songId, style_preset_id: presetId, title: "Cyber Dreams", intent: "", status: "in_progress",
    current_stage: "prompt", key_root: "A", key_mode: "minor", bpm: 120, voicings: "{}", created_at: ts, updated_at: ts,
  };
  const order = ["concept", "structure", "chords", "lyric_spec", "lyrics", "prompt"];
  const stages = order.map((type, ordinal) => ({
    id: uid(), song_id: songId, type, ordinal,
    status: ordinal <= 5 ? "done" : "pending", skill_id: null, created_at: ts, updated_at: ts,
  }));
  const stageOf = (t: string) => stages.find((s) => s.type === t)!;
  const artifact = (stageType: string, kind: string, data: Any) => ({
    id: uid(), song_id: songId, stage_id: stageOf(stageType).id, kind,
    content: JSON.stringify({ kind, text: "", data }), version: 1, approved: true, created_at: ts,
  });
  // the demo song's SECTION SPINE (docs/SECTION-SPINE-SPEC.md) — mirrors what
  // the core migration would build: chords-artifact sections in order (the
  // song has no structure artifact), plus the lyrics-only Bridge unioned last.
  // Seeding it lets the browser demo exercise the Phase-2 spine readers.
  const sections = ["Intro", "Verse 1", "Pre-Chorus / Build 1", "Chorus 1", "Bridge"].map((label, position) => ({
    id: uid(), song_id: songId, position, label, type: "", bars: 8, role: "", created_at: ts, updated_at: ts,
  }));
  const sectionIdOf = (label: string) => sections.find((s) => s.label === label)!.id;
  // chords carry explicit per-chord beats so the Composer lays them out at
  // the right widths (full-song export). Strings still work (default 4 beats).
  // The Chords stage lists each progression ONCE; the LYRICS below cycle it
  // (Chorus 1: 4 chords, 8 placements) — the lyric-sheet v3 repro: the
  // Composer must lay one chord span per sung placement, not per entry.
  // Each section entry carries its spine `section_id` (as the migration
  // attaches them) so id-based content matching is exercised too.
  const ch = (name: string, beats = 4) => ({ name, beats });
  const chordsData = {
    sections: [
      { section_id: sectionIdOf("Intro"), label: "Intro", chords: [ch("Am"), ch("Am"), ch("F"), ch("F")] },
      { section_id: sectionIdOf("Verse 1"), label: "Verse 1", chords: [ch("Dm"), ch("Bb"), ch("F"), ch("Am")] },
      { section_id: sectionIdOf("Pre-Chorus / Build 1"), label: "Pre-Chorus / Build 1", chords: [ch("Dm", 2), ch("Em", 2), ch("F", 2), ch("G", 2)] },
      { section_id: sectionIdOf("Chorus 1"), label: "Chorus 1", chords: [ch("C"), ch("G"), ch("Am"), ch("F")] },
    ],
  };
  const taggedLyrics = [
    "[Intro]",
    "",
    "[Verse 1]",
    "[Dm]The dashboard [Bb]glows a color that the [F]daylight never [Am]had",
    "[Dm]Black glass and a [Bb]low hum, and the [F]city breathing [Am]back",
    "[Dm]I don't ask where [Bb]I'm going — I just [F]follow how it [Am]shines",
    "",
    "[Pre-Chorus / Build 1]",
    "[Dm]Hold the wheel a [Em]little tighter, [F]neon in my [G]eyes",
    "",
    "[Chorus 1]",
    "[C]I dream in [G]synthwave, I [Am]dream in neon and [F]rain",
    "[C]Let the highway [G]hold me, let the [Am]cold light learn my [F]name",
    "",
    "[Bridge]", // no matching chords-artifact section — shapes come from the tags
    "[Dm]And the static [Em]starts to feel like a [F]hand",
    "[G]take me under, one more time",
  ].join("\n");
  // lyrics artifact derived from the tagged lyrics, KEEPING the inline
  // ChordPro [chord] tags — the user-placed word-level placements are the
  // ground truth the Sheet and the Composer (lyric sheet v3) lay from.
  const lyricsSections = (() => {
    const out: Any[] = []; let cur: Any | null = null;
    for (const line of taggedLyrics.split("\n")) {
      const t = line.trim(); const hm = t.match(/^\[([^\]]+)\]$/);
      if (hm) { cur = { section_id: sectionIdOf(hm[1]), label: hm[1], lines: [] }; out.push(cur); }
      else if (cur && t) cur.lines.push(line);
    }
    return out;
  })();
  // one saved Composer sketch so the library panel isn't empty on first open
  // (the blob is a valid v3 Composition — parseStoredComposition loads it)
  const seedComposition = {
    id: "seed-comp", version: 3, name: "Neon idea", key: { root: "A", mode: "minor" }, bpm: 112,
    bars: 8, totalTicks: 128,
    chords: [
      { id: "c1", degree: 1, seventh: false, start: 0, length: 16 },
      { id: "c2", degree: 6, seventh: false, start: 16, length: 16 },
      { id: "c3", degree: 3, seventh: false, start: 32, length: 16 },
      { id: "c4", degree: 7, seventh: false, start: 48, length: 16 },
    ],
    melody: [
      { id: "m1", degree: 5, octave: 0, start: 0, length: 8 },
      { id: "m2", degree: 4, octave: 0, start: 8, length: 8 },
      { id: "m3", degree: 3, octave: 0, start: 16, length: 16 },
    ],
    bass: [{ id: "b1", degree: 1, octave: 0, start: 0, length: 16 }],
    sections: [], lyrics: [],
  };
  return {
    presets: [
      {
        id: presetId, name: "Night Drive", genre: "synthwave", mood: "moody, propulsive",
        influences: "80s film scores, neon-noir", key_tempo_feel: "A minor, ~120 BPM",
        vocal_range: "mid baritone", themes: "motion, loneliness, the open road",
        lyric_exemplars: "",
        created_at: ts, updated_at: ts,
      },
    ],
    songs: [song], stages, sections,
    artifacts: [
      artifact("chords", "chords", chordsData),
      artifact("lyric_spec", "lyric_spec", {
        hook: "Cyber Dreams", premise: "chasing a feeling you can only reach at full speed on an empty highway",
        pov: "first person, present tense", setting: "a neon highway at 3am, dashboard glowing",
        arc: "restless and numb → wide awake and free",
        diction: "balanced", referenceVibe: "late-night, neon-lit, propulsive but lonely",
        beats: [
          { section_id: sectionIdOf("Verse 1"), section: "Verse 1", beat: "set the scene — the dashboard, the empty road, the restlessness" },
          { section_id: sectionIdOf("Chorus 1"), section: "Chorus 1", beat: "the release — dreaming in neon, finally feeling alive" },
        ],
        imageBank: ["dashboard glow", "cold glass", "tail lights", "static hum", "white lines"],
        avoid: ["chasing dreams", "fading light", "lost in time"],
      }),
      artifact("lyrics", "lyrics", { sections: lyricsSections }),
      artifact("prompt", "generation_prompt", { taggedLyrics }),
    ],
    skills, progressions: [], renders: [],
    compositions: [{ id: uid(), name: "Neon idea", song_id: null, data: JSON.stringify(seedComposition), created_at: ts, updated_at: ts }],
    settings: { claude_model: "", claude_bin: "", ableton_mcp: "", music_folder: "", analyzer_cmd: "" },
    // mock claude.ai subscription auth — starts signed in so the card looks real
    auth: { logged_in: true, account: "you@claude.ai", subscription: "Claude Pro" },
  };
}

let db = load();

// Mirror of core/src/db.rs `parse_key_tempo`: seed a new song's key/BPM from the
// preset's prose `key_tempo_feel`. First explicit key mention wins (uppercase
// note + major/maj/minor/min word, so "in a minor key" never reads as A minor);
// "~135–145 BPM" → rounded midpoint; unparseable → the A-minor/120 defaults.
function parseKeyTempo(feel: string): { root: string; mode: string; bpm: number } {
  const f = feel ?? "";
  let root = "A", mode = "minor", bpm = 120;
  const keyRe = /(?:^|[^A-Za-z0-9])([A-G][#b♯♭]?)[\s-]*([A-Za-z]+)/g;
  for (let m = keyRe.exec(f); m; m = keyRe.exec(f)) {
    const w = m[2].toLowerCase();
    if (w === "major" || w === "maj" || w === "minor" || w === "min") {
      root = m[1].replace("♯", "#").replace("♭", "b");
      mode = w.startsWith("maj") ? "major" : "minor";
      break;
    }
  }
  const range = /(\d+)\s*[-–—−]\s*(\d+)\s*bpm\b/i.exec(f);
  const single = /(\d+)\s*bpm\b/i.exec(f);
  const v = range ? Math.round((Number(range[1]) + Number(range[2])) / 2) : single ? Number(single[1]) : NaN;
  if (v >= 20 && v <= 300) bpm = v;
  return { root, mode, bpm };
}
function presetKeyTempo(presetId: string): { root: string; mode: string; bpm: number } {
  const preset = db.presets.find((p: Any) => p.id === presetId);
  return parseKeyTempo(preset?.key_tempo_feel ?? "");
}

const KINDS: Record<string, string> = {
  concept: "concept", structure: "structure", chords: "chords", lyric_spec: "lyric_spec", lyrics: "lyrics", prompt: "generation_prompt",
};
function currentArtifact(stageId: string) {
  return db.artifacts.filter((a: Any) => a.stage_id === stageId).sort((a: Any, b: Any) => b.version - a.version)[0] ?? null;
}
// mirror the SQL subquery: attach the current artifact's timestamp to each stage
function withArtifactAt(stage: Any) {
  return { ...stage, artifact_at: currentArtifact(stage.id)?.created_at ?? null };
}
function activeSkill(stageType: string) {
  return db.skills.find((s: Any) => s.stage_type === stageType && s.enabled) ?? null;
}
function advance(songId: string) {
  const stages = db.stages.filter((s: Any) => s.song_id === songId).sort((a: Any, b: Any) => a.ordinal - b.ordinal);
  // First stage needing ATTENTION: not done, or done-but-STALE (an earlier
  // stage has a newer artifact) — mirrors core tools::advance_song.
  const artifactAt = (st: Any) => {
    const revs = db.artifacts.filter((x: Any) => x.stage_id === st.id).sort((x: Any, y: Any) => y.version - x.version);
    return revs[0]?.created_at ?? null;
  };
  let newestUpstream: string | null = null;
  let next: Any | null = null;
  for (const st of stages) {
    const at = artifactAt(st);
    const stale = !!at && !!newestUpstream && at < newestUpstream;
    if (!next && (st.status !== "done" || stale)) next = st;
    if (at && (!newestUpstream || at > newestUpstream)) newestUpstream = at;
  }
  next = next ?? stages[stages.length - 1];
  const v = db.songs.find((x: Any) => x.id === songId);
  if (v && next) { v.current_stage = next.type; v.updated_at = now(); }
  return { current_stage: next?.type };
}

// ---- Paste-lyrics import (mock = the deterministic header split only) -------
// Mirrors core/src/agent.rs: `[Verse 1]` / Suno `[verse]` bracket headers and
// `Verse 1:` line-style headers; words are NEVER altered; no headers → one
// "Lyrics" section (the browser mock never calls Claude → used_claude:false).

const HEADER_WORDS = [
  "verse","chorus","prechorus","pre","post","postchorus","bridge","intro","outro","hook",
  "refrain","drop","build","buildup","breakdown","break","interlude","instrumental","solo",
  "tag","coda","vamp","middle","part","section","ending",
];
function headerLabel(line: string): string | null {
  const t = line.trim();
  if (t.length >= 3 && t.startsWith("[") && t.endsWith("]")) {
    const inner = t.slice(1, -1).trim();
    if (inner && !inner.includes("[") && !inner.includes("]")) return inner;
  }
  if (t.endsWith(":")) {
    const name = t.slice(0, -1).trim();
    if (name && name.length <= 40 && !name.includes(":")) {
      const first = (name.match(/^[A-Za-z]+/)?.[0] ?? "").toLowerCase();
      if (HEADER_WORDS.includes(first)) return name;
    }
  }
  return null;
}
function trimBlankEdges(lines: string[]): string[] {
  const out = [...lines];
  while (out.length && !out[0].trim()) out.shift();
  while (out.length && !out[out.length - 1].trim()) out.pop();
  return out;
}
function parsePastedLyrics(text: string): { sections: { label: string; lines: string[] }[]; used_claude: boolean } {
  const sections: { label: string; lines: string[] }[] = [];
  let current: { label: string; lines: string[] } | null = null;
  const preamble: string[] = [];
  let found = false;
  for (const raw of text.split("\n")) {
    const line = raw.replace(/\r$/, "");
    const label = headerLabel(line);
    if (label != null) {
      found = true;
      if (current) sections.push({ label: current.label, lines: trimBlankEdges(current.lines) });
      current = { label, lines: [] };
    } else if (current) current.lines.push(line);
    else preamble.push(line);
  }
  if (!found) return { sections: [{ label: "Lyrics", lines: trimBlankEdges(text.split("\n").map((l) => l.replace(/\r$/, ""))) }], used_claude: false };
  if (current) sections.push({ label: current.label, lines: trimBlankEdges(current.lines) });
  const pre = trimBlankEdges(preamble);
  if (pre.length) sections.unshift({ label: "Lyrics", lines: pre });
  return { sections, used_claude: false };
}
const normLabel = (s: string) => s.trim().toLowerCase().split(/\s+/).join(" ");

// ---- Feature B2: chords back-fill + key inference from inline [chord] tags --
// Mirrors core/src/agent.rs (parse_chord_tag / line_chord_tags /
// collapse_progression / infer_key_from_tags).

/** Split a chord tag into root+quality, or null when it isn't chord-shaped. */
function parseChordTag(name: string): { root: string; quality: string } | null {
  const m = name.match(/^([A-G][#b]?)(\S*)$/);
  if (!m) return null;
  const quality = m[2];
  const ok = quality === "" || /^(m|dim|aug|sus|add|[0-9(/+])/.test(quality);
  return ok ? { root: m[1], quality } : null;
}
/** A lyric line's inline chord tags, in order (chord-shaped tags only). */
function lineChordTags(line: string): string[] {
  const tags: string[] = [];
  const re = /\[([^\]]+)\]/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(line))) {
    const name = m[1].trim();
    if (name && parseChordTag(name)) tags.push(name);
  }
  return tags;
}
/** Collapse a tag sequence to ONE progression pass when it repeats exactly. */
function collapseProgression(seq: string[]): string[] {
  for (let d = 1; d <= seq.length; d++) {
    if (seq.length % d === 0 && seq.every((x, i) => x === seq[i % d])) return seq.slice(0, d);
  }
  return seq;
}
/** Infer the key from the paste's tags: tonic = most frequent root (ties →
 *  first-seen), minor when the tonic's tags are predominantly minor. */
function inferKeyFromTags(tags: string[]): { root: string; mode: string } | null {
  const stats = new Map<string, { total: number; minor: number }>(); // insertion order = first-seen
  for (const t of tags) {
    const p = parseChordTag(t);
    if (!p) continue;
    const s = stats.get(p.root) ?? { total: 0, minor: 0 };
    s.total += 1;
    if (/^m/.test(p.quality) && !/^maj/.test(p.quality)) s.minor += 1;
    stats.set(p.root, s);
  }
  let best: string | null = null;
  for (const [root, s] of stats) if (best === null || s.total > stats.get(best)!.total) best = root; // strict > keeps the first on ties
  if (best === null) return null;
  const s = stats.get(best)!;
  return { root: best, mode: s.minor * 2 > s.total ? "minor" : "major" };
}
/** Replace the Lyrics artifact + back-fill Structure to the pasted sections (labels/order). */
function importLyricsIntoSong(songId: string, text: string) {
  const parsed = parsePastedLyrics(text);
  if (parsed.sections.every((s) => s.lines.every((l) => !l.trim()))) throw new Error("no lyrics to import — paste some text first");
  const song = db.songs.find((v: Any) => v.id === songId);
  if (!song) throw new Error("song not found");
  const stageOf = (t: string) => db.stages.find((s: Any) => s.song_id === songId && s.type === t);
  const lyricsStage = stageOf("lyrics"), structureStage = stageOf("structure");
  if (!lyricsStage || !structureStage) throw new Error("song has no lyrics/structure stage");
  const push = (stage: Any, kind: string, content: string) => {
    const ver = (currentArtifact(stage.id)?.version ?? 0) + 1;
    db.artifacts.push({ id: uid(), song_id: songId, stage_id: stage.id, kind, content, version: ver, approved: false, created_at: now() });
    stage.status = "done"; stage.updated_at = now();
  };
  // Lyrics — pasted words verbatim, editor-style text, no frozen flags carried
  const lyricsData = { sections: parsed.sections.map((s) => ({ label: s.label, lines: s.lines })) };
  const lyricsText = parsed.sections.map((s) => `[${s.label}]\n${s.lines.join("\n")}`).join("\n\n");
  push(lyricsStage, "lyrics", JSON.stringify({ kind: "lyrics", text: lyricsText, data: lyricsData }));
  // Structure — back-fill labels/order; keep bars/role (+ lock) on label match
  let prior: Any | null = null;
  try { prior = JSON.parse(currentArtifact(structureStage.id)?.content ?? "")?.data ?? null; } catch {}
  const priorSecs: Any[] = Array.isArray(prior?.sections) ? prior.sections : [];
  const sections = parsed.sections.map((p) => {
    const old = priorSecs.find((s) => normLabel(s.label ?? s.type ?? "") === normLabel(p.label));
    return old
      ? { type: old.type ?? "", label: p.label, bars: Number(old.bars ?? 8), role: old.role ?? "", ...(old.frozen ? { frozen: true } : {}) }
      : { type: "", label: p.label, bars: 8, role: "" };
  });
  // The SONG owns key/tempo (docs/SONG-FACTS.md) — no embedded copies; only
  // the prose notes carry over (mirrors core agent.rs apply_parsed_lyrics).
  const sData = { keyNote: prior?.keyNote ?? "", tempoNote: prior?.tempoNote ?? "", sections };
  push(structureStage, "structure", JSON.stringify({ kind: "structure", text: structureText(sData), data: sData }));
  // Chords back-fill from inline [chord] tags (Feature B2 #2) — mirrors agent.rs:
  // per section, the tag sequence collapsed to one progression pass, beats 4;
  // untagged sections empty; no tags anywhere → Chords untouched.
  const tagSeqs = parsed.sections.map((s) => s.lines.flatMap(lineChordTags));
  if (tagSeqs.some((t) => t.length)) {
    const chordsStage = stageOf("chords");
    if (chordsStage) {
      const cSecs = parsed.sections.map((p, i) => ({
        label: p.label,
        chords: collapseProgression(tagSeqs[i]).map((name) => ({ name, beats: 4 })),
      }));
      push(chordsStage, "chords", JSON.stringify({ kind: "chords", text: chordsText(cSecs), data: { sections: cSecs } }));
    }
  }
}

// ---- Composer export (composition → song) — mirrors core/src/agent.rs -------
// The sections arrive RESOLVED (label/bars/chords{name,beats}); 🔒 frozen
// chord/structure sections are skipped and preserved (re-inserted if dropped).

function mergeFrozen(priorSecs: Any[], newSecs: Any[]): Any[] {
  const frozen = priorSecs
    .map((s, i) => ({ i, s }))
    .filter(({ s }) => !!s.frozen)
    .map(({ i, s }) => ({ i, label: normLabel(s.label ?? s.type ?? ""), sec: { ...s, frozen: true } }));
  const out = [...newSecs];
  for (const f of frozen) {
    const pos = out.findIndex((s) => normLabel(s.label ?? s.type ?? "") === f.label);
    if (pos >= 0) out[pos] = f.sec;
    else out.splice(Math.min(f.i, out.length), 0, f.sec);
  }
  return out;
}
function chordsText(secs: Any[]): string {
  return secs.map((s) => `${s.label ?? "Section"}: ${(s.chords ?? []).map((c: Any) => (typeof c === "string" ? c : c.name)).join(" ")}`).join("\n");
}
// Mirror of StructureEditor.structureToMarkdown / core render.rs for the new
// data shape: no KEY/TEMPO fact lines (the SONG owns key/tempo — docs/SONG-FACTS.md).
function structureText(d: Any): string {
  const out: string[] = [];
  if (d.keyNote) out.push(`**KEY NOTE:** ${d.keyNote}`);
  if (d.tempoNote) out.push(`**TEMPO NOTE:** ${d.tempoNote}`);
  if (out.length) out.push("");
  out.push("**SECTION MAP**", "");
  out.push(...d.sections.map((s: Any, i: number) => `${i + 1}. **${s.label}** (${s.bars} bars)${s.role ? ` — ${s.role}` : ""}`));
  return out.join("\n");
}
/** Write resolved sections into a song's Chords + Structure stages. */
function exportSectionsIntoSong(songId: string, sections: Any[]): string[] {
  if (!Array.isArray(sections) || !sections.length) throw new Error("nothing to export — the composition has no sections");
  const song = db.songs.find((v: Any) => v.id === songId);
  if (!song) throw new Error("song not found");
  const stageOf = (t: string) => db.stages.find((s: Any) => s.song_id === songId && s.type === t);
  const chordsStage = stageOf("chords"), structureStage = stageOf("structure");
  if (!chordsStage || !structureStage) throw new Error("song has no chords/structure stage");
  const push = (stage: Any, kind: string, content: string) => {
    const ver = (currentArtifact(stage.id)?.version ?? 0) + 1;
    db.artifacts.push({ id: uid(), song_id: songId, stage_id: stage.id, kind, content, version: ver, approved: false, created_at: now() });
    stage.status = "done"; stage.updated_at = now();
  };
  // Chords: the export, with prior frozen sections spliced back verbatim
  let priorC: Any | null = null;
  try { priorC = JSON.parse(currentArtifact(chordsStage.id)?.content ?? "")?.data ?? null; } catch {}
  const priorCSecs: Any[] = Array.isArray(priorC?.sections) ? priorC.sections : [];
  const skipped = priorCSecs.filter((s) => !!s.frozen).map((s) => s.label ?? s.type ?? "");
  const newCSecs = sections.map((s) => ({ label: s.label, chords: (s.chords ?? []).map((c: Any) => ({ name: c.name, beats: Math.max(1, Number(c.beats) || 4) })) }));
  const cData = { sections: mergeFrozen(priorCSecs, newCSecs) };
  push(chordsStage, "chords", JSON.stringify({ kind: "chords", text: chordsText(cData.sections), data: cData }));
  // Structure: back-fill labels/order; bars/role/type (+ 🔒) preserved on match
  let priorS: Any | null = null;
  try { priorS = JSON.parse(currentArtifact(structureStage.id)?.content ?? "")?.data ?? null; } catch {}
  const priorSSecs: Any[] = Array.isArray(priorS?.sections) ? priorS.sections : [];
  const newSSecs = sections.map((p) => {
    const old = priorSSecs.find((s) => normLabel(s.label ?? s.type ?? "") === normLabel(p.label));
    return old
      ? { type: old.type ?? "", label: p.label, bars: Number(old.bars ?? Math.max(1, Number(p.bars) || 8)), role: old.role ?? "", ...(old.frozen ? { frozen: true } : {}) }
      : { type: "", label: p.label, bars: Math.max(1, Number(p.bars) || 8), role: "" };
  });
  // No embedded key/bpm — the SONG owns them (docs/SONG-FACTS.md); notes carry over.
  const sData = {
    keyNote: priorS?.keyNote ?? "", tempoNote: priorS?.tempoNote ?? "",
    sections: mergeFrozen(priorSSecs, newSSecs),
  };
  push(structureStage, "structure", JSON.stringify({ kind: "structure", text: structureText(sData), data: sData }));
  return skipped;
}

// The generated types carry i64 fields as `bigint` (Artifact.version,
// Song.bpm, Stage.ordinal). The mock stores plain numbers internally
// (localStorage is JSON), so convert at the reply boundary — the mock emits
// exactly what the generated types promise.
function toArtifact(art: Any): Artifact;
function toArtifact(art: Any | null): Artifact | null;
function toArtifact(art: Any | null) {
  // `label ?? null` migrates mock DBs persisted before revision labels existed
  return art ? ({ ...art, label: art.label ?? null, version: BigInt(art.version) } as Artifact) : null;
}
// `intent ?? ""` migrates mock DBs persisted before the North Star field existed
const toSong = (v: Any): Song => ({ ...v, intent: v.intent ?? "", bpm: BigInt(v.bpm) } as Song);
const toStage = (s: Any): Stage => ({ ...s, ordinal: BigInt(s.ordinal) } as Stage);
const toSection = (s: Any): Section => ({ ...s, position: BigInt(s.position), bars: BigInt(s.bars) } as Section);

// ---- Section spine (docs/SECTION-SPINE-SPEC.md — Phase 1) — mirrors db.rs ---
// `db.sections ??= []` back-fills mock DBs persisted before the spine existed.
function songSections(songId: string): Any[] {
  db.sections ??= [];
  return db.sections.filter((x: Any) => x.song_id === songId).sort((x: Any, y: Any) => x.position - y.position);
}

/** One handler per CommandMap command — a missing or mistyped handler is a
 *  COMPILE error (audit Tier-2 #7: api/mock parity by construction). */
type MockHandlers = {
  [C in keyof CommandMap]: (a: CommandArgs<C>) => CommandResult<C> | Promise<CommandResult<C>>;
};

const handlers: MockHandlers = {
  list_style_presets: () => db.presets,
  get_style_preset: (a) => db.presets.find((p: Any) => p.id === a.id) ?? null,
  create_style_preset: (a) => { const p = { id: uid(), ...a.input, created_at: now(), updated_at: now() }; db.presets.push(p); return p; },
  update_style_preset: (a) => { const p = db.presets.find((x: Any) => x.id === a.id); Object.assign(p, a.input, { updated_at: now() }); return p; },
  // lyric_exemplars stays empty on generate — the user's taste lever, never invented
  generate_style_preset: (a) => ({ name: a.name, genre: "(mock) genre", mood: "moody", influences: "describe the sound",
    key_tempo_feel: "A minor, 120 BPM", vocal_range: "mid", themes: `themes for ${a.name}`, lyric_exemplars: "" }),
  create_song: (a) => {
    const id = uid();
    const kt = presetKeyTempo(a.stylePresetId); // seed key/BPM from the preset's prose
    const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", intent: (a.intent ?? "").trim(), status: "in_progress",
      current_stage: "concept", key_root: kt.root, key_mode: kt.mode, bpm: kt.bpm, voicings: "{}", created_at: now(), updated_at: now() };
    db.songs.unshift(v);
    STAGE_ORDER.forEach((type, ordinal) =>
      db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
    return toSong(v);
  },
  list_songs: () => db.songs.map(toSong),
  get_song: (a) => {
    const song = db.songs.find((v: Any) => v.id === a.id);
    if (!song) return null;
    const preset = db.presets.find((p: Any) => p.id === song.style_preset_id);
    const stages = db.stages.filter((s: Any) => s.song_id === a.id).sort((x: Any, y: Any) => x.ordinal - y.ordinal).map(withArtifactAt).map(toStage);
    return { song: toSong(song), preset, stages };
  },
  update_song_status: (a) => { const v = db.songs.find((x: Any) => x.id === a.id); v.status = a.status; v.updated_at = now(); return toSong(v); },
  update_song_title: (a) => { const v = db.songs.find((x: Any) => x.id === a.id); v.title = a.title; v.updated_at = now(); return toSong(v); },
  update_song_intent: (a) => { const v = db.songs.find((x: Any) => x.id === a.id); v.intent = a.intent; v.updated_at = now(); return toSong(v); },
  update_song_key: (a) => { const v = db.songs.find((x: Any) => x.id === a.id); v.key_root = a.root; v.key_mode = a.mode; v.bpm = a.bpm; v.updated_at = now(); return toSong(v); },
  update_song_voicings: (a) => { const v = db.songs.find((x: Any) => x.id === a.id); v.voicings = a.voicings; v.updated_at = now(); return toSong(v); },
  // section spine (Phase 1 CRUD) — mirrors db.rs semantics
  list_sections: (a) => songSections(a.songId).map(toSection),
  create_section: (a) => {
    const rows = songSections(a.songId);
    const end = rows.length ? rows[rows.length - 1].position + 1 : 0;
    const pos = a.position == null ? end : Math.max(0, Math.min(a.position, end));
    rows.filter((x) => x.position >= pos).forEach((x) => (x.position += 1));
    const sec = { id: uid(), song_id: a.songId, position: pos, label: a.label, type: a.sectionType,
      bars: Math.max(1, a.bars), role: a.role, created_at: now(), updated_at: now() };
    db.sections.push(sec);
    return toSection(sec);
  },
  update_section: (a) => {
    const sec = (db.sections ?? []).find((x: Any) => x.id === a.id);
    if (!sec) throw new Error("section not found");
    Object.assign(sec, { label: a.label, type: a.sectionType, bars: Math.max(1, a.bars), role: a.role, updated_at: now() });
    return toSection(sec);
  },
  delete_section: (a) => {
    const sec = (db.sections ?? []).find((x: Any) => x.id === a.id);
    if (!sec) throw new Error("section not found");
    db.sections = db.sections.filter((x: Any) => x.id !== a.id);
    songSections(sec.song_id).filter((x) => x.position > sec.position).forEach((x) => (x.position -= 1));
  },
  reorder_sections: (a) => {
    const rows = songSections(a.songId);
    const have = new Set(rows.map((x) => x.id));
    if (a.sectionIds.length !== rows.length || a.sectionIds.some((id) => !have.has(id)) || new Set(a.sectionIds).size !== a.sectionIds.length)
      throw new Error(`reorder_sections needs every section id of the song exactly once (${rows.length} sections)`);
    a.sectionIds.forEach((id, pos) => {
      const sec = rows.find((x) => x.id === id)!;
      sec.position = pos; sec.updated_at = now();
    });
    return songSections(a.songId).map(toSection);
  },
  refine_field: (a) => `(mock) ${a.fieldLabel}: ${a.instruction}`,
  delete_song: (a) => {
    db.songs = db.songs.filter((v: Any) => v.id !== a.id);
    const sids = db.stages.filter((s: Any) => s.song_id === a.id).map((s: Any) => s.id);
    db.stages = db.stages.filter((s: Any) => s.song_id !== a.id);
    db.artifacts = db.artifacts.filter((ar: Any) => !sids.includes(ar.stage_id));
    db.sections = (db.sections ?? []).filter((x: Any) => x.song_id !== a.id);
  },
  get_stage: (a) => {
    const stage = db.stages.find((s: Any) => s.id === a.id);
    if (!stage) return null;
    return { stage: toStage(withArtifactAt(stage)), artifact: toArtifact(currentArtifact(a.id)), skill: activeSkill(stage.type) };
  },
  run_stage: (a) => {
    const stage = db.stages.find((s: Any) => s.id === a.stageId);
    // North Star parity: a Concept-stage seed becomes the song's intent (only when empty)
    if (stage.type === "concept" && a.userInput?.trim()) {
      const song = db.songs.find((v: Any) => v.id === stage.song_id);
      if (song && !(song.intent ?? "").trim()) { song.intent = a.userInput.trim(); song.updated_at = now(); }
    }
    stage.status = "in_progress";
    const kind = KINDS[stage.type] ?? "artifact";
    const text = `# ${kind} (mock)\n\nSimulated ${stage.type} for this song. Run in the Tauri app with Claude for real output.\n` +
      (a.userInput ? `\nYour seed:\n${a.userInput}\n` : "");
    const ver = (currentArtifact(a.stageId)?.version ?? 0) + 1;
    const art = { id: uid(), song_id: stage.song_id, stage_id: a.stageId, kind,
      content: JSON.stringify({ kind, text, data: null }), version: ver, approved: false, created_at: now() };
    db.artifacts.push(art);
    return toArtifact(art);
  },
  cancel_stage: () => undefined, // mock runs finish instantly — nothing to cancel
  approve_stage: (a) => {
    const stage = db.stages.find((s: Any) => s.id === a.stageId);
    const art = currentArtifact(a.stageId);
    if (art) art.approved = true;
    stage.status = "done";
    advance(stage.song_id);
    return { ok: true };
  },
  advance_stage: (a) => advance(a.songId),
  get_artifact: (a) => toArtifact(db.artifacts.find((x: Any) => x.id === a.id) ?? null),
  save_artifact: (a) => {
    const ver = (currentArtifact(a.stageId ?? "")?.version ?? 0) + 1;
    const art = { id: uid(), song_id: a.songId, stage_id: a.stageId ?? null, kind: a.kind, content: a.content, version: ver, approved: false, created_at: now() };
    db.artifacts.push(art);
    return toArtifact(art);
  },
  list_artifact_revisions: (a) =>
    db.artifacts.filter((x: Any) => x.stage_id === a.stageId).sort((x: Any, y: Any) => y.version - x.version).map(toArtifact),
  revert_artifact: (a) => {
    const src = db.artifacts.find((x: Any) => x.id === a.artifactId);
    const ver = (currentArtifact(src.stage_id)?.version ?? 0) + 1;
    // the restored revision starts unlabeled, like the core (label stays NULL)
    const art = { ...src, id: uid(), version: ver, approved: false, label: null, created_at: now() };
    db.artifacts.push(art);
    return toArtifact(art);
  },
  set_artifact_label: (a) => {
    const x = db.artifacts.find((y: Any) => y.id === a.artifactId);
    if (x) x.label = a.label;
  },
  list_skills: () => db.skills,
  get_skill: (a) => db.skills.find((s: Any) => s.id === a.id) ?? null,
  create_skill: (a) => { const s = { id: uid(), ...a.input, source: "user", enabled: true, created_at: now(), updated_at: now() }; db.skills.push(s); return s; },
  update_skill: (a) => { const s = db.skills.find((x: Any) => x.id === a.id); Object.assign(s, a.input, { updated_at: now() }); return s; },
  set_skill_enabled: (a) => { const s = db.skills.find((x: Any) => x.id === a.id); s.enabled = a.enabled; return s; },
  list_progressions: () => db.progressions,
  save_progression: (a) => { const p = { id: uid(), name: a.name, chords: a.chords, created_at: now() }; db.progressions.unshift(p); return p; },
  delete_progression: (a) => { db.progressions = db.progressions.filter((x: Any) => x.id !== a.id); },
  // saved compositions (`db.compositions ??= []` back-fills mock DBs seeded before Phase 3)
  list_compositions: () => {
    db.compositions ??= [];
    // light listing (no data blob), newest first — mirrors db.rs
    return [...db.compositions]
      .sort((x: Any, y: Any) => (y.updated_at > x.updated_at ? 1 : -1))
      .map(({ data, ...meta }: Any) => meta as CompositionMeta);
  },
  get_composition: (a) => { db.compositions ??= []; return db.compositions.find((c: Any) => c.id === a.id) ?? null; },
  save_composition: (a) => {
    db.compositions ??= [];
    JSON.parse(a.data); // reject garbage, like the core does
    if (a.id) {
      const c = db.compositions.find((x: Any) => x.id === a.id);
      if (!c) throw new Error("composition not found");
      Object.assign(c, { name: a.name, song_id: a.songId ?? null, data: a.data, updated_at: now() });
      return c;
    }
    const c = { id: uid(), name: a.name, song_id: a.songId ?? null, data: a.data, created_at: now(), updated_at: now() };
    db.compositions.unshift(c);
    return c;
  },
  delete_composition: (a) => { db.compositions = (db.compositions ?? []).filter((x: Any) => x.id !== a.id); },
  list_renders: (a) => db.renders.filter((x: Any) => x.song_id === a.songId),
  add_render: (a) => { const x = { id: uid(), song_id: a.songId, label: a.label || "Render", file_path: a.filePath, source: a.source || "", notes: a.notes || "", is_pick: false, created_at: now() }; db.renders.unshift(x); return x; },
  set_render_pick: (a) => { const x = db.renders.find((y: Any) => y.id === a.id); if (a.isPick) db.renders.filter((y: Any) => y.song_id === x.song_id).forEach((y: Any) => (y.is_pick = false)); if (x) x.is_pick = a.isPick; },
  delete_render: (a) => { db.renders = db.renders.filter((x: Any) => x.id !== a.id); },
  import_reference: () => db.songs[0]?.id ?? null, // mock: just open the demo song
  // paste-lyrics import (words verbatim; mock = deterministic header split only)
  parse_pasted_lyrics: (a) => parsePastedLyrics(a.text ?? ""),
  import_lyrics: (a) => { importLyricsIntoSong(a.songId, a.text ?? ""); },
  create_song_from_lyrics: (a) => {
    const id = uid();
    const kt = presetKeyTempo(a.stylePresetId); // seed key/BPM from the preset's prose
    const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", intent: (a.intent ?? "").trim(), status: "in_progress",
      current_stage: "concept", key_root: kt.root, key_mode: kt.mode, bpm: kt.bpm, voicings: "{}", created_at: now(), updated_at: now() };
    db.songs.unshift(v);
    STAGE_ORDER.forEach((type, ordinal) =>
      db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
    // Key inference from inline [chord] tags (Feature B2 #3) — create-only;
    // BPM keeps the preset seeding. Set BEFORE the import so the Structure
    // back-fill renders the inferred key. (import_lyrics never touches key.)
    const key = inferKeyFromTags(parsePastedLyrics(a.text ?? "").sections.flatMap((s) => s.lines.flatMap(lineChordTags)));
    if (key) { v.key_root = key.root; v.key_mode = key.mode; }
    importLyricsIntoSong(id, a.text ?? "");
    return toSong(v);
  },
  // Composer export (composition → song): resolved sections in, 🔒 respected
  export_composition_to_song: (a) => {
    const skipped = exportSectionsIntoSong(a.songId, JSON.parse(a.sectionsJson ?? "[]"));
    return { ok: true, song_id: a.songId, skipped_frozen: skipped };
  },
  create_song_from_composition: (a) => {
    const sections = JSON.parse(a.sectionsJson ?? "[]");
    if (!Array.isArray(sections) || !sections.length) throw new Error("nothing to export — the composition has no sections");
    const id = uid();
    const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", status: "in_progress",
      current_stage: "concept", key_root: a.keyRoot || "A", key_mode: a.keyMode || "minor", bpm: Number(a.bpm) || 120, voicings: "{}", created_at: now(), updated_at: now() };
    db.songs.unshift(v);
    STAGE_ORDER.forEach((type, ordinal) =>
      db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
    exportSectionsIntoSong(id, sections);
    return toSong(v);
  },
  self_check_stage: (a) => toArtifact(currentArtifact(a.stageId)), // mock: no-op refine
  get_settings: () => db.settings,
  set_settings: (a) => { db.settings = a.settings; return db.settings; },
  list_tools: () => MOCK_TOOLS,
  mcp_config: () => ({ db_path: "(browser mock)", command_hint: "Run the Tauri app for a real MCP config." }),
  claude_status: () => ({ found: false, version: null, model: db.settings.claude_model, bin: "" }),
  claude_auth_status: () => {
    if (!db.auth) db.auth = { logged_in: false, account: null, subscription: null };
    const li = !!db.auth.logged_in;
    return {
      found: true, bin: "/opt/homebrew/bin/claude",
      logged_in: li, account: db.auth.account ?? null, subscription: db.auth.subscription ?? null,
      api_key_set: false,
      connectors_hint: li
        ? "Signed in on your subscription with no API key set — your connectors load automatically."
        : "Sign in with your claude.ai account to use your subscription. Keep ANTHROPIC_API_KEY unset so your connectors load.",
      connectors_url: "https://claude.ai/settings/connectors",
    };
  },
  claude_login: () => ({ url: "https://claude.com/cai/oauth/authorize?code=true&client_id=mock&state=mock",
    instructions: "(mock) Finish signing in in the browser, then paste the code below and Submit." }),
  claude_login_submit_code: () => {
    db.auth = { logged_in: true, account: "you@claude.ai", subscription: "Claude Pro" };
    return { success: true, message: "Signed in. Re-checking status…" };
  },
  claude_login_cancel: () => undefined,
  claude_logout: () => { db.auth = { logged_in: false, account: null, subscription: null }; },
  open_url: () => undefined,
  test_claude: () => "✅ Live Claude responded: READY (mock)",
  detect_ableton_mcp: () => ({ found: false }),
  test_ableton: () => "(mock) Ableton test runs only in the desktop app.",
  reset_ableton: () => "(mock) reset runs only in the desktop app.",
  ableton_build: () => "(mock) Ableton build runs only in the desktop app.",
  ableton_build_clips: () => "(mock) Ableton clip build runs only in the desktop app.",
  ableton_build_song: () => "(mock) Ableton song stub runs only in the desktop app.",
  chat_send: () => "mock-session",
  write_png: () => { throw new Error("(mock) PNG export runs only in the desktop app."); },
};

export async function mockCall<C extends keyof CommandMap>(cmd: C, a: CommandArgs<C>): Promise<CommandResult<C>> {
  const h = handlers[cmd] as (x: CommandArgs<C>) => CommandResult<C> | Promise<CommandResult<C>>;
  if (!h) throw new Error(`mock: unknown command '${cmd}'`);
  const out = await h(a);
  save(db);
  return out;
}

const MOCK_TOOLS = [
  "list_style_presets","get_style_preset","create_style_preset","update_style_preset","generate_style_preset",
  "create_song","list_songs","get_song","update_song_status","update_song_title","update_song_intent","delete_song",
  "list_sections","create_section","update_section","delete_section","reorder_sections",
  "get_stage","run_stage","approve_stage","advance_stage",
  "get_artifact","save_artifact","list_artifact_revisions","revert_artifact","set_artifact_label",
  "list_skills","get_skill","create_skill","update_skill","set_skill_enabled",
  "list_progressions","save_progression","delete_progression",
  "list_compositions","get_composition","save_composition","delete_composition",
  "list_renders","add_render","set_render_pick","delete_render",
  "ableton_build_song","analyze_reference",
  "get_settings","set_settings",
].map((name) => ({ name, description: "", destructive: name === "delete_song" || name === "delete_progression" || name === "delete_composition" || name === "delete_section" }));
