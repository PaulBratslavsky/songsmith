// Typed IPC layer. Every Rust tool/command is wrapped here once. In Tauri it
// calls `invoke`; in a plain browser it falls back to the in-memory mock.

import type {
  Artifact,
  CompositionMeta,
  CompositionRow,
  Settings,
  Progression,
  Render,
  Skill,
  SkillInput,
  Song,
  SongDetail,
  StageDetail,
  StyleInput,
  StylePreset,
} from "./generated";
import { mockCall } from "./mockApi";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  return mockCall<T>(cmd, args ?? {});
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
  return call<string>("write_png", { path, bytes });
}

export type ToolInfo = { name: string; description: string; destructive: boolean };
export type McpConfig = { db_path: string; token: string; command_hint: string };
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
  listStylePresets: () => call<StylePreset[]>("list_style_presets"),
  getStylePreset: (id: string) => call<StylePreset | null>("get_style_preset", { id }),
  createStylePreset: (input: StyleInput) => call<StylePreset>("create_style_preset", { input }),
  updateStylePreset: (id: string, input: StyleInput) => call<StylePreset>("update_style_preset", { id, input }),
  generateStylePreset: (name: string, notes?: string) =>
    call<StyleInput>("generate_style_preset", { name, notes: notes ?? null }),

  // songs & stages
  createSong: (stylePresetId: string, title: string) => call<Song>("create_song", { stylePresetId, title }),
  listSongs: () => call<Song[]>("list_songs"),
  getSong: (id: string) => call<SongDetail | null>("get_song", { id }),
  updateSongStatus: (id: string, status: string) => call<Song>("update_song_status", { id, status }),
  updateSongTitle: (id: string, title: string) => call<Song>("update_song_title", { id, title }),
  updateSongKey: (id: string, root: string, mode: string, bpm: number) => call<Song>("update_song_key", { id, root, mode, bpm }),
  updateSongVoicings: (id: string, voicings: string) => call<Song>("update_song_voicings", { id, voicings }),
  importReference: (audioPath: string) => call<string>("import_reference", { audioPath }),
  // paste-lyrics import (words kept verbatim — parse/tag only, never rewrite)
  parsePastedLyrics: (text: string) => call<ParsedLyrics>("parse_pasted_lyrics", { text }),
  importLyrics: (songId: string, text: string) => call<void>("import_lyrics", { songId, text }),
  createSongFromLyrics: (stylePresetId: string, title: string, text: string) =>
    call<Song>("create_song_from_lyrics", { stylePresetId, title, text }),
  // Composer export (composition → song). `sectionsJson` is the RESOLVED
  // sections array from lib/music/compose/compositionToSong.ts.
  exportCompositionToSong: (songId: string, sectionsJson: string) =>
    call<ExportResult>("export_composition_to_song", { songId, sectionsJson }),
  createSongFromComposition: (stylePresetId: string, title: string, keyRoot: string, keyMode: string, bpm: number, sectionsJson: string) =>
    call<Song>("create_song_from_composition", { stylePresetId, title, keyRoot, keyMode, bpm, sectionsJson }),
  refineField: (stageLabel: string, fieldLabel: string, current: string, instruction: string) =>
    call<string>("refine_field", { stageLabel, fieldLabel, current, instruction }),
  deleteSong: (id: string) => call<void>("delete_song", { id }),
  getStage: (id: string) => call<StageDetail | null>("get_stage", { id }),
  runStage: (stageId: string, userInput?: string) =>
    call<Artifact>("run_stage", { stageId, userInput: userInput ?? null }),
  cancelStage: (stageId: string) => call<void>("cancel_stage", { stageId }),
  selfCheckStage: (stageId: string) => call<Artifact>("self_check_stage", { stageId }),
  approveStage: (stageId: string) => call<unknown>("approve_stage", { stageId }),
  advanceStage: (songId: string) => call<unknown>("advance_stage", { songId }),

  // artifacts
  getArtifact: (id: string) => call<Artifact | null>("get_artifact", { id }),
  saveArtifact: (songId: string, stageId: string | null, kind: string, content: string) =>
    call<Artifact>("save_artifact", { songId, stageId, kind, content }),
  listArtifactRevisions: (stageId: string) => call<Artifact[]>("list_artifact_revisions", { stageId }),
  revertArtifact: (artifactId: string) => call<Artifact>("revert_artifact", { artifactId }),

  // skills
  listSkills: () => call<Skill[]>("list_skills"),
  getSkill: (id: string) => call<Skill | null>("get_skill", { id }),
  createSkill: (input: SkillInput) => call<Skill>("create_skill", { input }),
  updateSkill: (id: string, input: SkillInput) => call<Skill>("update_skill", { id, input }),
  setSkillEnabled: (id: string, enabled: boolean) => call<Skill>("set_skill_enabled", { id, enabled }),

  // saved chord progressions (reusable across songs)
  listProgressions: () => call<Progression[]>("list_progressions"),
  saveProgression: (name: string, chords: string[]) => call<Progression>("save_progression", { name, chords }),
  deleteProgression: (id: string) => call<void>("delete_progression", { id }),

  // saved compositions (Composer sketches / full-song exports; libSQL-backed)
  listCompositions: () => call<CompositionMeta[]>("list_compositions"),
  getComposition: (id: string) => call<CompositionRow | null>("get_composition", { id }),
  /** `id: null` inserts (adopt the returned row id); a row id updates in place. */
  saveComposition: (id: string | null, name: string, songId: string | null, data: string) =>
    call<CompositionRow>("save_composition", { id, name, songId, data }),
  deleteComposition: (id: string) => call<void>("delete_composition", { id }),

  // final renders (audio versions referenced on disk)
  listRenders: (songId: string) => call<Render[]>("list_renders", { songId }),
  addRender: (songId: string, label: string, filePath: string, source: string, notes: string) =>
    call<Render>("add_render", { songId, label, filePath, source, notes }),
  setRenderPick: (id: string, isPick: boolean) => call<void>("set_render_pick", { id, isPick }),
  deleteRender: (id: string) => call<void>("delete_render", { id }),

  // settings & meta
  getSettings: () => call<Settings>("get_settings"),
  setSettings: (settings: Settings) => call<Settings>("set_settings", { settings }),
  listTools: () => call<ToolInfo[]>("list_tools"),
  mcpConfig: () => call<McpConfig>("mcp_config"),
  claudeStatus: () => call<ClaudeStatus>("claude_status"),

  // claude.ai account auth (subscription only — never an API key)
  claudeAuthStatus: () => call<ClaudeAuthStatus>("claude_auth_status"),
  claudeLogin: () => call<LoginResult>("claude_login"),
  claudeLoginSubmitCode: (code: string) => call<LoginSubmitResult>("claude_login_submit_code", { code }),
  claudeLoginCancel: () => call<void>("claude_login_cancel"),
  claudeLogout: () => call<void>("claude_logout"),
  openUrl: (url: string) => call<void>("open_url", { url }),
  testClaude: () => call<string>("test_claude"),

  testAbleton: () => call<string>("test_ableton"),
  resetAbleton: () => call<string>("reset_ableton"),
  abletonBuild: (songId: string) => call<string>("ableton_build", { songId }),
  abletonBuildClips: (songId: string) => call<string>("ableton_build_clips", { songId }),
  abletonBuildSong: (songId: string) => call<string>("ableton_build_song", { songId }),
  detectAbletonMcp: () => call<{ found: boolean; name?: string; entry?: any }>("detect_ableton_mcp"),
  chatSend: (message: string, sessionId?: string, songId?: string) =>
    call<string>("chat_send", { message, sessionId: sessionId ?? null, songId: songId ?? null }),
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
