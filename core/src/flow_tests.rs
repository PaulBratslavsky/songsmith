//! Whole-song-flow test suite (Tier A: deterministic) — BUILD-FLOW-SUITE.md.
//!
//! Walks the WHOLE song flow (create → concept → structure → chords →
//! lyric_spec → lyrics → prompt) through the REAL orchestration code
//! (`run_stage`, `save_artifact_guarded`, the paste importers, snapshot
//! revert, approve/advance) with a scripted fake Claude. Each scenario
//! reproduces a bug class hit in live testing. In-memory DBs only.
//!
//! FAKE-CLAUDE MECHANISM (a deliberate deviation from the brief's
//! `SONGSMITH_MOCK_CLAUDE` instruction): that env var is process-global, and
//! `agent::tests` serializes its own use of it behind a PRIVATE lock this
//! module cannot share — two lock instances over one global raced hard in
//! practice (≈5/8 suite runs failed; one even attempted a REAL `claude`
//! spawn after a cross-module `remove_var`). Instead, each test here points
//! `settings.claude_bin` at its own tiny shell script that emits the canned
//! output as a stream-json `result` line — the run goes through the real
//! engine spawn/stream path with NO process-global state. The one remaining
//! exposure (`call_claude` checks the env var before spawning, so a
//! concurrently-running `agent::tests` window could leak ITS canned value
//! into one of our runs) is closed by detect-undo-retry in `run_mock`.
//!
//! Append-only test integration: helpers are duplicated from `agent::tests`
//! (which is private) rather than reaching into it.

use crate::agent::{self, run_stage, save_artifact_guarded, RunOutcome};
use crate::db;
use crate::models::*;
use crate::render::{render_stage_text, structure_spine_text};
use crate::{spine, tools};
use libsql::{params, Builder, Connection};
use serde_json::{json, Value};

// The reverse-context banners (private consts in agent.rs — duplicated here
// verbatim as the EXPECTED prompt contract; a drift in either copy should
// fail this suite loudly).
const DERIVE_BANNER: &str = "ALREADY-WRITTEN LATER STAGES (this song was imported lyrics-first) — DERIVE this stage FROM them; stay consistent; do not contradict or invent a different song.";
const REGEN_BANNER_PREFIX: &str = "LATER STAGES ALREADY EXIST (reference only — they may be OUTDATED).";

// libSQL `:memory:` gives each connection its OWN database — migrate/seed the
// one connection and reuse it for the whole test.
async fn mem_conn() -> (libsql::Database, libsql::Connection) {
    let db = Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    db::migrate(&conn).await.unwrap();
    db::seed_skills(&conn).await.unwrap();
    (db, conn)
}

/// A per-test fake `claude` CLI: a unique temp-dir shell script that drains
/// stdin (the engine writes the prompt there) and prints ONE stream-json
/// `result` line read from a canned file the test rewrites before every step.
/// Purely test-local — no env vars, no shared state between tests.
struct FakeClaude {
    dir: std::path::PathBuf,
    canned: std::path::PathBuf,
    settings: Settings,
}

impl FakeClaude {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("songsmith-flow-{}", db::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let canned = dir.join("canned.jsonl");
        std::fs::write(&canned, b"").unwrap();
        let script = dir.join("fake-claude.sh");
        std::fs::write(&script, format!("#!/bin/sh\ncat >/dev/null\nexec cat '{}'\n", canned.display())).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let settings = Settings { claude_bin: script.to_string_lossy().into_owned(), ..Settings::default() };
        FakeClaude { dir, canned, settings }
    }

    /// Script the NEXT Claude reply verbatim.
    fn reply(&self, mock: &str) {
        let line = json!({ "type": "result", "result": mock, "is_error": false, "subtype": "success" }).to_string();
        std::fs::write(&self.canned, format!("{line}\n")).unwrap();
    }
}

