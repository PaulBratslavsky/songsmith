//! Tauri shell: wires the Rust core to the React frontend over IPC commands and
//! streams long-running stage runs back as events. The core owns all state.
//! Claude (the user's Claude Code subscription) is the engine, via the CLI + MCP.

use libsql::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
use song_core::models::*;
use song_core::{agent, db, tools};

struct AppState {
    conn: Connection,
    db_path: String,
    /// stage ids with an in-flight run, to dedupe concurrent requests.
    inflight: Mutex<HashSet<String>>,
    /// cancel handles for in-flight stage runs, keyed by stage id — `cancel_stage`
    /// fires the token; the run's own error path resets status + inflight.
    running: Mutex<HashMap<String, agent::CancelToken>>,
    /// the live `claude auth login --claudeai` child, kept alive so its stdin
    /// stays open for the pasted OAuth code (see `claude_login`).
    login_child: Mutex<Option<std::process::Child>>,
}

type R<T> = Result<T, String>;
fn e2s<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

// ---- Style presets ---------------------------------------------------------

#[tauri::command]
async fn list_style_presets(state: State<'_, AppState>) -> R<Vec<StylePreset>> {
    db::list_presets(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn get_style_preset(state: State<'_, AppState>, id: String) -> R<Option<StylePreset>> {
    db::get_preset(&state.conn, &id).await.map_err(e2s)
}
#[tauri::command]
async fn create_style_preset(state: State<'_, AppState>, input: StyleInput) -> R<StylePreset> {
    db::create_preset(&state.conn, input).await.map_err(e2s)
}
#[tauri::command]
async fn update_style_preset(state: State<'_, AppState>, id: String, input: StyleInput) -> R<StylePreset> {
    db::update_preset(&state.conn, &id, input).await.map_err(e2s)
}

/// Auto-generate a style preset from a name/seed. Streams `preset_token` events.
#[tauri::command]
async fn generate_style_preset(app: tauri::AppHandle, state: State<'_, AppState>, name: String, notes: Option<String>) -> R<StyleInput> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    agent::generate_style_preset(&state.conn, &settings, &name, notes.as_deref(), move |tok| {
        let _ = app.emit("preset_token", serde_json::json!({ "token": tok }));
    })
    .await
    .map_err(e2s)
}

// ---- Songs & stages --------------------------------------------------------

