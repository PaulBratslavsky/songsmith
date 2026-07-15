//! Ableton Live integration: the Remote Script socket protocol (127.0.0.1:9877)
//! and the deterministic Arrangement builders (locators / section clips / the
//! full MIDI song stub). Moved out of the Tauri layer (audit Tier-2 #10) so the
//! logic is testable and callable over MCP — no MCP client or LLM in the loop
//! here, just direct socket commands.

use crate::db;
use crate::midi::{part_notes, section_parts};
use anyhow::{anyhow, Result};
use libsql::Connection;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::Duration;

/// Send one JSON command to Ableton's Remote Script socket and read one JSON reply.
pub fn ableton_cmd(stream: &mut TcpStream, body: Value) -> Result<Value> {
    use std::io::{Read, Write};
    stream.write_all(&serde_json::to_vec(&body)?)?;
    let mut acc = Vec::new();
    let mut buf = [0u8; 8192];
    for _ in 0..64 {
        let n = stream.read(&mut buf)?;
        if n == 0 { break; }
        acc.extend_from_slice(&buf[..n]);
        if let Ok(v) = serde_json::from_slice::<Value>(&acc) { return Ok(v); }
    }
    Err(anyhow!("incomplete response"))
}

/// Color (RGB int) for a section, by type — Live snaps to the nearest swatch.
pub fn clip_color(label: &str) -> i64 {
    let l = label.to_lowercase();
    if l.contains("pre") { 0xFF9500 }          // pre-chorus → orange
    else if l.contains("chorus") { 0x4CD964 }   // chorus → green
    else if l.contains("verse") { 0x3DC2FF }    // verse → blue
    else if l.contains("break") || l.contains("bridge") || l.contains("build") { 0xAF52DE } // purple
    else if l.contains("intro") || l.contains("outro") { 0x8E8E93 } // gray
    else { 0xCBCBCB }
}

/// A distinct clip color per part so the arrangement reads at a glance.
pub fn part_color(part: &str) -> i64 {
    match part {
        "Bass" => 0x5C7CFA, "Chords" => 0x12B886, "Pad" => 0x9775FA,
        "Chord melody" => 0x4DABF7, "Filler" => 0x51CF66, "Arp" => 0xFFD43B,
        _ => 0xCBCBCB,
    }
}

// Style mapping lives in midi::profile_for_genre (arrangement profiles,
// Phase 1) — the old groove_for_genre bool is gone.

// ---- Section readers (THE one section parser — was triplicated) -------------

/// `(section_id?, label, bars)` rows for a song: the section SPINE when it has
/// rows (Phase 2, docs/SECTION-SPINE-SPEC.md — the spine owns identity/order/
/// bars), else parsed from the Structure artifact exactly as before (legacy /
/// unmigrated fallback, where entries carry no id).
async fn section_rows(conn: &Connection, song_id: &str) -> Vec<(Option<String>, String, i64)> {
    if let Ok(rows) = db::list_sections(conn, song_id).await {
        if !rows.is_empty() {
            return rows.into_iter().map(|r| (Some(r.id), r.label, r.bars)).collect();
        }
    }
    let mut out = Vec::new();
    if let Ok(stages) = db::list_stages(conn, song_id).await {
        if let Some(st) = stages.iter().find(|s| s.r#type == "structure") {
            if let Ok(Some(art)) = db::current_artifact(conn, &st.id).await {
                if let Ok(v) = serde_json::from_str::<Value>(&art.content) {
                    if let Some(secs) = v.get("data").and_then(|d| d.get("sections")).and_then(|s| s.as_array()) {
                        for s in secs {
                            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("Section").to_string();
                            let bars = s.get("bars").and_then(|x| x.as_i64()).unwrap_or(8);
                            out.push((None, label, bars));
                        }
                    }
                }
            }
        }
    }
    out
}

/// Section (label, bars) list for a song — the single section parser
/// (previously triplicated across the Tauri layer: chat preamble +
/// ableton_build + song_sections). Reads the SPINE when the song has one,
/// falling back to the Structure artifact for legacy songs.
pub async fn song_sections(conn: &Connection, song_id: &str) -> Vec<(String, i64)> {
    section_rows(conn, song_id).await.into_iter().map(|(_, label, bars)| (label, bars)).collect()
}

