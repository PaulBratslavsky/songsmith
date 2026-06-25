//! The stage agent loop.
//!
//! `run_stage` loads the stage's Skill + the StylePreset + all prior approved
//! artifacts, calls **Claude** (the user's Claude Code subscription, via the
//! `claude` CLI), streams the output, and writes the resulting Artifact. There
//! is no local model — Claude is the engine.

use crate::db;
use crate::models::*;
use anyhow::{anyhow, Result};
use libsql::Connection;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

/// The artifact `kind` produced by each stage type.
pub fn kind_for_stage(stage_type: &str) -> &'static str {
    match stage_type {
        "concept" => "concept",
        "structure" => "structure",
        "chords" => "chords",
        "lyric_spec" => "lyric_spec",
        "lyrics" => "lyrics",
        "prompt" => "generation_prompt",
        _ => "artifact",
    }
}

pub struct RunOutcome {
    pub artifact: Artifact,
    pub raw_output: String,
}

/// Run a stage. `user_input` carries an optional seed (the producer's own
/// chords/lyrics/title). `on_token` receives streamed text chunks.
pub async fn run_stage<F>(
    conn: &Connection,
    settings: &Settings,
    stage_id: &str,
    user_input: Option<String>,
    on_token: F,
) -> Result<RunOutcome>
where
    F: Fn(String) + Send,
{
    let stage = db::get_stage(conn, stage_id).await?.ok_or_else(|| anyhow!("stage not found"))?;
    let song = db::get_song(conn, &stage.song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let preset = db::get_preset(conn, &song.style_preset_id).await?.ok_or_else(|| anyhow!("style preset not found"))?;
    let skill = db::get_active_skill_for_stage(conn, &stage.r#type)
        .await?
        .ok_or_else(|| anyhow!("no enabled skill for stage '{}'", stage.r#type))?;

    db::set_stage_status(conn, stage_id, "in_progress").await?;
    db::set_song_current_stage(conn, &song.id, &stage.r#type).await?;
    if stage.skill_id.as_deref() != Some(skill.id.as_str()) {
        db::set_stage_skill(conn, stage_id, &skill.id).await?;
    }

    let prior = gather_prior_context(conn, &stage.song_id, stage.ordinal).await?;
    let system = build_system_prompt(&skill, &preset, &song);
    let user = build_user_prompt(&stage.r#type, &prior, user_input.as_deref());

    let text = call_claude(settings, &system, &user, &on_token).await?;
    let data = extract_json(&text);
    let content = json!({ "kind": kind_for_stage(&stage.r#type), "text": text, "data": data }).to_string();

    let artifact = db::save_artifact(conn, &song.id, Some(stage_id), kind_for_stage(&stage.r#type), &content).await?;
    Ok(RunOutcome { artifact, raw_output: text })
}

/// Auto-generate a style preset from a seed (name / vibe / reference), grounded
/// in the style-level skill. Returns a filled `StyleInput`; `name` is verbatim.
pub async fn generate_style_preset<F>(
    conn: &Connection,
    settings: &Settings,
    seed_name: &str,
    seed_notes: Option<&str>,
    on_token: F,
) -> Result<StyleInput>
where
    F: Fn(String) + Send,
{
    let method = db::list_skills(conn)
        .await?
        .into_iter()
        .filter(|s| s.stage_type == "style" && s.enabled)
        .map(|s| format!("# {}\n{}", s.name, s.instructions))
        .collect::<Vec<_>>()
        .join("\n\n");

    let system = format!(
        "You design a reusable artist/style preset for a music producer, grounded in the method below.\n\n\
         {method}\n\n\
         Be specific and decisive; never name real artists to imitate — describe the sound. \
         Respond with ONLY a single fenced ```json block of exactly these string keys:\n\
         {{\"genre\":\"...\",\"mood\":\"...\",\"influences\":\"...\",\"key_tempo_feel\":\"...\",\"vocal_range\":\"...\",\"themes\":\"...\"}}",
        method = if method.is_empty() { "(no style skill installed)".to_string() } else { method }
    );
    let user = format!("Seed / name: {seed_name}\nNotes: {notes}\n\nProduce the style preset JSON now.", notes = seed_notes.unwrap_or("(none)"));

    let text = call_claude(settings, &system, &user, &on_token).await?;
    let d = extract_json(&text).ok_or_else(|| anyhow!("the model did not return a JSON preset. Raw output:\n{}", text.trim()))?;
    let g = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    Ok(StyleInput {
        name: seed_name.to_string(),
        genre: g("genre"),
        mood: g("mood"),
        influences: g("influences"),
        key_tempo_feel: g("key_tempo_feel"),
        vocal_range: g("vocal_range"),
        themes: g("themes"),
    })
}

async fn gather_prior_context(conn: &Connection, song_id: &str, ordinal: i64) -> Result<String> {
    let stages = db::list_stages(conn, song_id).await?;
    let mut blocks = Vec::new();
    for s in stages.into_iter().filter(|s| s.ordinal < ordinal) {
        // Use each prior stage's CURRENT artifact (not only approved ones) so edits
        // cascade: re-running a downstream stage always builds on the latest upstream
        // content. Saving an edit creates a new (unapproved) revision, and the user
        // expects that to be what downstream sees.
        if let Some(art) = db::current_artifact(conn, &s.id).await? {
            let body = serde_json::from_str::<Value>(&art.content)
                .ok()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .unwrap_or(art.content.clone());
            blocks.push(format!("### {} output\n{}", stage_label(&s.r#type), body));
        }
    }
    Ok(blocks.join("\n\n"))
}

fn build_system_prompt(skill: &Skill, preset: &StylePreset, song: &Song) -> String {
    format!(
        "{instructions}\n\n\
         ----- STYLE PRESET (persistent context — read this on every stage) -----\n\
         Project: {name}\n\
         Genre: {genre}\n\
         Mood & energy: {mood}\n\
         Influences (describe the sound, don't clone): {influences}\n\
         Key / tempo feel: {ktf}\n\
         Vocal range: {vr}\n\
         Recurring themes: {themes}\n\
         Current song key/tempo: {root} {mode}, {bpm} BPM\n\
         -----------------------------------------------------------------------\n\
         Honor the style and themes above. Never name real artists to imitate or quote their lyrics.",
        instructions = skill.instructions,
        name = preset.name, genre = preset.genre, mood = preset.mood, influences = preset.influences,
        ktf = preset.key_tempo_feel, vr = preset.vocal_range, themes = preset.themes,
        root = song.key_root, mode = song.key_mode, bpm = song.bpm,
    )
}

fn build_user_prompt(stage_type: &str, prior: &str, user_input: Option<&str>) -> String {
    let mut p = String::new();
    if !prior.is_empty() {
        p.push_str("Approved outputs from earlier stages (carry these forward):\n\n");
        p.push_str(prior);
        p.push_str("\n\n");
    }
    if let Some(input) = user_input {
        if !input.trim().is_empty() {
            p.push_str("The producer's own seed for this stage (build around it):\n\n");
            p.push_str(input.trim());
            p.push_str("\n\n");
        }
    }
    p.push_str(&format!(
        "Produce the {} now, following your skill exactly. End with the artifact as a single fenced ```json block matching the documented shape.",
        stage_label(stage_type)
    ));
    p
}

/// Import a reference track: run the local analyzer (perception), have the
/// Reference Analyst skill interpret it (cognition), then create a new song with
/// its Structure + Chords populated. Returns the new song id. Audio stays local.
pub async fn import_reference(conn: &Connection, settings: &Settings, audio_path: &str) -> Result<String> {
    // 1. perception — reuse the analyzer the MCP tool runs
    let raw = crate::tools::dispatch(conn, settings, "analyze_reference", &json!({ "audio_path": audio_path })).await?;

    // 2. cognition — the Reference Analyst skill turns raw MIR into Structure + Chords
    let skill = db::get_active_skill_for_stage(conn, "reference").await?
        .ok_or_else(|| anyhow!("the Reference Analyst skill is missing"))?;
    let user = format!("Analyzer output (raw perception):\n\n{}", serde_json::to_string_pretty(&raw)?);
    let out = call_claude(settings, &skill.instructions, &user, &|_| {}).await?;
    let parsed = extract_json(&out).ok_or_else(|| anyhow!("could not parse the Reference Analyst output"))?;
    let structure = parsed.get("structure").cloned().ok_or_else(|| anyhow!("analysis had no structure"))?;
    let chords = parsed.get("chords").cloned().unwrap_or_else(|| json!({ "sections": [] }));

    // 3. a song to hold it (reuse the first preset, or a minimal one)
    let preset_id = match db::list_presets(conn).await?.into_iter().next() {
        Some(p) => p.id,
        None => db::create_preset(conn, StyleInput {
            name: "Imported".into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(),
        }).await?.id,
    };
    let title = std::path::Path::new(audio_path).file_stem()
        .map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Imported reference".into());
    let song = db::create_song(conn, &preset_id, &title).await?;

    // 4. key/tempo + Structure + Chords artifacts
    let root = structure.pointer("/key/root").and_then(|v| v.as_str()).unwrap_or("A");
    let mode = structure.pointer("/key/mode").and_then(|v| v.as_str()).unwrap_or("minor");
    let bpm = structure.get("bpm").and_then(|v| v.as_i64()).unwrap_or(120);
    db::update_song_key(conn, &song.id, root, mode, bpm).await?;

    let stages = db::list_stages(conn, &song.id).await?;
    let stage_id = |t: &str| stages.iter().find(|s| s.r#type == t).map(|s| s.id.clone());
    if let Some(sid) = stage_id("structure") {
        let content = json!({ "kind": "structure", "text": structure_text(&structure), "data": structure }).to_string();
        db::save_artifact(conn, &song.id, Some(&sid), "structure", &content).await?;
        let _ = db::set_stage_status(conn, &sid, "done").await;
    }
    if let Some(cid) = stage_id("chords") {
        let content = json!({ "kind": "chords", "text": chords_text(&chords), "data": chords }).to_string();
        db::save_artifact(conn, &song.id, Some(&cid), "chords", &content).await?;
        let _ = db::set_stage_status(conn, &cid, "done").await;
    }
    Ok(song.id)
}

fn structure_text(s: &Value) -> String {
    let root = s.pointer("/key/root").and_then(|v| v.as_str()).unwrap_or("");
    let mode = s.pointer("/key/mode").and_then(|v| v.as_str()).unwrap_or("");
    let bpm = s.get("bpm").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut out = format!("**KEY:** {root} {mode}\n**TEMPO:** {bpm} BPM\n\n**SECTION MAP**\n");
    if let Some(arr) = s.get("sections").and_then(|v| v.as_array()) {
        for (i, sec) in arr.iter().enumerate() {
            let label = sec.get("label").and_then(|v| v.as_str()).unwrap_or("Section");
            let bars = sec.get("bars").and_then(|v| v.as_i64()).unwrap_or(0);
            let role = sec.get("role").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("{}. **{label}** ({bars} bars) — {role}\n", i + 1));
        }
    }
    out
}

fn chords_text(c: &Value) -> String {
    let mut out = String::new();
    if let Some(arr) = c.get("sections").and_then(|v| v.as_array()) {
        for sec in arr {
            let label = sec.get("label").and_then(|v| v.as_str()).unwrap_or("Section");
            let names: Vec<&str> = sec.get("chords").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            out.push_str(&format!("{label}: {}\n", names.join(" ")));
        }
    }
    out
}

/// Refine a single field of a song spec via Claude — returns ONLY the new value
/// for that field, so the caller can drop it straight into the structured object.
pub async fn refine_field(settings: &Settings, stage_label: &str, field_label: &str, current: &str, instruction: &str) -> Result<String> {
    let system = "You refine exactly ONE field of a song's structured spec. Return ONLY the new value for that field — no preamble, no explanation, no markdown code fences, no surrounding quotes. If the field is a list, separate items with ' · '. Keep the same voice and length unless the request says otherwise.";
    let user = format!(
        "Stage: {stage_label}\nField: {field_label}\n\nCurrent value:\n{current}\n\nProducer's request: {instruction}\n\nReturn ONLY the new {field_label} value.",
    );
    let text = call_claude(settings, system, &user, &|_: String| {}).await?;
    let t = text.trim();
    // strip an accidental ```fence``` or wrapping quotes if the model added them
    let t = t.strip_prefix("```").map(|s| s.trim_start_matches(|c: char| c.is_alphanumeric()).trim()).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    Ok(t.trim().to_string())
}

/// Generate text with Claude by driving the `claude` CLI headless with
/// stream-json. Streams text deltas via `on_token`, returns the final answer.
async fn call_claude<F>(settings: &Settings, system: &str, user: &str, on_token: &F) -> Result<String>
where
    F: Fn(String) + Send,
{
    let bin = if settings.claude_bin.is_empty() { "claude".to_string() } else { settings.claude_bin.clone() };
    let mut cmd = tokio::process::Command::new(&bin);
    // This app drives Claude via your Claude Code / claude.ai subscription login.
    // If ANTHROPIC_API_KEY (or the helper var) is inherited from the launching
    // environment, the CLI silently switches to API billing and disables connectors.
    // Strip them so it always uses the subscription login.
    cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
    cmd.arg("-p").arg(user)
        .arg("--append-system-prompt").arg(system)
        .arg("--output-format").arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--no-session-persistence")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if !settings.claude_model.is_empty() {
        cmd.arg("--model").arg(&settings.claude_model);
    }

    let mut child = cmd.spawn().map_err(|e| anyhow!("could not start the claude CLI ({bin}): {e}. Install Claude Code and sign in."))?;
    let stdout = child.stdout.take().unwrap();
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let (mut result_text, mut assistant_text, mut streamed) = (String::new(), String::new(), String::new());

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v["type"].as_str() {
            Some("stream_event") => {
                let ev = &v["event"];
                if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                    if let Some(tok) = ev["delta"]["text"].as_str() {
                        if !tok.is_empty() {
                            streamed.push_str(tok);
                            on_token(tok.to_string());
                        }
                    }
                }
            }
            Some("assistant") => {
                if let Some(content) = v["message"]["content"].as_array() {
                    let mut t = String::new();
                    for b in content {
                        if b["type"] == "text" {
                            if let Some(s) = b["text"].as_str() { t.push_str(s); }
                        }
                    }
                    if !t.is_empty() { assistant_text = t; }
                }
            }
            Some("result") => {
                if let Some(r) = v["result"].as_str() { result_text = r.to_string(); }
            }
            _ => {}
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut se) = child.stderr.take() {
            let _ = se.read_to_string(&mut err).await;
        }
        let err = err.trim();
        return Err(anyhow!("claude CLI failed: {}", if err.is_empty() { "is Claude Code signed in? run `claude` once to authenticate." } else { err }));
    }

    let out = if !result_text.trim().is_empty() { result_text }
        else if !assistant_text.trim().is_empty() { assistant_text }
        else { streamed };
    if out.trim().is_empty() {
        return Err(anyhow!("claude returned no output"));
    }
    Ok(out)
}

/// Extract the first JSON object from model output (fenced or bare).
pub fn extract_json(text: &str) -> Option<Value> {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(after[..end].trim()) {
                return Some(v);
            }
        }
    }
    let bytes = text.as_bytes();
    if let Some(open) = text.find('{') {
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for i in open..bytes.len() {
            let c = bytes[i] as char;
            if in_str {
                if esc { esc = false; } else if c == '\\' { esc = true; } else if c == '"' { in_str = false; }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<Value>(&text[open..=i]) { return Some(v); }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}
