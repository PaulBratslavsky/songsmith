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

// ---- Per-section Freeze (regeneration-safe) -------------------------------
//
// A section can carry an optional `"frozen": true` flag in a stage artifact's
// `data.sections[]` (or `data.beats[]` for lyric_spec). A frozen section is a
// HARD guarantee: when the stage regenerates, the prior frozen section is
// spliced back verbatim over whatever Claude produced — the lock is enforced
// deterministically, not just via the prompt. When nothing is frozen the merge
// is a no-op and behavior is identical to before.

/// The only stages with section-based artifacts that support freezing.
fn is_section_stage(stage_type: &str) -> bool {
    matches!(stage_type, "structure" | "chords" | "lyric_spec" | "lyrics")
}

/// `(array key, label key)` for a section-based stage. lyric_spec's per-section
/// unit is the beat sheet (`beats[]`, keyed by `section`); the rest use
/// `sections[]` keyed by `label`.
fn section_keys(stage_type: &str) -> (&'static str, &'static str) {
    match stage_type {
        "lyric_spec" => ("beats", "section"),
        _ => ("sections", "label"),
    }
}

/// Read a section's label (the chords/structure/lyrics editors fall back from
/// `label` to `type`; matching is case/space-insensitive on the normalized form).
fn section_label(stage_type: &str, sec: &Value) -> String {
    let (_, lbl_key) = section_keys(stage_type);
    sec.get(lbl_key)
        .and_then(|v| v.as_str())
        .or_else(|| sec.get("type").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string()
}

fn norm_label(s: &str) -> String {
    s.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Does this artifact `data` carry any frozen sections? The write-boundary guard
/// and the merge path share this one detection (same section-array keys).
fn has_frozen_sections(stage_type: &str, data: &Value) -> bool {
    if !is_section_stage(stage_type) {
        return false;
    }
    let (arr_key, _) = section_keys(stage_type);
    data.get(arr_key)
        .and_then(|v| v.as_array())
        .is_some_and(|secs| secs.iter().any(|s| s.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false)))
}

/// Deterministically splice prior **frozen** sections into the regenerated
/// artifact's `data`. Match by label (case/space-insensitive); if Claude dropped
/// a frozen section, re-insert it at its original index; always carry `frozen:true`.
/// Returns the merged `data` Value. Non-section stages return `new_data` unchanged.
pub fn merge_frozen_sections(stage_type: &str, prior_data: &Value, new_data: &Value) -> Value {
    if !is_section_stage(stage_type) {
        return new_data.clone();
    }
    let (arr_key, _) = section_keys(stage_type);
    let prior_secs = prior_data.get(arr_key).and_then(|v| v.as_array());
    let Some(prior_secs) = prior_secs else { return new_data.clone() };

    // collect (original_index, label, section) for each prior-frozen section
    let frozen: Vec<(usize, String, Value)> = prior_secs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false))
        .map(|(i, s)| {
            let mut s = s.clone();
            // ensure the flag is carried forward verbatim
            if let Some(obj) = s.as_object_mut() {
                obj.insert("frozen".into(), Value::Bool(true));
            }
            (i, norm_label(&section_label(stage_type, &s)), s)
        })
        .collect();

    if frozen.is_empty() {
        return new_data.clone();
    }

    let mut merged = new_data.clone();
    if !merged.is_object() {
        merged = json!({});
    }
    let new_secs = merged
        .get(arr_key)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = new_secs.clone();

    for (orig_idx, flabel, fsec) in &frozen {
        // overwrite the matching new section verbatim …
        if let Some(pos) = out
            .iter()
            .position(|s| norm_label(&section_label(stage_type, s)) == *flabel)
        {
            out[pos] = fsec.clone();
        } else {
            // … or re-insert it at (close to) its original position if dropped.
            let at = (*orig_idx).min(out.len());
            out.insert(at, fsec.clone());
        }
    }

    merged[arr_key] = Value::Array(out);
    merged
}

/// Render the human-readable `text` for a section-based stage from its merged
/// `data`, mirroring exactly how each stage's editor serializes text → so the
/// artifact `text` (which `gather_prior_context` reads) stays consistent after a
/// splice. Returns `None` for non-section stages (keep Claude's text).
fn render_stage_text(stage_type: &str, data: &Value) -> Option<String> {
    match stage_type {
        "structure" => Some(structure_editor_text(data)),
        "chords" => Some(chords_editor_text(data)),
        "lyrics" => Some(lyrics_text(data)),
        "lyric_spec" => Some(lyric_spec_text(data)),
        _ => None,
    }
}

/// Mirror of `Composer`'s save: `label: name name …` per section. Unlike the
/// `chords_text` used by reference-import (string chords), the Composer stores
/// each chord as `{name,beats}`, so read `name` (falling back to a bare string).
fn chords_editor_text(c: &Value) -> String {
    let mut out = Vec::new();
    if let Some(arr) = c.get("sections").and_then(|v| v.as_array()) {
        for sec in arr {
            let label = sec.get("label").and_then(|v| v.as_str())
                .or_else(|| sec.get("type").and_then(|v| v.as_str())).unwrap_or("Section");
            let names: Vec<String> = sec.get("chords").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| {
                    x.as_str().map(String::from).or_else(|| x.get("name").and_then(|n| n.as_str()).map(String::from))
                }).collect())
                .unwrap_or_default();
            out.push(format!("{label}: {}", names.join(" ")));
        }
    }
    out.join("\n")
}

