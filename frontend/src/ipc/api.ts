// Typed IPC layer. Every Rust tool/command is wrapped here once. In Tauri it
// calls `invoke`; in a plain browser it falls back to the in-memory mock.

import type {
  Artifact,
  CompositionMeta,
  CompositionRow,
  Settings,
  Outline,
  Progression,
  Render,
  Section,
  Skill,
  SkillInput,
  Song,
  SongDetail,
  StageDetail,
  StyleInput,
  StylePreset,
  RunResult,
  StageDraft,
} from "./generated";
import { mockCall } from "./mockApi";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Every Tauri command the frontend dispatches, with its exact args + result.
 * `call` and the browser mock are both typed against it, so a missing,
 * renamed, or mistyped command/handler is a COMPILE error on either side
 * (audit Tier-2 #7 — api/mock parity by construction).
 */
export type CommandMap = {
  // style presets
  list_style_presets: { args: Record<string, never>; result: StylePreset[] };
  get_style_preset: { args: { id: string }; result: StylePreset | null };
  create_style_preset: { args: { input: StyleInput }; result: StylePreset };
  update_style_preset: { args: { id: string; input: StyleInput }; result: StylePreset };
  set_preset_arrangement: { args: { id: string; arrangement: string }; result: StylePreset };
  generate_preset_arrangement: { args: { id: string }; result: StylePreset };
  generate_style_preset: { args: { name: string; notes: string | null }; result: StyleInput };
  // songs & stages
  create_song: { args: { stylePresetId: string; title: string; intent: string | null }; result: Song };
  list_songs: { args: Record<string, never>; result: Song[] };
  get_song: { args: { id: string }; result: SongDetail | null };
  update_song_status: { args: { id: string; status: string }; result: Song };
  update_song_title: { args: { id: string; title: string }; result: Song };
  update_song_intent: { args: { id: string; intent: string }; result: Song };
  update_song_key: { args: { id: string; root: string; mode: string; bpm: number }; result: Song };
  update_song_voicings: { args: { id: string; voicings: string }; result: Song };
  // section spine (docs/SECTION-SPINE-SPEC.md) — the single source of truth
  // for section identity/order/label/type/bars/role
  list_sections: { args: { songId: string }; result: Section[] };
  create_section: {
    args: { songId: string; label: string; sectionType: string; bars: number; role: string; position: number | null };
    result: Section;
  };
  update_section: { args: { id: string; label: string; sectionType: string; bars: number; role: string }; result: Section };
  delete_section: { args: { id: string }; result: void };
  reorder_sections: { args: { songId: string; sectionIds: string[] }; result: Section[] };
  union_spine_sections: { args: { songId: string }; result: Section[] };
  import_reference: { args: { audioPath: string }; result: string };
  parse_pasted_lyrics: { args: { text: string }; result: ParsedLyrics };
  import_lyrics: { args: { songId: string; text: string }; result: void };
  create_song_from_lyrics: { args: { stylePresetId: string; title: string; text: string; intent: string | null }; result: Song };
  export_composition_to_song: { args: { songId: string; sectionsJson: string }; result: ExportResult };
  create_song_from_composition: {
    args: { stylePresetId: string; title: string; keyRoot: string; keyMode: string; bpm: number; sectionsJson: string };
    result: Song;
  };
  refine_field: {
    args: { stageLabel: string; fieldLabel: string; current: string; instruction: string; songId: string | null };
    result: string;
  };
  delete_song: { args: { id: string }; result: void };
  get_stage: { args: { id: string }; result: StageDetail | null };
  run_stage: { args: { stageId: string; userInput: string | null }; result: RunResult };
  accept_stage_draft: { args: { stageId: string }; result: Artifact };
  ableton_build_progression: { args: { chords: string[]; beats: number[] | null; inversions: number[] | null; bpm: number | null }; result: string };
  ableton_build_composition: { args: { bpm: number; lengthBeats: number; tracks: { name: string; notes: unknown[] }[] }; result: string };
  ableton_build_outline: { args: { bpm: number; sections: [string, number][] }; result: string };
  list_outlines: { args: Record<string, never>; result: Outline[] };
  save_outline: { args: { name: string; bpm: number; sections: [string, number][] }; result: Outline };
  delete_outline: { args: { id: string }; result: null };
  midi_list_inputs: { args: Record<string, never>; result: string[] };
  midi_open_input: { args: { index: number }; result: string };
  midi_close_input: { args: Record<string, never>; result: null };
  discard_stage_draft: { args: { stageId: string }; result: null };
  cancel_stage: { args: { stageId: string }; result: void };
  self_check_stage: { args: { stageId: string }; result: StageDraft };
  approve_stage: { args: { stageId: string }; result: unknown };
  advance_stage: { args: { songId: string }; result: unknown };
  // artifacts
  get_artifact: { args: { id: string }; result: Artifact | null };
  save_artifact: { args: { songId: string; stageId: string | null; kind: string; content: string }; result: Artifact };
  list_artifact_revisions: { args: { stageId: string }; result: Artifact[] };
  revert_artifact: { args: { artifactId: string }; result: Artifact };
  set_artifact_label: { args: { artifactId: string; label: string | null }; result: void };
  // skills
  list_skills: { args: Record<string, never>; result: Skill[] };
  get_skill: { args: { id: string }; result: Skill | null };
  create_skill: { args: { input: SkillInput }; result: Skill };
  update_skill: { args: { id: string; input: SkillInput }; result: Skill };
  set_skill_enabled: { args: { id: string; enabled: boolean }; result: Skill };
  // saved chord progressions
  list_progressions: { args: Record<string, never>; result: Progression[] };
  save_progression: { args: { name: string; chords: string[]; picks: string | null }; result: Progression };
  update_progression: { args: { id: string; name: string; chords: string[]; picks: string | null }; result: Progression };
  delete_progression: { args: { id: string }; result: void };
  // saved compositions
  list_compositions: { args: Record<string, never>; result: CompositionMeta[] };
  get_composition: { args: { id: string }; result: CompositionRow | null };
  save_composition: { args: { id: string | null; name: string; songId: string | null; data: string }; result: CompositionRow };
  delete_composition: { args: { id: string }; result: void };
  // final renders
  list_renders: { args: { songId: string }; result: Render[] };
  add_render: { args: { songId: string; label: string; filePath: string; source: string; notes: string }; result: Render };
  set_render_pick: { args: { id: string; isPick: boolean }; result: void };
  delete_render: { args: { id: string }; result: void };
  // settings & meta
  get_settings: { args: Record<string, never>; result: Settings };
  set_settings: { args: { settings: Settings }; result: Settings };
  list_tools: { args: Record<string, never>; result: ToolInfo[] };
  mcp_config: { args: Record<string, never>; result: McpConfig };
  claude_status: { args: Record<string, never>; result: ClaudeStatus };
  // claude.ai account auth
  claude_auth_status: { args: Record<string, never>; result: ClaudeAuthStatus };
  claude_login: { args: Record<string, never>; result: LoginResult };
  claude_login_submit_code: { args: { code: string }; result: LoginSubmitResult };
  claude_login_cancel: { args: Record<string, never>; result: void };
  claude_logout: { args: Record<string, never>; result: void };
  open_url: { args: { url: string }; result: void };
  test_claude: { args: Record<string, never>; result: string };
  // ableton & chat
  test_ableton: { args: Record<string, never>; result: string };
  reset_ableton: { args: Record<string, never>; result: string };
  ableton_build: { args: { songId: string }; result: string };
  ableton_build_clips: { args: { songId: string }; result: string };
  ableton_build_song: { args: { songId: string }; result: string };
  detect_ableton_mcp: { args: Record<string, never>; result: { found: boolean; name?: string; entry?: unknown } };
  chat_send: { args: { message: string; sessionId: string | null; songId: string | null }; result: string };
  // desktop-only file helper (savePng)
  write_png: { args: { path: string; bytes: number[] }; result: string };
};
export type CommandArgs<C extends keyof CommandMap> = CommandMap[C]["args"];
export type CommandResult<C extends keyof CommandMap> = CommandMap[C]["result"];

async function call<C extends keyof CommandMap>(cmd: C, args?: CommandArgs<C>): Promise<CommandResult<C>> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke(cmd, args) as Promise<CommandResult<C>>;
  }
  return mockCall(cmd, (args ?? {}) as CommandArgs<C>);
}

