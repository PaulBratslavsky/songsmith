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
        "Lead" => 0xFF6B9D,
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
/// `section_invs` (optional, aligned with `sections` by index) carries each
/// section's per-chord inversion picks — the Chord Builder's progression
/// export uses it; song builds pass `&[]` (root position).
/// `takes` maps a TRACK name ("Lead", "Pad", "Bass", "Chords", "Arp") to
/// Claude-written ABSOLUTE notes per section index (empty inner vec = that
/// section sits out). "Lead" exists only via takes; for the others a
/// non-empty take section replaces the profile formula (empty falls back).
pub fn build_song(bpm: i64, sections: &[(String, i64, Vec<(String, i64)>)], section_invs: &[Vec<i64>], takes: &std::collections::HashMap<String, Vec<Vec<Value>>>, profile: &crate::midi::ArrangementProfile, progress: &dyn Fn(String)) -> Result<String> {
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
    // of stacking duplicate track sets (needs the patched Remote Script).
    // ALL known names are cleared — a profile switch must remove tracks the
    // new profile no longer builds.
    let all_track_names = ["Sections", "Bass", "Chords", "Pad", "Chord melody", "Lead", "Filler", "Arp", "Drums"];
    progress("Clearing previously built tracks…".into());
    let cleared = ableton_cmd(&mut s, json!({"type":"clear_named_tracks","params":{"names": all_track_names}}))
        .ok().and_then(|v| v.get("result").and_then(|r| r.get("deleted")).and_then(|n| n.as_i64())).unwrap_or(0);
    nap();

    // Parts the PROFILE disables generate zero notes in every section —
    // creating their tracks lays a column of EMPTY clips that reads as a
    // failed build (user report 2026-07-24: phonk's trap-808 profile has
    // pad off, so "Black" exported with a blank Pad track). Skip them.
    // "Lead" exists only when the Melodist wrote a melody (its own track —
    // user decision 2026-07-28: never hijack Chord melody, never overwrite).
    // A generated take OVERRIDES a profile-disabled part: explicit user
    // intent beats the genre default.
    let has_take = |name: &str| takes.get(name).is_some_and(|t| t.iter().any(|m| !m.is_empty()));
    let track_names: Vec<&str> = all_track_names.iter().copied().filter(|p| match *p {
        "Pad" => profile.pad || has_take("Pad"),
        "Arp" => profile.arp != crate::midi::ArpRate::Off || has_take("Arp"),
        "Drums" => profile.drums != crate::midi::DrumPattern::Off,
        "Lead" => has_take("Lead"),
        _ => true,
    }).collect();

    // create the tracks fresh, capture their indices
    progress(format!("Creating {} MIDI tracks…", track_names.len()));
    let mut tracks: Vec<i64> = Vec::new();
    for &name in &track_names {
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
            // A written take for THIS section bypasses the energy map — the
            // Melodist/Arranger already chose its silences. Otherwise the
            // section-aware density applies (leave a gap so it builds).
            let part_take = takes.get(part);
            let take = part_take.and_then(|o| o.get(i)).filter(|n| !n.is_empty());
            if take.is_none() {
                if part == "Lead" { continue; } // Lead exists only via takes
                // A part that HAS a written take owns its silences: an empty
                // section means "sit out", so don't fall back to the formula
                // here — that made the full build contradict ⚡ → Live, which
                // leaves it silent (audit 2026-07-28).
                if part_take.is_some_and(|o| o.iter().any(|n| !n.is_empty())) { continue; }
                if !active.contains(&part) { continue; }
            }
            // Resolve the notes BEFORE creating anything: a part whose profile
            // disables it writes zero notes, and an EMPTY clip reads as a
            // broken export (audit 2026-07-28 — a Pad take covering only the
            // chorus used to lay blank Pad clips across every other section).
            let notes = if part == "Sections" {
                vec![]
            } else {
                match take {
                    Some(n) => n.clone(),
                    None => part_notes(part, chords, *bars, profile, section_invs.get(i).map(|v| v.as_slice()).unwrap_or(&[])),
                }
            };
            if part != "Sections" && notes.is_empty() { continue; } // no notes → no clip
            let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
            if part == "Sections" {
                let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": clip_color(label)}}));
            } else {
                let _ = ableton_cmd(&mut s, json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": ci, "notes": notes}}));
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

/// Stub a bare chord PROGRESSION in Ableton (the Chord Builder's export): one
/// "Progression" section, one bar per chord, through the same track builder a
/// song uses — Sections/Bass/Chords/Pad/etc. follow the given profile.
/// Sections-only song OUTLINE (Library / Outlines "→ Ableton"): a section
/// map laid as the color-coded Sections clip track — no chords, no notes,
/// and NO locators (user decision 2026-07-22: Live's cue API snaps markers
/// to the wrong bars, so they never matched the clip layout; stray cues from
/// earlier runs are cleared instead).
pub fn build_outline(bpm: i64, sections: &[(String, i64)]) -> Result<String> {
    if sections.is_empty() {
        return Ok("Nothing to build — the outline has no sections.".into());
    }
    // drop mispositioned markers left by older runs, then lay the clips
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    if let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)) {
        s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
        s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
        let _ = ableton_cmd(&mut s, json!({"type":"clear_cues","params":{}}));
    }
    build_clips(bpm, sections)
}

/// Lay a bare chord PROGRESSION into Ableton as ONE MIDI track (user decision
/// 2026-07-20: just the chords, not the 7-track song stub): a single
/// "Progression" track, one bar per chord, held triads voiced at the picked
/// inversions. Re-running clears and rebuilds only that track.
/// `beats` aligns with `chords` (empty = 4 beats each — one bar per chord).
pub fn build_progression(bpm: i64, chords: &[String], beats: &[i64], inversions: &[i64], profile: &crate::midi::ArrangementProfile, progress: &dyn Fn(String)) -> Result<String> {
    if chords.is_empty() {
        return Ok("Nothing to build — add chords to the progression first.".into());
    }
    progress("Connecting to Ableton (port 9877)…".into());
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let nap = || std::thread::sleep(Duration::from_millis(35));
    let _ = ableton_cmd(&mut s, json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));

    // rebuild-clean, but ONLY our own track — a song stub's tracks are left alone
    let _ = ableton_cmd(&mut s, json!({"type":"clear_named_tracks","params":{"names": ["Progression"]}}));
    nap();
    let ti = ableton_cmd(&mut s, json!({"type":"create_midi_track","params":{"index":-1}}))
        .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
        .unwrap_or(0);
    let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": "Progression"}}));
    nap();

    progress(format!("Placing {} chords…", chords.len()));
    let ch: Vec<(String, i64)> = chords.iter().enumerate()
        .map(|(i, c)| (c.clone(), beats.get(i).copied().filter(|&b| b > 0).unwrap_or(4)))
        .collect();
    let total_beats: i64 = ch.iter().map(|(_, b)| b).sum();
    let bars = (total_beats + 3) / 4;
    let notes = part_notes("Chords", &ch, bars, profile, inversions);
    let length = (bars * 4) as f64;
    let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": 0, "length": length}}));
    if !notes.is_empty() {
        let _ = ableton_cmd(&mut s, json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": 0, "notes": notes}}));
    }
    let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": 0, "name": chords.join(" · ")}}));
    let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": 0, "color": part_color("Chords")}}));
    let _ = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": 0, "destination_time": 0.0}}));

    let voiced = inversions.iter().any(|&i| i != 0);
    Ok(format!(
        "✓ Progression track: {} chords · {bars} bars @ {bpm} BPM{}",
        chords.len(),
        if voiced { " · picked inversions voiced" } else { "" }
    ))
}

/// Lay named MIDI tracks into Ableton — the Composer's FULL export (chords +
/// melody + bass lanes, notes already resolved to absolute MIDI by the
/// frontend's own playback resolvers, so this stays theory-free). Clears and
/// rebuilds ONLY the given track names; one clip per track at bar 1.
pub fn build_midi_tracks(bpm: i64, length_beats: f64, tracks: &[(String, Vec<Value>)], audio_path: Option<&str>, progress: &dyn Fn(String)) -> Result<String> {
    let live: Vec<&(String, Vec<Value>)> = tracks.iter().filter(|(_, n)| !n.is_empty()).collect();
    if live.is_empty() {
        return Ok("Nothing to build — the composition has no notes.".into());
    }
    progress("Connecting to Ableton (port 9877)…".into());
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let nap = || std::thread::sleep(Duration::from_millis(35));
    let _ = ableton_cmd(&mut s, json!({"type":"set_tempo","params":{"tempo": bpm as f64}}));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));

    // "Reference" is always in the clear list so an export WITHOUT audio
    // still sweeps a previous export's reference track
    let mut names: Vec<&str> = live.iter().map(|(n, _)| n.as_str()).collect();
    names.push("Reference");
    let _ = ableton_cmd(&mut s, json!({"type":"clear_named_tracks","params":{"names": names}}));
    nap();

    let length = length_beats.max(4.0);
    let mut log = vec![format!("tempo {bpm} BPM · {} bars", (length / 4.0).ceil() as i64)];
    for (name, notes) in &live {
        progress(format!("Building {name} ({} notes)…", notes.len()));
        let ti = ableton_cmd(&mut s, json!({"type":"create_midi_track","params":{"index":-1}}))
            .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
            .unwrap_or(0);
        let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": name}}));
        let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": 0, "length": length}}));
        let _ = ableton_cmd(&mut s, json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": 0, "notes": notes}}));
        let part = if name.contains("Bass") { "Bass" } else if name.contains("Melody") { "Chord melody" } else { "Chords" };
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": 0, "color": part_color(part)}}));
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": 0, "name": name}}));
        let _ = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": 0, "destination_time": 0.0}}));
        log.push(format!("✓ {name} · {} notes", notes.len()));
        nap();
    }

    // The AI render rides along as a "Reference" AUDIO clip at bar 1 — the
    // Composer's A/B carried into Live. Failures never sink the MIDI build.
    if let Some(path) = audio_path {
        progress("Attaching the reference audio (Live imports the file — can take a moment)…".into());
        let ti = ableton_cmd(&mut s, json!({"type":"create_audio_track","params":{"index":-1}}))
            .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()));
        match ti {
            Some(ti) => {
                let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": "Reference"}}));
                // audio import decodes the file — the script allows itself 60s here
                s.set_read_timeout(Some(Duration::from_secs(65))).ok();
                let created = ableton_cmd(&mut s, json!({"type":"create_audio_clip","params":{"track_index": ti, "clip_index": 0, "path": path}}));
                s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
                match created {
                    Ok(_) => {
                        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": 0, "name": "Reference (AI render)"}}));
                        let _ = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": 0, "destination_time": 0.0}}));
                        log.push("✓ Reference audio at bar 1 — mute/solo it to A/B against your build".into());
                    }
                    Err(e) => log.push(format!("⚠ reference audio skipped: {e}")),
                }
            }
            None => log.push("⚠ reference audio skipped: Live can't create audio tracks yet — restart Live to load the updated AbletonMCP Remote Script".into()),
        }
    }
    Ok(log.join("\n"))
}