/// Mirror of `StructureEditor.structureToMarkdown`.
fn structure_editor_text(d: &Value) -> String {
    let root = d.pointer("/key/root").and_then(|v| v.as_str()).unwrap_or("");
    let mode = d.pointer("/key/mode").and_then(|v| v.as_str()).unwrap_or("");
    let bpm = d.get("bpm").and_then(|v| v.as_i64()).unwrap_or(120);
    let key_note = d.get("keyNote").and_then(|v| v.as_str()).unwrap_or("");
    let tempo_note = d.get("tempoNote").and_then(|v| v.as_str()).unwrap_or("");
    let mut lines = vec![
        format!("**KEY:** {root} {mode}{}", if key_note.is_empty() { String::new() } else { format!(" — {key_note}") }),
        format!("**TEMPO:** {bpm} BPM{}", if tempo_note.is_empty() { String::new() } else { format!(" — {tempo_note}") }),
        String::new(),
        "**SECTION MAP**".into(),
        String::new(),
    ];
    if let Some(arr) = d.get("sections").and_then(|v| v.as_array()) {
        for (i, sec) in arr.iter().enumerate() {
            let label = sec.get("label").and_then(|v| v.as_str())
                .or_else(|| sec.get("type").and_then(|v| v.as_str())).unwrap_or("");
            let bars = sec.get("bars").and_then(|v| v.as_i64()).unwrap_or(8);
            let role = sec.get("role").and_then(|v| v.as_str()).unwrap_or("");
            lines.push(format!("{}. **{label}** ({bars} bars){}", i + 1, if role.is_empty() { String::new() } else { format!(" — {role}") }));
        }
    }
    lines.join("\n")
}

/// Mirror of `LyricsEditor`'s save: `[label]\n<lines>` blocks, each line a
/// ChordPro string (the lyrics `data.sections[].lines` are already strings).
fn lyrics_text(d: &Value) -> String {
    let mut blocks = Vec::new();
    if let Some(arr) = d.get("sections").and_then(|v| v.as_array()) {
        for sec in arr {
            let label = sec.get("label").and_then(|v| v.as_str())
                .or_else(|| sec.get("type").and_then(|v| v.as_str())).unwrap_or("Section");
            let lines: Vec<String> = sec.get("lines").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            blocks.push(format!("[{label}]\n{}", lines.join("\n")));
        }
    }
    blocks.join("\n\n")
}

/// Mirror of `LyricSpecEditor.specToMarkdown` (scalar fields + the beat sheet).
fn lyric_spec_text(d: &Value) -> String {
    let g = |k: &str| d.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let diction = {
        let dc = g("diction");
        if dc.is_empty() { "balanced" } else { dc }
    };
    let diction_gloss = match diction {
        "plain-spoken" => "conversational, almost no metaphor",
        "literary" => "dense, poetic, image-rich",
        _ => "mostly plain with a few sharp images",
    };
    let mut lines = vec![
        format!("**HOOK:** {}", g("hook")),
        format!("**PREMISE:** {}", g("premise")),
        format!("**POV / TENSE:** {}", g("pov")),
        format!("**SETTING:** {}", g("setting")),
        format!("**ARC:** {}", g("arc")),
        format!("**DICTION:** {diction} ({diction_gloss})"),
    ];
    let rv = g("referenceVibe");
    lines.push(if rv.is_empty() { String::new() } else { format!("**REFERENCE VIBE:** {rv}") });
    lines.push(String::new());
    lines.push("**SONG MAP (beat sheet)**".into());
    if let Some(arr) = d.get("beats").and_then(|v| v.as_array()) {
        for b in arr {
            let section = b.get("section").and_then(|v| v.as_str())
                .or_else(|| b.get("label").and_then(|v| v.as_str())).unwrap_or("");
            let beat = b.get("beat").and_then(|v| v.as_str())
                .or_else(|| b.get("text").and_then(|v| v.as_str())).unwrap_or("");
            lines.push(format!("- {section}: {beat}"));
        }
    }
    lines.push(String::new());
    let join_list = |k: &str| d.get(k).and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" · "))
        .unwrap_or_default();
    lines.push(format!("**IMAGE BANK:** {}", join_list("imageBank")));
    lines.push(format!("**AVOID:** {}", join_list("avoid")));
    lines.join("\n")
}

/// Render the prior frozen sections as text to inject into the generation prompt,
/// so the regenerated unlocked sections stay coherent with the locked ones.
fn frozen_prompt_block(stage_type: &str, prior_data: &Value) -> Option<String> {
    if !is_section_stage(stage_type) {
        return None;
    }
    let (arr_key, _) = section_keys(stage_type);
    let secs = prior_data.get(arr_key).and_then(|v| v.as_array())?;
    let frozen: Vec<&Value> = secs
        .iter()
        .filter(|s| s.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false))
        .collect();
    if frozen.is_empty() {
        return None;
    }
    // render only the frozen subset through the same per-stage renderer
    let subset = json!({ arr_key: frozen });
    let rendered = render_stage_text(stage_type, &subset).unwrap_or_default();
    let labels: Vec<String> = frozen.iter().map(|s| section_label(stage_type, s)).filter(|l| !l.is_empty()).collect();
    Some(format!(
        "LOCKED SECTIONS — these are FINAL. Reproduce them EXACTLY (verbatim, same labels: {}) and only (re)write the other sections so they stay coherent with these:\n\n{}",
        labels.join(", "),
        rendered.trim()
    ))
}