/// Sections enriched with each section's progression as (chord, beats). The
/// Chords artifact's per-section CONTENT attaches by `section_id` when both
/// sides carry one (the migration adds ids), else by exact label — the same
/// tolerance every Phase-2 reader keeps for legacy data.
pub async fn song_parts(conn: &Connection, song_id: &str) -> Vec<(String, i64, Vec<(String, i64)>)> {
    let secs = section_rows(conn, song_id).await;
    let mut by_id: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    let mut by_label: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    if let Ok(stages) = db::list_stages(conn, song_id).await {
        if let Some(st) = stages.iter().find(|s| s.r#type == "chords") {
            if let Ok(Some(art)) = db::current_artifact(conn, &st.id).await {
                if let Ok(v) = serde_json::from_str::<Value>(&art.content) {
                    if let Some(arr) = v.pointer("/data/sections").and_then(|s| s.as_array()) {
                        for s in arr {
                            let label = s.get("label").and_then(|x| x.as_str()).unwrap_or("").to_string();
                            let chords: Vec<(String, i64)> = s.get("chords").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|c| {
                                let name = c.as_str().map(String::from).or_else(|| c.get("name").and_then(|n| n.as_str()).map(String::from))?;
                                let beats = c.get("beats").and_then(|b| b.as_i64()).filter(|&b| b > 0).unwrap_or(4);
                                Some((name, beats))
                            }).collect()).unwrap_or_default();
                            if let Some(id) = s.get("section_id").and_then(|x| x.as_str()) {
                                by_id.insert(id.to_string(), chords.clone());
                            }
                            by_label.insert(label, chords);
                        }
                    }
                }
            }
        }
    }
    secs.into_iter().map(|(id, label, bars)| {
        let ch = id
            .as_deref()
            .and_then(|i| by_id.get(i))
            .or_else(|| by_label.get(&label))
            .cloned()
            .unwrap_or_default();
        (label, bars, ch)
    }).collect()
}

// ---- Blocking socket builders (run these on a blocking thread) --------------

