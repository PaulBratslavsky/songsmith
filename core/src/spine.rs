//! Section-spine WRITERS (docs/SECTION-SPINE-SPEC.md, Phases 3+4).
//!
//! The spine (`section` table) owns section identity/order/label/type/bars/role.
//! This module is the write-side counterpart of the Phase-2 readers:
//!
//! - `build_run_content` — the stage-run pipeline (run_stage + self_check):
//!   attaches `section_id`s to the model's output, splices frozen sections
//!   (id-first), reconciles a STRUCTURE run into the spine (create / rename /
//!   position-match / delete-or-keep per D2), drops sections a NON-structure
//!   run invented (D3), re-renders the text and appends visible ⚠ warn lines.
//!   Since Phase 4 a structure run's artifact keeps ONLY `{keyNote,tempoNote}`
//!   — the spine IS the sections; its text renders the map from the spine.
//! - `sync_spine` — the user-authority REPLACE used by imports and the
//!   Composer export: the spine becomes exactly the given ordered section
//!   list (existing rows matched by id, then norm-label, keep their ids).
//! - `spine_snapshot` / `snapshot_of` — the light `[{section_id,label,position}]`
//!   block each writer embeds beside `data` (journal stays self-contained; see
//!   spec §Snapshots).
//! - `restore_snapshot_rows` / `revert_artifact` — snapshot-based restore:
//!   reverting a revision re-creates spine rows it references that no longer
//!   exist (same ids), so old content reattaches; rows that exist keep their
//!   current form (the snapshot only fills gaps).
//! - `union_artifact_sections` — the mid-session spine-birth union: when a
//!   structure save/run creates a song's FIRST rows, sections living only in
//!   other stages' artifacts are appended (first-seen order), exactly like the
//!   startup migration's union step.
//!
//! Songs with NO spine rows keep every legacy path byte-identical: stage runs
//! fall back to `freeze::build_merged_content` + `enforce_song_key_tempo`
//! exactly as before (only a structure run may BIRTH a spine — proposing
//! sections is its job).

use crate::agent::{enforce_song_key_tempo, kind_for_stage};
use crate::db;
use crate::engine::extract_json;
use crate::freeze::{
    build_merged_content, is_section_stage, merge_frozen_sections, norm_label, section_keys, section_label,
};
use crate::models::{Artifact, Section, Song};
use crate::render::render_stage_text;
use anyhow::Result;
use libsql::Connection;
use serde_json::{json, Value};
use std::collections::HashSet;

/// A section entry's bar count, tolerant of string-typed numbers; default 8.
fn entry_bars(sec: &Value) -> i64 {
    sec.get("bars")
        .and_then(|b| b.as_i64().or_else(|| b.as_str().and_then(|t| t.trim().parse().ok())))
        .filter(|b| *b >= 1)
        .unwrap_or(8)
}
fn entry_str(sec: &Value, key: &str) -> String {
    sec.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}
fn entry_frozen(sec: &Value) -> bool {
    sec.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false)
}
fn entry_id(sec: &Value) -> Option<String> {
    sec.get("section_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from)
}

/// The light spine snapshot embedded beside `data` on every spine-era write
/// (same shape the migration writes): `[{section_id, label, position}]`.
pub(crate) fn snapshot_of(rows: &[Section]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| json!({ "section_id": r.id, "label": r.label, "position": r.position }))
            .collect(),
    )
}
pub(crate) async fn spine_snapshot(conn: &Connection, song_id: &str) -> Result<Value> {
    Ok(snapshot_of(&db::list_sections(conn, song_id).await?))
}

/// Snapshot-based restore (spec §Snapshots): re-create the spine rows an
/// artifact's `spine_snapshot` references that no longer exist — with the SAME
/// ids, so the restored content's `section_id`s reattach — at their snapshot
/// positions (clamped). Rows that still exist keep their current form: the
/// snapshot only fills gaps. Artifacts without a snapshot (legacy revisions)
/// are a no-op; their readers keep the label-fallback path.
pub async fn restore_snapshot_rows(conn: &Connection, artifact: &Artifact) -> Result<()> {
    let Ok(v) = serde_json::from_str::<Value>(&artifact.content) else { return Ok(()) };
    let Some(snap) = v.get("spine_snapshot").and_then(|s| s.as_array()) else { return Ok(()) };
    let existing: HashSet<String> =
        db::list_sections(conn, &artifact.song_id).await?.into_iter().map(|r| r.id).collect();
    let mut missing: Vec<(i64, String, String)> = snap
        .iter()
        .filter_map(|e| {
            let id = e.get("section_id").and_then(|x| x.as_str()).filter(|s| !s.is_empty())?;
            if existing.contains(id) {
                return None;
            }
            let label = e.get("label").and_then(|l| l.as_str()).unwrap_or("Section");
            let pos = e.get("position").and_then(|p| p.as_i64()).unwrap_or(0);
            Some((pos, id.to_string(), label.to_string()))
        })
        .collect();
    missing.sort_by_key(|(pos, _, _)| *pos); // re-insert low positions first so clamping lands right
    for (pos, id, label) in missing {
        db::restore_section_row(conn, &id, &artifact.song_id, pos, &label).await?;
    }
    Ok(())
}

/// The DIRECT (user-authority) revert: re-create missing spine rows from the
/// revision's snapshot, then restore the content verbatim as a new revision.
/// The guarded (Claude/MCP) counterpart is `freeze::revert_artifact_guarded`,
/// which does the same snapshot restore and then passes the content through
/// the freeze/normalization write boundary.
pub async fn revert_artifact(conn: &Connection, artifact_id: &str) -> Result<Artifact> {
    let t = db::get_artifact(conn, artifact_id).await?.ok_or_else(|| anyhow::anyhow!("artifact not found"))?;
    restore_snapshot_rows(conn, &t).await?;
    db::save_artifact(conn, &t.song_id, t.stage_id.as_deref(), &t.kind, &t.content).await
}