/// Push ONE written take into Live as its own named track ("Lead", "Pad",
/// "Bass", "Chords", "Arp") — the NON-DESTRUCTIVE path (user decision
/// 2026-07-28): no tempo change, no clearing of any other track, no full
/// rebuild. A previous track of the SAME name is replaced (that one is
/// ours); everything else in the session — a prior full build, hand edits —
/// stays untouched. Clips land at the right bars so it lines up.
pub fn build_take_track(track: &str, sections: &[(String, i64)], section_notes: &[Vec<Value>], progress: &dyn Fn(String)) -> Result<String> {
    if !section_notes.iter().any(|m| !m.is_empty()) {
        return Ok(format!("No {track} take to send — write one first (🎶)."));
    }
    progress("Connecting to Ableton (port 9877)…".into());
    let addr: std::net::SocketAddr = "127.0.0.1:9877".parse()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
        .map_err(|e| anyhow!("Can't reach Ableton on 9877 ({e}). Open Live (AbletonMCP on) and free the connection."))?;
    s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
    s.set_write_timeout(Some(Duration::from_millis(2000))).ok();
    let nap = || std::thread::sleep(Duration::from_millis(35));
    let _ = ableton_cmd(&mut s, json!({"type":"switch_to_arrangement_view","params":{}}));
    // ADD a take, never replace one: pick the first free "<Part> N" name rather
    // than deleting the existing track. `clear_named_tracks` DELETES matching
    // tracks, so the old path threw away the previous take and any hand edits
    // made to it in Live (user-hit, 2026-08-03).
    progress("Reading the Live set's track names…".into());
    let existing = live_track_names(&mut s);
    // `track` stays the BASE part name (it keys part_color); `track_name` is
    // what this take is actually called in Live.
    let track_name = free_take_track_name(&existing, track);
    nap();
    let ti = ableton_cmd(&mut s, json!({"type":"create_midi_track","params":{"index":-1}}))
        .ok().and_then(|v| v.get("result").and_then(|r| r.get("index")).and_then(|n| n.as_i64()))
        .ok_or_else(|| anyhow!("could not create the {track_name} track"))?;
    let _ = ableton_cmd(&mut s, json!({"type":"set_track_name","params":{"track_index": ti, "name": track_name}}));
    nap();

    let mut log = vec![format!("{track_name} — a NEW track; earlier takes and every other track untouched")];
    let (mut bar, mut ci) = (1i64, 0i64);
    for (i, (label, bars)) in sections.iter().enumerate() {
        let notes = section_notes.get(i).cloned().unwrap_or_default();
        let dest = ((bar - 1) as f64) * 4.0;
        bar += bars;
        if notes.is_empty() { continue; } // silence by choice — no clip
        progress(format!("Laying {label} ({} notes)…", notes.len()));
        let length = (*bars as f64) * 4.0;
        let _ = ableton_cmd(&mut s, json!({"type":"create_clip","params":{"track_index": ti, "clip_index": ci, "length": length}}));
        let _ = ableton_cmd(&mut s, json!({"type":"add_notes_to_clip","params":{"track_index": ti, "clip_index": ci, "notes": notes}}));
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_color","params":{"track_index": ti, "clip_index": ci, "color": part_color(track)}}));
        let _ = ableton_cmd(&mut s, json!({"type":"set_clip_name","params":{"track_index": ti, "clip_index": ci, "name": label}}));
        let _ = ableton_cmd(&mut s, json!({"type":"duplicate_session_clip_to_arrangement","params":{"track_index": ti, "clip_index": ci, "destination_time": dest}}));
        log.push(format!("✓ {label} @ bar {} · {} notes", bar - bars, notes.len()));
        ci += 1;
        nap();
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
    // stored per-preset profile wins; genre-keyword mapping is the fallback
    let preset = db::get_preset(conn, &song.style_preset_id).await.ok().flatten();
    let profile = preset
        .as_ref()
        .and_then(|p| crate::midi::profile_from_json(&p.arrangement))
        .unwrap_or_else(|| *preset.as_ref().map(|p| crate::midi::profile_for_genre(&p.genre)).unwrap_or(&crate::midi::POP_DEFAULT));
    let takes = takes_for_sections(conn, &song, &sections, profile.vel_scale).await;
    let bpm = song.bpm;
    tokio::task::spawn_blocking(move || build_song(bpm, &sections, &[], &takes, &profile, &progress)).await?
}

/// The lowercase part key ↔ Live track name mapping for written takes.
fn take_track_name(part: &str) -> Option<&'static str> {
    match part {
        "lead" => Some("Lead"), "bass" => Some("Bass"), "pad" => Some("Pad"),
        "chords" => Some("Chords"), "arp" => Some("Arp"),
        _ => None,
    }
}