#[tauri::command]
async fn create_song(state: State<'_, AppState>, style_preset_id: String, title: String, intent: Option<String>) -> R<Song> {
    let song = db::create_song(&state.conn, &style_preset_id, &title).await.map_err(e2s)?;
    // optional north-star brief straight from the create form
    match intent.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(i) => db::update_song_intent(&state.conn, &song.id, i).await.map_err(e2s),
        None => Ok(song),
    }
}
#[tauri::command]
async fn list_songs(state: State<'_, AppState>) -> R<Vec<Song>> {
    db::list_songs(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn get_song(state: State<'_, AppState>, id: String) -> R<Option<SongDetail>> {
    db::get_song_detail(&state.conn, &id).await.map_err(e2s)
}
#[tauri::command]
async fn update_song_status(state: State<'_, AppState>, id: String, status: String) -> R<Song> {
    db::update_song_status(&state.conn, &id, &status).await.map_err(e2s)
}
#[tauri::command]
async fn update_song_title(state: State<'_, AppState>, id: String, title: String) -> R<Song> {
    db::update_song_title(&state.conn, &id, &title).await.map_err(e2s)
}
#[tauri::command]
async fn update_song_intent(state: State<'_, AppState>, id: String, intent: String) -> R<Song> {
    db::update_song_intent(&state.conn, &id, &intent).await.map_err(e2s)
}
#[tauri::command]
async fn update_song_key(state: State<'_, AppState>, id: String, root: String, mode: String, bpm: i64) -> R<Song> {
    db::update_song_key(&state.conn, &id, &root, &mode, bpm).await.map_err(e2s)
}
#[tauri::command]
async fn update_song_voicings(state: State<'_, AppState>, id: String, voicings: String) -> R<Song> {
    db::update_song_voicings(&state.conn, &id, &voicings).await.map_err(e2s)
}
#[tauri::command]
async fn import_reference(state: State<'_, AppState>, audio_path: String) -> R<String> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    song_core::agent::import_reference(&state.conn, &settings, &audio_path).await.map_err(e2s)
}
#[tauri::command]
async fn self_check_stage(state: State<'_, AppState>, stage_id: String) -> R<Artifact> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    song_core::agent::self_check_stage(&state.conn, &settings, &stage_id).await.map_err(e2s)
}
#[tauri::command]
async fn refine_field(state: State<'_, AppState>, stage_label: String, field_label: String, current: String, instruction: String, song_id: Option<String>) -> R<String> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    agent::refine_field(&state.conn, &settings, song_id.as_deref(), &stage_label, &field_label, &current, &instruction)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
async fn delete_song(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_song(&state.conn, &id).await.map_err(e2s)
}

// ---- Section spine (docs/SECTION-SPINE-SPEC.md — Phase 1: CRUD only) --------

#[tauri::command]
async fn list_sections(state: State<'_, AppState>, song_id: String) -> R<Vec<Section>> {
    db::list_sections(&state.conn, &song_id).await.map_err(e2s)
}
/// `position: None` appends at the end; `Some(p)` inserts at `p` (clamped).
#[tauri::command]
async fn create_section(state: State<'_, AppState>, song_id: String, label: String, section_type: String, bars: i64, role: String, position: Option<i64>) -> R<Section> {
    db::create_section(&state.conn, &song_id, &label, &section_type, bars, &role, position).await.map_err(e2s)
}
/// Form only (label/type/bars/role) — order changes go through `reorder_sections`.
#[tauri::command]
async fn update_section(state: State<'_, AppState>, id: String, label: String, section_type: String, bars: i64, role: String) -> R<Section> {
    db::update_section(&state.conn, &id, &label, &section_type, bars, &role).await.map_err(e2s)
}
#[tauri::command]
async fn delete_section(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_section(&state.conn, &id).await.map_err(e2s)
}
/// `section_ids` must be every section id of the song, each once, in the new order.
#[tauri::command]
async fn reorder_sections(state: State<'_, AppState>, song_id: String, section_ids: Vec<String>) -> R<Vec<Section>> {
    db::reorder_sections(&state.conn, &song_id, &section_ids).await.map_err(e2s)
}
/// Mid-session spine-birth union (docs/SECTION-SPINE-SPEC.md Phase 4): append
/// rows for sections that exist only in stage artifacts (first-seen order) —
/// the StructureEditor calls this when its save creates a song's first rows,
/// so a lyrics-only Bridge is never orphaned. Idempotent.
#[tauri::command]
async fn union_spine_sections(state: State<'_, AppState>, song_id: String) -> R<Vec<Section>> {
    song_core::spine::union_artifact_sections(&state.conn, &song_id).await.map_err(e2s)
}

// ---- Paste-lyrics import (spec Feature B — words kept verbatim) -------------

/// Dry-run parse of pasted lyrics (drives the preview modal — saves nothing).
#[tauri::command]
async fn parse_pasted_lyrics(state: State<'_, AppState>, text: String) -> R<agent::ParsedLyrics> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    agent::parse_pasted_lyrics(&settings, &text).await.map_err(e2s)
}

/// Import pasted lyrics into an existing song: replace the Lyrics artifact and
/// back-fill Structure to match. USER authority — replaces locked sections too
/// (the UI warns before confirm).
#[tauri::command]
async fn import_lyrics(state: State<'_, AppState>, song_id: String, text: String) -> R<()> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    agent::import_lyrics(&state.conn, &settings, &song_id, &text).await.map(|_| ()).map_err(e2s)
}

/// New song from pasted lyrics: mirrors `create_song`'s inputs, then imports.
#[tauri::command]
async fn create_song_from_lyrics(state: State<'_, AppState>, style_preset_id: String, title: String, text: String, intent: Option<String>) -> R<Song> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    let song = agent::create_song_from_lyrics(&state.conn, &settings, &style_preset_id, &title, &text).await.map_err(e2s)?;
    // optional north-star brief straight from the create form
    match intent.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        Some(i) => db::update_song_intent(&state.conn, &song.id, i).await.map_err(e2s),
        None => Ok(song),
    }
}

// ---- Composer export (composition → song) -----------------------------------

/// Export a composition's RESOLVED sections (label/bars/chords{name,beats} —
/// the frontend already mapped degrees to absolute names) into the source
/// song's Chords + Structure stages. 🔒 frozen sections are skipped and
/// preserved; returns `{ ok, skipped_frozen }`.
#[tauri::command]
async fn export_composition_to_song(state: State<'_, AppState>, song_id: String, sections_json: String) -> R<serde_json::Value> {
    agent::export_composition_to_song(&state.conn, &song_id, &sections_json).await.map_err(e2s)
}

/// New song from a composition: mirrors the create flow's preset/title inputs,
/// carries the composition's key/bpm, populates Structure + Chords from the
/// resolved sections. Lyrics stay empty.
#[tauri::command]
async fn create_song_from_composition(state: State<'_, AppState>, style_preset_id: String, title: String, key_root: String, key_mode: String, bpm: i64, sections_json: String) -> R<Song> {
    agent::create_song_from_composition(&state.conn, &style_preset_id, &title, &key_root, &key_mode, bpm, &sections_json).await.map_err(e2s)
}
#[tauri::command]
async fn get_stage(state: State<'_, AppState>, id: String) -> R<Option<StageDetail>> {
    db::get_stage_detail(&state.conn, &id).await.map_err(e2s)
}

/// Run a stage. Streams `stage_token` then a final `stage_done`. Deduped and
/// cancellable via `cancel_stage`.
#[tauri::command]
async fn run_stage(app: tauri::AppHandle, state: State<'_, AppState>, stage_id: String, user_input: Option<String>) -> R<Artifact> {
    {
        let mut set = state.inflight.lock().unwrap();
        if set.contains(&stage_id) {
            return Err("This stage is already running.".into());
        }
        set.insert(stage_id.clone());
    }
    let cancel = agent::CancelToken::new();
    state.running.lock().unwrap().insert(stage_id.clone(), cancel.clone());
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    let app2 = app.clone();
    let sid = stage_id.clone();
    let result = agent::run_stage(&state.conn, &settings, &stage_id, user_input, move |tok| {
        let _ = app2.emit("stage_token", serde_json::json!({ "stage_id": sid, "token": tok }));
    }, Some(cancel))
    .await;
    state.running.lock().unwrap().remove(&stage_id);
    state.inflight.lock().unwrap().remove(&stage_id);
    let outcome = result.map_err(e2s)?;
    let _ = app.emit("stage_done", serde_json::json!({ "stage_id": stage_id, "artifact_id": outcome.artifact.id }));
    Ok(outcome.artifact)
}

/// Cancel an in-flight stage run: fire its cancel token — the run's Claude child
/// is killed + reaped, `run_stage`'s error path restores the stage status and
/// clears the inflight entry. With no live run this doubles as a stranded-stage
/// rescue (e.g. after a crash left a stage stuck "in_progress").
#[tauri::command]
async fn cancel_stage(state: State<'_, AppState>, stage_id: String) -> R<()> {
    let token = state.running.lock().unwrap().remove(&stage_id);
    let had_run = token.is_some();
    if let Some(t) = token {
        t.cancel();
    }
    // defensive: idempotent no-op when the run's own cleanup already got there
    state.inflight.lock().unwrap().remove(&stage_id);
    if !had_run {
        // stranded-stage rescue: reset a stuck "in_progress" with no live run
        if let Ok(Some(stage)) = db::get_stage(&state.conn, &stage_id).await {
            if stage.status == "in_progress" {
                let cur = db::current_artifact(&state.conn, &stage_id).await.ok().flatten();
                match cur {
                    // an approved artifact means the stage had finished — restore "done"
                    Some(a) if a.approved => { let _ = db::set_stage_status(&state.conn, &stage_id, "done").await; }
                    // an unapproved artifact + in_progress is the normal post-run state — leave it
                    Some(_) => {}
                    // no artifact at all: the run died before producing anything
                    None => { let _ = db::set_stage_status(&state.conn, &stage_id, "pending").await; }
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn approve_stage(state: State<'_, AppState>, stage_id: String) -> R<serde_json::Value> {
    tools::approve_stage(&state.conn, &stage_id).await.map_err(e2s)
}
#[tauri::command]
async fn advance_stage(state: State<'_, AppState>, song_id: String) -> R<serde_json::Value> {
    tools::advance_song(&state.conn, &song_id).await.map_err(e2s)
}

// ---- Artifacts -------------------------------------------------------------

#[tauri::command]
async fn get_artifact(state: State<'_, AppState>, id: String) -> R<Option<Artifact>> {
    db::get_artifact(&state.conn, &id).await.map_err(e2s)
}
/// TRUST MODEL: this command is the USER's own editor saving — it is the
/// unlock/rewrite authority and deliberately bypasses the freeze guard (that is
/// how unfreezing works). Claude-driven writes go through the guarded MCP tool
/// arms in `tools::dispatch` instead and can never violate frozen sections.
#[tauri::command]
async fn save_artifact(state: State<'_, AppState>, song_id: String, stage_id: Option<String>, kind: String, content: String) -> R<Artifact> {
    db::save_artifact(&state.conn, &song_id, stage_id.as_deref(), &kind, &content).await.map_err(e2s)
}
#[tauri::command]
async fn list_artifact_revisions(state: State<'_, AppState>, stage_id: String) -> R<Vec<Artifact>> {
    db::list_artifact_revisions(&state.conn, &stage_id).await.map_err(e2s)
}
/// Direct (user-authority) revert — snapshot-based (docs/SECTION-SPINE-SPEC.md
/// §Snapshots): spine rows the revision references that no longer exist are
/// re-created from its embedded `spine_snapshot` before the content is
/// restored verbatim, so its section_ids reattach.
#[tauri::command]
async fn revert_artifact(state: State<'_, AppState>, artifact_id: String) -> R<Artifact> {
    song_core::spine::revert_artifact(&state.conn, &artifact_id).await.map_err(e2s)
}
/// Name (or clear) a revision in the History timeline — metadata only.
#[tauri::command]
async fn set_artifact_label(state: State<'_, AppState>, artifact_id: String, label: Option<String>) -> R<()> {
    db::set_artifact_label(&state.conn, &artifact_id, label.as_deref()).await.map_err(e2s)
}

// ---- Skills ----------------------------------------------------------------

#[tauri::command]
async fn list_skills(state: State<'_, AppState>) -> R<Vec<Skill>> {
    db::list_skills(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn get_skill(state: State<'_, AppState>, id: String) -> R<Option<Skill>> {
    db::get_skill(&state.conn, &id).await.map_err(e2s)
}
#[tauri::command]
async fn create_skill(state: State<'_, AppState>, input: SkillInput) -> R<Skill> {
    db::create_skill(&state.conn, input).await.map_err(e2s)
}
#[tauri::command]
async fn update_skill(state: State<'_, AppState>, id: String, input: SkillInput) -> R<Skill> {
    db::update_skill(&state.conn, &id, input).await.map_err(e2s)
}
#[tauri::command]
async fn set_skill_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> R<Skill> {
    db::set_skill_enabled(&state.conn, &id, enabled).await.map_err(e2s)
}

// ---- Saved progressions ----------------------------------------------------

#[tauri::command]
async fn list_progressions(state: State<'_, AppState>) -> R<Vec<Progression>> {
    db::list_progressions(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn save_progression(state: State<'_, AppState>, name: String, chords: Vec<String>) -> R<Progression> {
    db::create_progression(&state.conn, &name, &chords).await.map_err(e2s)
}
#[tauri::command]
async fn delete_progression(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_progression(&state.conn, &id).await.map_err(e2s)
}

// ---- Saved compositions (Composer sketches / full-song exports) -------------

#[tauri::command]
async fn list_compositions(state: State<'_, AppState>) -> R<Vec<CompositionMeta>> {
    db::list_compositions(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn get_composition(state: State<'_, AppState>, id: String) -> R<Option<CompositionRow>> {
    db::get_composition(&state.conn, &id).await.map_err(e2s)
}
/// `id: None` inserts (the frontend adopts the minted row id); `Some` updates
/// in place. `data` is the zod-validated Composition JSON blob.
#[tauri::command]
async fn save_composition(state: State<'_, AppState>, id: Option<String>, name: String, song_id: Option<String>, data: String) -> R<CompositionRow> {
    db::save_composition(&state.conn, id.as_deref(), &name, song_id.as_deref(), &data).await.map_err(e2s)
}
#[tauri::command]
async fn delete_composition(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_composition(&state.conn, &id).await.map_err(e2s)
}

// ---- Final renders ---------------------------------------------------------

#[tauri::command]
async fn list_renders(state: State<'_, AppState>, song_id: String) -> R<Vec<Render>> {
    db::list_renders(&state.conn, &song_id).await.map_err(e2s)
}
#[tauri::command]
async fn add_render(state: State<'_, AppState>, song_id: String, label: String, file_path: String, source: String, notes: String) -> R<Render> {
    db::create_render(&state.conn, &song_id, &label, &file_path, &source, &notes).await.map_err(e2s)
}
#[tauri::command]
async fn set_render_pick(state: State<'_, AppState>, id: String, is_pick: bool) -> R<()> {
    db::set_render_pick(&state.conn, &id, is_pick).await.map_err(e2s)
}
#[tauri::command]
async fn delete_render(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_render(&state.conn, &id).await.map_err(e2s)
}

// ---- Settings & meta -------------------------------------------------------

#[tauri::command]
async fn get_settings(state: State<'_, AppState>) -> R<Settings> {
    db::get_settings(&state.conn).await.map_err(e2s)
}
#[tauri::command]
async fn set_settings(state: State<'_, AppState>, settings: Settings) -> R<Settings> {
    db::set_settings(&state.conn, &settings).await.map_err(e2s)?;
    Ok(settings)
}
#[tauri::command]
fn list_tools() -> Vec<serde_json::Value> {
    tools::registry().into_iter().map(|t| serde_json::json!({ "name": t.name, "description": t.description, "destructive": t.destructive })).collect()
}
#[tauri::command]
async fn mcp_config(state: State<'_, AppState>) -> R<serde_json::Value> {
    // NOTE: the old "token" field was generated + advertised but never
    // validated anywhere — deleted (audit Tier-2 #13) rather than pretending
    // it's auth. Access control = filesystem access to the DB path.
    Ok(serde_json::json!({
        "db_path": state.db_path,
        "command_hint": format!("SONGSMITH_DB=\"{}\" claude mcp add songsmith --scope user -- /path/to/mcp-shim", state.db_path),
    }))
}

fn find_claude() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("CLAUDE_BIN") {
        let p = std::path::PathBuf::from(p);
        if p.exists() { return Some(p); }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    for c in [format!("{home}/.local/bin/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into(), "/usr/bin/claude".into()] {
        let p = std::path::PathBuf::from(&c);
        if p.exists() { return Some(p); }
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    if let Ok(out) = std::process::Command::new(shell).args(["-lc", "command -v claude"]).output() {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !path.is_empty() && std::path::Path::new(&path).exists() {
            return Some(path.into());
        }
    }
    None
}

fn find_shim(resource_dir: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    let mut c: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() { c.push(d.join("mcp-shim")); }
    }
    if let Some(r) = resource_dir { c.push(r.join("mcp-shim")); }
    for p in ["target/debug/mcp-shim", "target/release/mcp-shim", "../target/debug/mcp-shim", "../../target/debug/mcp-shim", "../../target/release/mcp-shim"] {
        c.push(std::path::PathBuf::from(p));
    }
    c.into_iter().find(|p| p.exists())
}

fn ensure_mcp_config(app: &tauri::AppHandle, db_path: &str, ableton: &str) -> Result<std::path::PathBuf, String> {
    let shim = find_shim(app.path().resource_dir().ok()).ok_or("mcp-shim binary not found")?;
    let mut servers = serde_json::json!({
        "songsmith": { "command": shim.to_string_lossy(), "env": { "SONGSMITH_DB": db_path } }
    });
    // merge an optional extra MCP server (e.g. Ableton), under the name "ableton"
    if let Ok(entry) = serde_json::from_str::<serde_json::Value>(ableton.trim()) {
        if entry.is_object() {
            servers["ableton"] = entry;
        }
    }
    let cfg = serde_json::json!({ "mcpServers": servers });
    let dir = std::path::Path::new(db_path).parent().ok_or("bad db path")?;
    let path = dir.join("mcp.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&cfg).unwrap()).map_err(e2s)?;
    Ok(path)
}

/// Try to find an Ableton MCP server in the user's Claude Desktop config so the
/// user can connect it with one click. Returns the server entry JSON or null.
#[tauri::command]
fn detect_ableton_mcp() -> serde_json::Value {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = format!("{home}/Library/Application Support/Claude/claude_desktop_config.json");
    let Ok(txt) = std::fs::read_to_string(&path) else { return serde_json::json!({ "found": false }) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { return serde_json::json!({ "found": false }) };
    if let Some(servers) = v.get("mcpServers").and_then(|m| m.as_object()) {
        for (name, entry) in servers {
            let hay = format!("{name} {entry}").to_lowercase();
            if hay.contains("ableton") {
                return serde_json::json!({ "found": true, "name": name, "entry": entry });
            }
        }
    }
    serde_json::json!({ "found": false })
}

#[tauri::command]
fn mcp_setup_command(app: tauri::AppHandle, state: State<'_, AppState>) -> serde_json::Value {
    let Some(shim) = find_shim(app.path().resource_dir().ok()) else {
        return serde_json::json!({ "available": false });
    };
    let shim = shim.to_string_lossy().to_string();
    let db = &state.db_path;
    let command = format!(
        "chmod +x \"{shim}\" 2>/dev/null; claude mcp get songsmith >/dev/null 2>&1 || \
         claude mcp add songsmith --scope user --env \"SONGSMITH_DB={db}\" -- \"{shim}\""
    );
    serde_json::json!({ "available": true, "shim_path": shim, "command": command })
}

/// Chat with Claude headless + the harness MCP server. Streams `chat_event`.
#[tauri::command]
async fn chat_send(app: tauri::AppHandle, state: State<'_, AppState>, message: String, session_id: Option<String>, song_id: Option<String>) -> R<String> {
    let claude = find_claude().ok_or("The `claude` CLI was not found. Install Claude Code and sign in.")?;
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    let cfg = ensure_mcp_config(&app, &state.db_path, &settings.ableton_mcp)?;
    let resuming = session_id.is_some();
    let sid = session_id.unwrap_or_else(song_core::db::new_id);

    // Standing context: which song the user is viewing, plus how to act in Ableton
    // and a hard honesty rule (Claude must not claim tool results it didn't get).
    let mut preamble = String::from(
        "You are the in-app assistant for Songsmith Studio, a songwriting harness where Claude is the engine. \
The Songsmith MCP (mcp__songsmith__*) reads/writes the song; the Ableton MCP (mcp__ableton__*) drives Ableton Live.\n\n\
HONESTY: Never claim an action succeeded unless the matching tool call actually returned success. \
If the Ableton tools (mcp__ableton__*) are not present, tell the user plainly that Ableton isn't connected \
(Settings → Ableton MCP) — do NOT describe imaginary results or output a table pretending it's done.\n\n\
ABLETON STRUCTURE: the connected ableton MCP (ableton_mcp 1.2.0) has NO locator tool — never claim you made \
locators. To lay out the song: list the mcp__ableton__ tools, then set_tempo, switch_to_arrangement_view, and for \
EACH section in order create_clip + set_clip_name (section name) + duplicate_to_arrangement at the running bar \
offset from the section bar counts, so the timeline shows named clips per section. Always report exactly what you \
created; if the ableton tools aren't reachable, say so plainly.",
    );
    if let Some(ref id) = song_id {
        if let Ok(Some(s)) = db::get_song(&state.conn, id).await {
            preamble.push_str(&format!(
                "\n\nCURRENT SONG: the user is viewing \"{}\" (song_id {}) — {} {}, {} BPM, on the \"{}\" stage. \
When they say \"this song\"/\"the song\"/\"here\", act on song_id {}.",
                s.title, s.id, s.key_root, s.key_mode, s.bpm, s.current_stage, s.id,
            ));
        }
        // Embed the section map (with bar counts) from the Structure stage so the
        // chat can place one Ableton locator per section precisely, without
        // guessing — parsed by the ONE core section parser (ableton::song_sections).
        let sections = song_core::ableton::song_sections(&state.conn, id).await;
        if !sections.is_empty() {
            let list: Vec<String> = sections.iter().map(|(label, bars)| format!("{label} ({bars} bars)")).collect();
            preamble.push_str(&format!(
                "\n\nSECTIONS (in order, with bar counts) — make exactly one Ableton locator per \
section, placed at the running bar offset from these counts: {}.",
                list.join(", "),
            ));
        }
    }

    let mut args: Vec<String> = vec![
        "-p".into(), message,
        "--output-format".into(), "stream-json".into(), "--verbose".into(),
        "--mcp-config".into(), cfg.to_string_lossy().to_string(),
        "--allowedTools".into(), "mcp__songsmith".into(), "mcp__ableton".into(),
        "--disallowedTools".into(), "mcp__songsmith__delete_song".into(),
        "--permission-mode".into(), "acceptEdits".into(),
        "--append-system-prompt".into(), preamble,
        "-n".into(), "songsmith".into(),
    ];
    if resuming { args.push("--resume".into()); args.push(sid.clone()); }
    else { args.push("--session-id".into()); args.push(sid.clone()); }

    let app2 = app.clone();
    let sid2 = sid.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::BufRead;
        let mut child = std::process::Command::new(claude)
            .args(&args)
            // use the Claude Code subscription login, not an inherited API key
            .env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start claude: {e}"))?;
        let stdout = child.stdout.take().unwrap();
        // drain stderr CONCURRENTLY — reading it only after wait() deadlocks once
        // the CLI writes more than the pipe buffer (~64KB) mid-run
        let stderr = child.stderr.take();
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(mut se) = stderr {
                use std::io::Read;
                let _ = se.read_to_string(&mut buf);
            }
            buf
        });
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() { continue; }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                let _ = app2.emit("chat_event", serde_json::json!({ "session_id": sid2, "event": v }));
            }
        }
        let status = child.wait().map_err(e2s)?;
        let err = stderr_thread.join().unwrap_or_default();
        if !status.success() {
            let err = err.trim().to_string();
            return Err(if err.is_empty() { "claude exited with an error".into() } else { err });
        }
        Ok(())
    })
    .await
    .map_err(e2s)?;

    match result {
        Ok(()) => { let _ = app.emit("chat_done", serde_json::json!({ "session_id": sid })); Ok(sid) }
        Err(e) => { let _ = app.emit("chat_error", serde_json::json!({ "session_id": sid, "error": e })); Err(e) }
    }
}

fn ensure_claude_bin(conn: &Connection) {
    let Ok(mut s) = tauri::async_runtime::block_on(db::get_settings(conn)) else { return };
    if !s.claude_bin.is_empty() && std::path::Path::new(&s.claude_bin).exists() { return; }
    if let Some(p) = find_claude() {
        s.claude_bin = p.to_string_lossy().to_string();
        let _ = tauri::async_runtime::block_on(db::set_settings(conn, &s));
    }
}

#[tauri::command]
async fn claude_status(state: State<'_, AppState>) -> R<serde_json::Value> {
    let s = db::get_settings(&state.conn).await.map_err(e2s)?;
    let bin = find_claude();
    let found = bin.is_some();
    let version = tokio::task::spawn_blocking(move || {
        bin.and_then(|b| std::process::Command::new(b).arg("--version").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()))
    })
    .await
    .unwrap_or(None);
    Ok(serde_json::json!({ "found": found, "version": version, "model": s.claude_model, "bin": s.claude_bin }))
}

// ---- Connect Claude account (subscription auth — never an API key) ----------

const CONNECTORS_URL: &str = "https://claude.ai/settings/connectors";

/// Is an Anthropic API key / auth token present in the app's environment? A key
/// forces API-billing mode and **disables connectors**, so we surface it as a
/// warning. The app never sets one — it only ever strips it from spawned
/// `claude` processes (which is exactly what keeps connectors enabled).
fn api_key_in_env() -> bool {
    std::env::var_os("ANTHROPIC_API_KEY").is_some() || std::env::var_os("ANTHROPIC_AUTH_TOKEN").is_some()
}

/// Read the subscription auth state from the CLI's own `claude auth status --json`
/// (API key stripped so we report the *subscription* login, not an inherited key).
#[tauri::command]
async fn claude_auth_status(state: State<'_, AppState>) -> R<serde_json::Value> {
    let s = db::get_settings(&state.conn).await.map_err(e2s)?;
    let bin = find_claude();
    let found = bin.is_some();
    let bin_path = bin
        .as_ref()
        .map(|b| b.to_string_lossy().to_string())
        .unwrap_or_else(|| s.claude_bin.clone());
    let api_key_set = api_key_in_env();

    // Probe the CLI's own auth status with the API key stripped, so we read the
    // claude.ai **subscription** login rather than an inherited API key.
    let probe = tokio::task::spawn_blocking(move || -> Option<serde_json::Value> {
        let bin = bin?;
        let out = std::process::Command::new(bin)
            .args(["auth", "status", "--json"])
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .output()
            .ok()?;
        serde_json::from_slice::<serde_json::Value>(&out.stdout).ok()
    })
    .await
    .unwrap_or(None);

    let logged_in = probe.as_ref().and_then(|v| v.get("loggedIn")).and_then(|v| v.as_bool()).unwrap_or(false);
    let account = probe.as_ref().and_then(|v| {
        v.get("email").and_then(|e| e.as_str()).filter(|e| !e.is_empty())
            .or_else(|| v.get("orgName").and_then(|o| o.as_str()).filter(|o| !o.is_empty()))
            .map(|s| s.to_string())
    });
    let subscription = probe.as_ref().and_then(|v| v.get("subscriptionType")).and_then(|v| v.as_str()).map(|s| s.to_string());

    let connectors_hint = if api_key_set {
        "An Anthropic API key is set in this app's environment — it forces API billing and turns OFF your connectors. Unset ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN; the app strips them from Claude anyway, which is what keeps your connectors available.".to_string()
    } else if logged_in {
        "Signed in on your subscription with no API key set — your connectors load automatically. Connect them on the claude.ai connectors page.".to_string()
    } else {
        "Sign in with your claude.ai account to use your subscription. Keep ANTHROPIC_API_KEY unset so your connectors load.".to_string()
    };

    Ok(serde_json::json!({
        "found": found,
        "bin": bin_path,
        "logged_in": logged_in,
        "account": account,
        "subscription": subscription,
        "api_key_set": api_key_set,
        "connectors_hint": connectors_hint,
        "connectors_url": CONNECTORS_URL,
    }))
}

/// Scan a chunk of CLI output for the first `https://…` URL (the printed OAuth
/// link), returning it trimmed at the first whitespace.
fn extract_url(text: &str) -> Option<String> {
    let idx = text.find("https://")?;
    let url: String = text[idx..].split_whitespace().next().unwrap_or("").to_string();
    if url.is_empty() { None } else { Some(url) }
}

/// Spawn the CLI subscription login (`claude auth login --claudeai`) and **stream**
/// its stdout/stderr as `login_event` events so the UI can show the auth URL and
/// progress, opening the browser when the URL is printed. The API key is stripped
/// so this signs into the claude.ai subscription (not an API console).
///
/// The CLI's OAuth flow prints the auth URL, then the prompt `Paste code here if
/// prompted > ` (no trailing newline) and **blocks reading the code on stdin**. So
/// we spawn it with **stdin = piped** and keep the child alive in
/// `AppState.login_child`; the user finishes in the browser, copies the code, and
/// `claude_login_submit_code` writes it to that stdin to complete the flow — no
/// terminal required.
#[tauri::command]
async fn claude_login(app: tauri::AppHandle, state: State<'_, AppState>) -> R<serde_json::Value> {
    let claude = find_claude().ok_or("The `claude` CLI was not found. Install Claude Code first.")?;

    // If a previous attempt is still around, kill it so we start clean.
    if let Some(mut old) = state.login_child.lock().unwrap().take() {
        let _ = old.kill();
        let _ = old.wait();
    }

    let mut child = tokio::task::spawn_blocking(move || -> Result<std::process::Child, String> {
        std::process::Command::new(&claude)
            .args(["auth", "login", "--claudeai"])
            // subscription login, never an API key
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            // piped stdin: the flow prints the URL then blocks on `Paste code here`
            // — we keep the child alive and write the pasted code to this stdin.
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not start claude: {e}"))
    })
    .await
    .map_err(e2s)??;

    // Take stdout/stderr to stream; leave stdin attached on the child we hold.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Stream stdout in chunks (the "Paste code" prompt has no trailing newline,
    // so line-buffered reads would never surface it; read raw bytes instead).
    if let Some(mut out) = stdout {
        let app2 = app.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 1024];
            let mut found_url = false;
            loop {
                match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                        if !found_url {
                            if let Some(url) = extract_url(&chunk) {
                                found_url = true;
                                let _ = open_url_inner(&url);
                                let _ = app2.emit("login_event", serde_json::json!({ "url": url }));
                            }
                        }
                        let _ = app2.emit("login_event", serde_json::json!({ "line": chunk.trim_end_matches('\n') }));
                    }
                }
            }
        });
    }
    // Stream stderr (extra hints / errors).
    if let Some(mut err) = stderr {
        let app2 = app.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 1024];
            loop {
                match err.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]);
                        let line = chunk.trim();
                        if !line.is_empty() {
                            let _ = app2.emit("login_event", serde_json::json!({ "line": line }));
                        }
                    }
                }
            }
        });
    }

    // Hold the live child so its stdin stays open for the pasted code.
    *state.login_child.lock().unwrap() = Some(child);

    Ok(serde_json::json!({
        "url": null,
        "instructions": "Finish signing in in the browser tab that opened, then paste the authentication code shown by claude.ai into the box below and click Submit. If no tab opened, copy the auth URL above into your browser first.",
    }))
}

