//! The tool registry: one definition per capability, callable by the UI, the
//! agent loop, and Claude over MCP. `dispatch` executes a tool by name.

use crate::models::*;
use crate::{agent, db};
use anyhow::{anyhow, Result};
use libsql::Connection;
use serde_json::{json, Value};

#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub destructive: bool,
    pub input_schema: Value,
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": props, "required": required })
}

pub fn registry() -> Vec<ToolSpec> {
    let s = |t: &str| json!({ "type": "string", "description": t });
    let style_props = json!({
        "name": s(""), "genre": s(""), "mood": s(""), "influences": s(""),
        "key_tempo_feel": s(""), "vocal_range": s(""), "themes": s("")
    });
    vec![
        ToolSpec { name: "list_style_presets", description: "List all style presets.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_style_preset", description: "Get a style preset by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "create_style_preset", description: "Create a style preset (genre, mood, influences, key/tempo, vocal range, themes).", destructive: false, input_schema: obj(style_props.clone(), &["name"]) },
        ToolSpec { name: "update_style_preset", description: "Update a style preset.", destructive: false, input_schema: obj({ let mut p = style_props.clone(); p["id"] = s("preset id"); p }, &["id"]) },
        ToolSpec { name: "generate_style_preset", description: "Auto-generate a style preset from a name/seed using the style skill.", destructive: false, input_schema: obj(json!({"name": s("name or seed"),"notes": s("optional context")}), &["name"]) },
        ToolSpec { name: "create_song", description: "Start a new song; seeds the ordered stage spec.", destructive: false, input_schema: obj(json!({"style_preset_id": s(""),"title": s("working title")}), &["style_preset_id"]) },
        ToolSpec { name: "list_songs", description: "List all songs with status and current stage.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_song", description: "Get a song with its preset and stages.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "update_song_status", description: "Set a song's status (in_progress/done/archived).", destructive: false, input_schema: obj(json!({"id": s(""),"status": s("")}), &["id","status"]) },
        ToolSpec { name: "update_song_title", description: "Rename a song (set its title).", destructive: false, input_schema: obj(json!({"id": s(""),"title": s("")}), &["id","title"]) },
        ToolSpec { name: "delete_song", description: "Delete a song and all its stages/artifacts.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "get_stage", description: "Get a stage with its current artifact and active skill.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "run_stage", description: "Run a stage: load skill + style preset + prior approved artifacts, call Claude, write the artifact.", destructive: false, input_schema: obj(json!({"stage_id": s(""),"user_input": s("optional seed (your own chords/lyrics/title)")}), &["stage_id"]) },
        ToolSpec { name: "approve_stage", description: "Approve a stage's current artifact, mark it done, advance the song.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "advance_stage", description: "Move a song to its next pending stage.", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "get_artifact", description: "Get an artifact by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "save_artifact", description: "Save a new artifact revision for a stage.", destructive: false, input_schema: obj(json!({"song_id": s(""),"stage_id": s(""),"kind": s(""),"content": s("JSON content")}), &["song_id","kind","content"]) },
        ToolSpec { name: "list_artifact_revisions", description: "List all revisions of a stage's artifact, newest first.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "revert_artifact", description: "Restore a prior artifact revision as a new revision.", destructive: false, input_schema: obj(json!({"artifact_id": s("")}), &["artifact_id"]) },
        ToolSpec { name: "list_skills", description: "List all skills.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_skill", description: "Get a skill by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "create_skill", description: "Create a user skill (custom songwriting method).", destructive: false, input_schema: obj(json!({"key": s(""),"name": s(""),"stage_type": s(""),"instructions": s("")}), &["key","name","stage_type","instructions"]) },
        ToolSpec { name: "update_skill", description: "Update a skill's content.", destructive: false, input_schema: obj(json!({"id": s(""),"key": s(""),"name": s(""),"stage_type": s(""),"instructions": s("")}), &["id"]) },
        ToolSpec { name: "set_skill_enabled", description: "Enable or disable a skill.", destructive: false, input_schema: obj(json!({"id": s(""),"enabled": {"type":"boolean"}}), &["id","enabled"]) },
        ToolSpec { name: "list_progressions", description: "List saved chord progressions (reusable across songs).", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "save_progression", description: "Save a reusable chord progression by name.", destructive: false, input_schema: obj(json!({"name": s(""),"chords": {"type":"array","items":{"type":"string"}}}), &["name","chords"]) },
        ToolSpec { name: "delete_progression", description: "Delete a saved chord progression.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "list_renders", description: "List a song's final audio renders (versions referenced on disk).", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "add_render", description: "Add a final render: a label + file path on disk (Suno/Udio/Ableton take).", destructive: false, input_schema: obj(json!({"song_id": s(""),"label": s(""),"file_path": s(""),"source": s(""),"notes": s("")}), &["song_id","file_path"]) },
        ToolSpec { name: "set_render_pick", description: "Mark a render as the chosen winner for its song.", destructive: false, input_schema: obj(json!({"id": s(""),"is_pick": {"type":"boolean"}}), &["id","is_pick"]) },
        ToolSpec { name: "delete_render", description: "Remove a render reference (does not delete the file).", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "analyze_reference", description: "Analyze a local audio file (the perception layer for importing a reference): returns raw tempo, a key guess, bar-level chord candidates, and rough section boundaries as JSON. Interpret it with the Reference Analyst method — correct the key from the chord content, snap tempo, clean chords to the diatonic set, derive form from chord repetition — then create a song and save its Structure + Chords.", destructive: false, input_schema: obj(json!({"audio_path": s("absolute path to the local audio file")}), &["audio_path"]) },
        ToolSpec { name: "get_settings", description: "Get app settings.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "set_settings", description: "Update app settings.", destructive: false, input_schema: obj(json!({"settings": {"type":"object"}}), &["settings"]) },
    ]
}

pub fn tool_names() -> Vec<&'static str> {
    registry().into_iter().map(|t| t.name).collect()
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key).and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing string arg '{key}'"))
}
fn arg_opt<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

/// Run the configured local analyzer CLI on an audio file and return its JSON.
/// The audio path is appended as the final argument. Audio never leaves the machine.
async fn run_analyzer(settings: &Settings, audio_path: &str) -> Result<Value> {
    let cmd = settings.analyzer_cmd.trim();
    if cmd.is_empty() {
        return Err(anyhow!("reference analyzer is not configured — set the analyzer command in Settings (e.g. \"/path/.venv/bin/python /path/analyze.py\")"));
    }
    let mut parts = cmd.split_whitespace();
    let program = parts.next().ok_or_else(|| anyhow!("empty analyzer command"))?;
    let mut command = tokio::process::Command::new(program);
    for a in parts { command.arg(a); }
    command.arg(audio_path);
    let out = command.output().await.map_err(|e| anyhow!("could not run analyzer ({program}): {e}"))?;
    if !out.status.success() {
        return Err(anyhow!("analyzer failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    Ok(serde_json::from_str::<Value>(stdout.trim()).unwrap_or_else(|_| json!({ "raw": stdout })))
}
fn style_input(args: &Value) -> StyleInput {
    StyleInput {
        name: arg_opt(args, "name").unwrap_or_default().into(),
        genre: arg_opt(args, "genre").unwrap_or_default().into(),
        mood: arg_opt(args, "mood").unwrap_or_default().into(),
        influences: arg_opt(args, "influences").unwrap_or_default().into(),
        key_tempo_feel: arg_opt(args, "key_tempo_feel").unwrap_or_default().into(),
        vocal_range: arg_opt(args, "vocal_range").unwrap_or_default().into(),
        themes: arg_opt(args, "themes").unwrap_or_default().into(),
    }
}
fn skill_input(args: &Value) -> SkillInput {
    SkillInput {
        key: arg_opt(args, "key").unwrap_or_default().into(),
        name: arg_opt(args, "name").unwrap_or_default().into(),
        stage_type: arg_opt(args, "stage_type").unwrap_or_default().into(),
        instructions: arg_opt(args, "instructions").unwrap_or_default().into(),
    }
}

pub async fn approve_stage(conn: &Connection, stage_id: &str) -> Result<Value> {
    let stage = db::get_stage(conn, stage_id).await?.ok_or_else(|| anyhow!("stage not found"))?;
    if let Some(art) = db::current_artifact(conn, stage_id).await? {
        db::set_artifact_approved(conn, &art.id, true).await?;
    }
    db::set_stage_status(conn, stage_id, "done").await?;
    advance_song(conn, &stage.song_id).await?;
    Ok(json!({ "ok": true, "stage_id": stage_id }))
}

pub async fn advance_song(conn: &Connection, song_id: &str) -> Result<Value> {
    let stages = db::list_stages(conn, song_id).await?;
    let next = stages.iter().find(|s| s.status != "done");
    if let Some(s) = next {
        db::set_song_current_stage(conn, song_id, &s.r#type).await?;
        Ok(json!({ "current_stage": s.r#type }))
    } else if let Some(last) = stages.last() {
        db::set_song_current_stage(conn, song_id, &last.r#type).await?;
        Ok(json!({ "current_stage": last.r#type, "complete": true }))
    } else {
        Ok(json!({}))
    }
}

pub async fn dispatch(conn: &Connection, settings: &Settings, name: &str, args: &Value) -> Result<Value> {
    fn v<T: serde::Serialize>(x: T) -> Result<Value> {
        Ok(serde_json::to_value(x).unwrap())
    }
    match name {
        "list_style_presets" => v(db::list_presets(conn).await?),
        "get_style_preset" => v(db::get_preset(conn, arg(args, "id")?).await?),
        "create_style_preset" => v(db::create_preset(conn, style_input(args)).await?),
        "update_style_preset" => v(db::update_preset(conn, arg(args, "id")?, style_input(args)).await?),
        "generate_style_preset" => v(agent::generate_style_preset(conn, settings, arg(args, "name")?, arg_opt(args, "notes"), |_| {}).await?),
        "create_song" => v(db::create_song(conn, arg(args, "style_preset_id")?, arg_opt(args, "title").unwrap_or("Untitled song")).await?),
        "list_songs" => v(db::list_songs(conn).await?),
        "get_song" => v(db::get_song_detail(conn, arg(args, "id")?).await?),
        "update_song_status" => v(db::update_song_status(conn, arg(args, "id")?, arg(args, "status")?).await?),
        "update_song_title" => v(db::update_song_title(conn, arg(args, "id")?, arg(args, "title")?).await?),
        "delete_song" => { db::delete_song(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "get_stage" => v(db::get_stage_detail(conn, arg(args, "id")?).await?),
        "run_stage" => v(agent::run_stage(conn, settings, arg(args, "stage_id")?, arg_opt(args, "user_input").map(String::from), |_| {}).await?.artifact),
        "approve_stage" => approve_stage(conn, arg(args, "stage_id")?).await,
        "advance_stage" => advance_song(conn, arg(args, "song_id")?).await,
        "get_artifact" => v(db::get_artifact(conn, arg(args, "id")?).await?),
        "save_artifact" => v(db::save_artifact(conn, arg(args, "song_id")?, arg_opt(args, "stage_id"), arg(args, "kind")?, arg(args, "content")?).await?),
        "list_artifact_revisions" => v(db::list_artifact_revisions(conn, arg(args, "stage_id")?).await?),
        "revert_artifact" => v(db::revert_artifact(conn, arg(args, "artifact_id")?).await?),
        "list_skills" => v(db::list_skills(conn).await?),
        "get_skill" => v(db::get_skill(conn, arg(args, "id")?).await?),
        "create_skill" => v(db::create_skill(conn, skill_input(args)).await?),
        "update_skill" => v(db::update_skill(conn, arg(args, "id")?, skill_input(args)).await?),
        "set_skill_enabled" => v(db::set_skill_enabled(conn, arg(args, "id")?, args.get("enabled").and_then(|b| b.as_bool()).unwrap_or(true)).await?),
        "list_progressions" => v(db::list_progressions(conn).await?),
        "save_progression" => {
            let chords: Vec<String> = args.get("chords").and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            v(db::create_progression(conn, arg(args, "name")?, &chords).await?)
        }
        "delete_progression" => { db::delete_progression(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "list_renders" => v(db::list_renders(conn, arg(args, "song_id")?).await?),
        "add_render" => v(db::create_render(conn, arg(args, "song_id")?, arg_opt(args, "label").unwrap_or("Render"), arg(args, "file_path")?, arg_opt(args, "source").unwrap_or(""), arg_opt(args, "notes").unwrap_or("")).await?),
        "set_render_pick" => { db::set_render_pick(conn, arg(args, "id")?, args.get("is_pick").and_then(|b| b.as_bool()).unwrap_or(true)).await?; Ok(json!({ "ok": true })) }
        "delete_render" => { db::delete_render(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "analyze_reference" => run_analyzer(settings, arg(args, "audio_path")?).await,
        "get_settings" => v(db::get_settings(conn).await?),
        "set_settings" => {
            let st: Settings = serde_json::from_value(args.get("settings").cloned().unwrap_or(json!({})))?;
            db::set_settings(conn, &st).await?;
            v(st)
        }
        other => Err(anyhow!("unknown tool '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mock_has_every_tool() {
        let mock = include_str!("../../frontend/src/ipc/mockApi.ts");
        let missing: Vec<&str> = tool_names().into_iter().filter(|n| !mock.contains(&format!("\"{n}\""))).collect();
        assert!(missing.is_empty(), "tools missing from mockApi.ts: {missing:?}");
    }
}
