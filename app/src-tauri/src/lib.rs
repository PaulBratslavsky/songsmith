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
async fn create_song(state: State<'_, AppState>, style_preset_id: String, title: String) -> R<Song> {
    db::create_song(&state.conn, &style_preset_id, &title).await.map_err(e2s)
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
async fn refine_field(state: State<'_, AppState>, stage_label: String, field_label: String, current: String, instruction: String) -> R<String> {
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    agent::refine_field(&settings, &stage_label, &field_label, &current, &instruction).await.map_err(|e| e.to_string())
}
#[tauri::command]
async fn delete_song(state: State<'_, AppState>, id: String) -> R<()> {
    db::delete_song(&state.conn, &id).await.map_err(e2s)
}
#[tauri::command]
async fn get_stage(state: State<'_, AppState>, id: String) -> R<Option<StageDetail>> {
    db::get_stage_detail(&state.conn, &id).await.map_err(e2s)
}

/// Run a stage. Streams `stage_token` then a final `stage_done`. Deduped.
#[tauri::command]
async fn run_stage(app: tauri::AppHandle, state: State<'_, AppState>, stage_id: String, user_input: Option<String>) -> R<Artifact> {
    {
        let mut set = state.inflight.lock().unwrap();
        if set.contains(&stage_id) {
            return Err("This stage is already running.".into());
        }
        set.insert(stage_id.clone());
    }
    let settings = db::get_settings(&state.conn).await.map_err(e2s)?;
    let app2 = app.clone();
    let sid = stage_id.clone();
    let result = agent::run_stage(&state.conn, &settings, &stage_id, user_input, move |tok| {
        let _ = app2.emit("stage_token", serde_json::json!({ "stage_id": sid, "token": tok }));
    })
    .await;
    state.inflight.lock().unwrap().remove(&stage_id);
    let outcome = result.map_err(e2s)?;
    let _ = app.emit("stage_done", serde_json::json!({ "stage_id": stage_id, "artifact_id": outcome.artifact.id }));
    Ok(outcome.artifact)
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
#[tauri::command]
async fn save_artifact(state: State<'_, AppState>, song_id: String, stage_id: Option<String>, kind: String, content: String) -> R<Artifact> {
    db::save_artifact(&state.conn, &song_id, stage_id.as_deref(), &kind, &content).await.map_err(e2s)
}
#[tauri::command]
async fn list_artifact_revisions(state: State<'_, AppState>, stage_id: String) -> R<Vec<Artifact>> {
    db::list_artifact_revisions(&state.conn, &stage_id).await.map_err(e2s)
}
#[tauri::command]
async fn revert_artifact(state: State<'_, AppState>, artifact_id: String) -> R<Artifact> {
    db::revert_artifact(&state.conn, &artifact_id).await.map_err(e2s)
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
    let token = db::ensure_mcp_token(&state.conn).await.map_err(e2s)?;
    Ok(serde_json::json!({
        "db_path": state.db_path, "token": token,
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
        // chat can place one Ableton locator per section precisely, without guessing.
        if let Ok(stages) = db::list_stages(&state.conn, id).await {
            if let Some(st) = stages.iter().find(|s| s.r#type == "structure") {
                if let Ok(Some(art)) = db::current_artifact(&state.conn, &st.id).await {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&art.content) {
                        if let Some(secs) = v.get("data").and_then(|d| d.get("sections")).and_then(|s| s.as_array()) {
                            let list: Vec<String> = secs.iter().filter_map(|s| {
                                let label = s.get("label").and_then(|x| x.as_str())?;
                                match s.get("bars").and_then(|x| x.as_i64()) {
                                    Some(b) => Some(format!("{label} ({b} bars)")),
                                    None => Some(label.to_string()),
                                }
                            }).collect();
                            if !list.is_empty() {
                                preamble.push_str(&format!(
                                    "\n\nSECTIONS (in order, with bar counts) — make exactly one Ableton locator per \
section, placed at the running bar offset from these counts: {}.",
                                    list.join(", "),
                                ));
                            }
                        }
                    }
                }
            }
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
        for line in std::io::BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() { continue; }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                let _ = app2.emit("chat_event", serde_json::json!({ "session_id": sid2, "event": v }));
            }
        }
        let status = child.wait().map_err(e2s)?;
        if !status.success() {
            let mut err = String::new();
            if let Some(mut se) = child.stderr.take() {
                use std::io::Read;
                let _ = se.read_to_string(&mut err);
            }
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

/// Send one JSON command to Ableton's Remote Script socket and read one JSON reply.
fn ableton_cmd(stream: &mut std::net::TcpStream, body: serde_json::Value) -> Result<serde_json::Value, String> {
    use std::io::{Read, Write};
    stream.write_all(&serde_json::to_vec(&body).map_err(e2s)?).map_err(e2s)?;
    let mut acc = Vec::new();
    let mut buf = [0u8; 8192];
    for _ in 0..64 {
        let n = stream.read(&mut buf).map_err(e2s)?;
        if n == 0 { break; }
        acc.extend_from_slice(&buf[..n]);
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&acc) { return Ok(v); }
    }
    Err("incomplete response".into())
}

/// Programmatically build the song's section structure in Ableton's Arrangement
/// as named LOCATORS — talks straight to the Remote Script socket (no MCP, no LLM),
/// looping every section deterministically.
#[tauri::command]
async fn ableton_build(state: State<'_, AppState>, song_id: String) -> R<String> {
    use std::net::TcpStream;
    use std::time::Duration;
    let song = db::get_song(&state.conn, &song_id).await.map_err(e2s)?.ok_or("song not found")?;
    // sections (label + bars) from the Structure stage
    let mut sections: Vec<(String, i64)> = Vec::new();
    if let Ok(stages) = db::list_stages(&state.conn, &song_id).await {
        if let Some(st) = stages.iter().find(|s| s.r#type == "structure") {
            if let Ok(Some(art)) = db::current_artifact(&state.conn, &st.id).await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&art.content) {
                    if let Some(secs) = v.get("data").and_then(|d| d.get("sections")).and_then(|s| s.as_array()) {
                        for s in secs {
                            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("Section").to_string();
                            let bars = s.get("bars").and_then(|x| x.as_i64()).unwrap_or(8);
                            sections.push((label, bars));
                        }
                    }
                }
            }
        }
    }
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let addr = "127.0.0.1:9877".parse().map_err(e2s)?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
            .map_err(|e| format!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP control surface on) and Free the connection first."))?;
        s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
        s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"switch_to_arrangement_view","params":{}}));
        // start clean so re-runs are idempotent — delete one cue per call (each on its
        // own tick) until none remain (ignored if the Live script predates clear_cues)
        for _ in 0..80 {
            let done = match ableton_cmd(&mut s, serde_json::json!({"type":"clear_cues","params":{}})) {
                Ok(v) => match v.get("result").and_then(|r| r.get("remaining")).and_then(|n| n.as_i64()) {
                    Some(0) | None => true,
                    Some(_) => false,
                },
                Err(_) => true,
            };
            if done { break; }
            std::thread::sleep(Duration::from_millis(50)); // each delete on its own tick
        }

        // (beat offset, bar number, label) per section
        let mut marks: Vec<(f64, i64, String)> = Vec::new();
        let mut bar = 1i64;
        for (label, bars) in &sections {
            marks.push(((bar - 1) as f64 * 4.0, bar, label.clone()));
            bar += bars;
        }
        // PASS 1 — create the locators. A short gap between each keeps every cue op
        // on its own Live tick, so the "does a cue already exist here?" check reads a
        // settled list and never toggles-deletes a previous one (the source of the
        // non-deterministic results).
        for (t, _, label) in &marks {
            let _ = ableton_cmd(&mut s, serde_json::json!({"type":"create_locator","params":{"time": t, "name": label}}));
            std::thread::sleep(Duration::from_millis(60));
        }
        std::thread::sleep(Duration::from_millis(300)); // let the cue list settle
        // PASS 2 — cues are settled; rename-only (never toggles, so nothing is deleted)
        let mut log = vec![format!("tempo {bpm} BPM · {} sections", marks.len())];
        for (t, barno, label) in &marks {
            match ableton_cmd(&mut s, serde_json::json!({"type":"rename_cue","params":{"time": t, "name": label}})) {
                Ok(v) => {
                    let named = v.get("result").and_then(|r| r.get("name")).and_then(|n| n.as_str()).is_some();
                    let skipped = v.get("result").and_then(|r| r.get("skipped")).is_some();
                    if named { log.push(format!("✓ {label} @ bar {barno}")); }
                    else if skipped { log.push(format!("⤬ {label} @ bar {barno} — past arrangement end")); }
                    else { log.push(format!("⚠️ {label} @ bar {barno} — not named ({v})")); }
                }
                Err(e) => log.push(format!("⚠️ {label}: {e}")),
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Ok(log.join("\n"))
    })
    .await
    .map_err(e2s)?
}