/// Write the pasted OAuth **authorization code** (+ newline) to the held login
/// child's stdin, wait for it to exit, and emit `login_done` with success/failure.
/// Clears the stored child either way (the attempt is over).
#[tauri::command]
async fn claude_login_submit_code(app: tauri::AppHandle, state: State<'_, AppState>, code: String) -> R<serde_json::Value> {
    let mut child = state
        .login_child
        .lock()
        .unwrap()
        .take()
        .ok_or("No login in progress. Click Log in first.")?;

    let result = tokio::task::spawn_blocking(move || -> Result<bool, String> {
        use std::io::Write;
        {
            let stdin = child.stdin.as_mut().ok_or("login process has no stdin")?;
            stdin
                .write_all(format!("{}\n", code.trim()).as_bytes())
                .map_err(|e| format!("could not send code: {e}"))?;
            stdin.flush().ok();
        }
        // Dropping stdin signals EOF after the code line; then wait for exit.
        drop(child.stdin.take());
        let status = child.wait().map_err(e2s)?;
        Ok(status.success())
    })
    .await
    .map_err(e2s)?;

    let success = result?;
    let _ = app.emit("login_done", serde_json::json!({ "success": success }));
    if success {
        Ok(serde_json::json!({ "success": true, "message": "Signed in. Re-checking status…" }))
    } else {
        Err("Login did not complete — the code may be wrong or expired. Click Log in to try again.".into())
    }
}