/// Mid-session spine-birth union (spec §Migration step 2, applied in-session):
/// append a row (defaults: type ""/bars 8/role "") for every section that
/// exists only in a stage artifact — the "Bridge only in lyrics" case — in
/// first-seen order, exactly like the startup migration's union. Idempotent:
/// known labels are skipped. Returns the resulting spine.
pub async fn union_artifact_sections(conn: &Connection, song_id: &str) -> Result<Vec<Section>> {
    let mut known: HashSet<String> = db::list_sections(conn, song_id)
        .await?
        .iter()
        .map(|r| norm_label(&r.label))
        .filter(|l| !l.is_empty())
        .collect();
    for label in db::artifact_section_labels(conn, song_id).await? {
        let norm = norm_label(&label);
        if norm.is_empty() || known.contains(&norm) {
            continue;
        }
        known.insert(norm);
        db::create_section(conn, song_id, &label, "", 8, "", None).await?;
    }
    db::list_sections(conn, song_id).await
}

/// Spine section ids (and normalized labels, for legacy entries without ids)
/// that still CARRY CONTENT in a non-structure stage artifact — chords with
/// chords, lyrics with words, beats with text. Drives the D2 keep-vs-delete
/// decision when a structure run drops a section.
async fn content_bearing(conn: &Connection, song_id: &str) -> Result<(HashSet<String>, HashSet<String>)> {
    let mut ids = HashSet::new();
    let mut labels = HashSet::new();
    let stages = db::list_stages(conn, song_id).await?;
    for st in stages.iter().filter(|s| matches!(s.r#type.as_str(), "chords" | "lyric_spec" | "lyrics")) {
        let Some(a) = db::current_artifact(conn, &st.id).await? else { continue };
        let Some(data) = serde_json::from_str::<Value>(&a.content).ok().and_then(|v| v.get("data").cloned()) else {
            continue;
        };
        let (arr_key, _) = section_keys(&st.r#type);
        let Some(arr) = data.get(arr_key).and_then(|v| v.as_array()) else { continue };
        for e in arr {
            let has = match st.r#type.as_str() {
                "chords" => e.get("chords").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()),
                "lyrics" => e
                    .get("lines")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| a.iter().any(|l| l.as_str().is_some_and(|s| !s.trim().is_empty()))),
                _ => e.get("beat").and_then(|v| v.as_str()).is_some_and(|s| !s.trim().is_empty()),
            };
            if !has {
                continue;
            }
            if let Some(id) = entry_id(e) {
                ids.insert(id);
            }
            let l = norm_label(&section_label(&st.r#type, e));
            if !l.is_empty() {
                labels.insert(l);
            }
        }
    }
    Ok((ids, labels))
}

/// Map a NON-structure run's output sections/beats onto the spine: entries
/// match a spine row by `section_id` (frozen splices carry one), else by
/// normalized label — matched entries get the row's id attached; unmatched
/// entries are DROPPED with a warn (D3: non-structure runs never create
/// sections). Consume-once, so duplicate output labels can't share a row.
/// Shared with the guarded MCP save (`freeze::save_artifact_guarded`) so
/// chat-driven saves stay id-coherent too (Phase 4).
pub(crate) fn attach_ids_non_structure(stage_type: &str, data: &mut Value, spine: &[Section], warns: &mut Vec<String>) {
    let (arr_key, _) = section_keys(stage_type);
    let Some(arr) = data.get_mut(arr_key).and_then(|v| v.as_array_mut()) else { return };
    let mut consumed = vec![false; spine.len()];
    let mut kept: Vec<Value> = Vec::with_capacity(arr.len());
    for mut e in arr.drain(..) {
        let by_id = entry_id(&e).and_then(|id| spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && r.id == id).map(|(ri, _)| ri));
        let ri = by_id.or_else(|| {
            let l = norm_label(&section_label(stage_type, &e));
            spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && norm_label(&r.label) == l).map(|(ri, _)| ri)
        });
        match ri {
            Some(ri) => {
                consumed[ri] = true;
                if let Some(obj) = e.as_object_mut() {
                    obj.insert("section_id".into(), json!(spine[ri].id));
                }
                kept.push(e);
            }
            None => {
                let label = section_label(stage_type, &e);
                warns.push(format!(
                    "⚠ Dropped \"{label}\" from the model output — it is not one of this song's sections (sections are managed in the Structure stage)."
                ));
            }
        }
    }
    *arr = kept;
}

