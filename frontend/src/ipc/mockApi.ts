// In-memory mock of the Rust tool registry, for running the UI in a plain
// browser (no Tauri). Every registry command must exist here (mock-parity).

import { STAGE_ORDER } from "./api";

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
    id: songId, style_preset_id: presetId, title: "Cyber Dreams", status: "in_progress",
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
  // chords carry explicit per-chord beats so the Composer lays them out at
  // the right widths (full-song export). Strings still work (default 4 beats).
  const ch = (name: string, beats = 4) => ({ name, beats });
  const chordsData = {
    sections: [
      { label: "Intro", chords: [ch("Am"), ch("Am"), ch("F"), ch("F")] },
      { label: "Verse 1", chords: [ch("Dm"), ch("Bb"), ch("F"), ch("Am"), ch("Dm"), ch("Bb"), ch("F"), ch("Am")] },
      { label: "Pre-Chorus / Build 1", chords: [ch("Dm", 2), ch("Em", 2), ch("F", 2), ch("G", 2)] },
      { label: "Chorus 1", chords: [ch("C"), ch("G"), ch("Am"), ch("F"), ch("C"), ch("G"), ch("Am"), ch("F")] },
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
  // lyrics artifact derived from the tagged lyrics (chords stripped), so the
  // Builder/Sheet (which derive chord-over-lyric from chords + lyrics) have data
  const lyricsSections = (() => {
    const out: Any[] = []; let cur: Any | null = null;
    for (const line of taggedLyrics.split("\n")) {
      const t = line.trim(); const hm = t.match(/^\[([^\]]+)\]$/);
      if (hm) { cur = { label: hm[1], lines: [] }; out.push(cur); }
      else if (cur && t) cur.lines.push(line.replace(/\[[^\]]+\]/g, ""));
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
        created_at: ts, updated_at: ts,
      },
    ],
    songs: [song], stages,
    artifacts: [
      artifact("chords", "chords", chordsData),
      artifact("lyric_spec", "lyric_spec", {
        hook: "Cyber Dreams", premise: "chasing a feeling you can only reach at full speed on an empty highway",
        pov: "first person, present tense", setting: "a neon highway at 3am, dashboard glowing",
        arc: "restless and numb → wide awake and free",
        diction: "balanced", referenceVibe: "late-night, neon-lit, propulsive but lonely",
        beats: [
          { section: "Verse 1", beat: "set the scene — the dashboard, the empty road, the restlessness" },
          { section: "Chorus 1", beat: "the release — dreaming in neon, finally feeling alive" },
        ],
        imageBank: ["dashboard glow", "cold glass", "tail lights", "static hum", "white lines"],
        avoid: ["chasing dreams", "fading light", "lost in time"],
      }),
      artifact("lyrics", "lyrics", { sections: lyricsSections }),
      artifact("prompt", "generation_prompt", { taggedLyrics }),
    ],
    skills, progressions: [], renders: [],
    compositions: [{ id: uid(), name: "Neon idea", song_id: null, data: JSON.stringify(seedComposition), created_at: ts, updated_at: ts }],
    settings: { claude_model: "", claude_bin: "", mcp_token: "mock-token", ableton_mcp: "", music_folder: "", analyzer_cmd: "" },
    // mock claude.ai subscription auth — starts signed in so the card looks real
    auth: { logged_in: true, account: "you@claude.ai", subscription: "Claude Pro" },
  };
}

let db = load();
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
  const next = stages.find((s: Any) => s.status !== "done") ?? stages[stages.length - 1];
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
  const sData = {
    key: { root: prior?.key?.root ?? song.key_root, mode: prior?.key?.mode ?? song.key_mode },
    bpm: prior?.bpm ?? song.bpm, keyNote: prior?.keyNote ?? "", tempoNote: prior?.tempoNote ?? "", sections,
  };
  const sText = [
    `**KEY:** ${sData.key.root} ${sData.key.mode}`, `**TEMPO:** ${sData.bpm} BPM`, "", "**SECTION MAP**", "",
    ...sections.map((s, i) => `${i + 1}. **${s.label}** (${s.bars} bars)${s.role ? ` — ${s.role}` : ""}`),
  ].join("\n");
  push(structureStage, "structure", JSON.stringify({ kind: "structure", text: sText, data: sData }));
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
function structureText(d: Any): string {
  return [
    `**KEY:** ${d.key.root} ${d.key.mode}`, `**TEMPO:** ${d.bpm} BPM`, "", "**SECTION MAP**", "",
    ...d.sections.map((s: Any, i: number) => `${i + 1}. **${s.label}** (${s.bars} bars)${s.role ? ` — ${s.role}` : ""}`),
  ].join("\n");
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
  const sData = {
    key: { root: priorS?.key?.root ?? song.key_root, mode: priorS?.key?.mode ?? song.key_mode },
    bpm: priorS?.bpm ?? song.bpm, keyNote: priorS?.keyNote ?? "", tempoNote: priorS?.tempoNote ?? "",
    sections: mergeFrozen(priorSSecs, newSSecs),
  };
  push(structureStage, "structure", JSON.stringify({ kind: "structure", text: structureText(sData), data: sData }));
  return skipped;
}