/// Section (label, bars) list from a song's Structure stage.
async fn song_sections(conn: &Connection, song_id: &str) -> Vec<(String, i64)> {
    let mut out = Vec::new();
    if let Ok(stages) = db::list_stages(conn, song_id).await {
        if let Some(st) = stages.iter().find(|s| s.r#type == "structure") {
            if let Ok(Some(art)) = db::current_artifact(conn, &st.id).await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&art.content) {
                    if let Some(secs) = v.get("data").and_then(|d| d.get("sections")).and_then(|s| s.as_array()) {
                        for s in secs {
                            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("Section").to_string();
                            let bars = s.get("bars").and_then(|x| x.as_i64()).unwrap_or(8);
                            out.push((label, bars));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Color (RGB int) for a section, by type — Live snaps to the nearest swatch.
fn clip_color(label: &str) -> i64 {
    let l = label.to_lowercase();
    if l.contains("pre") { 0xFF9500 }          // pre-chorus → orange
    else if l.contains("chorus") { 0x4CD964 }   // chorus → green
    else if l.contains("verse") { 0x3DC2FF }    // verse → blue
    else if l.contains("break") || l.contains("bridge") || l.contains("build") { 0xAF52DE } // purple
    else if l.contains("intro") || l.contains("outro") { 0x8E8E93 } // gray
    else { 0xCBCBCB }
}

/// Programmatically build the song structure in Ableton's Arrangement as named,
/// color-coded CLIPS on a "Sections" track — direct socket, no MCP/LLM.
#[tauri::command]
async fn ableton_build_clips(state: State<'_, AppState>, song_id: String) -> R<String> {
    use std::net::TcpStream;
    use std::time::Duration;
    let song = db::get_song(&state.conn, &song_id).await.map_err(e2s)?.ok_or("song not found")?;
    let sections = song_sections(&state.conn, &song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let addr = "127.0.0.1:9877".parse().map_err(e2s)?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
            .map_err(|e| format!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
        s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
        s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"switch_to_arrangement_view","params":{}}));
        // create the "Sections" track and find its index
        let ti = ableton_cmd(&mut s, serde_json::json!({"type":"create_midi_track","params":{"index":-1}}))
            .ok()
            .and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
            .or_else(|| ableton_cmd(&mut s, serde_json::json!({"type":"get_session_info","params":{}})).ok()
                .and_then(|v| v.get("result").and_then(|r| r.get("track_count")).and_then(|n| n.as_i64()))
                .map(|c| c - 1))
            .unwrap_or(0);
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_track_name","params":{"track_index": ti, "name": "Sections"}}));

        let mut log = vec![format!("tempo {bpm} BPM · {} clips on track {ti}", sections.len())];
        let mut bar = 1i64;
        for (i, (label, bars)) in sections.iter().enumerate() {
            let ci = i as i64;
            let length = (*bars as f64) * 4.0;
            let dest = ((bar - 1) as f64) * 4.0;
            let _ = ableton_cmd(&mut s, serde_json::json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
            std::thread::sleep(Duration::from_millis(40));
            let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": ci, "name": label}}));
            let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": clip_color(label)}}));
            let dup = ableton_cmd(&mut s, serde_json::json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": ci, "destination_time": dest}}));
            std::thread::sleep(Duration::from_millis(40));
            match dup {
                Ok(v) if v.get("status").and_then(|x| x.as_str()) == Some("success") => log.push(format!("✓ {label} @ bar {bar} ({bars} bars)")),
                Ok(v) => log.push(format!("⚠️ {label} @ bar {bar}: {v}")),
                Err(e) => log.push(format!("⚠️ {label}: {e}")),
            }
            bar += bars;
        }
        Ok(log.join("\n"))
    })
    .await
    .map_err(e2s)?
}

// --- song-stub generation: chords -> MIDI parts ---------------------------------

/// Parse a chord name into (root pitch-class 0-11, chord-tone semitone offsets).
fn chord_tones(name: &str) -> Option<(i64, Vec<i64>)> {
    let b = name.as_bytes();
    if b.is_empty() { return None; }
    let mut pc: i64 = match b[0].to_ascii_uppercase() {
        b'C' => 0, b'D' => 2, b'E' => 4, b'F' => 5, b'G' => 7, b'A' => 9, b'B' => 11, _ => return None,
    };
    let mut i = 1;
    if i < b.len() && b[i] == b'#' { pc = (pc + 1) % 12; i += 1; }
    else if i < b.len() && b[i] == b'b' { pc = (pc + 11) % 12; i += 1; }
    let rest = &name[i..];
    let tones = if rest.starts_with("dim") { vec![0, 3, 6] }
        else if rest.starts_with("aug") { vec![0, 4, 8] }
        else if rest.starts_with("sus2") { vec![0, 2, 7] }
        else if rest.starts_with("sus4") { vec![0, 5, 7] }
        else if rest.starts_with("maj7") { vec![0, 4, 7, 11] }
        else if rest.starts_with('m') && !rest.starts_with("maj") {
            if rest.contains('7') { vec![0, 3, 7, 10] } else { vec![0, 3, 7] }
        }
        else if rest.starts_with('7') || rest.starts_with("dom7") { vec![0, 4, 7, 10] }
        else { vec![0, 4, 7] };
    Some((pc, tones))
}

fn mk_note(pitch: i64, start: f64, dur: f64, vel: i64) -> serde_json::Value {
    serde_json::json!({ "pitch": pitch.clamp(0, 127), "start_time": start, "duration": dur, "velocity": vel, "mute": false })
}

/// Which parts play in a section — thins arrangement so it BUILDS with the energy
/// arc (intro = sparse → chorus/drop = everything) instead of all parts everywhere.
fn section_parts(label: &str) -> &'static [&'static str] {
    let l = label.to_lowercase();
    if l.contains("intro") || l.contains("outro") { &["Sections", "Bass", "Chords"] }
    else if l.contains("pre") || l.contains("build") { &["Sections", "Bass", "Chords", "Chord melody", "Filler"] }
    else if l.contains("break") || l.contains("bridge") { &["Sections", "Bass", "Chords", "Filler"] }
    else if l.contains("verse") { &["Sections", "Bass", "Chords", "Chord melody"] }
    else { &["Sections", "Bass", "Chords", "Chord melody", "Filler", "Arp"] } // chorus / drop / hook / default
}

/// Generate the MIDI notes for one part over a section (bars × looped chords).
/// `groove` (genre-driven) swaps sustained pads for rhythmic stabs/faster arps.
/// Velocities are accented on beat 1 and ghosted off-beat to avoid a robotic grid.
fn part_notes(part: &str, chords: &[String], bars: i64, groove: bool) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if chords.is_empty() { return out; }
    for b in 0..bars {
        let name = &chords[b as usize % chords.len()];
        let Some((pc, tones)) = chord_tones(name) else { continue };
        let start = (b * 4) as f64; // 4/4, one chord per bar
        let third = tones.get(1).copied().unwrap_or(4);
        let top = tones.last().copied().unwrap_or(7);
        match part {
            // root octave 2 with movement to the 5th on beat 3; punchy on 1
            "Bass" => if groove {
                out.push(mk_note(36 + pc, start, 1.0, 112));
                out.push(mk_note(36 + pc, start + 1.5, 0.5, 82));        // off-beat sub push
                out.push(mk_note(36 + pc + 7, start + 2.0, 1.5, 96));   // 5th on 3
            } else {
                out.push(mk_note(36 + pc, start, 2.5, 106));
                out.push(mk_note(36 + pc + 7, start + 2.0, 2.0, 88));
            },
            // sustained pad, or two stabs (1 + 3) when grooving
            "Chords" => if groove {
                for (off, vel) in [(0.0_f64, 90), (2.0, 76)] { for t in &tones { out.push(mk_note(48 + pc + t, start + off, 0.9, vel)); } }
            } else {
                for t in &tones { out.push(mk_note(48 + pc + t, start, 4.0, 78)); }
            },
            // a small contour: top tone on 1, step down to the 3rd on 3
            "Chord melody" => {
                out.push(mk_note(60 + pc + top, start, 1.5, 96));
                out.push(mk_note(60 + pc + third, start + 2.0, 2.0, 82));
            }
            // off-beat triad stabs on the "and" of 2 and 4
            "Filler" => for off in [1.5_f64, 3.5] { for t in &tones { out.push(mk_note(48 + pc + t, start + off, 0.5, 68)); } },
            // arpeggio cycling chord tones, climbing an octave each pass (16ths grooving, else 8ths)
            "Arp" => {
                let (steps, dur) = if groove { (16, 0.25) } else { (8, 0.5) };
                for step in 0..steps {
                    let t = tones[step % tones.len()];
                    let oct = ((step / tones.len()) % 2) as i64 * 12;
                    let vel = if step % 4 == 0 { 86 } else { 64 };
                    out.push(mk_note(60 + pc + t + oct, start + step as f64 * dur, dur, vel));
                }
            }
            _ => {}
        }
    }
    out
}

/// Structure sections enriched with each section's looped chord progression.
async fn song_parts(conn: &Connection, song_id: &str) -> Vec<(String, i64, Vec<String>)> {
    let secs = song_sections(conn, song_id).await;
    let mut cmap: HashMap<String, Vec<String>> = HashMap::new();
    if let Ok(stages) = db::list_stages(conn, song_id).await {
        if let Some(st) = stages.iter().find(|s| s.r#type == "chords") {
            if let Ok(Some(art)) = db::current_artifact(conn, &st.id).await {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&art.content) {
                    if let Some(arr) = v.pointer("/data/sections").and_then(|s| s.as_array()) {
                        for s in arr {
                            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let chords = s.get("chords").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|c|
                                c.as_str().map(String::from).or_else(|| c.get("name").and_then(|n| n.as_str()).map(String::from))
                            ).collect()).unwrap_or_default();
                            cmap.insert(label, chords);
                        }
                    }
                }
            }
        }
    }
    secs.into_iter().map(|(label, bars)| { let ch = cmap.get(&label).cloned().unwrap_or_default(); (label, bars, ch) }).collect()
}