/// Reconcile a STRUCTURE run's (frozen-merged) sections into the spine — the
/// spec's crux. Matching per output entry, consume-once:
///   1. `section_id` (frozen splices carry one),
///   2. exact normalized label (rename-safe both ways),
///   3. same position + same type (a rename the model did in place),
///   4. else CREATE a row at the output position.
/// Matched non-frozen entries update the row's label/type/bars/role; FROZEN
/// entries keep their row verbatim (form-level lock). Spine rows missing from
/// the output are DELETED only when no stage artifact carries content for
/// them; else KEPT with a warn (D2 — deletion of content is a user action).
/// Every non-frozen entry ends up carrying its row's `section_id`.
async fn reconcile_structure_run(conn: &Connection, song_id: &str, data: &mut Value, warns: &mut Vec<String>) -> Result<()> {
    let spine = db::list_sections(conn, song_id).await?;
    let entries: Vec<Value> = match data.get("sections").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return Ok(()), // no section-shaped output — never wipe the spine over garbage
    };

    let mut consumed = vec![false; spine.len()];
    let mut row_of: Vec<Option<usize>> = vec![None; entries.len()];
    // pass 1: by id
    for (i, e) in entries.iter().enumerate() {
        if let Some(id) = entry_id(e) {
            if let Some(ri) = spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && r.id == id).map(|(ri, _)| ri) {
                consumed[ri] = true;
                row_of[i] = Some(ri);
            }
        }
    }
    // pass 2: exact norm-label
    for (i, e) in entries.iter().enumerate() {
        if row_of[i].is_some() {
            continue;
        }
        let l = norm_label(&section_label("structure", e));
        if let Some(ri) = spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && norm_label(&r.label) == l).map(|(ri, _)| ri) {
            consumed[ri] = true;
            row_of[i] = Some(ri);
        }
    }
    // pass 3: same position + same NON-EMPTY type (an in-place rename — two
    // empty types are no evidence of identity, so they never match here)
    for (i, e) in entries.iter().enumerate() {
        if row_of[i].is_some() {
            continue;
        }
        let ty = norm_label(&entry_str(e, "type"));
        if ty.is_empty() {
            continue;
        }
        if let Some(ri) = spine
            .iter()
            .enumerate()
            .find(|(ri, r)| !consumed[*ri] && r.position == i as i64 && norm_label(&r.r#type) == ty)
            .map(|(ri, _)| ri)
        {
            consumed[ri] = true;
            row_of[i] = Some(ri);
        }
    }

    // update matched rows / create new ones (in output order → append + reorder)
    let mut updated: Vec<Value> = Vec::with_capacity(entries.len());
    let mut ordered_ids: Vec<String> = Vec::with_capacity(entries.len());
    for (i, mut e) in entries.into_iter().enumerate() {
        let frozen = entry_frozen(&e);
        let label = section_label("structure", &e);
        let (ty, bars, role) = (entry_str(&e, "type"), entry_bars(&e), entry_str(&e, "role"));
        let id = match row_of[i] {
            Some(ri) => {
                let row = &spine[ri];
                if !frozen && (row.label != label || row.r#type != ty || row.bars != bars || row.role != role) {
                    db::update_section(conn, &row.id, &label, &ty, bars, &role).await?;
                }
                row.id.clone()
            }
            None => db::create_section(conn, song_id, &label, &ty, bars, &role, None).await?.id,
        };
        // frozen entries stay byte-verbatim (their prior form, id included when
        // the prior had one); everything else carries its spine id
        if !frozen {
            if let Some(obj) = e.as_object_mut() {
                obj.insert("section_id".into(), json!(id));
            }
        }
        ordered_ids.push(id);
        updated.push(e);
    }

    // rows the model dropped: delete when content-free, keep + warn otherwise (D2)
    let mut kept: Vec<(i64, String)> = Vec::new(); // (original position, id)
    let mut need_content = None; // computed lazily — most runs drop nothing
    for (ri, row) in spine.iter().enumerate() {
        if consumed[ri] {
            continue;
        }
        if need_content.is_none() {
            need_content = Some(content_bearing(conn, song_id).await?);
        }
        let (cids, clabels) = need_content.as_ref().unwrap();
        if cids.contains(&row.id) || clabels.contains(&norm_label(&row.label)) {
            warns.push(format!(
                "⚠ The model dropped \"{}\" — kept, because Chords/Lyrics still carry content for it (deleting a section with content is your call, in the Structure editor).",
                row.label
            ));
            kept.push((row.position, row.id.clone()));
        } else {
            db::delete_section(conn, &row.id).await?;
        }
    }
    // final order: the output order, kept rows re-inserted near their old spot
    for (pos, id) in kept {
        let at = (pos.max(0) as usize).min(ordered_ids.len());
        ordered_ids.insert(at, id);
    }
    if !ordered_ids.is_empty() {
        db::reorder_sections(conn, song_id, &ordered_ids).await?;
    }
    data["sections"] = Value::Array(updated);
    Ok(())
}

/// Build a stage run's artifact content (run_stage + self_check share it) —
/// the Phase-3 write pipeline. Songs with no spine rows (and non-structure
/// stages on them) take the LEGACY path byte-identically: frozen merge via
/// `build_merged_content`, then `enforce_song_key_tempo`. With a spine (or on
/// any structure run, which may birth one):
///   1. non-structure: map output labels → section ids, drop invented ones (D3);
///   2. splice frozen sections back verbatim (id-first — rename-proof);
///   3. structure: reconcile the merged sections into the spine (create /
///      rename / delete-or-keep per D2; a spine-birth unions in artifact-only
///      sections), then keep ONLY `{keyNote,tempoNote}` in the artifact —
///      the spine owns the sections, the SONG owns key/tempo (Phase 4);
///   4. render `text` from the final data (structure: section map from the
///      SPINE) + append the ⚠ warn lines;
///   5. embed the spine snapshot beside `data`.
pub(crate) async fn build_run_content(
    conn: &Connection,
    song: &Song,
    stage_type: &str,
    raw_text: &str,
    prior_content: Option<&str>,
) -> Result<String> {
    let legacy = |raw: &str| -> Result<String> {
        let c = build_merged_content(stage_type, raw, prior_content)?;
        Ok(enforce_song_key_tempo(song, stage_type, &c))
    };
    if !is_section_stage(stage_type) {
        return legacy(raw_text);
    }
    let spine = db::list_sections(conn, &song.id).await?;
    if spine.is_empty() && stage_type != "structure" {
        return legacy(raw_text); // legacy song — Phase-2 readers keep working from labels
    }
    let Some(mut nd) = extract_json(raw_text) else {
        return legacy(raw_text); // handles the frozen-prior error identically
    };
    let (arr_key, _) = section_keys(stage_type);
    if !nd.get(arr_key).and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()) {
        return legacy(raw_text); // not section-shaped output — never touch the spine
    }

    let mut warns: Vec<String> = Vec::new();
    if stage_type != "structure" {
        attach_ids_non_structure(stage_type, &mut nd, &spine, &mut warns);
    }

    let prior_data = prior_content
        .and_then(|pc| serde_json::from_str::<Value>(pc).ok())
        .and_then(|v| v.get("data").cloned());
    let mut merged = match &prior_data {
        Some(pd) => merge_frozen_sections(stage_type, pd, &nd),
        None => nd,
    };

    if stage_type == "structure" {
        let birthing = spine.is_empty();
        reconcile_structure_run(conn, &song.id, &mut merged, &mut warns).await?;
        if birthing {
            // this run just BIRTHED the spine for a legacy/new song — union in
            // sections that exist only in other stages' artifacts, exactly
            // like the startup migration (spec §Migration step 2)
            union_artifact_sections(conn, &song.id).await?;
        }
        // Phase 4: the spine IS the sections (and the SONG owns key/tempo) —
        // the structure artifact keeps only the prose notes
        merged = json!({
            "keyNote": merged.get("keyNote").and_then(|v| v.as_str()).unwrap_or(""),
            "tempoNote": merged.get("tempoNote").and_then(|v| v.as_str()).unwrap_or(""),
        });
    }

    let rows = db::list_sections(conn, &song.id).await?;
    let mut text = if stage_type == "structure" {
        crate::render::structure_spine_text(&merged, &rows)
    } else {
        render_stage_text(stage_type, &merged).unwrap_or_else(|| raw_text.to_string())
    };
    for w in &warns {
        text.push_str("\n\n");
        text.push_str(w);
    }
    Ok(json!({ "kind": kind_for_stage(stage_type), "text": text, "data": merged, "spine_snapshot": snapshot_of(&rows) }).to_string())
}

