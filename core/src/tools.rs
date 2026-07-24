//! The tool registry: one definition per capability, callable by the UI, the
//! agent loop, and Claude over MCP. `dispatch` executes a tool by name.

use crate::models::*;
use crate::{ableton, agent, db};
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
        "key_tempo_feel": s(""), "vocal_range": s(""), "themes": s(""),
        "lyric_exemplars": s("a few lyric lines that calibrate the Lyricist's voice — never copied")
    });
    vec![
        ToolSpec { name: "list_style_presets", description: "List all style presets.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_style_preset", description: "Get a style preset by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "create_style_preset", description: "Create a style preset (genre, mood, influences, key/tempo, vocal range, themes).", destructive: false, input_schema: obj(style_props.clone(), &["name"]) },
        ToolSpec { name: "update_style_preset", description: "Update a style preset.", destructive: false, input_schema: obj({ let mut p = style_props.clone(); p["id"] = s("preset id"); p }, &["id"]) },
        ToolSpec { name: "set_preset_arrangement", description: "Store a style preset's Ableton arrangement profile JSON ({bass, sub_bass, chords, pad, arp, sparse_melody, vel_scale}); pass \"\" to clear back to the genre-keyword fallback.", destructive: false, input_schema: obj(json!({"id": s("preset id"),"arrangement": s("profile JSON or \"\"")}), &["id","arrangement"]) },
        ToolSpec { name: "generate_preset_arrangement", description: "Ask Claude to map a style preset onto the Ableton arrangement profile (bass figure, chord treatment, arp rate, density) and store it on the preset. The Build-in-Ableton stub then follows it instead of the genre-keyword fallback.", destructive: false, input_schema: obj(json!({"id": s("preset id")}), &["id"]) },
        ToolSpec { name: "generate_style_preset", description: "Auto-generate a style preset from a name/seed using the style skill.", destructive: false, input_schema: obj(json!({"name": s("name or seed"),"notes": s("optional context")}), &["name"]) },
        ToolSpec { name: "create_song_from_lyrics", description: "New song from pasted lyrics (verbatim — words are never rewritten): sections split on headers ([Verse 1] / **Verse 1** / Verse 1:), key inferred from inline [chord] tags when present, Structure and Chords back-filled.", destructive: false, input_schema: obj(json!({"style_preset_id": s(""),"title": s("working title"),"text": s("the full pasted lyrics")}), &["style_preset_id","text"]) },
        ToolSpec { name: "import_lyrics", description: "Paste completed lyrics into an existing song (verbatim — words are never rewritten): sections split on headers and REPLACE the song's section spine; the Lyrics stage gets the words as a new revision and Structure is back-filled. The song's key is not touched.", destructive: false, input_schema: obj(json!({"song_id": s(""),"text": s("the full pasted lyrics")}), &["song_id","text"]) },
        ToolSpec { name: "create_song", description: "Start a new song; seeds the ordered stage spec.", destructive: false, input_schema: obj(json!({"style_preset_id": s(""),"title": s("working title")}), &["style_preset_id"]) },
        ToolSpec { name: "list_songs", description: "List all songs with status and current stage.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_song", description: "Get a song with its preset and stages.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "update_song_status", description: "Set a song's status (in_progress/done/archived).", destructive: false, input_schema: obj(json!({"id": s(""),"status": s("")}), &["id","status"]) },
        ToolSpec { name: "update_song_title", description: "Rename a song (set its title).", destructive: false, input_schema: obj(json!({"id": s(""),"title": s("")}), &["id","title"]) },
        ToolSpec { name: "update_song_intent", description: "Set a song's intent — the producer's one-line brief every stage honors alongside the title (the north star).", destructive: false, input_schema: obj(json!({"id": s(""),"intent": s("one line — what this song is about")}), &["id","intent"]) },
        ToolSpec { name: "delete_song", description: "Delete a song and all its stages/artifacts.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "list_sections", description: "List a song's section spine (the single source of truth for section identity/order/label/type/bars/role), ordered by position.", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "create_section", description: "Add a section to a song's spine. Appends at the end unless `position` (0-based insert index) is given.", destructive: false, input_schema: obj(json!({"song_id": s(""),"label": s("e.g. \"Verse 1\""),"type": s("optional section type, e.g. verse/chorus/bridge"),"bars": {"type":"integer","description":"bar count (default 8)"},"role": s("optional arc role, e.g. \"opens the story\""),"position": {"type":"integer","description":"0-based insert index (omit to append)"}}), &["song_id","label"]) },
        ToolSpec { name: "update_section", description: "Update a spine section's form (label/type/bars/role) — omitted fields keep their current value. Order changes go through reorder_sections.", destructive: false, input_schema: obj(json!({"id": s("section id"),"label": s(""),"type": s(""),"bars": {"type":"integer"},"role": s("")}), &["id"]) },
        ToolSpec { name: "delete_section", description: "Delete a section from a song's spine (stage content keyed to it is orphaned — deletion of a content-bearing section is a user decision).", destructive: true, input_schema: obj(json!({"id": s("section id")}), &["id"]) },
        ToolSpec { name: "reorder_sections", description: "Reorder a song's spine: pass EVERY section id of the song, each exactly once, in the new order.", destructive: false, input_schema: obj(json!({"song_id": s(""),"section_ids": {"type":"array","items":{"type":"string"},"description":"all of the song's section ids in the new order"}}), &["song_id","section_ids"]) },
        ToolSpec { name: "get_stage", description: "Get a stage with its current artifact and active skill.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "run_stage", description: "Run a stage: load skill + style preset + prior approved artifacts, call Claude. FIRST run saves the artifact directly; a RE-run lands as a PENDING DRAFT (the current artifact is untouched) — review it with get_stage, then accept_stage_draft or discard_stage_draft.", destructive: false, input_schema: obj(json!({"stage_id": s(""),"user_input": s("optional seed (your own chords/lyrics/title)")}), &["stage_id"]) },
        ToolSpec { name: "accept_stage_draft", description: "Accept a stage's pending regeneration draft: it becomes the new current revision (frozen sections still protected); the draft clears.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "discard_stage_draft", description: "Discard a stage's pending regeneration draft — the current artifact was never touched.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "approve_stage", description: "Approve a stage's current artifact, mark it done, advance the song.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "advance_stage", description: "Move a song to its next pending stage.", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "get_artifact", description: "Get an artifact by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "save_artifact", description: "Save a new artifact revision for a stage.", destructive: false, input_schema: obj(json!({"song_id": s(""),"stage_id": s(""),"kind": s(""),"content": s("JSON content")}), &["song_id","kind","content"]) },
        ToolSpec { name: "list_artifact_revisions", description: "List all revisions of a stage's artifact, newest first.", destructive: false, input_schema: obj(json!({"stage_id": s("")}), &["stage_id"]) },
        ToolSpec { name: "revert_artifact", description: "Restore a prior artifact revision as a new revision.", destructive: false, input_schema: obj(json!({"artifact_id": s("")}), &["artifact_id"]) },
        ToolSpec { name: "set_artifact_label", description: "Name an artifact revision (metadata only — content untouched). The label shows in the History timeline; omit `label` to clear it.", destructive: false, input_schema: obj(json!({"artifact_id": s(""),"label": s("short name for the revision, e.g. \"pre-chorus rewrite\" (omit to clear)")}), &["artifact_id"]) },
        ToolSpec { name: "list_skills", description: "List all skills.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_skill", description: "Get a skill by id.", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "create_skill", description: "Create a user skill (custom songwriting method).", destructive: false, input_schema: obj(json!({"key": s(""),"name": s(""),"stage_type": s(""),"instructions": s("")}), &["key","name","stage_type","instructions"]) },
        ToolSpec { name: "update_skill", description: "Update a skill's content.", destructive: false, input_schema: obj(json!({"id": s(""),"key": s(""),"name": s(""),"stage_type": s(""),"instructions": s("")}), &["id"]) },
        ToolSpec { name: "set_skill_enabled", description: "Enable or disable a skill.", destructive: false, input_schema: obj(json!({"id": s(""),"enabled": {"type":"boolean"}}), &["id","enabled"]) },
        ToolSpec { name: "list_outlines", description: "List saved song outlines (section skeletons + tempo, no musical content).", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "save_outline", description: "Save a reusable song outline: named ordered sections (label + bars) and a tempo. Export any outline to Ableton with ableton_build_outline.", destructive: false, input_schema: obj(json!({"name": s(""),"bpm": {"type":"integer"},"sections": {"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"bars":{"type":"integer"}},"required":["label"]}}}), &["name","sections"]) },
        ToolSpec { name: "delete_outline", description: "Delete a saved song outline.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "list_progressions", description: "List saved chord progressions (reusable across songs).", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "save_progression", description: "Save a reusable chord progression by name (optionally with per-chord shape picks JSON).", destructive: false, input_schema: obj(json!({"name": s(""),"chords": {"type":"array","items":{"type":"string"}},"picks": s("optional per-chord picks JSON: [{\"g\":0,\"p\":2,\"a\":0}, …]")}), &["name","chords"]) },
        ToolSpec { name: "update_progression", description: "Update a saved chord progression in place (name, chords, optional picks JSON).", destructive: false, input_schema: obj(json!({"id": s(""),"name": s(""),"chords": {"type":"array","items":{"type":"string"}},"picks": s("optional per-chord picks JSON")}), &["id","name","chords"]) },
        ToolSpec { name: "delete_progression", description: "Delete a saved chord progression.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "list_compositions", description: "List saved Composer compositions (visual melody/chords/bass sketches): name, linked song, timestamps — no data blobs.", destructive: false, input_schema: obj(json!({}), &[]) },
        ToolSpec { name: "get_composition", description: "Get a saved composition by id, including its full Composition JSON blob (`data`).", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "save_composition", description: "Save a Composer composition. Omit `id` to insert a new one (a fresh id is minted); pass `id` to update that composition in place. `data` must be the Composition JSON blob.", destructive: false, input_schema: obj(json!({"id": s("existing composition id (omit to insert)"),"name": s(""),"song_id": s("source song id for full-song imports (optional)"),"data": s("Composition JSON blob")}), &["name","data"]) },
        ToolSpec { name: "delete_composition", description: "Delete a saved composition.", destructive: true, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "list_renders", description: "List a song's final audio renders (versions referenced on disk).", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "add_render", description: "Add a final render: a label + file path on disk (Suno/Udio/Ableton take).", destructive: false, input_schema: obj(json!({"song_id": s(""),"label": s(""),"file_path": s(""),"source": s(""),"notes": s("")}), &["song_id","file_path"]) },
        ToolSpec { name: "set_render_pick", description: "Mark a render as the chosen winner for its song.", destructive: false, input_schema: obj(json!({"id": s(""),"is_pick": {"type":"boolean"}}), &["id","is_pick"]) },
        ToolSpec { name: "delete_render", description: "Remove a render reference (does not delete the file).", destructive: false, input_schema: obj(json!({"id": s("")}), &["id"]) },
        ToolSpec { name: "ableton_build_outline", description: "Stub a sections-only song OUTLINE in Ableton Live (no song needed): named color-coded section clips on a Sections track (no locators, no chords, no notes) — a skeleton to build into. Live must be open with AbletonMCP enabled.", destructive: false, input_schema: obj(json!({"sections": {"type":"array","items":{"type":"object","properties":{"label":{"type":"string"},"bars":{"type":"integer"}},"required":["label"]},"description":"ordered sections, e.g. [{\"label\":\"Intro\",\"bars\":8}, …]"},"bpm": {"type":"integer","description":"tempo (default 120)"}}), &["sections"]) },
        ToolSpec { name: "ableton_build_progression", description: "Stub a bare chord progression in Ableton Live's Arrangement (no song needed): one Progression section, one bar per chord, full Bass/Chords/Pad/Melody/Filler/Arp MIDI parts. Live must be open with AbletonMCP enabled.", destructive: false, input_schema: obj(json!({"chords": {"type":"array","items":{"type":"string"},"description":"chord names in order, e.g. [\"Bm\",\"A\",\"E\"]"},"inversions": {"type":"array","items":{"type":"integer"},"description":"optional closed-voicing inversion per chord (0 = root)"},"beats": {"type":"array","items":{"type":"integer"},"description":"optional beats per chord (default 4 = one bar each)"},"bpm": {"type":"integer","description":"tempo (default 120)"}}), &["chords"]) },
        ToolSpec { name: "ableton_build_song", description: "Stub the whole song in Ableton Live's Arrangement view: a named, color-coded Sections clip track plus Bass/Chords/Pad/Chord melody/Filler/Arp MIDI parts generated deterministically from the song's chord progression (per-section density follows the energy arc). Talks straight to the AbletonMCP Remote Script socket — Live must be open with the control surface enabled. MIDI-only; re-running clears and rebuilds the same tracks. Returns a per-section build log.", destructive: false, input_schema: obj(json!({"song_id": s("")}), &["song_id"]) },
        ToolSpec { name: "analyze_reference", description: "Analyze a local audio file (the perception layer for importing a reference): returns raw tempo, a key guess, bar-level chord candidates, and rough section boundaries as JSON. Interpret it with the Reference Analyst method — correct the key from the chord content, snap tempo, clean chords to the diatonic set, derive form from chord repetition — then create a song and save its Structure + Chords.", destructive: false, input_schema: obj(json!({"audio_path": s("absolute path to the local audio file"),"lyrics": {"type":"boolean","description":"also transcribe sung lyrics (local whisper — slower)"}}), &["audio_path"]) },
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
async fn run_analyzer(settings: &Settings, audio_path: &str, lyrics: bool) -> Result<Value> {
    let cmd = settings.analyzer_cmd.trim();
    if cmd.is_empty() {
        return Err(anyhow!("reference analyzer is not configured — set the analyzer command in Settings (e.g. \"/path/.venv/bin/python /path/analyze.py\")"));
    }
    let mut parts = cmd.split_whitespace();
    let program = parts.next().ok_or_else(|| anyhow!("empty analyzer command"))?;
    let mut command = tokio::process::Command::new(program);
    for a in parts { command.arg(a); }
    command.arg(audio_path);
    if lyrics { command.arg("--lyrics"); }
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
        lyric_exemplars: arg_opt(args, "lyric_exemplars").unwrap_or_default().into(),
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

/// Advance to the first stage that NEEDS ATTENTION: pending/in-progress, or
/// done-but-STALE (an earlier stage has a newer artifact — same rule the
/// checklist's ⚠ uses, `staleStageIds` in StageChecklist.tsx). Without the
/// stale check, a lyrics-first import skipped the ⚠-flagged Structure stage:
/// the UI warned "out of date" while advance hopped straight past it.
pub async fn advance_song(conn: &Connection, song_id: &str) -> Result<Value> {
    let stages = db::list_stages(conn, song_id).await?; // ordered by ordinal
    let mut newest_upstream: Option<String> = None;
    let mut next: Option<&crate::models::Stage> = None;
    for s in &stages {
        let stale = match (&s.artifact_at, &newest_upstream) {
            (Some(at), Some(up)) => at < up,
            _ => false,
        };
        if next.is_none() && (s.status != "done" || stale) {
            next = Some(s);
        }
        if let Some(at) = &s.artifact_at {
            if newest_upstream.as_deref().map(|up| at.as_str() > up).unwrap_or(true) {
                newest_upstream = Some(at.clone());
            }
        }
    }
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
        "update_style_preset" => {
            let id = arg(args, "id")?;
            let mut input = style_input(args);
            // a caller that omits lyric_exemplars (e.g. a chat edit passing the
            // classic 7-field set) must not silently wipe the user's exemplars
            if arg_opt(args, "lyric_exemplars").is_none() {
                if let Some(existing) = db::get_preset(conn, id).await? {
                    input.lyric_exemplars = existing.lyric_exemplars;
                }
            }
            v(db::update_preset(conn, id, input).await?)
        }
        "generate_style_preset" => v(agent::generate_style_preset(conn, settings, arg(args, "name")?, arg_opt(args, "notes"), |_| {}).await?),
        "set_preset_arrangement" => v(db::set_preset_arrangement(conn, arg(args, "id")?, arg(args, "arrangement")?).await?),
        "generate_preset_arrangement" => v(agent::generate_preset_arrangement(conn, settings, arg(args, "id")?).await?),
        "create_song_from_lyrics" => v(agent::create_song_from_lyrics(conn, settings, arg(args, "style_preset_id")?, arg_opt(args, "title").unwrap_or("Untitled song"), arg(args, "text")?).await?),
        "import_lyrics" => v(agent::import_lyrics(conn, settings, arg(args, "song_id")?, arg(args, "text")?).await?),
        "create_song" => v(db::create_song(conn, arg(args, "style_preset_id")?, arg_opt(args, "title").unwrap_or("Untitled song")).await?),
        "list_songs" => v(db::list_songs(conn).await?),
        "get_song" => v(db::get_song_detail(conn, arg(args, "id")?).await?),
        "update_song_status" => v(db::update_song_status(conn, arg(args, "id")?, arg(args, "status")?).await?),
        "update_song_title" => v(db::update_song_title(conn, arg(args, "id")?, arg(args, "title")?).await?),
        "update_song_intent" => v(db::update_song_intent(conn, arg(args, "id")?, arg(args, "intent")?).await?),
        "delete_song" => { db::delete_song(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "list_sections" => v(db::list_sections(conn, arg(args, "song_id")?).await?),
        "create_section" => v(db::create_section(
            conn,
            arg(args, "song_id")?,
            arg(args, "label")?,
            arg_opt(args, "type").unwrap_or(""),
            args.get("bars").and_then(|b| b.as_i64()).unwrap_or(8),
            arg_opt(args, "role").unwrap_or(""),
            args.get("position").and_then(|p| p.as_i64()),
        ).await?),
        "update_section" => {
            // partial update: omitted fields keep the row's current value
            let id = arg(args, "id")?;
            let cur = db::get_section(conn, id).await?.ok_or_else(|| anyhow!("section not found"))?;
            v(db::update_section(
                conn,
                id,
                arg_opt(args, "label").unwrap_or(&cur.label),
                arg_opt(args, "type").unwrap_or(&cur.r#type),
                args.get("bars").and_then(|b| b.as_i64()).unwrap_or(cur.bars),
                arg_opt(args, "role").unwrap_or(&cur.role),
            ).await?)
        }
        "delete_section" => { db::delete_section(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "reorder_sections" => {
            let ids: Vec<String> = args.get("section_ids").and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            v(db::reorder_sections(conn, arg(args, "song_id")?, &ids).await?)
        }
        "get_stage" => v(db::get_stage_detail(conn, arg(args, "id")?).await?),
        "run_stage" => {
            let o = agent::run_stage(conn, settings, arg(args, "stage_id")?, arg_opt(args, "user_input").map(String::from), |_| {}, None).await?;
            v(RunResult { artifact: o.artifact, draft: o.draft })
        }
        "accept_stage_draft" => v(agent::accept_stage_draft(conn, arg(args, "stage_id")?).await?),
        "discard_stage_draft" => { agent::discard_stage_draft(conn, arg(args, "stage_id")?).await?; Ok(json!({ "discarded": true })) }
        "approve_stage" => approve_stage(conn, arg(args, "stage_id")?).await,
        "advance_stage" => advance_song(conn, arg(args, "song_id")?).await,
        "get_artifact" => v(db::get_artifact(conn, arg(args, "id")?).await?),
        // TRUST MODEL: these two arms are Claude's write path (MCP / chat) — they go
        // through the freeze guard so frozen sections can never be overwritten or
        // resurrected-over. The user's own editor saves use the direct Tauri command
        // (`db::save_artifact`), which is the unlock/rewrite authority.
        "save_artifact" => v(agent::save_artifact_guarded(conn, arg(args, "song_id")?, arg_opt(args, "stage_id"), arg(args, "kind")?, arg(args, "content")?).await?),
        "list_artifact_revisions" => v(db::list_artifact_revisions(conn, arg(args, "stage_id")?).await?),
        "revert_artifact" => v(agent::revert_artifact_guarded(conn, arg(args, "artifact_id")?).await?),
        // metadata-only (never touches content) — no freeze guard needed
        "set_artifact_label" => { db::set_artifact_label(conn, arg(args, "artifact_id")?, arg_opt(args, "label")).await?; Ok(json!({ "ok": true })) }
        "list_skills" => v(db::list_skills(conn).await?),
        "get_skill" => v(db::get_skill(conn, arg(args, "id")?).await?),
        "create_skill" => v(db::create_skill(conn, skill_input(args)).await?),
        "update_skill" => v(db::update_skill(conn, arg(args, "id")?, skill_input(args)).await?),
        "set_skill_enabled" => v(db::set_skill_enabled(conn, arg(args, "id")?, args.get("enabled").and_then(|b| b.as_bool()).unwrap_or(true)).await?),
        "list_outlines" => v(db::list_outlines(conn).await?),
        "save_outline" => {
            let sections: Vec<(String, i64)> = args.get("sections").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| {
                    let label = x.get("label").and_then(|l| l.as_str())?.to_string();
                    let bars = x.get("bars").and_then(|b| b.as_i64()).filter(|&b| b > 0).unwrap_or(8);
                    Some((label, bars))
                }).collect())
                .unwrap_or_default();
            let bpm = args.get("bpm").and_then(|v| v.as_i64()).unwrap_or(120);
            v(db::create_outline(conn, arg(args, "name")?, bpm, &sections).await?)
        }
        "delete_outline" => { db::delete_outline(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "list_progressions" => v(db::list_progressions(conn).await?),
        "save_progression" => {
            let chords: Vec<String> = args.get("chords").and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            v(db::create_progression(conn, arg(args, "name")?, &chords, arg_opt(args, "picks").unwrap_or("")).await?)
        }
        "update_progression" => {
            let chords: Vec<String> = args.get("chords").and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            v(db::update_progression(conn, arg(args, "id")?, arg(args, "name")?, &chords, arg_opt(args, "picks").unwrap_or("")).await?)
        }
        "delete_progression" => { db::delete_progression(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "list_compositions" => v(db::list_compositions(conn).await?),
        "get_composition" => v(db::get_composition(conn, arg(args, "id")?).await?),
        "save_composition" => v(db::save_composition(conn, arg_opt(args, "id"), arg(args, "name")?, arg_opt(args, "song_id"), arg(args, "data")?).await?),
        "delete_composition" => { db::delete_composition(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "list_renders" => v(db::list_renders(conn, arg(args, "song_id")?).await?),
        "add_render" => v(db::create_render(conn, arg(args, "song_id")?, arg_opt(args, "label").unwrap_or("Render"), arg(args, "file_path")?, arg_opt(args, "source").unwrap_or(""), arg_opt(args, "notes").unwrap_or("")).await?),
        "set_render_pick" => { db::set_render_pick(conn, arg(args, "id")?, args.get("is_pick").and_then(|b| b.as_bool()).unwrap_or(true)).await?; Ok(json!({ "ok": true })) }
        "delete_render" => { db::delete_render(conn, arg(args, "id")?).await?; Ok(json!({ "ok": true })) }
        "ableton_build_outline" => {
            let sections: Vec<(String, i64)> = args.get("sections").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|s| {
                    let label = s.get("label").and_then(|l| l.as_str())?.to_string();
                    let bars = s.get("bars").and_then(|b| b.as_i64()).filter(|&b| b > 0).unwrap_or(8);
                    Some((label, bars))
                }).collect())
                .unwrap_or_default();
            let bpm = args.get("bpm").and_then(|v| v.as_i64()).unwrap_or(120);
            Ok(json!(tokio::task::spawn_blocking(move || ableton::build_outline(bpm, &sections)).await??))
        }
        "ableton_build_progression" => {
            let chords: Vec<String> = args.get("chords").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|c| c.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let invs: Vec<i64> = args.get("inversions").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|c| c.as_i64()).collect())
                .unwrap_or_default();
            let beats: Vec<i64> = args.get("beats").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|c| c.as_i64()).collect())
                .unwrap_or_default();
            let bpm = args.get("bpm").and_then(|v| v.as_i64()).unwrap_or(120);
            Ok(json!(tokio::task::spawn_blocking(move || {
                ableton::build_progression(bpm, &chords, &beats, &invs, &crate::midi::POP_DEFAULT, &|_| {})
            }).await??))
        }
        "ableton_build_song" => Ok(json!(ableton::build_song_for(conn, arg(args, "song_id")?, |_| {}).await?)),
        "analyze_reference" => run_analyzer(settings, arg(args, "audio_path")?, args.get("lyrics").and_then(|v| v.as_bool()).unwrap_or(false)).await,
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

    /// The five spine tools round-trip through `dispatch` (the UI/agent/MCP
    /// surface): create appends + inserts, update is PARTIAL (omitted fields
    /// keep their value), reorder takes the full permutation, delete removes.
    #[tokio::test]
    async fn section_tools_dispatch_round_trip() {
        let db = libsql::Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        crate::db::migrate(&conn).await.unwrap();
        let settings = Settings::default();
        let preset = dispatch(&conn, &settings, "create_style_preset", &json!({ "name": "P" })).await.unwrap();
        let song = dispatch(&conn, &settings, "create_song", &json!({ "style_preset_id": preset["id"], "title": "S" })).await.unwrap();
        let song_id = song["id"].as_str().unwrap().to_string();

        let verse = dispatch(&conn, &settings, "create_section",
            &json!({ "song_id": song_id, "label": "Verse 1", "type": "verse", "bars": 16, "role": "opens" })).await.unwrap();
        let chorus = dispatch(&conn, &settings, "create_section",
            &json!({ "song_id": song_id, "label": "Chorus" })).await.unwrap();
        assert_eq!(chorus["bars"], json!(8), "bars defaults to 8");
        let intro = dispatch(&conn, &settings, "create_section",
            &json!({ "song_id": song_id, "label": "Intro", "position": 0 })).await.unwrap();

        let list = dispatch(&conn, &settings, "list_sections", &json!({ "song_id": song_id })).await.unwrap();
        let labels: Vec<&str> = list.as_array().unwrap().iter().map(|x| x["label"].as_str().unwrap()).collect();
        assert_eq!(labels, vec!["Intro", "Verse 1", "Chorus"]);

        // partial update: only bars given — label/type/role must survive
        let updated = dispatch(&conn, &settings, "update_section",
            &json!({ "id": verse["id"], "bars": 12 })).await.unwrap();
        assert_eq!(updated["label"], json!("Verse 1"));
        assert_eq!(updated["type"], json!("verse"));
        assert_eq!(updated["bars"], json!(12));
        assert_eq!(updated["role"], json!("opens"));

        let reordered = dispatch(&conn, &settings, "reorder_sections",
            &json!({ "song_id": song_id, "section_ids": [chorus["id"], intro["id"], verse["id"]] })).await.unwrap();
        let labels: Vec<&str> = reordered.as_array().unwrap().iter().map(|x| x["label"].as_str().unwrap()).collect();
        assert_eq!(labels, vec!["Chorus", "Intro", "Verse 1"]);

        let gone = dispatch(&conn, &settings, "delete_section", &json!({ "id": intro["id"] })).await.unwrap();
        assert_eq!(gone["ok"], json!(true));
        let list = dispatch(&conn, &settings, "list_sections", &json!({ "song_id": song_id })).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
    }

    #[test]
    fn mock_has_every_tool() {
        let mock = include_str!("../../frontend/src/ipc/mockApi.ts");
        let missing: Vec<&str> = tool_names().into_iter().filter(|n| !mock.contains(&format!("\"{n}\""))).collect();
        assert!(missing.is_empty(), "tools missing from mockApi.ts: {missing:?}");
    }
}