/// Every track name currently in the Live set, read back via `get_session_info`
/// (for the count) then `get_track_info` per index. Deliberately uses only
/// commands the shipped remote script already has — adding a new one would
/// force a Live restart before this could work.
fn live_track_names(s: &mut TcpStream) -> Vec<String> {
    let count = ableton_cmd(s, json!({"type":"get_session_info","params":{}}))
        .ok()
        .and_then(|v| v.pointer("/result/track_count").and_then(|n| n.as_i64()))
        .unwrap_or(0);
    (0..count)
        .filter_map(|i| {
            ableton_cmd(s, json!({"type":"get_track_info","params":{"track_index": i}}))
                .ok()
                .and_then(|v| v.pointer("/result/name").and_then(|n| n.as_str()).map(String::from))
        })
        .collect()
}

/// The first FREE take-track name: "Lead" if nothing is called that, else
/// "Lead 2", "Lead 3", … Each push lands on its own track so a previous take —
/// and any hand editing done to it in Live — survives, and takes can be A/B'd
/// by soloing. Replaces the old delete-then-recreate, which destroyed the
/// previous take every time (user-hit, 2026-08-03).
fn free_take_track_name(existing: &[String], base: &str) -> String {
    let taken = |n: &str| existing.iter().any(|e| e.trim().eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    // start at 2 — the unsuffixed name IS take 1
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| base.to_string())
}

