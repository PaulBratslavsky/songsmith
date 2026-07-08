//! Per-section Freeze (regeneration-safe). Split out of `agent.rs` (audit
//! Tier-2 #11).
//!
//! A section can carry an optional `"frozen": true` flag in a stage artifact's
//! `data.sections[]` (or `data.beats[]` for lyric_spec). A frozen section is a
//! HARD guarantee: when the stage regenerates, the prior frozen section is
//! spliced back verbatim over whatever Claude produced — the lock is enforced
//! deterministically, not just via the prompt. When nothing is frozen the merge
//! is a no-op and behavior is identical to before.
//!
//! TRUST MODEL: UI editor saves are the *user's* authority — they call
//! `db::save_artifact` directly and may unlock/rewrite anything (that is how
//! unfreezing works). Claude-driven writes (the MCP `save_artifact` /
//! `revert_artifact` tools, and the agent loop via `build_merged_content`) must
//! NEVER violate frozen sections, so they come through the guarded wrappers here.

use crate::agent::kind_for_stage;
use crate::db;
use crate::engine::extract_json;
use crate::models::Artifact;
use crate::render::render_stage_text;
use anyhow::{anyhow, Result};
use libsql::Connection;
use serde_json::{json, Value};

/// The only stages with section-based artifacts that support freezing.
pub(crate) fn is_section_stage(stage_type: &str) -> bool {
    matches!(stage_type, "structure" | "chords" | "lyric_spec" | "lyrics")
}

/// `(array key, label key)` for a section-based stage. lyric_spec's per-section
/// unit is the beat sheet (`beats[]`, keyed by `section`); the rest use
/// `sections[]` keyed by `label`.
pub(crate) fn section_keys(stage_type: &str) -> (&'static str, &'static str) {
    match stage_type {
        "lyric_spec" => ("beats", "section"),
        _ => ("sections", "label"),
    }
}

/// Read a section's label (the chords/structure/lyrics editors fall back from
/// `label` to `type`; matching is case/space-insensitive on the normalized form).
pub(crate) fn section_label(stage_type: &str, sec: &Value) -> String {
    let (_, lbl_key) = section_keys(stage_type);
    sec.get(lbl_key)
        .and_then(|v| v.as_str())
        .or_else(|| sec.get("type").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string()
}

pub(crate) fn norm_label(s: &str) -> String {
    s.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Does this artifact `data` carry any frozen sections? The write-boundary guard
/// and the merge path share this one detection (same section-array keys).
pub(crate) fn has_frozen_sections(stage_type: &str, data: &Value) -> bool {
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

/// Render the prior frozen sections as text to inject into the generation prompt,
/// so the regenerated unlocked sections stay coherent with the locked ones.
pub(crate) fn frozen_prompt_block(stage_type: &str, prior_data: &Value) -> Option<String> {
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

/// Frozen section labels in a stage's `data` (for the skip report).
pub(crate) fn frozen_labels(stage_type: &str, data: &Value) -> Vec<String> {
    let (arr_key, _) = section_keys(stage_type);
    data.get(arr_key)
        .and_then(|v| v.as_array())
        .map(|secs| {
            secs.iter()
                .filter(|s| s.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false))
                .map(|s| section_label(stage_type, s))
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Build the merged, frozen-spliced artifact `content` JSON string from Claude's
/// raw `text` + the stage's prior current artifact. When nothing was frozen this
/// produces exactly the same `{kind,text,data}` as before (no-op). When the prior
/// has frozen sections and the model output has no parseable JSON, this is an
/// error — never save `{data: null}` over locked content.
pub(crate) fn build_merged_content(stage_type: &str, raw_text: &str, prior_content: Option<&str>) -> Result<String> {
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
                let prior_data = match db::current_artifact(conn, sid).await? {
                    Some(prior) => serde_json::from_str::<Value>(&prior.content)
                        .ok()
                        .and_then(|v| v.get("data").cloned()),
                    None => None,
                };
                let frozen = prior_data
                    .as_ref()
                    .map(|d| has_frozen_sections(&stage.r#type, d))
                    .unwrap_or(false);

                let incoming = match serde_json::from_str::<Value>(content) {
                    Ok(v) => v,
                    Err(_) if frozen => {
                        return Err(anyhow!(
                            "this stage has locked (frozen) sections and the incoming content is not valid JSON — refusing to save. Unlock the sections in the app to rewrite them."
                        ))
                    }
                    // non-JSON with nothing frozen: pass through unchanged (legacy behavior)
                    Err(_) => return db::save_artifact(conn, song_id, Some(sid), kind, content).await,
                };
                // accept the `{kind,text,data}` wrapper or bare data
                // (an object already carrying the stage's section array)
                let (arr_key, _) = section_keys(&stage.r#type);
                let incoming_data = incoming
                    .get("data")
                    .cloned()
                    .or_else(|| incoming.get(arr_key).is_some().then(|| incoming.clone()))
                    .unwrap_or(Value::Null);

                let final_data = match (&prior_data, frozen) {
                    (Some(pd), true) => merge_frozen_sections(&stage.r#type, pd, &incoming_data),
                    _ => incoming_data,
                };

                // NORMALIZE `text` from `data` whenever the data renders — not only on
                // frozen merges. Claude-originated saves (stage chat over MCP) were
                // storing commentary/changelogs as `text` while the real content sat in
                // `data`; downstream stages read `text` via gather_prior_context, so the
                // Generation Prompt received a changelog instead of the lyrics. The
                // write boundary now keeps text == render(data) for section stages.
                if let Some(text) = render_stage_text(&stage.r#type, &final_data) {
                    let guarded = json!({ "kind": kind, "text": text, "data": final_data }).to_string();
                    return db::save_artifact(conn, song_id, Some(sid), kind, &guarded).await;
                } else if frozen {
                    let text = incoming.get("text").and_then(|t| t.as_str()).unwrap_or_default();
                    let guarded = json!({ "kind": kind, "text": text, "data": final_data }).to_string();
                    return db::save_artifact(conn, song_id, Some(sid), kind, &guarded).await;
                }
                // unrenderable data with nothing frozen: pass through unchanged
                return db::save_artifact(conn, song_id, Some(sid), kind, content).await;
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