/// Build the merged, frozen-spliced artifact `content` JSON string from Claude's
/// raw `text` + the stage's prior current artifact. When nothing was frozen this
/// produces exactly the same `{kind,text,data}` as before (no-op). When the prior
/// has frozen sections and the model output has no parseable JSON, this is an
/// error — never save `{data: null}` over locked content.
fn build_merged_content(stage_type: &str, raw_text: &str, prior_content: Option<&str>) -> Result<String> {
    let kind = kind_for_stage(stage_type);
    let new_data = extract_json(raw_text);

    // Only section stages with prior frozen sections trigger a splice.
    if is_section_stage(stage_type) {
        if let Some(pc) = prior_content {
            let prior_data = serde_json::from_str::<Value>(pc)
                .ok()
                .and_then(|v| v.get("data").cloned());
            if let Some(prior_data) = prior_data {
                if has_frozen_sections(stage_type, &prior_data) {
                    let Some(nd) = new_data else {
                        return Err(anyhow!(
                            "model output had no parseable JSON; refusing to overwrite an artifact with locked sections — the prior revision stays current"
                        ));
                    };
                    let merged = merge_frozen_sections(stage_type, &prior_data, &nd);
                    let text = render_stage_text(stage_type, &merged).unwrap_or_else(|| raw_text.to_string());
                    return Ok(json!({ "kind": kind, "text": text, "data": merged }).to_string());
                }
            }
        }
    }
    Ok(json!({ "kind": kind, "text": raw_text, "data": new_data }).to_string())
}

// ---- The freeze write-boundary ----------------------------------------------
//
// TRUST MODEL: UI editor saves are the *user's* authority — they call
// `db::save_artifact` directly and may unlock/rewrite anything (that is how
// unfreezing works). Claude-driven writes (the MCP `save_artifact` /
// `revert_artifact` tools, and the agent loop via `build_merged_content`) must
// NEVER violate frozen sections, so they come through these guarded wrappers.