export async function listen<T>(event: string, cb: (payload: T) => void): Promise<() => void> {
  if (inTauri) {
    const { listen } = await import("@tauri-apps/api/event");
    return await listen<T>(event, (e) => cb(e.payload));
  }
  return () => {};
}

/** Native audio-file picker (Tauri only). Returns the chosen path or null. */
export async function pickAudioFile(defaultPath?: string): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({
    multiple: false,
    defaultPath,
    filters: [{ name: "Audio", extensions: ["mp3", "wav", "aiff", "aif", "m4a", "flac", "ogg"] }],
  });
  return typeof res === "string" ? res : null;
}
/** Native folder picker (Tauri only). Returns the chosen directory or null. */
export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}
export async function openFile(path: string) {
  if (!inTauri) return;
  const { openPath } = await import("@tauri-apps/plugin-opener");
  await openPath(path);
}
export async function revealFile(path: string) {
  if (!inTauri) return;
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}
/** Native save dialog → write PNG bytes → return the saved path (Tauri only). */
export async function savePng(defaultName: string, bytes: number[]): Promise<string | null> {
  if (!inTauri) return null;
  const { save } = await import("@tauri-apps/plugin-dialog");
  const path = await save({ defaultPath: defaultName, filters: [{ name: "PNG", extensions: ["png"] }] });
  if (!path) return null;
  return call("write_png", { path, bytes });
}