/// Kill the held login child (so a stuck/abandoned attempt can be reset) and clear it.
#[tauri::command]
async fn claude_login_cancel(state: State<'_, AppState>) -> R<()> {
    if let Some(mut child) = state.login_child.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(())
}

/// Sign out of the claude.ai subscription via `claude auth logout` (API key
/// stripped). The UI calls this then re-checks status.
#[tauri::command]
async fn claude_logout() -> R<()> {
    let claude = find_claude().ok_or("The `claude` CLI was not found.")?;
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let out = std::process::Command::new(&claude)
            .args(["auth", "logout"])
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .output()
            .map_err(|e| format!("could not start claude: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            Err(if msg.is_empty() { "logout failed".into() } else { msg })
        }
    })
    .await
    .map_err(e2s)?
}

/// Open a URL in the user's default browser (auth URL, connectors page).
fn open_url_inner(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(target_os = "linux")]
    let prog = "xdg-open";
    #[cfg(target_os = "windows")]
    let prog = "explorer";
    std::process::Command::new(prog).arg(url).spawn().map(|_| ()).map_err(e2s)
}

#[tauri::command]
fn open_url(url: String) -> R<()> {
    open_url_inner(&url)
}

/// Test the live subscription path end-to-end: ask Claude for one word with the
/// API key stripped. Success confirms the login + engine work; an auth error
/// means the user needs to (re-)sign in.
#[tauri::command]
async fn test_claude(state: State<'_, AppState>) -> R<String> {
    let claude = find_claude().ok_or("The `claude` CLI was not found. Install Claude Code and sign in.")?;
    let _ = &state;
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let out = std::process::Command::new(&claude)
            .args(["-p", "Reply with exactly: READY", "--max-turns", "1"])
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .output()
            .map_err(|e| format!("could not start claude: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if out.status.success() && !stdout.is_empty() {
            Ok(format!("✅ Live Claude responded: {stdout}"))
        } else {
            let msg = if !stderr.is_empty() { stderr } else { stdout };
            let lower = msg.to_lowercase();
            if lower.contains("login") || lower.contains("auth") || lower.contains("sign in") || lower.contains("unauthor") {
                Err(format!("Not signed in — run Log in, then Re-check. ({msg})"))
            } else {
                Err(format!("Claude returned an error: {msg}"))
            }
        }
    })
    .await
    .map_err(e2s)?
}