/// USER-AUTHORITY spine replace (paste-lyrics import, Composer export): the
/// spine becomes exactly `entries` (ordered structure-shaped sections —
/// label/type/bars/role, optional `section_id`/`frozen`). Existing rows are
/// matched by `section_id` first, then normalized label (consume-once), and
/// keep their ids; unmatched entries create rows; rows matched by nothing are
/// DELETED (imports/exports replace the section list — that is the user's
/// deliberate action). FROZEN entries keep their row verbatim and are never
/// mutated (byte-identity); everything else gets its `section_id` attached.
/// Returns the row ids aligned 1:1 with `entries`.
pub(crate) async fn sync_spine(conn: &Connection, song_id: &str, entries: &mut [Value]) -> Result<Vec<String>> {
    let spine = db::list_sections(conn, song_id).await?;
    let mut consumed = vec![false; spine.len()];
    let mut row_of: Vec<Option<usize>> = vec![None; entries.len()];
    for (i, e) in entries.iter().enumerate() {
        if let Some(id) = entry_id(e) {
            if let Some(ri) = spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && r.id == id).map(|(ri, _)| ri) {
                consumed[ri] = true;
                row_of[i] = Some(ri);
            }
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if row_of[i].is_some() {
            continue;
        }
        let l = norm_label(&section_label("structure", e));
        if let Some(ri) = spine.iter().enumerate().find(|(ri, r)| !consumed[*ri] && norm_label(&r.label) == l).map(|(ri, _)| ri) {
            consumed[ri] = true;
            row_of[i] = Some(ri);
        }
    }

    let mut ids: Vec<String> = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter_mut().enumerate() {
        let frozen = entry_frozen(e);
        let label = section_label("structure", e);
        let (ty, bars, role) = (entry_str(e, "type"), entry_bars(e), entry_str(e, "role"));
        let id = match row_of[i] {
            Some(ri) => {
                let row = &spine[ri];
                if !frozen && (row.label != label || row.r#type != ty || row.bars != bars || row.role != role) {
                    db::update_section(conn, &row.id, &label, &ty, bars, &role).await?;
                }
                row.id.clone()
            }
            None => db::create_section(conn, song_id, &label, &ty, bars, &role, None).await?.id,
        };
        if !frozen {
            if let Some(obj) = e.as_object_mut() {
                obj.insert("section_id".into(), json!(id));
            }
        }
        ids.push(id);
    }
    for (ri, row) in spine.iter().enumerate() {
        if !consumed[ri] {
            db::delete_section(conn, &row.id).await?;
        }
    }
    if !ids.is_empty() {
        db::reorder_sections(conn, song_id, &ids).await?;
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::StyleInput;
    use libsql::Builder;

    // libSQL `:memory:` gives each connection its OWN database — reuse the one
    // connection for the whole test (same pattern as agent.rs tests).
    async fn mem_conn() -> (libsql::Database, libsql::Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        db::migrate(&conn).await.unwrap();
        db::seed_skills(&conn).await.unwrap();
        (db, conn)
    }

    async fn make_song(conn: &Connection) -> Song {
        let preset = db::create_preset(conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        db::create_song(conn, &preset.id, "Spine Song").await.unwrap()
    }

    fn fenced(v: &Value) -> String {
        format!("```json\n{v}\n```")
    }

    async fn stage_id(conn: &Connection, song_id: &str, t: &str) -> String {
        db::list_stages(conn, song_id).await.unwrap().iter().find(|s| s.r#type == t).unwrap().id.clone()
    }

    /// STRUCTURE reconciliation (the spec's crux): exact-label match updates
    /// the row (bars/role), same-position+same-type keeps identity across a
    /// rename, a new output section CREATES a row at its output position, and
    /// a dropped CONTENT-FREE row is deleted. Phase 4: the artifact keeps only
    /// {keyNote,tempoNote} — the SPINE is the sections — and the text renders
    /// the section map from the spine; the snapshot lands beside `data`.
    #[tokio::test]
    async fn structure_run_reconciles_rename_position_create_delete() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let a = db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "open", None).await.unwrap();
        let b = db::create_section(&conn, &song.id, "Chorus", "chorus", 8, "lift", None).await.unwrap();
        let c = db::create_section(&conn, &song.id, "Outro", "", 4, "", None).await.unwrap();

        let raw = fenced(&json!({ "keyNote": "dark and low", "tempoNote": "", "sections": [
            { "type": "verse", "label": "Verse 1", "bars": 12, "role": "opens the story" },
            { "type": "chorus", "label": "Huge Chorus", "bars": 8, "role": "lift" },
            { "type": "", "label": "Bridge", "bars": 4, "role": "the turn" }
        ]}));
        let content = build_run_content(&conn, &song, "structure", &raw, None).await.unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Huge Chorus", "Bridge"]);
        assert_eq!(rows[0].id, a.id, "exact-label match keeps the row id");
        assert_eq!(rows[0].bars, 12, "bars update from the output");
        assert_eq!(rows[0].role, "opens the story");
        assert_eq!(rows[1].id, b.id, "same-position+same-type match keeps identity across the rename");
        assert!(rows.iter().all(|r| r.id != c.id), "dropped content-free row is deleted");

        // Phase 4: the artifact data is notes-only — the spine IS the sections
        assert_eq!(v["data"], json!({ "keyNote": "dark and low", "tempoNote": "" }), "structure data keeps only the prose notes");
        let snap = v["spine_snapshot"].as_array().unwrap();
        assert_eq!(snap.len(), 3, "snapshot embedded beside data");
        assert_eq!(snap[1]["section_id"], json!(b.id));
        assert_eq!(snap[1]["label"], json!("Huge Chorus"));
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("**KEY NOTE:** dark and low"), "notes render, got: {text}");
        assert!(text.contains("2. **Huge Chorus** (8 bars) — lift"), "text renders the section map FROM THE SPINE, got: {text}");
        assert!(text.contains("3. **Bridge** (4 bars) — the turn"), "got: {text}");
        assert!(!text.contains('⚠'), "clean reconciliation carries no warn lines");
    }

    /// D2: a section the model dropped is KEPT (with a visible ⚠ line in the
    /// artifact text) when another stage still carries content for it —
    /// deletion of a content-bearing section is a user action only.
    #[tokio::test]
    async fn structure_run_keeps_dropped_section_with_content_and_warns() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let a = db::create_section(&conn, &song.id, "Verse 1", "", 8, "", None).await.unwrap();
        let outro = db::create_section(&conn, &song.id, "Outro", "", 4, "", None).await.unwrap();

        // the Outro still has chords content, keyed by its section_id
        let ch_stage = stage_id(&conn, &song.id, "chords").await;
        let ch = json!({ "kind": "chords", "text": "Outro: Am", "data": { "sections": [
            { "section_id": outro.id, "label": "Outro", "chords": [{"name": "Am", "beats": 4}] }
        ]}}).to_string();
        db::save_artifact(&conn, &song.id, Some(&ch_stage), "chords", &ch).await.unwrap();

        let raw = fenced(&json!({ "sections": [ { "type": "", "label": "Verse 1", "bars": 8, "role": "" } ] }));
        let content = build_run_content(&conn, &song, "structure", &raw, None).await.unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Outro"], "content-bearing row survives the model dropping it");
        assert_eq!(rows[0].id, a.id);
        assert_eq!(rows[1].id, outro.id);
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("⚠") && text.contains("Outro") && text.contains("kept"), "visible warn line, got: {text}");
        // Phase 4: the artifact carries no sections — the kept row shows in the
        // spine-rendered text and the snapshot
        assert!(v["data"].get("sections").is_none(), "structure data is notes-only");
        assert!(text.contains("2. **Outro** (4 bars)"), "kept row renders from the spine, got: {text}");
        assert_eq!(v["spine_snapshot"].as_array().unwrap().len(), 2);
    }

    /// Frozen STRUCTURE sections are a form-level lock — proved against a
    /// LEGACY-shaped prior (sections still in the artifact data, the only
    /// place a structure lock can live): the spine row keeps label/bars/role
    /// untouched whatever the model proposed, and the new save comes out in
    /// the Phase-4 notes-only shape.
    #[tokio::test]
    async fn structure_run_frozen_section_keeps_row_and_entry_verbatim() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let intro = db::create_section(&conn, &song.id, "Intro", "intro", 4, "set the scene", None).await.unwrap();

        let st_stage = stage_id(&conn, &song.id, "structure").await;
        // legacy-shaped prior: sections (with the lock) still in the data
        let prior_data = json!({ "keyNote": "", "tempoNote": "", "sections": [
            { "section_id": intro.id, "type": "intro", "label": "Intro", "bars": 4, "role": "set the scene", "frozen": true }
        ]});
        let prior = json!({ "kind": "structure", "text": "…", "data": prior_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&st_stage), "structure", &prior).await.unwrap();

        // the model rewrites the locked section's form entirely
        let raw = fenced(&json!({ "sections": [ { "type": "intro", "label": "Intro", "bars": 16, "role": "massive wall of sound" } ] }));
        let content = build_run_content(&conn, &song, "structure", &raw, Some(&prior)).await.unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].id.as_str(), rows[0].bars, rows[0].role.as_str()), (intro.id.as_str(), 4, "set the scene"), "spine row kept verbatim");
        // Phase-4 shape: notes-only data, spine-rendered text shows the locked form
        assert!(v["data"].get("sections").is_none(), "new saves are notes-only");
        assert!(v["text"].as_str().unwrap().contains("1. **Intro** (4 bars) — set the scene"), "text renders the KEPT form from the spine");
    }

    /// D3: a NON-structure run maps output labels → section ids and DROPS a
    /// section the model invented, with a visible ⚠ line — other stages never
    /// create/rename spine sections.
    #[tokio::test]
    async fn chords_run_attaches_ids_and_drops_invented_sections() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let a = db::create_section(&conn, &song.id, "Verse 1", "", 8, "", None).await.unwrap();
        let b = db::create_section(&conn, &song.id, "Chorus", "", 8, "", None).await.unwrap();

        let raw = fenced(&json!({ "sections": [
            { "label": "verse  1", "chords": [{"name": "Am", "beats": 4}] },
            { "label": "Chorus", "chords": [{"name": "F", "beats": 4}] },
            { "label": "Outro", "chords": [{"name": "C", "beats": 4}] }
        ]}));
        let content = build_run_content(&conn, &song, "chords", &raw, None).await.unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();

        let secs = v["data"]["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 2, "the invented Outro is dropped, never created (D3)");
        assert_eq!(secs[0]["section_id"], json!(a.id), "case/space-insensitive label match attaches the id");
        assert_eq!(secs[1]["section_id"], json!(b.id));
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("⚠") && text.contains("Outro"), "dropped section is called out, got: {text}");
        assert!(text.contains("Verse 1: Am") || text.contains("verse  1: Am"), "text re-rendered from data, got: {text}");
        // the spine itself is untouched by a chords run
        assert_eq!(db::list_sections(&conn, &song.id).await.unwrap().len(), 2);
    }

    /// THE MARQUEE TEST (spec §Test plan): rename a section on the SPINE, then
    /// regenerate the stage — its frozen content still merges onto the same
    /// section (matched by section_id, not label), with no duplicate. The
    /// label-match bug class dies here.
    #[tokio::test]
    async fn freeze_by_id_survives_spine_rename() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let row = db::create_section(&conn, &song.id, "Verse 1", "", 8, "", None).await.unwrap();

        let ch_stage = stage_id(&conn, &song.id, "chords").await;
        let prior_data = json!({ "sections": [
            { "section_id": row.id, "label": "Verse 1", "chords": [{"name": "Am", "beats": 4}, {"name": "F", "beats": 4}], "frozen": true }
        ]});
        let prior = json!({ "kind": "chords", "text": "Verse 1: Am F", "data": prior_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&ch_stage), "chords", &prior).await.unwrap();
        let frozen_entry = prior_data["sections"][0].clone();

        // the user renames the section on the spine…
        db::update_section(&conn, &row.id, "Verso Uno", "", 8, "").await.unwrap();

        // …and the model regenerates using the canonical (new) label
        let raw = fenced(&json!({ "sections": [ { "label": "Verso Uno", "chords": [{"name": "Dm", "beats": 4}] } ] }));
        let content = build_run_content(&conn, &song, "chords", &raw, Some(&prior)).await.unwrap();
        let v: Value = serde_json::from_str(&content).unwrap();

        let secs = v["data"]["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 1, "id-matched frozen merge must not duplicate the renamed section");
        assert_eq!(secs[0], frozen_entry, "frozen content survives the rename byte-identical");
    }

    /// Imports are user-authority spine writers: pasting lyrics REPLACES the
    /// spine to the pasted labels/order (a row matched by norm-label keeps its
    /// id + bars/role) and every saved artifact entry carries its section_id.
    #[tokio::test]
    async fn import_lyrics_creates_spine_rows_and_attaches_ids() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let verse = db::create_section(&conn, &song.id, "Verse 1", "verse", 16, "story", None).await.unwrap();
        let outro = db::create_section(&conn, &song.id, "Outro", "", 4, "", None).await.unwrap();
        let settings = db::get_settings(&conn).await.unwrap();

        let text = "[Verse 1]\n[D#m]New words all the way down\n\n[Chorus]\nSing it loud";
        crate::agent::import_lyrics(&conn, &settings, &song.id, text).await.unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus"], "spine replaced to the pasted labels/order");
        assert_eq!(rows[0].id, verse.id, "matched row keeps its id");
        assert_eq!(rows[0].bars, 16, "matched row keeps its bars");
        assert_eq!(rows[0].role, "story");
        assert!(rows.iter().all(|r| r.id != outro.id), "row missing from the paste is deleted (user authority)");

        // every content artifact the import saved keys its entries by section_id
        for t in ["lyrics", "chords"] {
            let sid = stage_id(&conn, &song.id, t).await;
            let art = db::current_artifact(&conn, &sid).await.unwrap().unwrap();
            let v: Value = serde_json::from_str(&art.content).unwrap();
            let secs = v["data"]["sections"].as_array().unwrap();
            assert_eq!(secs[0]["section_id"], json!(rows[0].id), "{t} entry keyed to the spine");
            assert_eq!(secs[1]["section_id"], json!(rows[1].id), "{t} entry keyed to the spine");
            assert!(v["spine_snapshot"].is_array(), "{t} save embeds the snapshot");
        }
        // the structure artifact is notes-only (Phase 4) — the spine holds the map
        let sid = stage_id(&conn, &song.id, "structure").await;
        let v: Value = serde_json::from_str(&db::current_artifact(&conn, &sid).await.unwrap().unwrap().content).unwrap();
        assert!(v["data"].get("sections").is_none(), "structure data carries no sections");
        assert_eq!(v["spine_snapshot"].as_array().unwrap().len(), 2);
        assert!(v["text"].as_str().unwrap().contains("1. **Verse 1** (16 bars) — story"), "structure text renders the spine map, got: {}", v["text"]);
    }

    /// Composer round-trip (D4): the export maps back through section_id — a
    /// RENAMED composition section still updates its original row (id match),
    /// its BARS take the exported value, and a genuinely new section gets a
    /// row; both saved artifacts carry section_ids.
    #[tokio::test]
    async fn export_round_trip_updates_spine_bars_by_id_and_creates_rows() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let verse = db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "story", None).await.unwrap();

        let resolved = json!([
            { "label": "Verse One (renamed)", "bars": 12, "section_id": verse.id, "chords": [{"name": "Am", "beats": 4}] },
            { "label": "Bridge", "bars": 4, "chords": [{"name": "F", "beats": 4}] }
        ]).to_string();
        crate::agent::export_composition_to_song(&conn, &song.id, &resolved).await.unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse One (renamed)", "Bridge"]);
        assert_eq!(rows[0].id, verse.id, "id match survives the rename");
        assert_eq!(rows[0].bars, 12, "matched row's bars update from the export (D4)");
        assert_eq!(rows[0].role, "story", "role survives");
        assert_eq!(rows[1].bars, 4, "new section's row takes the exported bars");

        let sid = stage_id(&conn, &song.id, "chords").await;
        let v: Value = serde_json::from_str(&db::current_artifact(&conn, &sid).await.unwrap().unwrap().content).unwrap();
        let secs = v["data"]["sections"].as_array().unwrap();
        assert_eq!(secs[0]["section_id"], json!(rows[0].id), "chords entries keyed to the spine");
        assert_eq!(secs[1]["section_id"], json!(rows[1].id), "chords entries keyed to the spine");
        // structure artifact: notes-only, spine-rendered text (Phase 4)
        let sid = stage_id(&conn, &song.id, "structure").await;
        let v: Value = serde_json::from_str(&db::current_artifact(&conn, &sid).await.unwrap().unwrap().content).unwrap();
        assert!(v["data"].get("sections").is_none(), "structure data carries no sections");
        assert!(v["text"].as_str().unwrap().contains("1. **Verse One (renamed)** (12 bars) — story"), "got: {}", v["text"]);
    }

    /// Phase 4 §Snapshots: restoring a revision that references a DELETED
    /// section re-creates the row from the artifact's spine_snapshot (same id,
    /// same label, snapshot position) so the content reattaches — while rows
    /// that still exist keep their current form untouched. Both revert paths
    /// (the direct/user one and the guarded MCP one) restore the rows.
    #[tokio::test]
    async fn revert_recreates_deleted_sections_from_snapshot() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        let verse = db::create_section(&conn, &song.id, "Verse 1", "", 8, "", None).await.unwrap();
        let outro = db::create_section(&conn, &song.id, "Outro", "", 4, "", None).await.unwrap();

        // a chords revision keyed to both rows, snapshot embedded (a core write)
        let ch_stage = stage_id(&conn, &song.id, "chords").await;
        let data = json!({ "sections": [
            { "section_id": verse.id, "label": "Verse 1", "chords": [{"name": "Am", "beats": 4}] },
            { "section_id": outro.id, "label": "Outro", "chords": [{"name": "F", "beats": 4}] }
        ]});
        let snapshot = spine_snapshot(&conn, &song.id).await.unwrap();
        let content = json!({ "kind": "chords", "text": "Verse 1: Am\nOutro: F", "data": data, "spine_snapshot": snapshot }).to_string();
        let old = db::save_artifact(&conn, &song.id, Some(&ch_stage), "chords", &content).await.unwrap();

        // the user deletes the Outro section, then renames Verse 1 (rows drift)
        db::delete_section(&conn, &outro.id).await.unwrap();
        db::update_section(&conn, &verse.id, "Verso Uno", "", 12, "").await.unwrap();

        // DIRECT revert: the Outro row comes back (same id/label/position);
        // the surviving row keeps its CURRENT form (snapshot only fills gaps)
        let restored = revert_artifact(&conn, &old.id).await.unwrap();
        assert_eq!(restored.content, content, "direct revert restores the content verbatim");
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].id.as_str(), rows[0].label.as_str(), rows[0].bars), (verse.id.as_str(), "Verso Uno", 12), "existing row keeps its current form");
        assert_eq!((rows[1].id.as_str(), rows[1].label.as_str(), rows[1].position), (outro.id.as_str(), "Outro", 1), "deleted row re-created from the snapshot, same id + position");

        // the restored content renders against the spine again (id match)
        db::delete_section(&conn, &outro.id).await.unwrap();
        // GUARDED (MCP) revert restores the row too, then normalizes the save
        let settings = db::get_settings(&conn).await.unwrap();
        let reverted = crate::tools::dispatch(&conn, &settings, "revert_artifact", &json!({ "artifact_id": old.id })).await.unwrap();
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert!(rows.iter().any(|r| r.id == outro.id), "guarded revert re-creates the row from the snapshot");
        let v: Value = serde_json::from_str(reverted["content"].as_str().unwrap()).unwrap();
        let secs = v["data"]["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 2, "both sections survive the guarded revert");
        assert_eq!(secs[1]["section_id"], json!(outro.id), "content reattaches to the restored row by id");
    }

    /// Phase 4 (item 4): the guarded MCP `save_artifact` normalizes label-keyed
    /// incoming sections onto the spine — matched labels get section_ids, an
    /// invented section is dropped with a visible ⚠ line (create=NEVER, like
    /// non-structure runs) — and the save embeds the snapshot.
    #[tokio::test]
    async fn mcp_save_normalizes_labels_to_ids_and_drops_invented() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let song = make_song(&conn).await;
        let a = db::create_section(&conn, &song.id, "Verse 1", "", 8, "", None).await.unwrap();
        let ch_stage = stage_id(&conn, &song.id, "chords").await;

        let incoming = json!({ "kind": "chords", "text": "whatever", "data": { "sections": [
            { "label": "verse 1", "chords": [{"name": "Am", "beats": 4}] },
            { "label": "Secret Outro", "chords": [{"name": "C", "beats": 4}] }
        ]}}).to_string();
        let saved = crate::tools::dispatch(&conn, &settings, "save_artifact", &json!({
            "song_id": song.id, "stage_id": ch_stage, "kind": "chords", "content": incoming,
        })).await.unwrap();

        let v: Value = serde_json::from_str(saved["content"].as_str().unwrap()).unwrap();
        let secs = v["data"]["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 1, "the invented section is dropped (create=NEVER)");
        assert_eq!(secs[0]["section_id"], json!(a.id), "label-keyed entry mapped to its section_id");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains('⚠') && text.contains("Secret Outro"), "dropped section called out, got: {text}");
        assert!(v["spine_snapshot"].is_array(), "guarded save embeds the snapshot");
        // the spine itself is untouched by an MCP save
        assert_eq!(db::list_sections(&conn, &song.id).await.unwrap().len(), 1);
    }

    /// Phase 4 (item 1+4): a guarded MCP save on the STRUCTURE stage of a
    /// spine song keeps only the notes — incoming sections are ignored with a
    /// visible ⚠ note (sections are spine-managed), the text renders the map
    /// from the SPINE, and the spine is not modified.
    #[tokio::test]
    async fn mcp_structure_save_is_notes_only_and_never_edits_spine() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let song = make_song(&conn).await;
        db::create_section(&conn, &song.id, "Verse 1", "", 8, "open", None).await.unwrap();
        let st_stage = stage_id(&conn, &song.id, "structure").await;

        let incoming = json!({ "kind": "structure", "text": "whatever", "data": {
            "keyNote": "serves the vocal", "tempoNote": "",
            "sections": [ { "label": "Verse 1", "bars": 32, "role": "rewritten" }, { "label": "Invented", "bars": 8, "role": "" } ]
        }}).to_string();
        let saved = crate::tools::dispatch(&conn, &settings, "save_artifact", &json!({
            "song_id": song.id, "stage_id": st_stage, "kind": "structure", "content": incoming,
        })).await.unwrap();

        let v: Value = serde_json::from_str(saved["content"].as_str().unwrap()).unwrap();
        assert_eq!(v["data"], json!({ "keyNote": "serves the vocal", "tempoNote": "" }), "notes-only data");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("1. **Verse 1** (8 bars) — open"), "text renders the SPINE's map (not the incoming sections), got: {text}");
        assert!(text.contains('⚠') && text.contains("not applied"), "ignored sections are called out, got: {text}");
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(rows.len(), 1, "an MCP save never creates sections");
        assert_eq!(rows[0].bars, 8, "an MCP save never edits a row's form");
    }

    /// Phase 4 (item 5): a structure run that BIRTHS the spine for a legacy
    /// song unions in sections that exist only in other stages' artifacts —
    /// the "Bridge only in lyrics" case — appended in first-seen order, like
    /// the startup migration.
    #[tokio::test]
    async fn structure_run_birth_unions_in_artifact_only_sections() {
        let (_db, conn) = mem_conn().await;
        let song = make_song(&conn).await;
        assert!(db::list_sections(&conn, &song.id).await.unwrap().is_empty(), "legacy song: no spine yet");

        // label-keyed legacy artifacts: chords has Verse 1; lyrics has a Bridge nothing else knows
        let ch_stage = stage_id(&conn, &song.id, "chords").await;
        db::save_artifact(&conn, &song.id, Some(&ch_stage), "chords", &json!({ "kind": "chords", "text": "", "data": {
            "sections": [ { "label": "Verse 1", "chords": [{"name": "Am", "beats": 4}] } ]
        }}).to_string()).await.unwrap();
        let ly_stage = stage_id(&conn, &song.id, "lyrics").await;
        db::save_artifact(&conn, &song.id, Some(&ly_stage), "lyrics", &json!({ "kind": "lyrics", "text": "", "data": {
            "sections": [ { "label": "Verse 1", "lines": ["words"] }, { "label": "Bridge", "lines": ["the turn"] } ]
        }}).to_string()).await.unwrap();

        // first structure run proposes Verse 1 + Chorus (no Bridge)
        let raw = fenced(&json!({ "keyNote": "", "tempoNote": "", "sections": [
            { "type": "verse", "label": "Verse 1", "bars": 8, "role": "open" },
            { "type": "chorus", "label": "Chorus", "bars": 8, "role": "lift" }
        ]}));
        let content = build_run_content(&conn, &song, "structure", &raw, None).await.unwrap();

        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"], "the lyrics-only Bridge is unioned in, appended last");
        assert_eq!(rows[2].bars, 8, "unioned row takes the migration defaults");
        let v: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["spine_snapshot"].as_array().unwrap().len(), 3, "snapshot includes the unioned row");
        assert!(v["text"].as_str().unwrap().contains("3. **Bridge** (8 bars)"), "text shows the unioned section, got: {}", v["text"]);
    }
}