/// Guarded artifact save for Claude-originated writes. For a section-based stage
/// whose prior current artifact has frozen sections: parse the incoming content,
/// splice the frozen sections back verbatim (`merge_frozen_sections`), re-render
/// `text` from the merged data, and save that. Incoming content that is not
/// valid JSON while the prior has frozen sections is an error — nothing is
/// saved. Non-section stages / no frozen sections fall through to a plain save
/// (behavior identical to an unguarded save).
pub async fn save_artifact_guarded(
    conn: &Connection,
    song_id: &str,
    stage_id: Option<&str>,
    kind: &str,
    content: &str,
) -> Result<Artifact> {
    if let Some(sid) = stage_id {
        if let Some(stage) = db::get_stage(conn, sid).await? {
            if is_section_stage(&stage.r#type) {
                if let Some(prior) = db::current_artifact(conn, sid).await? {
                    let prior_data = serde_json::from_str::<Value>(&prior.content)
                        .ok()
                        .and_then(|v| v.get("data").cloned());
                    if let Some(prior_data) = prior_data {
                        if has_frozen_sections(&stage.r#type, &prior_data) {
                            let incoming = serde_json::from_str::<Value>(content).map_err(|_| anyhow!(
                                "this stage has locked (frozen) sections and the incoming content is not valid JSON — refusing to save. Unlock the sections in the app to rewrite them."
                            ))?;
                            // accept the `{kind,text,data}` wrapper or bare data
                            // (an object already carrying the stage's section array)
                            let (arr_key, _) = section_keys(&stage.r#type);
                            let incoming_data = incoming
                                .get("data")
                                .cloned()
                                .or_else(|| incoming.get(arr_key).is_some().then(|| incoming.clone()))
                                .unwrap_or(Value::Null);
                            let merged = merge_frozen_sections(&stage.r#type, &prior_data, &incoming_data);
                            let text = render_stage_text(&stage.r#type, &merged)
                                .or_else(|| incoming.get("text").and_then(|t| t.as_str()).map(String::from))
                                .unwrap_or_default();
                            let guarded = json!({ "kind": kind, "text": text, "data": merged }).to_string();
                            return db::save_artifact(conn, song_id, Some(sid), kind, &guarded).await;
                        }
                    }
                }
            }
        }
    }
    db::save_artifact(conn, song_id, stage_id, kind, content).await
}

/// Guarded revert for Claude-originated calls: restoring an old revision must
/// not resurrect pre-freeze content over currently-frozen sections — the
/// reverted content passes through the same write-boundary guard.
pub async fn revert_artifact_guarded(conn: &Connection, artifact_id: &str) -> Result<Artifact> {
    let t = db::get_artifact(conn, artifact_id).await?.ok_or_else(|| anyhow!("artifact not found"))?;
    save_artifact_guarded(conn, &t.song_id, t.stage_id.as_deref(), &t.kind, &t.content).await
}

// ---- Cancellation ------------------------------------------------------------

/// Cancels an in-flight Claude call. Clone it freely; `cancel()` releases every
/// current *and future* `cancelled().await` (a pre-fired token resolves
/// immediately, so there is no arm-before-fire race).
#[derive(Clone)]
pub struct CancelToken(std::sync::Arc<tokio::sync::watch::Sender<bool>>);

impl CancelToken {
    pub fn new() -> Self {
        Self(std::sync::Arc::new(tokio::sync::watch::channel(false).0))
    }
    pub fn cancel(&self) {
        // send_replace, not send: `send` is a no-op while no receiver exists yet,
        // which would drop a cancel fired before the call loop starts listening
        let _ = self.0.send_replace(true);
    }
    /// Resolves once `cancel()` has been called (immediately if it already was).
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // wait_for checks the current value first; Err (sender dropped) cannot
        // happen while `self` holds the sender.
        let _ = rx.wait_for(|c| *c).await;
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Run a stage. `user_input` carries an optional seed (the producer's own
/// chords/lyrics/title). `on_token` receives streamed text chunks. `cancel`
/// (when given) aborts the in-flight Claude call — the run errors out and the
/// stage status is restored.
pub async fn run_stage<F>(
    conn: &Connection,
    settings: &Settings,
    stage_id: &str,
    user_input: Option<String>,
    on_token: F,
    cancel: Option<CancelToken>,
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

    let prior_status = stage.status.clone();
    db::set_stage_status(conn, stage_id, "in_progress").await?;

    let run = async {
        db::set_song_current_stage(conn, &song.id, &stage.r#type).await?;
        if stage.skill_id.as_deref() != Some(skill.id.as_str()) {
            db::set_stage_skill(conn, stage_id, &skill.id).await?;
        }

        // the stage's prior current artifact — its frozen sections are protected
        let prior_artifact = db::current_artifact(conn, stage_id).await?;
        let prior_data = prior_artifact.as_ref().and_then(|a| {
            serde_json::from_str::<Value>(&a.content).ok().and_then(|v| v.get("data").cloned())
        });

        let prior = gather_prior_context(conn, &stage.song_id, stage.ordinal).await?;
        let system = build_system_prompt(&skill, &preset, &song);
        let mut user = build_user_prompt(&stage.r#type, &prior, user_input.as_deref());
        if let Some(pd) = &prior_data {
            if let Some(block) = frozen_prompt_block(&stage.r#type, pd) {
                user.push_str("\n\n");
                user.push_str(&block);
            }
        }

        let text = call_claude(settings, &system, &user, &on_token, cancel.as_ref()).await?;
        let content = build_merged_content(&stage.r#type, &text, prior_artifact.as_ref().map(|a| a.content.as_str()))?;

        let artifact = db::save_artifact(conn, &song.id, Some(stage_id), kind_for_stage(&stage.r#type), &content).await?;
        Ok::<RunOutcome, anyhow::Error>(RunOutcome { artifact, raw_output: text })
    };

    match run.await {
        Ok(outcome) => Ok(outcome),
        Err(e) => {
            // never strand the stage "in_progress" on a failed/cancelled run
            let _ = db::set_stage_status(conn, stage_id, &prior_status).await;
            Err(e)
        }
    }
}

/// Self-test + refine pass: the model critiques its own stage output against the
/// skill and coherence checks (title lands as the hook, sections fit the spec, no
/// clichés / over-writing), then returns a revised version saved as a new revision.
pub async fn self_check_stage(conn: &Connection, settings: &Settings, stage_id: &str) -> Result<Artifact> {
    let stage = db::get_stage(conn, stage_id).await?.ok_or_else(|| anyhow!("stage not found"))?;
    let song = db::get_song(conn, &stage.song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let preset = db::get_preset(conn, &song.style_preset_id).await?.ok_or_else(|| anyhow!("style preset not found"))?;
    let skill = db::get_active_skill_for_stage(conn, &stage.r#type).await?
        .ok_or_else(|| anyhow!("no enabled skill for stage '{}'", stage.r#type))?;
    let current = db::current_artifact(conn, stage_id).await?
        .ok_or_else(|| anyhow!("nothing to self-check yet — run this stage first"))?;
    let cur_text = serde_json::from_str::<Value>(&current.content).ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        .unwrap_or_else(|| current.content.clone());

    // busy signal + reset-on-error, same as run_stage
    let prior_status = stage.status.clone();
    db::set_stage_status(conn, stage_id, "in_progress").await?;

    let run = async {
        let prior = gather_prior_context(conn, &stage.song_id, stage.ordinal).await?;
        let system = build_system_prompt(&skill, &preset, &song);
        let checks = "SELF-TEST then REVISE. You wrote the output below. Audit it hard and rewrite it, fixing every issue you find:\n\
1. TITLE/HOOK: does the song's title (from the Concept) actually land as the chorus hook line? If not, work it in so the chorus sings the title. ONLY if the song clearly found a stronger, more specific hook, build the chorus around that instead and keep it consistent across every chorus.\n\
2. COHERENCE: every section must fit the Lyric Spec's beat sheet and the concept — no section that drifts off-theme, contradicts the story, or repeats instead of develops.\n\
3. CRAFT: cut clichés, weak \"to be\" verbs, abstract emotion-words, forced rhymes, and over-written \"poetic\" lines that no one would actually sing; keep it human and singable.\n\
4. Preserve the inline [chord] tags and the section labels exactly.\n\
Return ONLY the revised result as the single fenced ```json block your skill specifies — no commentary.";
        let mut user = format!(
            "Prior stages (context):\n\n{prior}\n\n---\nYOUR CURRENT {} OUTPUT TO SELF-TEST AND REVISE:\n\n{cur_text}\n\n{checks}",
            stage_label(&stage.r#type)
        );
        // frozen sections are FINAL even through a self-test/revise pass
        let prior_data = serde_json::from_str::<Value>(&current.content).ok().and_then(|v| v.get("data").cloned());
        if let Some(pd) = &prior_data {
            if let Some(block) = frozen_prompt_block(&stage.r#type, pd) {
                user.push_str("\n\n");
                user.push_str(&block);
            }
        }

        let text = call_claude(settings, &system, &user, &|_| {}, None).await?;
        let content = build_merged_content(&stage.r#type, &text, Some(&current.content))?;
        let artifact = db::save_artifact(conn, &song.id, Some(stage_id), kind_for_stage(&stage.r#type), &content).await?;
        Ok::<Artifact, anyhow::Error>(artifact)
    };

    match run.await {
        Ok(artifact) => Ok(artifact),
        Err(e) => {
            let _ = db::set_stage_status(conn, stage_id, &prior_status).await;
            Err(e)
        }
    }
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

    let text = call_claude(settings, &system, &user, &on_token, None).await?;
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
    let out = call_claude(settings, &skill.instructions, &user, &|_| {}, None).await?;
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
    let text = call_claude(settings, system, &user, &|_: String| {}, None).await?;
    let t = text.trim();
    // strip an accidental ```fence``` or wrapping quotes if the model added them
    let t = t.strip_prefix("```").map(|s| s.trim_start_matches(|c: char| c.is_alphanumeric()).trim()).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    Ok(t.trim().to_string())
}

/// Wall-clock limit for one Claude CLI call: 600s, overridable via the
/// `SONGSMITH_CLAUDE_TIMEOUT_SECS` env var.
fn claude_timeout() -> std::time::Duration {
    let secs = std::env::var("SONGSMITH_CLAUDE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

/// Generate text with Claude by driving the `claude` CLI headless with
/// stream-json. Streams text deltas via `on_token`, returns the final answer.
/// Hardened: `kill_on_drop`, concurrent stderr drain (read-after-wait deadlocks
/// past ~64KB), a wall-clock timeout, an optional `cancel` token, and the
/// `result` event's `is_error`/`subtype` treated as failure.
async fn call_claude<F>(settings: &Settings, system: &str, user: &str, on_token: &F, cancel: Option<&CancelToken>) -> Result<String>
where
    F: Fn(String) + Send,
{
    // Test hook: when SONGSMITH_MOCK_CLAUDE is set, return its value verbatim as
    // Claude's output instead of shelling out to the CLI. Lets the freeze/merge
    // logic be exercised end-to-end in `cargo test` with no Claude subscription.
    if let Ok(canned) = std::env::var("SONGSMITH_MOCK_CLAUDE") {
        on_token(canned.clone());
        return Ok(canned);
    }

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
        .stderr(std::process::Stdio::piped())
        // no zombie `claude` if this future is dropped (timeout / cancel / panic)
        .kill_on_drop(true);
    if !settings.claude_model.is_empty() {
        cmd.arg("--model").arg(&settings.claude_model);
    }

    let mut child = cmd.spawn().map_err(|e| anyhow!("could not start the claude CLI ({bin}): {e}. Install Claude Code and sign in."))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("claude stdout unavailable"))?;

    // Drain stderr CONCURRENTLY with the stdout read — reading it only after
    // `wait()` deadlocks once the CLI writes more than the pipe buffer (~64KB).
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(mut se) = stderr {
            let _ = se.read_to_string(&mut buf).await;
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let (mut result_text, mut assistant_text, mut streamed) = (String::new(), String::new(), String::new());
    let (mut result_is_error, mut result_subtype) = (false, None::<String>);

    let read_loop = async {
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
                    result_is_error = v["is_error"].as_bool().unwrap_or(false);
                    result_subtype = v["subtype"].as_str().map(String::from);
                }
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    // Read until EOF, a timeout, or a cancel — whichever comes first.
    let timeout = claude_timeout();
    let cancel_token = cancel.cloned().unwrap_or_default(); // a fresh token never fires
    enum End { Done(Result<()>), TimedOut, Cancelled }
    let end = tokio::select! {
        r = tokio::time::timeout(timeout, read_loop) => match r {
            Ok(inner) => End::Done(inner),
            Err(_) => End::TimedOut,
        },
        _ = cancel_token.cancelled() => End::Cancelled,
    };
    match end {
        End::Done(Ok(())) => {}
        End::Done(Err(e)) => {
            let _ = child.start_kill();
            let _ = child.wait().await; // reap
            return Err(anyhow!("error reading claude output: {e}"));
        }
        End::TimedOut => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(anyhow!(
                "claude timed out after {}s and the run was stopped (set SONGSMITH_CLAUDE_TIMEOUT_SECS to change the limit)",
                timeout.as_secs()
            ));
        }
        End::Cancelled => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(anyhow!("run cancelled"));
        }
    }

    let status = child.wait().await?;
    let stderr_buf = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let err = stderr_buf.trim();
        return Err(anyhow!("claude CLI failed: {}", if err.is_empty() { "is Claude Code signed in? run `claude` once to authenticate." } else { err }));
    }

    // An error-shaped `result` event (is_error / non-"success" subtype) must be
    // surfaced as the failure it is — never saved as an artifact.
    if result_is_error || result_subtype.as_deref().is_some_and(|s| s != "success") {
        let msg = if !result_text.trim().is_empty() { result_text.trim().to_string() }
            else if !assistant_text.trim().is_empty() { assistant_text.trim().to_string() }
            else { "no error detail".to_string() };
        return Err(anyhow!("claude reported an error ({}): {msg}", result_subtype.as_deref().unwrap_or("is_error")));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use libsql::Builder;
    use std::sync::Mutex;

    // `call_claude` reads SONGSMITH_MOCK_CLAUDE from the process env, which is
    // global — serialize the tests that set it so they don't race.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn merge_frozen_sections_protects_locked_only() {
        // prior: section A frozen, section B unlocked
        let prior = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Am","beats":4}], "frozen": true },
            { "label": "Chorus", "chords": [{"name":"C","beats":4}] }
        ]});
        // new: Claude rewrote BOTH and dropped the frozen one's original content
        let new = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":2}] },
            { "label": "Chorus", "chords": [{"name":"G","beats":4}] }
        ]});

        let merged = merge_frozen_sections("chords", &prior, &new);
        let secs = merged["sections"].as_array().unwrap();

        // frozen Verse 1 is byte-identical to the prior (+ frozen flag carried)
        let v = secs.iter().find(|s| s["label"] == "Verse 1").unwrap();
        assert_eq!(v["chords"][0]["name"], "Am");
        assert_eq!(v["frozen"], json!(true));
        // unlocked Chorus took Claude's new value
        let c = secs.iter().find(|s| s["label"] == "Chorus").unwrap();
        assert_eq!(c["chords"][0]["name"], "G");
    }

    #[test]
    fn merge_reinserts_dropped_frozen_section_at_original_index() {
        let prior = json!({ "sections": [
            { "label": "Intro", "chords": [{"name":"Am","beats":4}], "frozen": true },
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":4}] }
        ]});
        // Claude dropped the frozen Intro entirely
        let new = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"F","beats":4}] }
        ]});

        let merged = merge_frozen_sections("chords", &prior, &new);
        let secs = merged["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 2);
        // re-inserted at its original index 0, verbatim, flag carried
        assert_eq!(secs[0]["label"], "Intro");
        assert_eq!(secs[0]["chords"][0]["name"], "Am");
        assert_eq!(secs[0]["frozen"], json!(true));
    }

    #[test]
    fn merge_is_noop_when_nothing_frozen() {
        let prior = json!({ "sections": [ { "label": "A", "chords": [] } ] });
        let new = json!({ "sections": [ { "label": "A", "chords": [{"name":"C","beats":4}] } ] });
        let merged = merge_frozen_sections("chords", &prior, &new);
        assert_eq!(merged, new); // identical to today
    }

    #[test]
    fn merge_matches_label_case_and_space_insensitively() {
        let prior = json!({ "sections": [ { "label": "Pre-Chorus / Build 1", "chords": [{"name":"Am"}], "frozen": true } ] });
        let new = json!({ "sections": [ { "label": "pre-chorus / build  1", "chords": [{"name":"G"}] } ] });
        let merged = merge_frozen_sections("chords", &prior, &new);
        let secs = merged["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 1); // matched, not duplicated
        assert_eq!(secs[0]["chords"][0]["name"], "Am");
    }

    // libSQL `:memory:` gives each connection its OWN database, so return the
    // single connection migrated/seeded here and reuse it for the whole test.
    async fn mem_conn() -> (libsql::Database, libsql::Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        db::migrate(&conn).await.unwrap();
        db::seed_skills(&conn).await.unwrap();
        (db, conn)
    }

    /// run_stage-level proof: a FROZEN chords section is byte-identical before and
    /// after a regeneration, while an UNLOCKED section changes.
    #[tokio::test]
    async fn run_stage_keeps_frozen_section_byte_identical() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();

        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();

        // prior current artifact: Verse 1 frozen, Chorus unlocked
        let prior_data = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Am","beats":4},{"name":"F","beats":4}], "frozen": true },
            { "label": "Chorus", "chords": [{"name":"C","beats":4}] }
        ]});
        let prior_content = json!({ "kind": "chords", "text": chords_editor_text(&prior_data), "data": prior_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&chords_stage.id), "chords", &prior_content).await.unwrap();

        // capture the frozen section's exact prior JSON
        let prior_frozen = serde_json::from_str::<Value>(&prior_content).unwrap()
            ["data"]["sections"][0].clone();

        // Claude "regenerates" everything (ignores the lock entirely)
        let claude_out = "```json\n".to_string() + &json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":2},{"name":"Bb","beats":2}] },
            { "label": "Chorus", "chords": [{"name":"G","beats":4},{"name":"Em","beats":4}] }
        ]}).to_string() + "\n```";

        std::env::set_var("SONGSMITH_MOCK_CLAUDE", &claude_out);
        let outcome = run_stage(&conn, &settings, &chords_stage.id, None, |_| {}, None).await.unwrap();
        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");

        let new = serde_json::from_str::<Value>(&outcome.artifact.content).unwrap();
        let new_secs = new["data"]["sections"].as_array().unwrap();
        let new_verse = new_secs.iter().find(|s| s["label"] == "Verse 1").unwrap();
        let new_chorus = new_secs.iter().find(|s| s["label"] == "Chorus").unwrap();

        // HARD GUARANTEE: frozen section is byte-identical to before
        assert_eq!(*new_verse, prior_frozen, "frozen section must be untouched");
        assert_eq!(new_verse["frozen"], json!(true));
        // unlocked section changed to Claude's new output
        assert_eq!(new_chorus["chords"][0]["name"], "G");
        // text was rebuilt from merged data and reflects the frozen content
        let text = new["text"].as_str().unwrap();
        assert!(text.contains("Verse 1: Am F"), "rebuilt text keeps frozen chords, got: {text}");
        assert!(text.contains("Chorus: G Em"), "rebuilt text has new chorus, got: {text}");
    }

    /// When nothing is frozen, run_stage produces exactly the legacy
    /// `{kind,text:<raw>,data:<extracted>}` shape (behavior identical to today).
    #[tokio::test]
    async fn run_stage_no_frozen_is_unchanged() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();

        let claude_out = "```json\n".to_string() + &json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":4}] }
        ]}).to_string() + "\n```";

        std::env::set_var("SONGSMITH_MOCK_CLAUDE", &claude_out);
        let outcome = run_stage(&conn, &settings, &chords_stage.id, None, |_| {}, None).await.unwrap();
        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");

        let expected = json!({ "kind": "chords", "text": claude_out, "data": extract_json(&claude_out) }).to_string();
        assert_eq!(outcome.artifact.content, expected, "no-frozen path must match the legacy content exactly");
    }

    /// Shared fixture: a song whose chords stage has a current artifact with
    /// Verse 1 FROZEN (Am F) and Chorus unlocked (C). Returns (song, stage,
    /// prior artifact content, the frozen section's exact JSON).
    async fn frozen_chords_fixture(conn: &libsql::Connection) -> (Song, Stage, String, Value) {
        let preset = db::create_preset(conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(),
        }).await.unwrap();
        let song = db::create_song(conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap().clone();

        let prior_data = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Am","beats":4},{"name":"F","beats":4}], "frozen": true },
            { "label": "Chorus", "chords": [{"name":"C","beats":4}] }
        ]});
        let prior_content = json!({ "kind": "chords", "text": chords_editor_text(&prior_data), "data": prior_data }).to_string();
        db::save_artifact(conn, &song.id, Some(&chords_stage.id), "chords", &prior_content).await.unwrap();
        let prior_frozen = serde_json::from_str::<Value>(&prior_content).unwrap()["data"]["sections"][0].clone();
        (song, chords_stage, prior_content, prior_frozen)
    }

    /// (a) A Claude-driven MCP `save_artifact` with a frozen prior keeps the
    /// frozen section byte-identical (and its `frozen` flag), while unlocked
    /// sections take the incoming values.
    #[tokio::test]
    async fn mcp_save_artifact_respects_frozen() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let (song, stage, _prior, prior_frozen) = frozen_chords_fixture(&conn).await;

        // Claude tries to rewrite BOTH sections and drop the frozen flag
        let incoming = json!({ "kind": "chords", "text": "whatever", "data": { "sections": [
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":2}] },
            { "label": "Chorus", "chords": [{"name":"G","beats":4}] }
        ]}}).to_string();
        let saved = crate::tools::dispatch(&conn, &settings, "save_artifact", &json!({
            "song_id": song.id, "stage_id": stage.id, "kind": "chords", "content": incoming,
        })).await.unwrap();

        let content = serde_json::from_str::<Value>(saved["content"].as_str().unwrap()).unwrap();
        let secs = content["data"]["sections"].as_array().unwrap();
        let verse = secs.iter().find(|s| s["label"] == "Verse 1").unwrap();
        let chorus = secs.iter().find(|s| s["label"] == "Chorus").unwrap();
        assert_eq!(*verse, prior_frozen, "frozen section must survive an MCP save byte-identical");
        assert_eq!(verse["frozen"], json!(true));
        assert_eq!(chorus["chords"][0]["name"], "G", "unlocked section takes the incoming value");
        // text is re-rendered from the merged data
        let text = content["text"].as_str().unwrap();
        assert!(text.contains("Verse 1: Am F"), "rebuilt text keeps frozen chords, got: {text}");
        assert!(text.contains("Chorus: G"), "rebuilt text has the new chorus, got: {text}");
    }

    /// (b) Unparseable MCP content + a frozen prior is an ERROR — nothing is
    /// saved and the prior revision stays current.
    #[tokio::test]
    async fn mcp_save_artifact_unparseable_with_frozen_errors() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let (song, stage, prior_content, _f) = frozen_chords_fixture(&conn).await;

        let res = crate::tools::dispatch(&conn, &settings, "save_artifact", &json!({
            "song_id": song.id, "stage_id": stage.id, "kind": "chords", "content": "definitely {{ not json",
        })).await;
        assert!(res.is_err(), "unparseable content over a frozen prior must error");

        let cur = db::current_artifact(&conn, &stage.id).await.unwrap().unwrap();
        assert_eq!(cur.version, 1, "no new revision may be created");
        assert_eq!(cur.content, prior_content, "the frozen prior stays current");
    }

    /// (c) The direct (UI editor) path is the user's authority: it can unfreeze
    /// and rewrite a frozen section — that is how unlocking works.
    #[tokio::test]
    async fn ui_save_artifact_can_unfreeze_and_rewrite() {
        let (_db, conn) = mem_conn().await;
        let (song, stage, _prior, _f) = frozen_chords_fixture(&conn).await;

        // the editor saves with the lock removed and the verse rewritten
        let unlocked = json!({ "kind": "chords", "text": "Verse 1: Dm", "data": { "sections": [
            { "label": "Verse 1", "chords": [{"name":"Dm","beats":4}] }
        ]}}).to_string();
        db::save_artifact(&conn, &song.id, Some(&stage.id), "chords", &unlocked).await.unwrap();

        let cur = db::current_artifact(&conn, &stage.id).await.unwrap().unwrap();
        assert_eq!(cur.content, unlocked, "the user's direct save wins verbatim (unlock path)");
    }

    /// (d) `revert_artifact` over MCP cannot resurrect pre-freeze content over a
    /// currently-frozen section: the revert passes through the same guard.
    #[tokio::test]
    async fn mcp_revert_respects_frozen() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let stage = stages.iter().find(|s| s.r#type == "chords").unwrap();

        // v1: pre-freeze (Verse 1 = C, Chorus = F, nothing locked)
        let v1_data = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"C","beats":4}] },
            { "label": "Chorus", "chords": [{"name":"F","beats":4}] }
        ]});
        let v1_content = json!({ "kind": "chords", "text": chords_editor_text(&v1_data), "data": v1_data }).to_string();
        let v1 = db::save_artifact(&conn, &song.id, Some(&stage.id), "chords", &v1_content).await.unwrap();

        // v2: the user froze Verse 1 as Am
        let v2_data = json!({ "sections": [
            { "label": "Verse 1", "chords": [{"name":"Am","beats":4}], "frozen": true },
            { "label": "Chorus", "chords": [{"name":"G","beats":4}] }
        ]});
        let v2_content = json!({ "kind": "chords", "text": chords_editor_text(&v2_data), "data": v2_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&stage.id), "chords", &v2_content).await.unwrap();
        let frozen_verse = serde_json::from_str::<Value>(&v2_content).unwrap()["data"]["sections"][0].clone();

        // Claude reverts to v1 over MCP — the frozen Verse 1 must survive
        let reverted = crate::tools::dispatch(&conn, &settings, "revert_artifact", &json!({ "artifact_id": v1.id })).await.unwrap();
        let content = serde_json::from_str::<Value>(reverted["content"].as_str().unwrap()).unwrap();
        let secs = content["data"]["sections"].as_array().unwrap();
        let verse = secs.iter().find(|s| s["label"] == "Verse 1").unwrap();
        let chorus = secs.iter().find(|s| s["label"] == "Chorus").unwrap();
        assert_eq!(*verse, frozen_verse, "revert must not resurrect the pre-freeze verse");
        assert_eq!(verse["frozen"], json!(true));
        assert_eq!(chorus["chords"][0]["name"], "F", "unlocked section reverts to v1's value");
    }

    /// Parse-failure hole: frozen prior + model output with no parseable JSON →
    /// the run fails, the prior stays current, and the stage status is reset
    /// (not stranded "in_progress").
    #[tokio::test]
    async fn run_stage_parse_failure_with_frozen_errors_and_resets_status() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let (_song, stage, prior_content, _f) = frozen_chords_fixture(&conn).await;
        let prior_status = db::get_stage(&conn, &stage.id).await.unwrap().unwrap().status;

        std::env::set_var("SONGSMITH_MOCK_CLAUDE", "sorry, here are some vibes but no JSON");
        let res = run_stage(&conn, &settings, &stage.id, None, |_| {}, None).await;
        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");

        assert!(res.is_err(), "unparseable output over a frozen prior must fail the run");
        let cur = db::current_artifact(&conn, &stage.id).await.unwrap().unwrap();
        assert_eq!(cur.version, 1);
        assert_eq!(cur.content, prior_content, "prior artifact stays current");
        let after = db::get_stage(&conn, &stage.id).await.unwrap().unwrap();
        assert_eq!(after.status, prior_status, "stage status must be reset on error");
    }

    /// A pre-fired cancel token resolves immediately (no arm-before-fire race).
    #[tokio::test]
    async fn cancel_token_pre_fired_resolves_immediately() {
        let t = CancelToken::new();
        t.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(200), t.cancelled())
            .await
            .expect("a pre-cancelled token must resolve immediately");
    }
}