impl Drop for FakeClaude {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Put a song's spine back to exactly `want` (ids, labels, form, order) —
/// repairs the one case where an interference value (see `run_mock`) reached
/// a structure run and its reconcile touched the spine.
async fn restore_spine(conn: &Connection, song_id: &str, want: &[Section]) {
    let want_ids: std::collections::HashSet<&str> = want.iter().map(|r| r.id.as_str()).collect();
    for row in db::list_sections(conn, song_id).await.unwrap() {
        if !want_ids.contains(row.id.as_str()) {
            db::delete_section(conn, &row.id).await.unwrap();
        }
    }
    let have: std::collections::HashSet<String> =
        db::list_sections(conn, song_id).await.unwrap().into_iter().map(|r| r.id).collect();
    for row in want {
        if !have.contains(&row.id) {
            db::restore_section_row(conn, &row.id, song_id, row.position, &row.label).await.unwrap();
        }
        db::update_section(conn, &row.id, &row.label, &row.r#type, row.bars, &row.role).await.unwrap();
    }
    if !want.is_empty() {
        let ids: Vec<String> = want.iter().map(|r| r.id.clone()).collect();
        db::reorder_sections(conn, song_id, &ids).await.unwrap();
    }
}

/// One scripted stage run through the REAL `run_stage`. Self-healing against
/// the only cross-test hazard left: `agent::tests` (same binary, parallel
/// threads) briefly sets the process-global `SONGSMITH_MOCK_CLAUDE` for its
/// own runs, and `call_claude` reads that var before spawning the CLI — if a
/// window overlaps this run, the run returns the OTHER test's canned value.
/// Detect it (the raw output is not ours), undo the side effects (delete the
/// polluted revision, restore the spine), and retry. We never touch the env
/// var ourselves, so `agent::tests` keeps its own invariants.
async fn run_mock(conn: &Connection, fake: &FakeClaude, stage_id: &str, mock: &str) -> RunOutcome {
    fake.reply(mock);
    let stage = db::get_stage(conn, stage_id).await.unwrap().unwrap();
    for _ in 0..500 {
        // wait out any in-flight agent::tests mock window
        while std::env::var("SONGSMITH_MOCK_CLAUDE").is_ok() {
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        let spine_before = db::list_sections(conn, &stage.song_id).await.unwrap();
        match run_stage(conn, &fake.settings, stage_id, None, |_| {}, None).await {
            Ok(out) if out.raw_output == mock => return out,
            Ok(out) => {
                // another module's canned value leaked in — undo and retry
                if let Some(a) = &out.artifact {
                    conn.execute("DELETE FROM artifact WHERE id = ?1", params![a.id.clone()])
                        .await
                        .unwrap();
                }
                db::delete_stage_draft(conn, stage_id).await.unwrap();
                restore_spine(conn, &stage.song_id, &spine_before).await;
            }
            Err(_) => {} // interference errored the run; nothing was saved
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    panic!("run_mock: could not complete a clean scripted run for stage {stage_id}");
}

/// The artifact a scripted run LANDED: a first run's direct artifact, or (for
/// a regeneration, which now parks as a pending draft — regenerate-as-draft)
/// the draft auto-ACCEPTED into a revision. For tests whose subject isn't the
/// draft flow itself; the draft tests use run_mock + the draft API directly.
async fn run_mock_landed(conn: &Connection, fake: &FakeClaude, stage_id: &str, mock: &str) -> Artifact {
    let out = run_mock(conn, fake, stage_id, mock).await;
    match out.artifact {
        Some(a) => a,
        None => agent::accept_stage_draft(conn, stage_id).await.unwrap(),
    }
}

async fn make_song(conn: &Connection, title: &str) -> Song {
    let preset = db::create_preset(conn, StyleInput {
        name: "Flow Test".into(), genre: "synth pop".into(), mood: "".into(), influences: "".into(),
        key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
    }).await.unwrap();
    db::create_song(conn, &preset.id, title).await.unwrap()
}

async fn stage_of(conn: &Connection, song_id: &str, t: &str) -> Stage {
    db::list_stages(conn, song_id).await.unwrap().into_iter().find(|s| s.r#type == t).unwrap()
}

fn fenced(v: &Value) -> String {
    format!("```json\n{v}\n```")
}

fn content(a: &Artifact) -> Value {
    serde_json::from_str(&a.content).unwrap()
}

async fn stage_status(conn: &Connection, stage_id: &str) -> String {
    db::get_stage(conn, stage_id).await.unwrap().unwrap().status
}

async fn current_stage(conn: &Connection, song_id: &str) -> String {
    db::get_song(conn, song_id).await.unwrap().unwrap().current_stage
}

/// The spine's full observable form — id/label/bars/role/position — for
/// "this stage must not touch the spine" assertions.
async fn spine_fp(conn: &Connection, song_id: &str) -> Vec<(String, String, i64, String, i64)> {
    db::list_sections(conn, song_id).await.unwrap().into_iter()
        .map(|r| (r.id, r.label, r.bars, r.role, r.position))
        .collect()
}

/// Scenario 1 — HAPPY PATH LIFECYCLE: every stage runs in order with valid
/// mocked JSON; after each run the artifact's `text` is rebuilt from `data`,
/// the stage stays unapproved until `approve_stage`, the spine only changes on
/// the structure run, and the final Generation Prompt text carries the song's
/// real key/BPM with no `{PASTE}`-style placeholder.
#[tokio::test]
async fn flow_happy_path_full_lifecycle() {
    let (_db, conn) = mem_conn().await;
    let fake = FakeClaude::new();
    let song = make_song(&conn, "Neon Rain").await; // defaults: A minor, 120 BPM
    assert_eq!((song.key_root.as_str(), song.key_mode.as_str(), song.bpm), ("A", "minor", 120));
    let stages = db::list_stages(&conn, &song.id).await.unwrap();
    let stage = |t: &str| stages.iter().find(|s| s.r#type == t).unwrap();

    // ---- CONCEPT --------------------------------------------------------
    let concept_out = fenced(&json!({
        "title": "Neon Rain", "alternates": ["City Static"],
        "hook": "we dance where the rain glows",
        "theme": "finding each other in a flooded city",
        "emotionalArc": "lonely → found", "mood": ["wet", "electric"]
    }));
    let a = run_mock_landed(&conn, &fake, &stage("concept").id, &concept_out).await;
    assert_eq!(a.kind, "concept");
    assert_eq!(a.version, 1);
    assert!(!a.approved, "a run never self-approves");
    let v = content(&a);
    assert_eq!(v["text"].as_str().unwrap(), concept_out, "non-section stage keeps the raw model text");
    assert_eq!(v["data"]["hook"], "we dance where the rain glows");
    assert_eq!(stage_status(&conn, &stage("concept").id).await, "in_progress", "run leaves the stage unapproved");
    assert!(db::list_sections(&conn, &song.id).await.unwrap().is_empty(), "concept must not touch the spine");
    tools::approve_stage(&conn, &stage("concept").id).await.unwrap();
    assert_eq!(stage_status(&conn, &stage("concept").id).await, "done");
    assert_eq!(current_stage(&conn, &song.id).await, "structure");

    // ---- STRUCTURE (births the spine) ------------------------------------
    let structure_out = fenced(&json!({
        "keyNote": "minor key carries the ache", "tempoNote": "steady mid-tempo",
        "sections": [
            { "type": "verse",  "label": "Verse 1", "bars": 8, "role": "set the scene" },
            { "type": "chorus", "label": "Chorus",  "bars": 8, "role": "the release" },
            { "type": "bridge", "label": "Bridge",  "bars": 4, "role": "the turn" }
        ]
    }));
    let a = run_mock_landed(&conn, &fake, &stage("structure").id, &structure_out).await;
    let v = content(&a);
    // Phase 4: notes-only data — the SPINE owns the sections, the SONG the key/bpm
    assert_eq!(v["data"], json!({ "keyNote": "minor key carries the ache", "tempoNote": "steady mid-tempo" }));
    let rows = db::list_sections(&conn, &song.id).await.unwrap();
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"], "the structure run births the spine");
    assert_eq!((rows[0].bars, rows[0].role.as_str()), (8, "set the scene"));
    assert_eq!(rows[2].bars, 4);
    assert_eq!(v["text"].as_str().unwrap(), structure_spine_text(&v["data"], &rows), "text rebuilt from data + spine");
    assert!(v["spine_snapshot"].is_array());
    tools::approve_stage(&conn, &stage("structure").id).await.unwrap();
    assert_eq!(current_stage(&conn, &song.id).await, "chords");
    let spine_after_structure = spine_fp(&conn, &song.id).await;

    // ---- CHORDS ----------------------------------------------------------
    let chords_out = fenced(&json!({ "sections": [
        { "label": "Verse 1", "chords": [{"name":"Am","beats":4},{"name":"F","beats":4}] },
        { "label": "Chorus",  "chords": [{"name":"C","beats":4},{"name":"G","beats":4}] },
        { "label": "Bridge",  "chords": [{"name":"Dm","beats":4}] }
    ]}));
    let a = run_mock_landed(&conn, &fake, &stage("chords").id, &chords_out).await;
    let v = content(&a);
    assert_eq!(v["text"].as_str().unwrap(), render_stage_text("chords", &v["data"]).unwrap(), "text rebuilt from data");
    assert!(v["text"].as_str().unwrap().contains("Verse 1: Am F"));
    assert!(!v["text"].as_str().unwrap().contains('⚠'), "clean run carries no warn lines");
    let secs = v["data"]["sections"].as_array().unwrap();
    assert_eq!(secs[0]["section_id"], json!(rows[0].id), "chords entries keyed to the spine");
    assert_eq!(spine_fp(&conn, &song.id).await, spine_after_structure, "a chords run must not touch the spine");
    tools::approve_stage(&conn, &stage("chords").id).await.unwrap();
    assert_eq!(current_stage(&conn, &song.id).await, "lyric_spec");

    // ---- LYRIC SPEC ------------------------------------------------------
    let spec_out = fenced(&json!({
        "hook": "we dance where the rain glows",
        "premise": "two strangers meet in a flooded neon city",
        "pov": "first person, present tense", "setting": "flooded midnight streets",
        "arc": "lonely → found", "diction": "balanced", "referenceVibe": "",
        "beats": [
            { "section": "Verse 1", "beat": "alone, counting ripples where she stood" },
            { "section": "Chorus",  "beat": "the meeting — rain turns to light" },
            { "section": "Bridge",  "beat": "the doubt, then the choice to stay" }
        ],
        "imageBank": ["neon on black water"], "avoid": ["generic rain clichés"]
    }));
    let a = run_mock_landed(&conn, &fake, &stage("lyric_spec").id, &spec_out).await;
    let v = content(&a);
    assert_eq!(v["text"].as_str().unwrap(), render_stage_text("lyric_spec", &v["data"]).unwrap(), "text rebuilt from data");
    assert!(v["text"].as_str().unwrap().contains("**HOOK:** we dance where the rain glows"));
    assert_eq!(v["data"]["beats"][0]["section_id"], json!(rows[0].id), "beats keyed to the spine");
    assert_eq!(spine_fp(&conn, &song.id).await, spine_after_structure, "a lyric_spec run must not touch the spine");
    tools::approve_stage(&conn, &stage("lyric_spec").id).await.unwrap();
    assert_eq!(current_stage(&conn, &song.id).await, "lyrics");

    // ---- LYRICS ----------------------------------------------------------
    let lyrics_out = fenced(&json!({ "sections": [
        { "label": "Verse 1", "lines": ["[Am]Streetlights drown in [F]silver water", "I count the ripples where you were"] },
        { "label": "Chorus",  "lines": ["[C]We dance where the [G]rain glows", "Nothing cold can find us now"] },
        { "label": "Bridge",  "lines": ["[Dm]Maybe the flood was the only way home"] }
    ]}));
    let a = run_mock_landed(&conn, &fake, &stage("lyrics").id, &lyrics_out).await;
    let v = content(&a);
    assert_eq!(v["text"].as_str().unwrap(), render_stage_text("lyrics", &v["data"]).unwrap(), "text rebuilt from data");
    assert!(v["text"].as_str().unwrap().contains("[Am]Streetlights drown in [F]silver water"));
    assert_eq!(v["data"]["sections"][2]["section_id"], json!(rows[2].id));
    assert_eq!(spine_fp(&conn, &song.id).await, spine_after_structure, "a lyrics run must not touch the spine");
    tools::approve_stage(&conn, &stage("lyrics").id).await.unwrap();
    assert_eq!(current_stage(&conn, &song.id).await, "prompt");

    // ---- GENERATION PROMPT ------------------------------------------------
    // its user prompt sees the whole song: the canonical spine block + the lyrics
    let p = agent::stage_user_prompt(&conn, stage("prompt"), None).await.unwrap();
    assert!(p.contains("SECTIONS (canonical"), "prompt leads with the spine block, got: {p}");
    assert!(p.contains("rain glows"), "prompt carries the written lyrics, got: {p}");
    let prompt_out = "SONG PROMPT — Neon Rain\n\
        Style: rain-slick synth pop, A minor, 120 BPM, intimate vocal, wet neon atmosphere.\n\n\
        [Verse 1]\n[Am]Streetlights drown in [F]silver water\nI count the ripples where you were\n\n\
        [Chorus]\n[C]We dance where the [G]rain glows\nNothing cold can find us now\n\n\
        [Bridge]\n[Dm]Maybe the flood was the only way home\n";
    let a = run_mock_landed(&conn, &fake, &stage("prompt").id, prompt_out).await;
    assert_eq!(a.kind, "generation_prompt");
    let v = content(&a);
    let text = v["text"].as_str().unwrap();
    assert_eq!(text, prompt_out, "the prompt stage keeps the model text verbatim");
    assert!(!text.contains("{PASTE"), "no paste-here placeholder may survive to the final prompt");
    assert!(text.contains("A minor") && text.contains("120 BPM"), "final prompt references the song's real key/BPM");
    assert_eq!(spine_fp(&conn, &song.id).await, spine_after_structure, "a prompt run must not touch the spine");
    tools::approve_stage(&conn, &stage("prompt").id).await.unwrap();

    // everything approved in order → the song is complete
    let done = tools::advance_song(&conn, &song.id).await.unwrap();
    assert_eq!(done["complete"], json!(true), "all stages done → complete, got: {done}");
    assert_eq!(done["current_stage"], "prompt");
    for t in STAGE_ORDER {
        assert_eq!(stage_status(&conn, &stage_of(&conn, &song.id, t).await.id).await, "done", "{t} must be done");
    }
}

/// Scenario 2 — SONG FACTS CAN'T BE CLOBBERED: the producer set F# minor / 109;
/// a structure run whose model output smuggles `key`/`bpm` into the data can't
/// change the song row, and the saved structure data carries NO key/bpm fields
/// at all (the Phase-4 notes-only schema drop). The structure prompt carries
/// the KEY/TEMPO AUTHORITY block naming the real facts.
#[tokio::test]
async fn flow_song_facts_cannot_be_clobbered() {
    let (_db, conn) = mem_conn().await;
    let fake = FakeClaude::new();
    let song = make_song(&conn, "Key Authority").await;
    db::update_song_key(&conn, &song.id, "F#", "minor", 109).await.unwrap();
    let structure = stage_of(&conn, &song.id, "structure").await;

    // the run's user prompt carries the authority block naming F# minor / 109
    let p = agent::stage_user_prompt(&conn, &structure, None).await.unwrap();
    assert!(p.contains("KEY/TEMPO AUTHORITY: this song is F# minor at 109 BPM"), "got: {p}");
    assert!(p.contains("STALE"), "the block calls stale keys out, got: {p}");

    // the model tries to reset the key/tempo inside its data
    let out = fenced(&json!({
        "key": "A minor", "bpm": 138,
        "keyNote": "steady and dark", "tempoNote": "",
        "sections": [ { "type": "verse", "label": "Verse 1", "bars": 8, "role": "open" } ]
    }));
    let a = run_mock_landed(&conn, &fake, &structure.id, &out).await;

    // the song row is untouched
    let after = db::get_song(&conn, &song.id).await.unwrap().unwrap();
    assert_eq!((after.key_root.as_str(), after.key_mode.as_str(), after.bpm), ("F#", "minor", 109), "song facts survive the run");
    // the saved data carries NO key/bpm fields at all
    let v = content(&a);
    assert!(v["data"].get("key").is_none(), "structure data must not carry a key, got: {}", v["data"]);
    assert!(v["data"].get("bpm").is_none(), "structure data must not carry a bpm, got: {}", v["data"]);
    assert_eq!(v["data"], json!({ "keyNote": "steady and dark", "tempoNote": "" }));
    // and the rendered text carries no smuggled fact lines
    let text = v["text"].as_str().unwrap();
    assert!(!text.contains("A minor") && !text.contains("138"), "the model's fake facts must not render, got: {text}");
}

/// Scenario 3 — AI CAN'T INVENT OR SILENTLY DROP SECTIONS. Two halves of the
/// same guarantee, against a 3-section spine:
///   (a) D3 no-create — a NON-structure (chords) run that invents a 4th
///       section and omits one: no spine row appears, the invented entry is
///       dropped with a visible ⚠ line, the omitted row is still present.
///   (b) D2 keep-and-warn — a STRUCTURE run that omits a section other stages
///       still carry content for: the row is KEPT (original spot, same id)
///       and the artifact text carries the ⚠ warn `reconcile_structure_run`
///       emits. (A structure run inventing a section legitimately CREATES a
///       row — proposing sections is its job — so no-create is proven on the
///       non-structure side.)
#[tokio::test]
async fn flow_ai_cannot_invent_or_drop_sections() {
    let (_db, conn) = mem_conn().await;
    let fake = FakeClaude::new();
    let song = make_song(&conn, "Spine Guard").await;
    db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "", None).await.unwrap();
    let chorus = db::create_section(&conn, &song.id, "Chorus", "chorus", 8, "", None).await.unwrap();
    let bridge = db::create_section(&conn, &song.id, "Bridge", "", 4, "", None).await.unwrap();
    let before = spine_fp(&conn, &song.id).await;

    // (a) chords run invents "Outro" and omits "Bridge"
    let chords_stage = stage_of(&conn, &song.id, "chords").await;
    let chords_out = fenced(&json!({ "sections": [
        { "label": "Verse 1", "chords": [{"name":"Am","beats":4}] },
        { "label": "Chorus",  "chords": [{"name":"C","beats":4}] },
        { "label": "Outro",   "chords": [{"name":"G","beats":4}] }
    ]}));
    let a = run_mock_landed(&conn, &fake, &chords_stage.id, &chords_out).await;
    assert_eq!(spine_fp(&conn, &song.id).await, before, "a chords run must not create/rename/delete spine rows (D3)");
    let v = content(&a);
    let secs = v["data"]["sections"].as_array().unwrap();
    assert_eq!(secs.len(), 2, "the invented Outro is dropped, never created");
    assert!(secs.iter().all(|s| s["label"] != "Outro"));
    let text = v["text"].as_str().unwrap();
    assert!(text.contains('⚠') && text.contains("Outro"), "the drop is surfaced as a warn line, got: {text}");
    assert!(db::list_sections(&conn, &song.id).await.unwrap().iter().any(|r| r.id == bridge.id),
        "the omitted Bridge row is still present — a chords run cannot delete sections");

    // (b) structure run omits "Chorus", which now carries chords content
    let structure_stage = stage_of(&conn, &song.id, "structure").await;
    let structure_out = fenced(&json!({
        "keyNote": "", "tempoNote": "",
        "sections": [
            { "type": "verse", "label": "Verse 1", "bars": 8, "role": "" },
            { "type": "",      "label": "Bridge",  "bars": 4, "role": "" }
        ]
    }));
    let a = run_mock_landed(&conn, &fake, &structure_stage.id, &structure_out).await;
    let rows = db::list_sections(&conn, &song.id).await.unwrap();
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"], "the content-bearing Chorus is KEPT near its old spot (D2)");
    assert_eq!(rows[1].id, chorus.id, "the kept row keeps its identity");
    let text = content(&a)["text"].as_str().unwrap().to_string();
    assert!(text.contains('⚠') && text.contains("Chorus") && text.contains("kept"),
        "the keep is surfaced where reconcile_structure_run puts it (the artifact text), got: {text}");
}

/// Scenario 4 — FREEZE SURVIVES REGENERATION AT THE WRITE BOUNDARY: a 🔒
/// frozen lyrics section goes through a regeneration that rewrites every
/// section (and drops the lock flag); after `save_artifact_guarded` the frozen
/// section is byte-identical and the other sections took the new content.
#[tokio::test]
async fn flow_freeze_survives_regeneration_at_write_boundary() {
    let (_db, conn) = mem_conn().await;
    let song = make_song(&conn, "Frozen").await;
    let verse = db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "", None).await.unwrap();
    let chorus = db::create_section(&conn, &song.id, "Chorus", "chorus", 8, "", None).await.unwrap();
    let lyrics_stage = stage_of(&conn, &song.id, "lyrics").await;

    let prior_data = json!({ "sections": [
        { "section_id": verse.id, "label": "Verse 1", "lines": ["these words are locked", "keep every syllable"], "frozen": true },
        { "section_id": chorus.id, "label": "Chorus", "lines": ["old chorus line"] }
    ]});
    let prior = json!({
        "kind": "lyrics", "text": render_stage_text("lyrics", &prior_data).unwrap(),
        "data": prior_data, "spine_snapshot": spine::spine_snapshot(&conn, &song.id).await.unwrap()
    }).to_string();
    db::save_artifact(&conn, &song.id, Some(&lyrics_stage.id), "lyrics", &prior).await.unwrap();
    let frozen_entry = prior_data["sections"][0].clone();

    // the regen rewrites EVERYTHING and forgets the lock entirely
    let incoming = json!({ "kind": "lyrics", "text": "ignored commentary", "data": { "sections": [
        { "label": "Verse 1", "lines": ["totally new verse the model wrote"] },
        { "label": "Chorus",  "lines": ["new chorus line", "second new line"] }
    ]}}).to_string();
    let saved = save_artifact_guarded(&conn, &song.id, Some(&lyrics_stage.id), "lyrics", &incoming).await.unwrap();
    assert_eq!(saved.version, 2, "the regen lands as a new revision");

    let v = content(&saved);
    let secs = v["data"]["sections"].as_array().unwrap();
    assert_eq!(secs.len(), 2);
    assert_eq!(secs[0], frozen_entry, "the frozen section survives byte-identical (lines, id, flag)");
    assert_eq!(secs[0]["frozen"], json!(true));
    assert_eq!(secs[1]["lines"], json!(["new chorus line", "second new line"]), "unlocked sections take the new content");
    assert_eq!(secs[1]["section_id"], json!(chorus.id));
    let text = v["text"].as_str().unwrap();
    assert!(text.contains("these words are locked"), "rebuilt text carries the frozen lines, got: {text}");
    assert!(!text.contains("totally new verse"), "the model's rewrite of the locked section is gone, got: {text}");
}

/// Scenario 5 — PASTE FLOWS: a mixed paste (`**bold**` + `[bracket]` headers,
/// a Suno arrangement tag line, inline `[F#m]` chord tags) splits on the real
/// headers only, keeps the arrangement tag as a lyric-body line, builds the
/// spine 1:1, infers F# minor, and stores the words verbatim. A second
/// `import_lyrics` with different sectioning REPLACES the spine (old rows
/// gone, matched labels keep their ids).
#[tokio::test]
async fn flow_paste_flows() {
    let (_db, conn) = mem_conn().await;
    let settings = db::get_settings(&conn).await.unwrap();
    let preset = db::create_preset(&conn, StyleInput {
        name: "Flow Test".into(), genre: "phonk".into(), mood: "".into(), influences: "".into(),
        key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
    }).await.unwrap();

    let text = "**Verse 1**\n\
                [F#m]City lights are [A]calling me home\n\
                [F#m]Every street I [B]know by heart\n\n\
                [Chorus]\n\
                [staccato synth lead riff, electronic kick drum]\n\
                [F#m]We run until the [A]morning finds us\n\
                Hold on tight now";
    let song = agent::create_song_from_lyrics(&conn, &settings, &preset.id, "Pasted", text).await.unwrap();

    // key inferred from the tags: F# is the most frequent root, all its tags minor
    assert_eq!((song.key_root.as_str(), song.key_mode.as_str()), ("F#", "minor"), "key inferred from the [F#m] tags");

    // the spine matches the section labels 1:1 — real headers only
    let rows = db::list_sections(&conn, &song.id).await.unwrap();
    let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["Verse 1", "Chorus"], "bold + bracket headers split; the arrangement tag is NOT a section");

    // lyrics stored verbatim, arrangement tag kept as a body line, ids attached
    let lyrics_stage = stage_of(&conn, &song.id, "lyrics").await;
    let v = content(&db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().unwrap());
    let secs = v["data"]["sections"].as_array().unwrap();
    assert_eq!(secs.len(), 2);
    assert_eq!(secs[0]["label"], "Verse 1");
    assert_eq!(secs[0]["section_id"], json!(rows[0].id), "lyrics entries keyed to the spine 1:1");
    assert_eq!(secs[0]["lines"], json!(["[F#m]City lights are [A]calling me home", "[F#m]Every street I [B]know by heart"]));
    assert_eq!(
        secs[1]["lines"],
        json!(["[staccato synth lead riff, electronic kick drum]", "[F#m]We run until the [A]morning finds us", "Hold on tight now"]),
        "the Suno arrangement tag stays a lyric-body line, words verbatim"
    );
    assert_eq!(secs[1]["section_id"], json!(rows[1].id));

    // ---- second paste into the SAME song: SPINE REPLACE ------------------
    let verse_row_id = rows[0].id.clone();
    let chorus_row_id = rows[1].id.clone();
    let text2 = "[Verse 1]\nCompletely new verse words\n\n[Bridge]\nA different turn entirely";
    let parsed = agent::import_lyrics(&conn, &settings, &song.id, text2).await.unwrap();
    assert!(!parsed.used_claude, "header split is deterministic — no model involved");

    let rows2 = db::list_sections(&conn, &song.id).await.unwrap();
    let labels2: Vec<&str> = rows2.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels2, ["Verse 1", "Bridge"], "the spine is REPLACED to the new labels, in order");
    assert_eq!(rows2[0].id, verse_row_id, "a matched label keeps its row id");
    assert!(rows2.iter().all(|r| r.id != chorus_row_id), "the old Chorus row is gone");

    let v = content(&db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().unwrap());
    assert_eq!(v["data"]["sections"][0]["lines"], json!(["Completely new verse words"]), "the new paste replaced the lyrics verbatim");
    assert_eq!(v["data"]["sections"][1]["label"], "Bridge");
    // import into an EXISTING song never touches the key the user may have set
    let after = db::get_song(&conn, &song.id).await.unwrap().unwrap();
    assert_eq!((after.key_root.as_str(), after.key_mode.as_str()), ("F#", "minor"));
}

/// Scenario 6 — REVERSE CONTEXT: with a lyrics artifact but an EMPTY concept
/// stage, the concept prompt carries the DERIVE banner + the lyric lines; once
/// the concept HAS an artifact, a regeneration prompt carries the
/// reference-only REGEN banner instead (so real Claude stops copying stale
/// later stages back verbatim).
#[tokio::test]
async fn flow_reverse_context_banners() {
    let (_db, conn) = mem_conn().await;
    let settings = db::get_settings(&conn).await.unwrap();
    let preset = db::create_preset(&conn, StyleInput {
        name: "Flow Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
        key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
    }).await.unwrap();
    let text = "[Verse 1]\nMidnight, the room gone quiet\n\n[Chorus]\nPull me under, make me clean";
    let song = agent::create_song_from_lyrics(&conn, &settings, &preset.id, "Imported", text).await.unwrap();
    let concept = stage_of(&conn, &song.id, "concept").await;

    // empty concept stage → derive-from-later banner + the lyric lines
    let p = agent::stage_user_prompt(&conn, &concept, None).await.unwrap();
    assert!(p.contains(DERIVE_BANNER), "empty stage gets the derive banner, got: {p}");
    assert!(!p.contains(REGEN_BANNER_PREFIX), "no regen banner on an empty stage");
    assert!(p.contains("Pull me under, make me clean"), "the lyric lines ride along, got: {p}");
    assert!(p.contains("### Lyrics output"), "the later stage arrives as a labeled block, got: {p}");

    // the concept now HAS an artifact → a regeneration gets the reference banner
    db::save_artifact(&conn, &song.id, Some(&concept.id), "concept",
        &json!({ "kind": "concept", "text": "A drowning-baptism song.", "data": null }).to_string(),
    ).await.unwrap();
    let p2 = agent::stage_user_prompt(&conn, &concept, None).await.unwrap();
    assert!(p2.contains(REGEN_BANNER_PREFIX), "regen gets the reference-only banner, got: {p2}");
    assert!(!p2.contains(DERIVE_BANNER), "the derive banner must NOT appear on a regen");
    assert!(p2.contains("Pull me under, make me clean"), "the later content is still referenced, got: {p2}");
}

/// Scenario 7 — REVISION HISTORY + REVERT: two lyrics runs journal as v1/v2;
/// reverting to v1 lands v3 with content EQUAL to v1, and a spine row deleted
/// in between is re-created from v1's spine_snapshot (same id/label/position)
/// so the restored content reattaches.
#[tokio::test]
async fn flow_revision_history_and_revert() {
    let (_db, conn) = mem_conn().await;
    let fake = FakeClaude::new();
    let song = make_song(&conn, "History").await;
    db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "", None).await.unwrap();
    let chorus = db::create_section(&conn, &song.id, "Chorus", "chorus", 8, "", None).await.unwrap();
    let lyrics_stage = stage_of(&conn, &song.id, "lyrics").await;

    let out_a = fenced(&json!({ "sections": [
        { "label": "Verse 1", "lines": ["version one verse"] },
        { "label": "Chorus",  "lines": ["version one chorus"] }
    ]}));
    let v1 = run_mock_landed(&conn, &fake, &lyrics_stage.id, &out_a).await;
    assert_eq!(v1.version, 1);

    let out_b = fenced(&json!({ "sections": [
        { "label": "Verse 1", "lines": ["version two verse, reworked"] },
        { "label": "Chorus",  "lines": ["version two chorus, reworked"] }
    ]}));
    // the regen parks as a draft (regenerate-as-draft); accepting journals v2
    let out = run_mock(&conn, &fake, &lyrics_stage.id, &out_b).await;
    assert!(out.artifact.is_none() && out.draft.is_some(), "a regen lands as a pending draft");
    let v2 = agent::accept_stage_draft(&conn, &lyrics_stage.id).await.unwrap();
    assert_eq!(v2.version, 2);
    assert!(db::get_stage_draft(&conn, &lyrics_stage.id).await.unwrap().is_none(), "accept clears the draft");

    let revs = db::list_artifact_revisions(&conn, &lyrics_stage.id).await.unwrap();
    assert_eq!(revs.iter().map(|a| a.version).collect::<Vec<_>>(), vec![2, 1], "both runs journaled");
    assert!(content(&revs[0])["text"].as_str().unwrap().contains("version two verse"));

    // the spine CHANGES between v1 and the revert: the Chorus row is deleted
    db::delete_section(&conn, &chorus.id).await.unwrap();
    assert_eq!(db::list_sections(&conn, &song.id).await.unwrap().len(), 1);

    // revert to v1 (the direct/user path)
    let restored = spine::revert_artifact(&conn, &v1.id).await.unwrap();
    assert_eq!(restored.version, 3, "a revert journals as a NEW revision");
    assert_eq!(restored.content, v1.content, "v3 content equals v1, verbatim");
    let cur = db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().unwrap();
    assert_eq!(cur.id, restored.id, "the revert is now current");

    // the deleted spine row came back from v1's snapshot — same id/label/position
    let rows = db::list_sections(&conn, &song.id).await.unwrap();
    assert_eq!(rows.len(), 2, "spine restored from the snapshot");
    assert_eq!((rows[1].id.as_str(), rows[1].label.as_str(), rows[1].position), (chorus.id.as_str(), "Chorus", 1));
    // and the restored content reattaches to it by id
    let v = content(&restored);
    assert_eq!(v["data"]["sections"][1]["section_id"], json!(chorus.id));
}

/// Scenario 8 — APPROVE GATES ADVANCEMENT ONLY (regenerate-as-draft): a regen
/// parks as a PENDING DRAFT leaving the current revision untouched; ACCEPT
/// stacks the new revision without touching approval; DISCARD throws the draft
/// away; `approve_stage` marks done (+ approves the current revision) and
/// advancement moves the song pointer; a later stage's run approves NOTHING.
#[tokio::test]
async fn flow_approve_gates_advancement_only() {
    let (_db, conn) = mem_conn().await;
    let fake = FakeClaude::new();
    let song = make_song(&conn, "Gate").await;
    let concept = stage_of(&conn, &song.id, "concept").await;
    let structure = stage_of(&conn, &song.id, "structure").await;

    let mk_concept = |hook: &str| fenced(&json!({
        "title": "Gate", "alternates": [], "hook": hook, "theme": "t", "emotionalArc": "a", "mood": ["m"]
    }));

    // first run saves directly; the REGEN parks as a draft, current untouched
    let v1 = run_mock_landed(&conn, &fake, &concept.id, &mk_concept("first hook")).await;
    let out = run_mock(&conn, &fake, &concept.id, &mk_concept("second, better hook")).await;
    let draft = out.draft.expect("a regen lands as a pending draft");
    assert!(out.artifact.is_none());
    assert!(draft.content.contains("second, better hook"));
    let cur = db::current_artifact(&conn, &concept.id).await.unwrap().unwrap();
    assert_eq!((cur.id.as_str(), cur.version), (v1.id.as_str(), 1), "the draft leaves the current revision alone");

    // DISCARD throws it away without a trace in History
    agent::discard_stage_draft(&conn, &concept.id).await.unwrap();
    assert!(db::get_stage_draft(&conn, &concept.id).await.unwrap().is_none());
    assert_eq!(db::list_artifact_revisions(&conn, &concept.id).await.unwrap().len(), 1, "a discarded draft never journals");

    // regen again and ACCEPT → v2 stacks; nothing approved
    let out = run_mock(&conn, &fake, &concept.id, &mk_concept("second, better hook")).await;
    assert!(out.draft.is_some());
    let v2 = agent::accept_stage_draft(&conn, &concept.id).await.unwrap();
    assert_eq!((v1.version, v2.version), (1, 2), "accepting the draft stacks the new revision");
    let revs = db::list_artifact_revisions(&conn, &concept.id).await.unwrap();
    assert!(revs.iter().all(|a| !a.approved), "regenerating/accepting never touches approval");
    assert_ne!(stage_status(&conn, &concept.id).await, "done", "running is not approving");
    assert_eq!(current_stage(&conn, &song.id).await, "concept", "the song pointer has not advanced");

    // approve → current revision approved, stage done, pointer moves on
    tools::approve_stage(&conn, &concept.id).await.unwrap();
    let revs = db::list_artifact_revisions(&conn, &concept.id).await.unwrap();
    assert!(revs.iter().find(|a| a.version == 2).unwrap().approved, "approve marks the CURRENT revision");
    assert!(!revs.iter().find(|a| a.version == 1).unwrap().approved, "older revisions stay unapproved");
    assert_eq!(stage_status(&conn, &concept.id).await, "done");
    assert_eq!(current_stage(&conn, &song.id).await, "structure", "approval advanced the song pointer");
    let out = tools::advance_song(&conn, &song.id).await.unwrap();
    assert_eq!(out["current_stage"], "structure", "advance stops at the first stage needing attention");

    // a later stage's run approves nothing — not itself, not the concept
    let structure_out = fenced(&json!({
        "keyNote": "", "tempoNote": "",
        "sections": [ { "type": "verse", "label": "Verse 1", "bars": 8, "role": "" } ]
    }));
    let s1 = run_mock_landed(&conn, &fake, &structure.id, &structure_out).await;
    assert!(!s1.approved, "a run never self-approves");
    assert_ne!(stage_status(&conn, &structure.id).await, "done");
    assert_eq!(stage_status(&conn, &concept.id).await, "done", "the earlier approval is untouched");
    let concept_cur = db::current_artifact(&conn, &concept.id).await.unwrap().unwrap();
    assert!(concept_cur.approved, "the approved concept revision stays approved");
}