/// Write raw bytes (e.g. an exported PNG) to a user-chosen path.
#[tauri::command]
fn write_png(path: String, bytes: Vec<u8>) -> R<String> {
    std::fs::write(&path, bytes).map_err(e2s)?;
    Ok(path)
}

/// Probe the Ableton Live Remote Script socket (127.0.0.1:9877) directly — no MCP,
/// no Claude — and report whether Live is reachable, responding, or busy.
#[tauri::command]
fn test_ableton() -> R<String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;
    let addr = "127.0.0.1:9877".parse().map_err(e2s)?;
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) {
        Ok(s) => s,
        Err(e) => return Ok(format!(
            "❌ Can't reach Ableton on 127.0.0.1:9877 ({e}).\nOpen Live and enable the AbletonMCP control surface (Preferences → Link/Tempo/MIDI → Control Surface = AbletonMCP)."
        )),
    };
    stream.set_read_timeout(Some(Duration::from_millis(2500))).ok();
    stream.set_write_timeout(Some(Duration::from_millis(1500))).ok();
    if let Err(e) = stream.write_all(b"{\"type\":\"get_session_info\",\"params\":{}}") {
        return Ok(format!("⚠️ Connected to 9877 but couldn't send ({e})."));
    }
    let mut buf = [0u8; 8192];
    match stream.read(&mut buf) {
        Ok(0) | Err(_) => Ok(
            "⚠️ Live is listening but didn't respond — another MCP client is holding the Remote Script (only one connection at a time). Quit other clients (Claude Desktop, a stray `uvx ableton-mcp`) or click \"Free connection\"."
                .into(),
        ),
        Ok(n) => {
            let txt = String::from_utf8_lossy(&buf[..n]);
            let snippet: String = txt.chars().take(400).collect();
            if txt.contains("\"status\"") || txt.contains("tempo") {
                Ok(format!("✅ Ableton is responding on 9877.\n{snippet}"))
            } else {
                Ok(format!("⚠️ Unexpected reply: {snippet}"))
            }
        }
    }
}