/// Build the song's section structure in Ableton's Arrangement as named LOCATORS
/// — direct socket, no MCP/LLM, looping every section deterministically.
pub fn build_locators(bpm: i64, sections: &[(String, i64)]) -> Result<String> {
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP control surface on) and Free the connection first."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let _ = ableton_cmd(&mut s, json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));
    // start clean so re-runs are idempotent — delete one cue per call (each on its
    // own tick) until none remain (ignored if the Live script predates clear_cues)
    for _ in 0..80 {
        let done = match ableton_cmd(&mut s, json!({"type":"clear_cues","params":{}})) {
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
    for (label, bars) in sections {
        marks.push(((bar - 1) as f64 * 4.0, bar, label.clone()));
        bar += bars;
    }
    // PASS 1 — create the locators. A short gap between each keeps every cue op
    // on its own Live tick, so the "does a cue already exist here?" check reads a
    // settled list and never toggles-deletes a previous one (the source of the
    // non-deterministic results).
    for (t, _, label) in &marks {
        let _ = ableton_cmd(&mut s, json!({"type":"create_locator","params":{"time": t, "name": label}}));
        std::thread::sleep(Duration::from_millis(60));
    }
    std::thread::sleep(Duration::from_millis(300)); // let the cue list settle
    // PASS 2 — cues are settled; rename-only (never toggles, so nothing is deleted)
    let mut log = vec![format!("tempo {bpm} BPM · {} sections", marks.len())];
    for (t, barno, label) in &marks {
        match ableton_cmd(&mut s, json!({"type":"rename_cue","params":{"time": t, "name": label}})) {
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
}

/// Build the song structure in Ableton's Arrangement as named, color-coded CLIPS
/// on a "Sections" track — direct socket, no MCP/LLM.
pub fn build_clips(bpm: i64, sections: &[(String, i64)]) -> Result<String> {
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let _ = ableton_cmd(&mut s, json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));
    // create the "Sections" track and find its index
    let ti = ableton_cmd(&mut s, json!({"type":"create_midi_track","params":{"index":-1}}))
        .ok()
        .and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
        .or_else(|| ableton_cmd(&mut s, json!({"type":"get_session_info","params":{}})).ok()
            .and_then(|v| v.get("result").and_then(|r| r.get("track_count")).and_then(|n| n.as_i64()))
            .map(|c| c - 1))
        .unwrap_or(0);
    let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": "Sections"}}));

    let mut log = vec![format!("tempo {bpm} BPM · {} clips on track {ti}", sections.len())];
    let mut bar = 1i64;
    for (i, (label, bars)) in sections.iter().enumerate() {
        let ci = i as i64;
        let length = (*bars as f64) * 4.0;
        let dest = ((bar - 1) as f64) * 4.0;
        let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
        std::thread::sleep(Duration::from_millis(40));
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": ci, "name": label}}));
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": clip_color(label)}}));
        let dup = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": ci, "destination_time": dest}}));
        std::thread::sleep(Duration::from_millis(40));
        match dup {
            Ok(v) if v.get("status").and_then(|x| x.as_str()) == Some("success") => log.push(format!("✓ {label} @ bar {bar} ({bars} bars)")),
            Ok(v) => log.push(format!("⚠️ {label} @ bar {bar}: {v}")),
            Err(e) => log.push(format!("⚠️ {label}: {e}")),
        }
        bar += bars;
    }
    Ok(log.join("\n"))
}

/// Stub the whole song in Ableton's Arrangement: a named "Sections" clip track
/// plus Bass / Chords / Pad / Chord melody / Filler / Arp MIDI parts generated
/// from the chord progression — direct socket, no MCP/LLM. MIDI-only (you pick
/// the sounds).
pub fn build_song(bpm: i64, sections: &[(String, i64, Vec<(String, i64)>)], profile: &crate::midi::ArrangementProfile, progress: &dyn Fn(String)) -> Result<String> {
    progress("Connecting to Ableton (port 9877)…".into());
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let nap = || std::thread::sleep(Duration::from_millis(35));
    let _ = ableton_cmd(&mut s, json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));

    // clear our previously-built tracks so re-running rebuilds cleanly instead
    // of stacking duplicate track sets (needs the patched Remote Script)
    let track_names = ["Sections", "Bass", "Chords", "Pad", "Chord melody", "Filler", "Arp"];
    progress("Clearing previously built tracks…".into());
    let cleared = ableton_cmd(&mut s, json!({"type":"clear_named_tracks","params":{"names": track_names}}))
        .ok().and_then(|v| v.get("result").and_then(|r| r.get("deleted")).and_then(|n| n.as_i64())).unwrap_or(0);
    nap();

    // create the tracks fresh, capture their indices
    progress(format!("Creating {} MIDI tracks…", track_names.len()));
    let mut tracks: Vec<i64> = Vec::new();
    for name in track_names {
        let ti = ableton_cmd(&mut s, json!({"type":"create_midi_track","params":{"index":-1}}))
            .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
            .unwrap_or(tracks.last().map(|t| t + 1).unwrap_or(0));
        let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": name}}));
        tracks.push(ti);
        nap();
    }

    let mut log = vec![format!("{}tempo {bpm} BPM · {} arrangement · {} sections × up to {} tracks (density per section)", if cleared > 0 { format!("cleared {cleared} old tracks · ") } else { String::new() }, profile.name, sections.len(), track_names.len())];
    let mut bar = 1i64;
    for (i, (label, bars, chords)) in sections.iter().enumerate() {
        progress(format!("Building {label} — section {}/{} ({bars} bars)…", i + 1, sections.len()));
        let ci = i as i64;
        let length = (*bars as f64) * 4.0;
        let dest = ((bar - 1) as f64) * 4.0;
        let active = section_parts(label);
        for (t, &ti) in tracks.iter().enumerate() {
            let part = track_names[t];
            if !active.contains(&part) { continue; } // section-aware density: leave a gap so it builds
            let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
            if part == "Sections" {
                let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": clip_color(label)}}));
            } else {
                let notes = part_notes(part, chords, *bars, profile);
                if !notes.is_empty() {
                    let _ = ableton_cmd(&mut s, json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": ci, "notes": notes}}));
                }
                let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": part_color(part)}}));
            }
            let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": ci, "name": label}}));
            let _ = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": ci, "destination_time": dest}}));
            nap();
        }
        log.push(format!("✓ {label} @ bar {bar} ({bars} bars · {} chords)", chords.len()));
        bar += bars;
    }
    Ok(log.join("\n"))
}

// ---- Song-level orchestrators (fetch from the DB, then build) ---------------
// Shared by the Tauri commands AND the MCP `ableton_build_song` tool.

/// Locators for a song's sections (fetch song + structure, then `build_locators`).
pub async fn build_locators_for(conn: &Connection, song_id: &str) -> Result<String> {
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let sections = song_sections(conn, song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || build_locators(bpm, &sections)).await?
}

/// Section clips for a song (fetch song + structure, then `build_clips`).
pub async fn build_clips_for(conn: &Connection, song_id: &str) -> Result<String> {
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let sections = song_sections(conn, song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || build_clips(bpm, &sections)).await?
}

/// The full MIDI song stub (fetch song + sections + chords + the preset's
/// genre-driven groove, then `build_song`).
pub async fn build_song_for(conn: &Connection, song_id: &str, progress: impl Fn(String) + Send + 'static) -> Result<String> {
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let sections = song_parts(conn, song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    let profile = db::get_preset(conn, &song.style_preset_id).await.ok().flatten()
        .map(|p| crate::midi::profile_for_genre(&p.genre))
        .unwrap_or(&crate::midi::POP_DEFAULT);
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || build_song(bpm, &sections, profile, &progress)).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StyleInput;
    use libsql::Builder;

    async fn mem_conn() -> (libsql::Database, Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        db::migrate(&conn).await.unwrap();
        (db, conn)
    }

    /// The unified structure-section parser reads (label, bars) from the
    /// Structure stage and enriches them with each section's (chord, beats)
    /// from the Chords stage — string chords and {name,beats} objects both.
    #[tokio::test]
    async fn song_sections_and_parts_read_structure_and_chords() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "T".into(), genre: "phonk".into(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "T").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let sid = |t: &str| stages.iter().find(|s| s.r#type == t).unwrap().id.clone();

        let s_data = json!({ "sections": [
            { "label": "Verse 1", "bars": 16 },
            { "label": "Chorus" } // bars default to 8
        ]});
        db::save_artifact(&conn, &song.id, Some(&sid("structure")), "structure",
            &json!({ "kind": "structure", "text": "", "data": s_data }).to_string()).await.unwrap();
        let c_data = json!({ "sections": [
            { "label": "Verse 1", "chords": [{ "name": "Am", "beats": 2 }, "F"] }
        ]});
        db::save_artifact(&conn, &song.id, Some(&sid("chords")), "chords",
            &json!({ "kind": "chords", "text": "", "data": c_data }).to_string()).await.unwrap();

        assert_eq!(song_sections(&conn, &song.id).await,
            vec![("Verse 1".to_string(), 16), ("Chorus".to_string(), 8)]);
        let parts = song_parts(&conn, &song.id).await;
        assert_eq!(parts[0], ("Verse 1".to_string(), 16, vec![("Am".to_string(), 2), ("F".to_string(), 4)]));
        assert_eq!(parts[1], ("Chorus".to_string(), 8, vec![])); // no chords saved for it

        // a song with no structure artifact has no sections
        let bare = db::create_song(&conn, &preset.id, "Bare").await.unwrap();
        assert!(song_sections(&conn, &bare.id).await.is_empty());
    }

    /// Phase 2 (docs/SECTION-SPINE-SPEC.md): when the song has SPINE rows,
    /// song_sections reads them (identity/order/bars) instead of the Structure
    /// artifact, and song_parts attaches chords content by section_id first
    /// (surviving a spine rename), with label fallback for legacy entries.
    #[tokio::test]
    async fn song_sections_prefers_spine_over_structure_artifact() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "T".into(), genre: "rock".into(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "T").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let sid = |t: &str| stages.iter().find(|s| s.r#type == t).unwrap().id.clone();

        // a STALE structure artifact that disagrees with the spine on purpose
        let s_data = json!({ "sections": [{ "label": "Old Verse", "bars": 4 }] });
        db::save_artifact(&conn, &song.id, Some(&sid("structure")), "structure",
            &json!({ "kind": "structure", "text": "", "data": s_data }).to_string()).await.unwrap();

        let v1 = db::create_section(&conn, &song.id, "Verse 1", "verse", 16, "", None).await.unwrap();
        db::create_section(&conn, &song.id, "Chorus", "", 8, "", None).await.unwrap();

        assert_eq!(song_sections(&conn, &song.id).await,
            vec![("Verse 1".to_string(), 16), ("Chorus".to_string(), 8)],
            "the spine wins over the structure artifact");

        // chords content: Verse 1 attaches BY ID despite a mismatched label;
        // Chorus attaches by exact label (legacy entry, no id)
        let c_data = json!({ "sections": [
            { "section_id": v1.id, "label": "Renamed Verse", "chords": [{ "name": "Am", "beats": 2 }] },
            { "label": "Chorus", "chords": ["F"] }
        ]});
        db::save_artifact(&conn, &song.id, Some(&sid("chords")), "chords",
            &json!({ "kind": "chords", "text": "", "data": c_data }).to_string()).await.unwrap();

        let parts = song_parts(&conn, &song.id).await;
        assert_eq!(parts[0], ("Verse 1".to_string(), 16, vec![("Am".to_string(), 2)]), "id-matched content, spine label/bars");
        assert_eq!(parts[1], ("Chorus".to_string(), 8, vec![("F".to_string(), 4)]), "label-matched legacy content");
    }

    #[test]
    fn genre_maps_to_arrangement_profiles() {
        assert_eq!(crate::midi::profile_for_genre("Memphis Phonk").name, "trap 808");
        assert_eq!(crate::midi::profile_for_genre("synthwave").name, "synthwave drive");
        assert_eq!(crate::midi::profile_for_genre("folk ballad").name, "folk acoustic");
    }
}