/// Every stored take (Melodist lead + Arranger parts), converted from
/// degree-based note events to absolute-MIDI per section INDEX, keyed by
/// its Live track name. Empty map when nothing was written.
async fn takes_for_sections(conn: &Connection, song: &crate::models::Song, sections: &[(String, i64, Vec<(String, i64)>)], vel_scale: f64) -> std::collections::HashMap<String, Vec<Vec<Value>>> {
    let root_pc = crate::midi::chord_tones(&song.key_root).map(|(pc, _)| pc).unwrap_or(0);
    let minor = song.key_mode != "major";
    let convert = |raw: &str, part: &str| -> Vec<Vec<Value>> {
        let stored: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
        let take_secs = stored.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        // first-UNUSED label match: with `.find()`, two sections sharing a
        // label both got the first one's notes (audit 2026-07-28)
        let mut used: Vec<bool> = vec![false; take_secs.len()];
        sections.iter().map(|(label, _, _)| {
            let hit = take_secs.iter().enumerate().position(|(j, s)| {
                !used[j] && s.get("label").and_then(|v| v.as_str())
                    .map(|l| l.trim().eq_ignore_ascii_case(label.trim())).unwrap_or(false)
            });
            match hit {
                Some(j) => {
                    used[j] = true;
                    take_secs[j].get("notes").and_then(|v| v.as_array())
                        .map(|n| crate::midi::take_notes_abs(n, root_pc, minor, vel_scale, part))
                        .unwrap_or_default()
                }
                None => vec![],
            }
        }).collect()
    };
    let mut map = std::collections::HashMap::new();
    if let Ok(Some(raw)) = db::get_song_melody(conn, &song.id).await {
        map.insert("Lead".to_string(), convert(&raw, "lead"));
    }
    for (part, raw) in db::list_song_parts(conn, &song.id).await.unwrap_or_default() {
        if let Some(track) = take_track_name(&part) {
            map.insert(track.to_string(), convert(&raw, &part));
        }
    }
    map
}