/// Programmatically build the song's section structure in Ableton's Arrangement
/// as named LOCATORS — direct socket, no MCP/LLM. Thin wrapper over the core
/// builder (song_core::ableton), where the moved logic + section parsing live.
#[tauri::command]
async fn ableton_build(state: State<'_, AppState>, song_id: String) -> R<String> {
    song_core::ableton::build_locators_for(&state.conn, &song_id).await.map_err(e2s)
}

/// Build the song structure in Ableton's Arrangement as named, color-coded CLIPS
/// on a "Sections" track — thin wrapper over song_core::ableton::build_clips_for.
#[tauri::command]
async fn ableton_build_clips(state: State<'_, AppState>, song_id: String) -> R<String> {
    song_core::ableton::build_clips_for(&state.conn, &song_id).await.map_err(e2s)
}

/// Stub the whole song in Ableton's Arrangement: Sections clip track + Bass /
/// Chords / Pad / Chord melody / Filler / Arp MIDI parts from the progression.
/// Thin wrapper over song_core::ableton::build_song_for (also an MCP tool).
#[tauri::command]
async fn ableton_build_song(state: State<'_, AppState>, song_id: String) -> R<String> {
    song_core::ableton::build_song_for(&state.conn, &song_id).await.map_err(e2s)
}