/// Stub the whole song in Ableton's Arrangement: a named "Sections" clip track
/// plus Bass / Chords / Chord melody / Filler / Arp MIDI parts generated from the
/// chord progression — direct socket, no MCP/LLM. MIDI-only (you pick the sounds).
#[tauri::command]
async fn ableton_build_song(state: State<'_, AppState>, song_id: String) -> R<String> {
    use std::net::TcpStream;
    use std::time::Duration;
    let song = db::get_song(&state.conn, &song_id).await.map_err(e2s)?.ok_or("song not found")?;
    let sections = song_parts(&state.conn, &song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let bpm = song.bpm;
    // genre-driven groove: rhythmic (stabs / fast arp / punchy bass) vs sustained
    let groove = db::get_preset(&state.conn, &song.style_preset_id).await.ok().flatten()
        .map(|p| {
            let g = p.genre.to_lowercase();
            ["phonk", "trap", "electronic", "edm", "house", "techno", "dnb", "drum", "dance", "hip", "beat", "synthwave", "drill", "wave", "bass"]
                .iter().any(|k| g.contains(k))
        })
        .unwrap_or(true);
    tokio::task::spawn_blocking(move || -> Result<String, String> {
        let addr = "127.0.0.1:9877".parse().map_err(e2s)?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
            .map_err(|e| format!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
        s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
        s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
        let nap = || std::thread::sleep(Duration::from_millis(35));
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"switch_to_arrangement_view","params":{}}));

        // create the 6 tracks, capture their indices
        let track_names = ["Sections", "Bass", "Chords", "Chord melody", "Filler", "Arp"];
        let mut tracks: Vec<i64> = Vec::new();
        for name in track_names {
            let ti = ableton_cmd(&mut s, serde_json::json!({"type":"create_midi_track","params":{"index":-1}}))
                .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
                .unwrap_or(tracks.last().map(|t| t + 1).unwrap_or(0));
            let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_track_name","params":{"track_index": ti, "name": name}}));
            tracks.push(ti);
            nap();
        }

        let mut log = vec![format!("tempo {bpm} BPM · {} feel · {} sections × up to {} tracks (density per section)", if groove { "rhythmic groove" } else { "sustained" }, sections.len(), track_names.len())];
        let mut bar = 1i64;
        for (i, (label, bars, chords)) in sections.iter().enumerate() {
            let ci = i as i64;
            let length = (*bars as f64) * 4.0;
            let dest = ((bar - 1) as f64) * 4.0;
            let active = section_parts(label);
            for (t, &ti) in tracks.iter().enumerate() {
                let part = track_names[t];
                if !active.contains(&part) { continue; } // section-aware density: leave a gap so it builds
                let _ = ableton_cmd(&mut s, serde_json::json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
                if part == "Sections" {
                    let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": clip_color(label)}}));
                } else {
                    let notes = part_notes(part, chords, *bars, groove);
                    if !notes.is_empty() {
                        let _ = ableton_cmd(&mut s, serde_json::json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": ci, "notes": notes}}));
                    }
                }
                let _ = ableton_cmd(&mut s, serde_json::json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": ci, "name": label}}));
                let _ = ableton_cmd(&mut s, serde_json::json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": ci, "destination_time": dest}}));
                nap();
            }
            log.push(format!("✓ {label} @ bar {bar} ({bars} bars · {} chords)", chords.len()));
            bar += bars;
        }
        Ok(log.join("\n"))
    })
    .await
    .map_err(e2s)?
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
            let conn = database.connect().expect("connect database");
            std::mem::forget(database);

            ensure_claude_bin(&conn);
            app.manage(AppState { conn, db_path: db_path_str, inflight: Mutex::new(HashSet::new()) });
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
            update_song_key,
            update_song_voicings,
            import_reference,
            self_check_stage,
            refine_field,
            delete_song,
            get_stage,
            run_stage,
            approve_stage,
            advance_stage,
            get_artifact,
            save_artifact,
            list_artifact_revisions,
            revert_artifact,
            list_skills,
            get_skill,
            create_skill,
            update_skill,
            set_skill_enabled,
            list_progressions,
            save_progression,
            delete_progression,
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