/// One written take as its OWN Live track — the non-destructive push behind
/// "⚡ Lead → Live" and the per-part variation buttons. `part` is lowercase
/// ("lead", "bass", "pad", "chords", "arp").
pub async fn build_take_track_for(conn: &Connection, song_id: &str, part: &str, progress: impl Fn(String) + Send + 'static) -> Result<String> {
    let track = take_track_name(part).ok_or_else(|| anyhow!("unknown part '{part}'"))?;
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let sections = song_parts(conn, song_id).await;
    if sections.is_empty() {
        return Ok("No sections found — run the Structure stage first.".into());
    }
    // resolve the profile exactly as the full build does (stored arrangement,
    // else the genre keyword fallback) so a take sounds the SAME whichever
    // button pushed it (audit 2026-07-28)
    let preset = db::get_preset(conn, &song.style_preset_id).await.ok().flatten();
    let profile = preset.as_ref()
        .and_then(|p| crate::midi::profile_from_json(&p.arrangement))
        .unwrap_or_else(|| *preset.as_ref().map(|p| crate::midi::profile_for_genre(&p.genre)).unwrap_or(&crate::midi::POP_DEFAULT));
    let takes = takes_for_sections(conn, &song, &sections, profile.vel_scale).await;
    let section_notes = takes.get(track).cloned().unwrap_or_default();
    let secs: Vec<(String, i64)> = sections.iter().map(|(l, b, _)| (l.clone(), *b)).collect();
    let track = track.to_string();
    tokio::task::spawn_blocking(move || build_take_track(&track, &secs, &section_notes, &progress)).await?
}

