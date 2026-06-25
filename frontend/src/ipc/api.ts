// Typed IPC layer. Every Rust tool/command is wrapped here once. In Tauri it
// calls `invoke`; in a plain browser it falls back to the in-memory mock.

import type {
  Artifact,
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
  refineField: (stageLabel: string, fieldLabel: string, current: string, instruction: string) =>
    call<string>("refine_field", { stageLabel, fieldLabel, current, instruction }),
  deleteSong: (id: string) => call<void>("delete_song", { id }),
  getStage: (id: string) => call<StageDetail | null>("get_stage", { id }),
  runStage: (stageId: string, userInput?: string) =>
    call<Artifact>("run_stage", { stageId, userInput: userInput ?? null }),
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