/// Free the single Ableton socket by stopping stray standalone `ableton-mcp`
/// processes squatting on it (run this when Test reports "busy").
#[tauri::command]
fn reset_ableton() -> R<String> {
    let out = std::process::Command::new("pkill").arg("-f").arg("bin/ableton-mcp").output().map_err(e2s)?;
    let code = out.status.code().unwrap_or(-1);
    Ok(match code {
        0 => "Stopped stray ableton-mcp process(es). Re-run Test, then try Structure in Ableton.".into(),
        1 => "No stray ableton-mcp process found — the connection wasn't being squatted. If Test still says busy, check Claude Desktop.".into(),
        _ => format!("pkill exited with code {code}."),
    })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&dir).ok();
            let db_path = dir.join("songsmith-studio.db");
            let db_path_str = db_path.to_string_lossy().to_string();

            let database = tauri::async_runtime::block_on(song_core::db::open(&db_path)).expect("open database");
            // db::connect sets PRAGMA busy_timeout so writes racing the mcp-shim
            // wait for the lock instead of erroring "database is locked"
            let conn = tauri::async_runtime::block_on(song_core::db::connect(&database)).expect("connect database");
            std::mem::forget(database);

            ensure_claude_bin(&conn);
            app.manage(AppState { conn, db_path: db_path_str, inflight: Mutex::new(HashSet::new()), running: Mutex::new(HashMap::new()), login_child: Mutex::new(None) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_style_presets,
            get_style_preset,
            create_style_preset,
            update_style_preset,
            generate_style_preset,
            create_song,
            list_songs,
            get_song,
            update_song_status,
            update_song_title,
            update_song_intent,
            update_song_key,
            update_song_voicings,
            list_sections,
            create_section,
            update_section,
            delete_section,
            reorder_sections,
            union_spine_sections,
            import_reference,
            parse_pasted_lyrics,
            import_lyrics,
            create_song_from_lyrics,
            export_composition_to_song,
            create_song_from_composition,
            self_check_stage,
            refine_field,
            delete_song,
            get_stage,
            run_stage,
            cancel_stage,
            approve_stage,
            advance_stage,
            get_artifact,
            save_artifact,
            list_artifact_revisions,
            revert_artifact,
            set_artifact_label,
            list_skills,
            get_skill,
            create_skill,
            update_skill,
            set_skill_enabled,
            list_progressions,
            save_progression,
            delete_progression,
            list_compositions,
            get_composition,
            save_composition,
            delete_composition,
            list_renders,
            add_render,
            set_render_pick,
            delete_render,
            get_settings,
            set_settings,
            list_tools,
            mcp_config,
            mcp_setup_command,
            claude_status,
            claude_auth_status,
            claude_login,
            claude_login_submit_code,
            claude_login_cancel,
            claude_logout,
            open_url,
            test_claude,
            detect_ableton_mcp,
            chat_send,
            write_png,
            test_ableton,
            reset_ableton,
            ableton_build,
            ableton_build_clips,
            ableton_build_song,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    // The login flow's load-bearing pure bit: pulling the OAuth URL out of the
    // exact line `claude auth login --claudeai` prints (verified against the CLI).
    #[test]
    fn extracts_auth_url_from_cli_line() {
        let line = "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz";
        assert_eq!(
            extract_url(line).as_deref(),
            Some("https://claude.com/cai/oauth/authorize?code=true&client_id=abc&state=xyz"),
        );
    }

    #[test]
    fn extracts_url_stops_at_whitespace_and_handles_none() {
        assert_eq!(extract_url("go to https://x.test/auth now").as_deref(), Some("https://x.test/auth"));
        // The "Paste code here if prompted > " prompt carries no URL.
        assert_eq!(extract_url("Paste code here if prompted > "), None);
    }
}