export type ToolInfo = { name: string; description: string; destructive: boolean };
export type McpConfig = { db_path: string; command_hint: string };
export type ClaudeStatus = { found: boolean; version: string | null; model: string; bin: string };
export type ClaudeAuthStatus = {
  found: boolean;
  bin: string;
  logged_in: boolean;
  account: string | null;
  subscription: string | null;
  api_key_set: boolean;
  connectors_hint: string;
  connectors_url: string;
};
export type LoginResult = { url: string | null; instructions: string };
export type LoginSubmitResult = { success: boolean; message: string };
/** Pasted-lyrics parse result (words verbatim; `used_claude` = the fallback segmented unlabeled text). */
export type ParsedLyricSection = { label: string; lines: string[] };
export type ParsedLyrics = { sections: ParsedLyricSection[]; used_claude: boolean };
/** Composer export-to-song outcome — `skipped_frozen` lists the 🔒 section labels kept as-is. */
export type ExportResult = { ok: boolean; song_id: string; skipped_frozen: string[] };

export const api = {
  // style presets
  listStylePresets: () => call("list_style_presets"),
  getStylePreset: (id: string) => call("get_style_preset", { id }),
  createStylePreset: (input: StyleInput) => call("create_style_preset", { input }),
  updateStylePreset: (id: string, input: StyleInput) => call("update_style_preset", { id, input }),
  setPresetArrangement: (id: string, arrangement: string) => call("set_preset_arrangement", { id, arrangement }),
  generatePresetArrangement: (id: string) => call("generate_preset_arrangement", { id }),
  generateStylePreset: (name: string, notes?: string) =>
    call("generate_style_preset", { name, notes: notes ?? null }),

  // songs & stages
  createSong: (stylePresetId: string, title: string, intent?: string) =>
    call("create_song", { stylePresetId, title, intent: intent ?? null }),
  listSongs: () => call("list_songs"),
  getSong: (id: string) => call("get_song", { id }),
  updateSongStatus: (id: string, status: string) => call("update_song_status", { id, status }),
  updateSongTitle: (id: string, title: string) => call("update_song_title", { id, title }),
  // the producer's one-line brief — the north star every stage honors
  updateSongIntent: (id: string, intent: string) => call("update_song_intent", { id, intent }),
  updateSongKey: (id: string, root: string, mode: string, bpm: number) => call("update_song_key", { id, root, mode, bpm }),
  updateSongVoicings: (id: string, voicings: string) => call("update_song_voicings", { id, voicings }),
  // section spine (docs/SECTION-SPINE-SPEC.md) — the single source of truth
  // for section identity/order/label/type/bars/role; every consumer reads it
  listSections: (songId: string) => call("list_sections", { songId }),
  /** `position` omitted/null appends at the end; a number inserts there (clamped). */
  createSection: (songId: string, label: string, sectionType = "", bars = 8, role = "", position?: number) =>
    call("create_section", { songId, label, sectionType, bars, role, position: position ?? null }),
  /** form only (label/type/bars/role) — order changes go through reorderSections */
  updateSection: (id: string, label: string, sectionType: string, bars: number, role: string) =>
    call("update_section", { id, label, sectionType, bars, role }),
  deleteSection: (id: string) => call("delete_section", { id }),
  /** `sectionIds` must be every section id of the song, each once, in the new order */
  reorderSections: (songId: string, sectionIds: string[]) => call("reorder_sections", { songId, sectionIds }),
  /** mid-session spine-birth union (Phase 4): append rows for sections that
   *  exist only in stage artifacts (the lyrics-only Bridge case); idempotent */
  unionSpineSections: (songId: string) => call("union_spine_sections", { songId }),
  importReference: (audioPath: string) => call("import_reference", { audioPath }),
  // paste-lyrics import (words kept verbatim — parse/tag only, never rewrite)
  parsePastedLyrics: (text: string) => call("parse_pasted_lyrics", { text }),
  importLyrics: (songId: string, text: string) => call("import_lyrics", { songId, text }),
  createSongFromLyrics: (stylePresetId: string, title: string, text: string, intent?: string) =>
    call("create_song_from_lyrics", { stylePresetId, title, text, intent: intent ?? null }),
  // Composer export (composition → song). `sectionsJson` is the RESOLVED
  // sections array from lib/music/compose/compositionToSong.ts.
  exportCompositionToSong: (songId: string, sectionsJson: string) =>
    call("export_composition_to_song", { songId, sectionsJson }),
  createSongFromComposition: (stylePresetId: string, title: string, keyRoot: string, keyMode: string, bpm: number, sectionsJson: string) =>
    call("create_song_from_composition", { stylePresetId, title, keyRoot, keyMode, bpm, sectionsJson }),
  refineField: (stageLabel: string, fieldLabel: string, current: string, instruction: string, songId?: string) =>
    call("refine_field", { stageLabel, fieldLabel, current, instruction, songId: songId ?? null }),
  deleteSong: (id: string) => call("delete_song", { id }),
  getStage: (id: string) => call("get_stage", { id }),
  runStage: (stageId: string, userInput?: string) =>
    call("run_stage", { stageId, userInput: userInput ?? null }),
  cancelStage: (stageId: string) => call("cancel_stage", { stageId }),
  selfCheckStage: (stageId: string) => call("self_check_stage", { stageId }),
  approveStage: (stageId: string) => call("approve_stage", { stageId }),
  acceptStageDraft: (stageId: string) => call("accept_stage_draft", { stageId }),
  abletonBuildProgression: (chords: string[], inversions?: number[], bpm?: number, beats?: number[]) => call("ableton_build_progression", { chords, beats: beats ?? null, inversions: inversions ?? null, bpm: bpm ?? null }),
  abletonBuildComposition: (bpm: number, lengthBeats: number, tracks: { name: string; notes: unknown[] }[]) => call("ableton_build_composition", { bpm, lengthBeats, tracks }),
  abletonBuildOutline: (bpm: number, sections: [string, number][]) => call("ableton_build_outline", { bpm, sections }),
  listOutlines: () => call("list_outlines", {}),
  saveOutline: (name: string, bpm: number, sections: [string, number][]) => call("save_outline", { name, bpm, sections }),
  deleteOutline: (id: string) => call("delete_outline", { id }),
  midiListInputs: () => call("midi_list_inputs", {}),
  midiOpenInput: (index: number) => call("midi_open_input", { index }),
  midiCloseInput: () => call("midi_close_input", {}),
  discardStageDraft: (stageId: string) => call("discard_stage_draft", { stageId }),
  advanceStage: (songId: string) => call("advance_stage", { songId }),

  // artifacts
  getArtifact: (id: string) => call("get_artifact", { id }),
  saveArtifact: (songId: string, stageId: string | null, kind: string, content: string) =>
    call("save_artifact", { songId, stageId, kind, content }),
  listArtifactRevisions: (stageId: string) => call("list_artifact_revisions", { stageId }),
  revertArtifact: (artifactId: string) => call("revert_artifact", { artifactId }),
  /** name (or clear — null) a revision in the History timeline; metadata only */
  setArtifactLabel: (artifactId: string, label: string | null) => call("set_artifact_label", { artifactId, label }),

  // skills
  listSkills: () => call("list_skills"),
  getSkill: (id: string) => call("get_skill", { id }),
  createSkill: (input: SkillInput) => call("create_skill", { input }),
  updateSkill: (id: string, input: SkillInput) => call("update_skill", { id, input }),
  setSkillEnabled: (id: string, enabled: boolean) => call("set_skill_enabled", { id, enabled }),

  // saved chord progressions (reusable across songs)
  listProgressions: () => call("list_progressions"),
  saveProgression: (name: string, chords: string[], picks?: string) => call("save_progression", { name, chords, picks: picks ?? null }),
  updateProgression: (id: string, name: string, chords: string[], picks?: string) => call("update_progression", { id, name, chords, picks: picks ?? null }),
  deleteProgression: (id: string) => call("delete_progression", { id }),

  // saved compositions (Composer sketches / full-song exports; libSQL-backed)
  listCompositions: () => call("list_compositions"),
  getComposition: (id: string) => call("get_composition", { id }),
  /** `id: null` inserts (adopt the returned row id); a row id updates in place. */
  saveComposition: (id: string | null, name: string, songId: string | null, data: string) =>
    call("save_composition", { id, name, songId, data }),
  deleteComposition: (id: string) => call("delete_composition", { id }),

  // final renders (audio versions referenced on disk)
  listRenders: (songId: string) => call("list_renders", { songId }),
  addRender: (songId: string, label: string, filePath: string, source: string, notes: string) =>
    call("add_render", { songId, label, filePath, source, notes }),
  setRenderPick: (id: string, isPick: boolean) => call("set_render_pick", { id, isPick }),
  deleteRender: (id: string) => call("delete_render", { id }),

  // settings & meta
  getSettings: () => call("get_settings"),
  setSettings: (settings: Settings) => call("set_settings", { settings }),
  listTools: () => call("list_tools"),
  mcpConfig: () => call("mcp_config"),
  claudeStatus: () => call("claude_status"),

  // claude.ai account auth (subscription only — never an API key)
  claudeAuthStatus: () => call("claude_auth_status"),
  claudeLogin: () => call("claude_login"),
  claudeLoginSubmitCode: (code: string) => call("claude_login_submit_code", { code }),
  claudeLoginCancel: () => call("claude_login_cancel"),
  claudeLogout: () => call("claude_logout"),
  openUrl: (url: string) => call("open_url", { url }),
  testClaude: () => call("test_claude"),

  testAbleton: () => call("test_ableton"),
  resetAbleton: () => call("reset_ableton"),
  abletonBuild: (songId: string) => call("ableton_build", { songId }),
  abletonBuildClips: (songId: string) => call("ableton_build_clips", { songId }),
  abletonBuildSong: (songId: string) => call("ableton_build_song", { songId }),
  detectAbletonMcp: () => call("detect_ableton_mcp"),
  chatSend: (message: string, sessionId?: string, songId?: string) =>
    call("chat_send", { message, sessionId: sessionId ?? null, songId: songId ?? null }),
};

export const STAGE_ORDER = ["concept", "structure", "chords", "lyric_spec", "lyrics", "prompt"] as const;

export const STAGE_LABELS: Record<string, string> = {
  concept: "Concept",
  structure: "Structure",
  chords: "Chords",
  lyric_spec: "Lyric Spec",
  lyrics: "Lyrics",
  prompt: "Generation Prompt",
};