/// Back-compat wrapper: the Melodist lead push.
pub async fn build_melody_track_for(conn: &Connection, song_id: &str, progress: impl Fn(String) + Send + 'static) -> Result<String> {
    build_take_track_for(conn, song_id, "lead", progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::StyleInput;
    use libsql::Builder;

    /// The real thing, against a running Live. Ignored by default because it
    /// needs Ableton open with AbletonMCP listening on 9877 AND it mutates the
    /// open set (it adds one track). Run deliberately:
    ///
    ///   cargo test -p song_core -- --ignored take_push_adds_a_new_track
    ///
    /// Proves what the unit test above cannot: that the names we read back from
    /// Live are the ones `free_take_track_name` is actually choosing between.
    #[test]
    #[ignore]
    fn take_push_adds_a_new_track_in_a_live_session() {
        let before = {
            let mut s = TcpStream::connect_timeout(
                &"127.0.0.1:9877".parse().unwrap(), Duration::from_millis(1500),
            ).expect("Ableton must be open with AbletonMCP on 9877");
            s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
            live_track_names(&mut s)
        };
        let expected = free_take_track_name(&before, "Lead");
        assert!(!before.contains(&expected), "{expected} already exists — the name picker is wrong");

        // one bar, one note: enough to force the track + clip path
        let sections = vec![("Verse 1".to_string(), 1i64)];
        let notes = vec![vec![json!({"pitch": 60, "start_time": 0.0, "duration": 1.0, "velocity": 90})]];
        let out = build_take_track("Lead", &sections, &notes, &|_| {}).expect("push failed");
        assert!(out.contains(&expected), "log should name the new track: {out}");

        let after = {
            let mut s = TcpStream::connect_timeout(
                &"127.0.0.1:9877".parse().unwrap(), Duration::from_millis(1500),
            ).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(4000))).ok();
            live_track_names(&mut s)
        };
        assert!(after.contains(&expected), "'{expected}' should exist in Live now; got {after:?}");
        for old in &before {
            assert!(after.contains(old), "'{old}' was DELETED — the push must be additive; got {after:?}");
        }
    }

    /// A take push ADDS a track; it must never pick a name already in the set,
    /// because the old behaviour (delete-then-recreate) threw away the previous
    /// take and any hand edits made to it in Live.
    #[test]
    fn take_track_name_never_collides_with_an_existing_track() {
        // nothing there yet → the plain part name
        assert_eq!(free_take_track_name(&[], "Lead"), "Lead");
        // the unsuffixed name IS take 1, so the next one is 2
        assert_eq!(free_take_track_name(&["Lead".into()], "Lead"), "Lead 2");
        assert_eq!(free_take_track_name(&["Lead".into(), "Lead 2".into()], "Lead"), "Lead 3");
        // a gap is reused rather than skipped past
        assert_eq!(free_take_track_name(&["Lead".into(), "Lead 3".into()], "Lead"), "Lead 2");
        // other parts and unrelated tracks don't crowd the namespace
        assert_eq!(free_take_track_name(&["Bass".into(), "Drums".into()], "Lead"), "Lead");
        assert_eq!(free_take_track_name(&["Lead".into()], "Bass"), "Bass");
        // Live's names are user-editable: match case-insensitively and ignore
        // the padding Live leaves behind, or we'd hand back a duplicate
        assert_eq!(free_take_track_name(&["lead".into()], "Lead"), "Lead 2");
        assert_eq!(free_take_track_name(&["  Lead  ".into()], "Lead"), "Lead 2");
    }

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