export async function mockCall<T>(cmd: string, a: Any): Promise<T> {
  const r = (x: any) => { save(db); return x as T; };
  switch (cmd) {
    case "list_style_presets": return r(db.presets);
    case "get_style_preset": return r(db.presets.find((p: Any) => p.id === a.id) ?? null);
    case "create_style_preset": { const p = { id: uid(), ...a.input, created_at: now(), updated_at: now() }; db.presets.push(p); return r(p); }
    case "update_style_preset": { const p = db.presets.find((x: Any) => x.id === a.id); Object.assign(p, a.input, { updated_at: now() }); return r(p); }
    case "generate_style_preset":
      return r({ name: a.name, genre: "(mock) genre", mood: "moody", influences: "describe the sound",
        key_tempo_feel: "A minor, 120 BPM", vocal_range: "mid", themes: `themes for ${a.name}` });
    case "create_song": {
      const id = uid();
      const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", status: "in_progress",
        current_stage: "concept", key_root: "A", key_mode: "minor", bpm: 120, voicings: "{}", created_at: now(), updated_at: now() };
      db.songs.unshift(v);
      STAGE_ORDER.forEach((type, ordinal) =>
        db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
      return r(v);
    }
    case "list_songs": return r(db.songs);
    case "get_song": {
      const song = db.songs.find((v: Any) => v.id === a.id);
      if (!song) return r(null);
      const preset = db.presets.find((p: Any) => p.id === song.style_preset_id);
      const stages = db.stages.filter((s: Any) => s.song_id === a.id).sort((x: Any, y: Any) => x.ordinal - y.ordinal).map(withArtifactAt);
      return r({ song, preset, stages });
    }
    case "update_song_status": { const v = db.songs.find((x: Any) => x.id === a.id); v.status = a.status; v.updated_at = now(); return r(v); }
    case "update_song_title": { const v = db.songs.find((x: Any) => x.id === a.id); v.title = a.title; v.updated_at = now(); return r(v); }
    case "update_song_key": { const v = db.songs.find((x: Any) => x.id === a.id); v.key_root = a.root; v.key_mode = a.mode; v.bpm = a.bpm; v.updated_at = now(); return r(v); }
    case "update_song_voicings": { const v = db.songs.find((x: Any) => x.id === a.id); v.voicings = a.voicings; v.updated_at = now(); return r(v); }
    case "refine_field": return r(`(mock) ${a.fieldLabel}: ${a.instruction}`);
    case "delete_song": {
      db.songs = db.songs.filter((v: Any) => v.id !== a.id);
      const sids = db.stages.filter((s: Any) => s.song_id === a.id).map((s: Any) => s.id);
      db.stages = db.stages.filter((s: Any) => s.song_id !== a.id);
      db.artifacts = db.artifacts.filter((ar: Any) => !sids.includes(ar.stage_id));
      return r(undefined);
    }
    case "get_stage": {
      const stage = db.stages.find((s: Any) => s.id === a.id);
      if (!stage) return r(null);
      return r({ stage: withArtifactAt(stage), artifact: currentArtifact(a.id), skill: activeSkill(stage.type) });
    }
    case "run_stage": {
      const stage = db.stages.find((s: Any) => s.id === a.stageId);
      stage.status = "in_progress";
      const kind = KINDS[stage.type] ?? "artifact";
      const text = `# ${kind} (mock)\n\nSimulated ${stage.type} for this song. Run in the Tauri app with Claude for real output.\n` +
        (a.userInput ? `\nYour seed:\n${a.userInput}\n` : "");
      const ver = (currentArtifact(a.stageId)?.version ?? 0) + 1;
      const art = { id: uid(), song_id: stage.song_id, stage_id: a.stageId, kind,
        content: JSON.stringify({ kind, text, data: null }), version: ver, approved: false, created_at: now() };
      db.artifacts.push(art);
      return r(art);
    }
    case "cancel_stage": return r(undefined); // mock runs finish instantly — nothing to cancel
    case "approve_stage": {
      const stage = db.stages.find((s: Any) => s.id === a.stageId);
      const art = currentArtifact(a.stageId);
      if (art) art.approved = true;
      stage.status = "done";
      advance(stage.song_id);
      return r({ ok: true });
    }
    case "advance_stage": return r(advance(a.songId));
    case "get_artifact": return r(db.artifacts.find((x: Any) => x.id === a.id) ?? null);
    case "save_artifact": {
      const ver = (currentArtifact(a.stageId)?.version ?? 0) + 1;
      const art = { id: uid(), song_id: a.songId, stage_id: a.stageId ?? null, kind: a.kind, content: a.content, version: ver, approved: false, created_at: now() };
      db.artifacts.push(art);
      return r(art);
    }
    case "list_artifact_revisions":
      return r(db.artifacts.filter((x: Any) => x.stage_id === a.stageId).sort((x: Any, y: Any) => y.version - x.version));
    case "revert_artifact": {
      const src = db.artifacts.find((x: Any) => x.id === a.artifactId);
      const ver = (currentArtifact(src.stage_id)?.version ?? 0) + 1;
      const art = { ...src, id: uid(), version: ver, approved: false, created_at: now() };
      db.artifacts.push(art);
      return r(art);
    }
    case "list_skills": return r(db.skills);
    case "get_skill": return r(db.skills.find((s: Any) => s.id === a.id) ?? null);
    case "create_skill": { const s = { id: uid(), ...a.input, source: "user", enabled: true, created_at: now(), updated_at: now() }; db.skills.push(s); return r(s); }
    case "update_skill": { const s = db.skills.find((x: Any) => x.id === a.id); Object.assign(s, a.input, { updated_at: now() }); return r(s); }
    case "set_skill_enabled": { const s = db.skills.find((x: Any) => x.id === a.id); s.enabled = a.enabled; return r(s); }
    case "list_progressions": return r(db.progressions);
    case "save_progression": { const p = { id: uid(), name: a.name, chords: a.chords, created_at: now() }; db.progressions.unshift(p); return r(p); }
    case "delete_progression": db.progressions = db.progressions.filter((x: Any) => x.id !== a.id); return r(undefined);
    // saved compositions (`db.compositions ??= []` back-fills mock DBs seeded before Phase 3)
    case "list_compositions": {
      db.compositions ??= [];
      // light listing (no data blob), newest first — mirrors db.rs
      return r([...db.compositions]
        .sort((x: Any, y: Any) => (y.updated_at > x.updated_at ? 1 : -1))
        .map(({ data, ...meta }: Any) => meta));
    }
    case "get_composition": { db.compositions ??= []; return r(db.compositions.find((c: Any) => c.id === a.id) ?? null); }
    case "save_composition": {
      db.compositions ??= [];
      JSON.parse(a.data); // reject garbage, like the core does
      if (a.id) {
        const c = db.compositions.find((x: Any) => x.id === a.id);
        if (!c) throw new Error("composition not found");
        Object.assign(c, { name: a.name, song_id: a.songId ?? null, data: a.data, updated_at: now() });
        return r(c);
      }
      const c = { id: uid(), name: a.name, song_id: a.songId ?? null, data: a.data, created_at: now(), updated_at: now() };
      db.compositions.unshift(c);
      return r(c);
    }
    case "delete_composition": db.compositions = (db.compositions ?? []).filter((x: Any) => x.id !== a.id); return r(undefined);
    case "list_renders": return r(db.renders.filter((x: Any) => x.song_id === a.songId));
    case "add_render": { const x = { id: uid(), song_id: a.songId, label: a.label || "Render", file_path: a.filePath, source: a.source || "", notes: a.notes || "", is_pick: false, created_at: now() }; db.renders.unshift(x); return r(x); }
    case "set_render_pick": { const x = db.renders.find((y: Any) => y.id === a.id); if (a.isPick) db.renders.filter((y: Any) => y.song_id === x.song_id).forEach((y: Any) => (y.is_pick = false)); if (x) x.is_pick = a.isPick; return r(undefined); }
    case "delete_render": db.renders = db.renders.filter((x: Any) => x.id !== a.id); return r(undefined);
    case "analyze_reference": return r({
      duration_sec: 80, tempo_bpm: 95.7, key: { root: "E", mode: "minor", confidence: 0.66 },
      section_count: 3, note: "(mock) raw perception output",
      bar_chords: ["Am", "F", "C", "E", "Am", "F", "C", "E"].map((chord, i) => ({ bar: i + 1, time: i * 2, chord })),
      sections: [{ start_sec: 0, end_sec: 32, approx_bars: 8, chords: ["Am", "F", "C", "E"] }],
    });
    case "import_reference": return r(db.songs[0]?.id ?? null); // mock: just open the demo song
    // paste-lyrics import (words verbatim; mock = deterministic header split only)
    case "parse_pasted_lyrics": return r(parsePastedLyrics(a.text ?? ""));
    case "import_lyrics": { importLyricsIntoSong(a.songId, a.text ?? ""); return r(undefined); }
    case "create_song_from_lyrics": {
      const id = uid();
      const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", status: "in_progress",
        current_stage: "concept", key_root: "A", key_mode: "minor", bpm: 120, voicings: "{}", created_at: now(), updated_at: now() };
      db.songs.unshift(v);
      STAGE_ORDER.forEach((type, ordinal) =>
        db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
      importLyricsIntoSong(id, a.text ?? "");
      return r(v);
    }
    // Composer export (composition → song): resolved sections in, 🔒 respected
    case "export_composition_to_song": {
      const skipped = exportSectionsIntoSong(a.songId, JSON.parse(a.sectionsJson ?? "[]"));
      return r({ ok: true, song_id: a.songId, skipped_frozen: skipped });
    }
    case "create_song_from_composition": {
      const sections = JSON.parse(a.sectionsJson ?? "[]");
      if (!Array.isArray(sections) || !sections.length) throw new Error("nothing to export — the composition has no sections");
      const id = uid();
      const v = { id, style_preset_id: a.stylePresetId, title: a.title || "Untitled song", status: "in_progress",
        current_stage: "concept", key_root: a.keyRoot || "A", key_mode: a.keyMode || "minor", bpm: Number(a.bpm) || 120, voicings: "{}", created_at: now(), updated_at: now() };
      db.songs.unshift(v);
      STAGE_ORDER.forEach((type, ordinal) =>
        db.stages.push({ id: uid(), song_id: id, type, ordinal, status: "pending", skill_id: null, created_at: now(), updated_at: now() }));
      exportSectionsIntoSong(id, sections);
      return r(v);
    }
    case "self_check_stage": return r(currentArtifact(a.stageId)); // mock: no-op refine
    case "get_settings": return r(db.settings);
    case "set_settings": db.settings = a.settings; return r(db.settings);
    case "list_tools": return r(MOCK_TOOLS);
    case "mcp_config": return r({ db_path: "(browser mock)", token: "mock-token", command_hint: "Run the Tauri app for a real MCP config." });
    case "claude_status": return r({ found: false, version: null, model: db.settings.claude_model, bin: "" });
    case "claude_auth_status": {
      if (!db.auth) db.auth = { logged_in: false, account: null, subscription: null };
      const li = !!db.auth.logged_in;
      return r({
        found: true, bin: "/opt/homebrew/bin/claude",
        logged_in: li, account: db.auth.account ?? null, subscription: db.auth.subscription ?? null,
        api_key_set: false,
        connectors_hint: li
          ? "Signed in on your subscription with no API key set — your connectors load automatically."
          : "Sign in with your claude.ai account to use your subscription. Keep ANTHROPIC_API_KEY unset so your connectors load.",
        connectors_url: "https://claude.ai/settings/connectors",
      });
    }
    case "claude_login":
      return r({ url: "https://claude.com/cai/oauth/authorize?code=true&client_id=mock&state=mock",
        instructions: "(mock) Finish signing in in the browser, then paste the code below and Submit." });
    case "claude_login_submit_code":
      db.auth = { logged_in: true, account: "you@claude.ai", subscription: "Claude Pro" };
      return r({ success: true, message: "Signed in. Re-checking status…" });
    case "claude_login_cancel": return r(undefined);
    case "claude_logout":
      db.auth = { logged_in: false, account: null, subscription: null };
      return r(undefined);
    case "open_url": return r(undefined);
    case "test_claude": return r("✅ Live Claude responded: READY (mock)");
    case "detect_ableton_mcp": return r({ found: false });
    case "test_ableton": return r("(mock) Ableton test runs only in the desktop app.");
    case "reset_ableton": return r("(mock) reset runs only in the desktop app.");
    case "ableton_build": return r("(mock) Ableton build runs only in the desktop app.");
    case "ableton_build_clips": return r("(mock) Ableton clip build runs only in the desktop app.");
    case "ableton_build_song": return r("(mock) Ableton song stub runs only in the desktop app.");
    case "chat_send": return r("mock-session");
    default: throw new Error(`mock: unknown command '${cmd}'`);
  }
}

const MOCK_TOOLS = [
  "list_style_presets","get_style_preset","create_style_preset","update_style_preset","generate_style_preset",
  "create_song","list_songs","get_song","update_song_status","update_song_title","delete_song",
  "get_stage","run_stage","approve_stage","advance_stage",
  "get_artifact","save_artifact","list_artifact_revisions","revert_artifact",
  "list_skills","get_skill","create_skill","update_skill","set_skill_enabled",
  "list_progressions","save_progression","delete_progression",
  "list_compositions","get_composition","save_composition","delete_composition",
  "list_renders","add_render","set_render_pick","delete_render",
  "analyze_reference",
  "get_settings","set_settings",
].map((name) => ({ name, description: "", destructive: name === "delete_song" || name === "delete_progression" || name === "delete_composition" }));
