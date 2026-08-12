//! libSQL (SQLite-compatible) persistence: schema, migrations, seed, and CRUD.
//!
//! The Rust core owns all state. Claude (via the agent loop) and the UI both go
//! through these functions.

use crate::models::*;
use anyhow::{anyhow, Result};
use libsql::{params, Builder, Connection, Database};
use std::path::Path;

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub async fn open(path: &Path) -> Result<Database> {
    let db = Builder::new_local(path).build().await?;
    let conn = connect(&db).await?;
    migrate(&conn).await?;
    migrate_sections(&conn).await?;
    seed_skills(&conn).await?;
    Ok(db)
}

/// Connect + per-connection PRAGMAs. `busy_timeout` makes concurrent writers
/// (the app and the mcp-shim share one DB file from two processes) wait up to
/// 5s for the lock instead of erroring "database is locked". Every consumer of
/// the core (app, shim, tests) should get its connections through here.
pub async fn connect(db: &Database) -> Result<Connection> {
    let conn = db.connect()?;
    // PRAGMA returns a result row — use query (execute rejects row-returning statements)
    let mut rows = conn.query("PRAGMA busy_timeout = 5000", ()).await?;
    while rows.next().await?.is_some() {}
    Ok(conn)
}

pub async fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS style_preset (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, genre TEXT, mood TEXT,
          influences TEXT, key_tempo_feel TEXT, vocal_range TEXT, themes TEXT,
          created_at TEXT, updated_at TEXT
        );
        CREATE TABLE IF NOT EXISTS song (
          id TEXT PRIMARY KEY, style_preset_id TEXT NOT NULL REFERENCES style_preset(id),
          title TEXT, status TEXT NOT NULL DEFAULT 'in_progress', current_stage TEXT,
          key_root TEXT, key_mode TEXT, bpm INTEGER, created_at TEXT, updated_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_song_preset ON song(style_preset_id);
        CREATE TABLE IF NOT EXISTS stage (
          id TEXT PRIMARY KEY, song_id TEXT NOT NULL REFERENCES song(id),
          type TEXT NOT NULL, ordinal INTEGER NOT NULL,
          status TEXT NOT NULL DEFAULT 'pending', skill_id TEXT,
          created_at TEXT, updated_at TEXT, UNIQUE(song_id, type)
        );
        CREATE TABLE IF NOT EXISTS artifact (
          id TEXT PRIMARY KEY, song_id TEXT NOT NULL REFERENCES song(id),
          stage_id TEXT, kind TEXT NOT NULL, content TEXT NOT NULL,
          version INTEGER NOT NULL DEFAULT 1, approved INTEGER NOT NULL DEFAULT 0, created_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_artifact_song ON artifact(song_id);
        CREATE TABLE IF NOT EXISTS skill (
          id TEXT PRIMARY KEY, key TEXT UNIQUE NOT NULL, name TEXT NOT NULL,
          stage_type TEXT NOT NULL, instructions TEXT NOT NULL,
          source TEXT NOT NULL DEFAULT 'builtin', enabled INTEGER NOT NULL DEFAULT 1,
          created_at TEXT, updated_at TEXT
        );
        CREATE TABLE IF NOT EXISTS progression (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, chords TEXT NOT NULL, created_at TEXT
        );
        CREATE TABLE IF NOT EXISTS composition (
          id TEXT PRIMARY KEY, name TEXT NOT NULL, song_id TEXT,
          data TEXT NOT NULL, created_at TEXT, updated_at TEXT
        );
        CREATE TABLE IF NOT EXISTS render (
          id TEXT PRIMARY KEY, song_id TEXT NOT NULL REFERENCES song(id),
          label TEXT, file_path TEXT NOT NULL, source TEXT, notes TEXT,
          is_pick INTEGER NOT NULL DEFAULT 0, created_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_render_song ON render(song_id);
        CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE IF NOT EXISTS section (
          id TEXT PRIMARY KEY, song_id TEXT NOT NULL REFERENCES song(id),
          position INTEGER NOT NULL, label TEXT NOT NULL,
          type TEXT NOT NULL DEFAULT '', bars INTEGER NOT NULL DEFAULT 8,
          role TEXT NOT NULL DEFAULT '', created_at TEXT, updated_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_section_song ON section(song_id);
        "#,
    )
    .await?;
    // columns added after v0.1 — idempotent (errors if already present, ignored)
    let _ = conn.execute("ALTER TABLE song ADD COLUMN voicings TEXT NOT NULL DEFAULT '{}'", ()).await;
    let _ = conn.execute("ALTER TABLE song ADD COLUMN intent TEXT NOT NULL DEFAULT ''", ()).await;
    let _ = conn.execute("ALTER TABLE style_preset ADD COLUMN lyric_exemplars TEXT NOT NULL DEFAULT ''", ()).await;
    // per-preset Ableton arrangement profile JSON (Phase 2; '' = keyword fallback)
    let _ = conn.execute("ALTER TABLE style_preset ADD COLUMN arrangement TEXT NOT NULL DEFAULT ''", ()).await;
    // per-chord shape picks for saved progressions (voicings/inversions JSON)
    let _ = conn.execute("ALTER TABLE progression ADD COLUMN picks TEXT NOT NULL DEFAULT ''", ()).await;
    // Composer-ready analysis stored per render at import time
    let _ = conn.execute("ALTER TABLE render ADD COLUMN analysis TEXT NOT NULL DEFAULT ''", ()).await;
    // saved song outlines (Outline Builder): section skeletons + tempo
    conn.execute(
        "CREATE TABLE IF NOT EXISTS outline (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            bpm INTEGER NOT NULL DEFAULT 120,
            sections TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL
        )",
        (),
    ).await?;
    // Claude-written lead melody for the Ableton build (Melodist skill) —
    // one row per song, replaced on regenerate
    conn.execute(
        "CREATE TABLE IF NOT EXISTS song_melody (
            song_id TEXT PRIMARY KEY REFERENCES song(id),
            data TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        (),
    ).await?;
    // Claude-written PART takes (Arranger skill: bass/pad/chords/arp) — one
    // row per (song, part), replaced on regenerate
    conn.execute(
        "CREATE TABLE IF NOT EXISTS song_part (
            song_id TEXT NOT NULL REFERENCES song(id),
            part TEXT NOT NULL,
            data TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (song_id, part)
        )",
        (),
    ).await?;
    let _ = conn.execute("ALTER TABLE artifact ADD COLUMN label TEXT", ()).await;
    // regenerate-as-draft: at most ONE pending draft per stage, stored outside
    // the artifact history (discarded drafts never pollute revisions)
    conn.execute(
        "CREATE TABLE IF NOT EXISTS stage_draft (
            stage_id TEXT PRIMARY KEY,
            song_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
        (),
    ).await?;
    // retrofit the Lyric Spec stage (added between Chords and Lyrics) into existing
    // songs that predate it — make room by shifting Lyrics/Prompt, then insert.
    // TRANSACTIONAL: the shift + insert must land together — a crash between them
    // would re-shift ordinals on the next launch (permanent stage-order corruption).
    // Errors propagate; the transaction rolls back on failure so a retry is clean.
    let tx = conn.transaction().await?;
    tx.execute(
        "UPDATE stage SET ordinal = ordinal + 1 WHERE type IN ('lyrics','prompt') \
         AND song_id NOT IN (SELECT song_id FROM stage WHERE type='lyric_spec')", ()).await?;
    tx.execute(
        "INSERT INTO stage (id, song_id, type, ordinal, status, created_at, updated_at) \
         SELECT lower(hex(randomblob(16))), s.id, 'lyric_spec', 3, 'pending', \
                strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         FROM song s WHERE s.id NOT IN (SELECT song_id FROM stage WHERE type='lyric_spec')", ()).await?;
    tx.commit().await?;
    // one version number per (stage, version) — backs the atomic INSERT..SELECT MAX+1
    // in save_artifact (NULL stage_ids are exempt: SQLite treats NULLs as distinct).
    if conn
        .execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_artifact_stage_version ON artifact(stage_id, version)", ())
        .await
        .is_err()
    {
        // a pre-fix race left duplicate versions — renumber per stage (stable order:
        // old version, then created_at, then id), then the unique index must succeed
        conn.execute(
            "UPDATE artifact SET version = (
               SELECT rn FROM (
                 SELECT id, ROW_NUMBER() OVER (PARTITION BY stage_id ORDER BY version, created_at, id) AS rn
                 FROM artifact WHERE stage_id IS NOT NULL
               ) t WHERE t.id = artifact.id
             ) WHERE stage_id IS NOT NULL", ()).await?;
        conn.execute("CREATE UNIQUE INDEX IF NOT EXISTS idx_artifact_stage_version ON artifact(stage_id, version)", ()).await?;
    }
    Ok(())
}

// ---- Section-spine migration (docs/SECTION-SPINE-SPEC.md §Migration) --------

/// The section-bearing stages, in seed/first-seen order.
const SECTION_STAGES: [&str; 4] = ["structure", "chords", "lyric_spec", "lyrics"];

/// ONE-TIME spine migration for existing songs. Builds each song's `section`
/// rows from its CURRENT stage artifacts (structure's sections first — the form
/// owner — else chords', else lyrics'; sections that exist only in other stages
/// are unioned in, appended in first-seen order), then rewrites those artifacts
/// ADDITIVELY: `section_id` attached to every section entry / lyric_spec beat
/// alongside the existing label fields, plus a `spine_snapshot` beside `data`.
/// Labels are NOT removed — Phase-1 readers still use them, so app behavior is
/// unchanged until the Phase-2/3 consumers switch.
///
/// Per-song idempotent: a song that already has spine rows is skipped, so songs
/// created after this launch are migrated on a later startup (until the Phase-3
/// writers create spine rows at the source). Artifact rewrites are IN PLACE
/// (UPDATE, no new revision) — this is a schema migration, not an edit; the
/// journal and legacy revisions stay untouched (readers keep a label fallback).
pub async fn migrate_sections(conn: &Connection) -> Result<()> {
    let mut rows = conn
        .query("SELECT id FROM song WHERE id NOT IN (SELECT DISTINCT song_id FROM section)", ())
        .await?;
    let mut song_ids = Vec::new();
    while let Some(r) = rows.next().await? { song_ids.push(s(&r, 0)); }
    for song_id in song_ids {
        migrate_song_sections(conn, &song_id).await?;
    }
    Ok(())
}

struct SpineEntry {
    label: String,
    r#type: String,
    bars: i64,
    role: String,
}

/// A section entry's bar count, tolerant of string-typed numbers; default 8.
fn entry_bars(sec: &serde_json::Value) -> i64 {
    sec.get("bars")
        .and_then(|b| b.as_i64().or_else(|| b.as_str().and_then(|t| t.trim().parse().ok())))
        .filter(|b| *b >= 1)
        .unwrap_or(8)
}
fn entry_str(sec: &serde_json::Value, key: &str) -> String {
    sec.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

async fn migrate_song_sections(conn: &Connection, song_id: &str) -> Result<()> {
    use crate::freeze::{norm_label, section_keys, section_label};
    use serde_json::{json, Value};

    // 1) each section stage's CURRENT artifact, parsed (non-JSON tolerated: skipped)
    struct StageArtifact {
        stage_type: &'static str,
        artifact_id: String,
        content: Value,
    }
    let mut artifacts: Vec<StageArtifact> = Vec::new();
    for stage_type in SECTION_STAGES {
        let mut rows = conn
            .query("SELECT id FROM stage WHERE song_id = ?1 AND type = ?2", params![song_id, stage_type])
            .await?;
        let Some(r) = rows.next().await? else { continue };
        let stage_id = s(&r, 0);
        let Some(art) = current_artifact(conn, &stage_id).await? else { continue };
        let Ok(content) = serde_json::from_str::<Value>(&art.content) else { continue };
        if content.is_object() {
            artifacts.push(StageArtifact { stage_type, artifact_id: art.id, content });
        }
    }
    let sections_of = |sa: &StageArtifact| -> Vec<Value> {
        let (arr_key, _) = section_keys(sa.stage_type);
        sa.content
            .get("data")
            .and_then(|d| d.get(arr_key))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    // 2) seed the spine from structure (form owner: type/bars/role carry over),
    //    else chords, else lyrics — first with a non-empty section list
    let mut spine: Vec<SpineEntry> = Vec::new();
    let mut seed: Option<usize> = None; // index into `artifacts`
    for want in ["structure", "chords", "lyrics"] {
        let Some(ai) = artifacts.iter().position(|sa| sa.stage_type == want) else { continue };
        let secs = sections_of(&artifacts[ai]);
        if secs.is_empty() {
            continue;
        }
        let is_structure = want == "structure";
        spine = secs
            .iter()
            .map(|sec| SpineEntry {
                label: section_label(want, sec),
                r#type: if is_structure { entry_str(sec, "type") } else { String::new() },
                bars: if is_structure { entry_bars(sec) } else { 8 },
                role: if is_structure { entry_str(sec, "role") } else { String::new() },
            })
            .collect();
        seed = Some(ai);
        break;
    }

    // 3) union in sections that exist only in other stages, in first-seen order
    //    (covers the "Bridge only in lyrics" case); empty labels can't be keyed.
    //    `artifact_section_labels` is the shared union source — the mid-session
    //    spine-birth union (spine::union_artifact_sections, Phase 4) reuses it.
    let mut known: std::collections::HashSet<String> =
        spine.iter().map(|e| norm_label(&e.label)).filter(|l| !l.is_empty()).collect();
    for label in artifact_section_labels(conn, song_id).await? {
        let norm = norm_label(&label);
        if norm.is_empty() || known.contains(&norm) {
            continue;
        }
        known.insert(norm);
        spine.push(SpineEntry { label, r#type: String::new(), bars: 8, role: String::new() });
    }
    if spine.is_empty() {
        return Ok(()); // nothing section-shaped anywhere — nothing to migrate
    }

    // 4) mint ids; label → id map (first row wins on duplicate labels)
    let ids: Vec<String> = spine.iter().map(|_| new_id()).collect();
    let mut by_label: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (idx, e) in spine.iter().enumerate() {
        by_label.entry(norm_label(&e.label)).or_insert_with(|| ids[idx].clone());
    }
    let snapshot: Vec<Value> = spine
        .iter()
        .enumerate()
        .map(|(idx, e)| json!({ "section_id": ids[idx], "label": e.label, "position": idx }))
        .collect();

    // 5) rewrite each artifact additively: section_id per entry + the snapshot
    let mut updates: Vec<(String, String)> = Vec::new(); // (artifact id, new content)
    for (ai, sa) in artifacts.iter().enumerate() {
        let mut content = sa.content.clone();
        if !content.get("data").map(|d| d.is_object()).unwrap_or(false) {
            continue; // data: null (no structured content) — nothing to key
        }
        let (arr_key, _) = section_keys(sa.stage_type);
        if let Some(arr) = content["data"].get_mut(arr_key).and_then(|v| v.as_array_mut()) {
            for (idx, sec) in arr.iter_mut().enumerate() {
                // the seed's entries ARE the spine rows — attach 1:1 by index
                // (robust to duplicate labels); other stages match by label
                let id = if Some(ai) == seed {
                    ids.get(idx).cloned()
                } else {
                    by_label.get(&norm_label(&section_label(sa.stage_type, sec))).cloned()
                };
                if let (Some(id), Some(obj)) = (id, sec.as_object_mut()) {
                    obj.insert("section_id".into(), Value::String(id));
                }
            }
        }
        content["spine_snapshot"] = Value::Array(snapshot.clone());
        if content != sa.content {
            updates.push((sa.artifact_id.clone(), content.to_string()));
        }
    }

    // 6) rows + rewrites land together (spec: transactional)
    let ts = now();
    let tx = conn.transaction().await?;
    for (idx, e) in spine.iter().enumerate() {
        tx.execute(
            "INSERT INTO section (id, song_id, position, label, type, bars, role, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![ids[idx].clone(), song_id, idx as i64, e.label.clone(), e.r#type.clone(), e.bars, e.role.clone(), ts.clone()],
        ).await?;
    }
    for (artifact_id, content) in &updates {
        tx.execute("UPDATE artifact SET content = ?2 WHERE id = ?1", params![artifact_id.as_str(), content.as_str()]).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Every section LABEL carried by a song's CURRENT section-stage artifacts, in
/// stage order (structure → chords → lyric_spec → lyrics) then entry order —
/// the union source (spec §Migration step 2) shared by the startup migration
/// and the mid-session spine-birth union (`spine::union_artifact_sections`).
/// Non-JSON / non-section-shaped artifacts are skipped, same tolerance as the
/// migration.
pub(crate) async fn artifact_section_labels(conn: &Connection, song_id: &str) -> Result<Vec<String>> {
    use crate::freeze::{section_keys, section_label};
    let mut out = Vec::new();
    for stage_type in SECTION_STAGES {
        let mut rows = conn
            .query("SELECT id FROM stage WHERE song_id = ?1 AND type = ?2", params![song_id, stage_type])
            .await?;
        let Some(r) = rows.next().await? else { continue };
        let stage_id = s(&r, 0);
        let Some(art) = current_artifact(conn, &stage_id).await? else { continue };
        let Ok(content) = serde_json::from_str::<serde_json::Value>(&art.content) else { continue };
        let (arr_key, _) = section_keys(stage_type);
        let Some(arr) = content.get("data").and_then(|d| d.get(arr_key)).and_then(|v| v.as_array()) else { continue };
        for sec in arr {
            out.push(section_label(stage_type, sec));
        }
    }
    Ok(out)
}

const SEED_SKILLS: &[(&str, &str, &str, &str)] = &[
    ("songsmith-concept", "Song Concept", "concept", include_str!("skills/concept.md")),
    ("songsmith-structure", "Song Structure", "structure", include_str!("skills/structure.md")),
    ("songsmith-chords", "Chord Progressions", "chords", include_str!("skills/chords.md")),
    ("songsmith-lyric-spec", "Lyric Spec", "lyric_spec", include_str!("skills/lyric-spec.md")),
    ("songsmith-lyrics", "Lyricist", "lyrics", include_str!("skills/lyrics.md")),
    ("songsmith-prompt", "Generation Prompt", "prompt", include_str!("skills/prompt.md")),
    ("songsmith-reference", "Reference Analyst", "reference", include_str!("skills/reference.md")),
    ("songsmith-style", "Style Builder", "style", include_str!("skills/style.md")),
    ("songsmith-ableton", "Ableton Arrange", "ableton", include_str!("skills/ableton.md")),
    ("songsmith-melodist", "Melodist", "melodist", include_str!("skills/melodist.md")),
    ("songsmith-arranger", "Arranger", "arranger", include_str!("skills/arranger.md")),
];

fn strip_frontmatter(raw: &str) -> String {
    let t = raw.trim_start();
    if let Some(rest) = t.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            return rest[end + 4..].trim_start().to_string();
        }
    }
    raw.trim().to_string()
}

pub async fn seed_skills(conn: &Connection) -> Result<()> {
    for (key, name, stage_type, body) in SEED_SKILLS {
        let ts = now();
        let mut rows = conn
            .query("SELECT source, name, stage_type, instructions FROM skill WHERE key = ?1", params![*key])
            .await?;
        if let Some(r) = rows.next().await? {
            // Refresh untouched builtins to the latest embedded version; leave user-edited ones.
            // Only write when the embedded content actually DIFFERS — an unconditional UPDATE
            // bumped every builtin's updated_at on each launch, silently outranking
            // user-created skills in get_active_skill_for_stage's recency ordering.
            let embedded = strip_frontmatter(body);
            let unchanged = s(&r, 1) == *name && s(&r, 2) == *stage_type && s(&r, 3) == embedded;
            if s(&r, 0) == "builtin" && !unchanged {
                conn.execute(
                    "UPDATE skill SET name=?2, stage_type=?3, instructions=?4, updated_at=?5 WHERE key=?1 AND source='builtin'",
                    params![*key, *name, *stage_type, embedded, ts],
                ).await?;
            }
        } else {
            conn.execute(
                "INSERT INTO skill (id, key, name, stage_type, instructions, source, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'builtin', 1, ?6, ?6)",
                params![new_id(), *key, *name, *stage_type, strip_frontmatter(body), ts],
            ).await?;
        }
    }
    Ok(())
}

fn s(row: &libsql::Row, i: i32) -> String {
    row.get::<String>(i).unwrap_or_default()
}
fn so(row: &libsql::Row, i: i32) -> Option<String> {
    row.get::<Option<String>>(i).unwrap_or(None)
}
fn i(row: &libsql::Row, i: i32) -> i64 {
    row.get::<i64>(i).unwrap_or(0)
}

// ---- Style presets ---------------------------------------------------------

const PRESET_COLS: &str =
    "id, name, genre, mood, influences, key_tempo_feel, vocal_range, themes, lyric_exemplars, arrangement, created_at, updated_at";
fn map_preset(r: &libsql::Row) -> StylePreset {
    StylePreset {
        id: s(r, 0), name: s(r, 1), genre: s(r, 2), mood: s(r, 3), influences: s(r, 4),
        key_tempo_feel: s(r, 5), vocal_range: s(r, 6), themes: s(r, 7), lyric_exemplars: s(r, 8),
        arrangement: s(r, 9), created_at: s(r, 10), updated_at: s(r, 11),
    }
}

pub async fn list_presets(conn: &Connection) -> Result<Vec<StylePreset>> {
    let mut rows = conn.query(&format!("SELECT {PRESET_COLS} FROM style_preset ORDER BY created_at"), ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_preset(&r)); }
    Ok(out)
}
pub async fn get_preset(conn: &Connection, id: &str) -> Result<Option<StylePreset>> {
    let mut rows = conn.query(&format!("SELECT {PRESET_COLS} FROM style_preset WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_preset))
}
pub async fn create_preset(conn: &Connection, p: StyleInput) -> Result<StylePreset> {
    let id = new_id();
    let ts = now();
    conn.execute(
        "INSERT INTO style_preset (id, name, genre, mood, influences, key_tempo_feel, vocal_range, themes, lyric_exemplars, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        params![id.clone(), p.name, p.genre, p.mood, p.influences, p.key_tempo_feel, p.vocal_range, p.themes, p.lyric_exemplars, ts],
    ).await?;
    get_preset(conn, &id).await?.ok_or_else(|| anyhow!("preset not found after create"))
}
/// Store a preset's Ableton arrangement profile JSON ('' clears it back to
/// the genre-keyword fallback). Kept OUT of StyleInput so the classic 8-field
/// create/update paths (and every existing caller) stay untouched.
pub async fn set_preset_arrangement(conn: &Connection, id: &str, arrangement: &str) -> Result<StylePreset> {
    conn.execute(
        "UPDATE style_preset SET arrangement=?2, updated_at=?3 WHERE id=?1",
        params![id, arrangement, now()],
    ).await?;
    get_preset(conn, id).await?.ok_or_else(|| anyhow!("preset not found after update"))
}

pub async fn update_preset(conn: &Connection, id: &str, p: StyleInput) -> Result<StylePreset> {
    conn.execute(
        "UPDATE style_preset SET name=?2, genre=?3, mood=?4, influences=?5, key_tempo_feel=?6, vocal_range=?7, themes=?8, lyric_exemplars=?9, updated_at=?10 WHERE id=?1",
        params![id, p.name, p.genre, p.mood, p.influences, p.key_tempo_feel, p.vocal_range, p.themes, p.lyric_exemplars, now()],
    ).await?;
    get_preset(conn, id).await?.ok_or_else(|| anyhow!("preset not found after update"))
}

// ---- Songs & stages --------------------------------------------------------

/// Parse a key (root + mode) and a BPM out of a style preset's free-prose
/// `key_tempo_feel`, so new songs can seed from the preset instead of always
/// defaulting to A minor / 120. Pure and best-effort:
/// - key: the FIRST explicit key mention wins — an uppercase note letter
///   (optional #/b/♯/♭) immediately followed by a major/maj/minor/min word
///   ("F minor", "A min", "Dark minor key (F minor / …)" → F minor). The
///   uppercase requirement keeps "in a minor key" from reading as A minor.
/// - BPM: the number (or range midpoint, rounded) right before a "BPM" word —
///   "~135–145 BPM" → 140, "120 BPM" → 120. Values outside 20–300 are ignored.
/// Empty/unparseable prose returns (None, None).
pub fn parse_key_tempo(feel: &str) -> (Option<(String, String)>, Option<i64>) {
    (parse_feel_key(feel), parse_feel_bpm(feel))
}

fn parse_feel_key(feel: &str) -> Option<(String, String)> {
    let chars: Vec<char> = feel.chars().collect();
    let n = chars.len();
    for i in 0..n {
        let c = chars[i];
        if !('A'..='G').contains(&c) {
            continue;
        }
        // word boundary before the note letter ("(F minor" yes, "THE major" no)
        if i > 0 && chars[i - 1].is_alphanumeric() {
            continue;
        }
        let mut root = c.to_string();
        let mut j = i + 1;
        if j < n && matches!(chars[j], '#' | 'b' | '♯' | '♭') {
            root.push(match chars[j] {
                '♯' => '#',
                '♭' => 'b',
                other => other,
            });
            j += 1;
        }
        // the mode word must follow directly (whitespace/hyphen allowed: "F-minor")
        while j < n && (chars[j].is_whitespace() || chars[j] == '-') {
            j += 1;
        }
        let start = j;
        while j < n && chars[j].is_alphabetic() {
            j += 1;
        }
        let word: String = chars[start..j].iter().collect::<String>().to_lowercase();
        match word.as_str() {
            "major" | "maj" => return Some((root, "major".into())),
            "minor" | "min" => return Some((root, "minor".into())),
            _ => {}
        }
    }
    None
}

fn parse_feel_bpm(feel: &str) -> Option<i64> {
    let lower = feel.to_lowercase();
    let mut search = 0;
    while let Some(pos) = lower[search..].find("bpm") {
        let at = search + pos;
        // "bpm" as its own word
        let before_ok = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
        let after_ok = at + 3 >= lower.len() || !lower.as_bytes()[at + 3].is_ascii_alphanumeric();
        if before_ok && after_ok {
            if let Some(v) = bpm_number_before(&lower[..at]) {
                return Some(v);
            }
        }
        search = at + 3;
    }
    None
}

/// The trailing `N` or `N–M` (rounded midpoint) right before a "BPM" word.
fn bpm_number_before(prefix: &str) -> Option<i64> {
    let chars: Vec<char> = prefix.chars().collect();
    let mut i = chars.len();
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    let end2 = i;
    while i > 0 && chars[i - 1].is_ascii_digit() {
        i -= 1;
    }
    let n2: f64 = chars[i..end2].iter().collect::<String>().parse().ok()?;
    // optional range: "135–145" / "135-145" / "135 — 145"
    let mut j = i;
    while j > 0 && chars[j - 1].is_whitespace() {
        j -= 1;
    }
    let mut n1: Option<f64> = None;
    if j > 0 && matches!(chars[j - 1], '-' | '–' | '—' | '−') {
        j -= 1;
        while j > 0 && chars[j - 1].is_whitespace() {
            j -= 1;
        }
        let end1 = j;
        while j > 0 && chars[j - 1].is_ascii_digit() {
            j -= 1;
        }
        if j < end1 {
            n1 = chars[j..end1].iter().collect::<String>().parse().ok();
        }
    }
    let v = match n1 {
        Some(a) => ((a + n2) / 2.0).round() as i64,
        None => n2.round() as i64,
    };
    (20..=300).contains(&v).then_some(v)
}

const SONG_COLS: &str =
    "id, style_preset_id, title, status, current_stage, key_root, key_mode, bpm, created_at, updated_at, voicings, intent";
fn map_song(r: &libsql::Row) -> Song {
    let voicings = so(r, 10).filter(|v| !v.is_empty()).unwrap_or_else(|| "{}".into());
    let intent = so(r, 11).unwrap_or_default();
    Song {
        id: s(r, 0), style_preset_id: s(r, 1), title: s(r, 2), status: s(r, 3), current_stage: s(r, 4),
        key_root: s(r, 5), key_mode: s(r, 6), bpm: i(r, 7), created_at: s(r, 8), updated_at: s(r, 9), voicings, intent,
    }
}
// includes the current artifact's timestamp (max-version) as a correlated subquery
const STAGE_SELECT: &str = "SELECT id, song_id, type, ordinal, status, skill_id, created_at, updated_at, \
    (SELECT created_at FROM artifact WHERE stage_id = stage.id ORDER BY version DESC LIMIT 1) AS artifact_at, \
    (SELECT content LIKE '%\"imported\":true%' OR content LIKE '%\"verbatim\":true%' \
     FROM artifact WHERE stage_id = stage.id ORDER BY version DESC LIMIT 1) AS imported FROM stage";
fn map_stage(r: &libsql::Row) -> Stage {
    Stage {
        id: s(r, 0), song_id: s(r, 1), r#type: s(r, 2), ordinal: i(r, 3), status: s(r, 4),
        skill_id: so(r, 5), created_at: s(r, 6), updated_at: s(r, 7), artifact_at: so(r, 8),
        // SQLite has no bool: the LIKE yields 1/0, and NULL when the stage has
        // no artifact at all. `verbatim` is the pre-rename marker, still matched
        // so songs imported before this change keep working.
        imported: r.get::<Option<i64>>(9).ok().flatten().unwrap_or(0) != 0,
    }
}

pub async fn create_song(conn: &Connection, preset_id: &str, title: &str) -> Result<Song> {
    let id = new_id();
    let ts = now();
    // Seed key/BPM from the preset's `key_tempo_feel` prose when parseable —
    // a "Sinister Memphis Phonk" (F minor, ~135–145) preset shouldn't silently
    // produce A-minor/120 songs. Song-level fields keep overriding afterwards
    // (update_song_key), and explicit-key creators (composition import,
    // reference import) overwrite these defaults right after creation.
    let (key, bpm) = match get_preset(conn, preset_id).await? {
        Some(p) => parse_key_tempo(&p.key_tempo_feel),
        None => (None, None),
    };
    let (key_root, key_mode) = key.unwrap_or_else(|| ("A".into(), "minor".into()));
    let bpm = bpm.unwrap_or(120);
    // song + its stage spec land atomically — no half-created songs
    let tx = conn.transaction().await?;
    tx.execute(
        "INSERT INTO song (id, style_preset_id, title, status, current_stage, key_root, key_mode, bpm, intent, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'in_progress', 'concept', ?4, ?5, ?6, '', ?7, ?7)",
        params![id.clone(), preset_id, title, key_root, key_mode, bpm, ts.clone()],
    ).await?;
    for (ordinal, stage_type) in STAGE_ORDER.iter().enumerate() {
        tx.execute(
            "INSERT INTO stage (id, song_id, type, ordinal, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5)",
            params![new_id(), id.clone(), *stage_type, ordinal as i64, ts.clone()],
        ).await?;
    }
    tx.commit().await?;
    get_song(conn, &id).await?.ok_or_else(|| anyhow!("song not found after create"))
}

pub async fn list_songs(conn: &Connection) -> Result<Vec<Song>> {
    let mut rows = conn.query(&format!("SELECT {SONG_COLS} FROM song ORDER BY updated_at DESC"), ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_song(&r)); }
    Ok(out)
}
pub async fn get_song(conn: &Connection, id: &str) -> Result<Option<Song>> {
    let mut rows = conn.query(&format!("SELECT {SONG_COLS} FROM song WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_song))
}
pub async fn get_song_detail(conn: &Connection, id: &str) -> Result<Option<SongDetail>> {
    let Some(song) = get_song(conn, id).await? else { return Ok(None) };
    let Some(preset) = get_preset(conn, &song.style_preset_id).await? else { return Ok(None) };
    let stages = list_stages(conn, id).await?;
    Ok(Some(SongDetail { song, preset, stages }))
}
pub async fn update_song_status(conn: &Connection, id: &str, status: &str) -> Result<Song> {
    conn.execute("UPDATE song SET status=?2, updated_at=?3 WHERE id=?1", params![id, status, now()]).await?;
    get_song(conn, id).await?.ok_or_else(|| anyhow!("song not found after update"))
}
pub async fn update_song_title(conn: &Connection, id: &str, title: &str) -> Result<Song> {
    conn.execute("UPDATE song SET title=?2, updated_at=?3 WHERE id=?1", params![id, title, now()]).await?;
    get_song(conn, id).await?.ok_or_else(|| anyhow!("song not found after update"))
}
pub async fn update_song_intent(conn: &Connection, id: &str, intent: &str) -> Result<Song> {
    conn.execute("UPDATE song SET intent=?2, updated_at=?3 WHERE id=?1", params![id, intent, now()]).await?;
    get_song(conn, id).await?.ok_or_else(|| anyhow!("song not found after update"))
}
pub async fn update_song_voicings(conn: &Connection, id: &str, voicings: &str) -> Result<Song> {
    conn.execute("UPDATE song SET voicings=?2, updated_at=?3 WHERE id=?1", params![id, voicings, now()]).await?;
    get_song(conn, id).await?.ok_or_else(|| anyhow!("song not found after update"))
}
pub async fn update_song_key(conn: &Connection, id: &str, root: &str, mode: &str, bpm: i64) -> Result<Song> {
    conn.execute("UPDATE song SET key_root=?2, key_mode=?3, bpm=?4, updated_at=?5 WHERE id=?1", params![id, root, mode, bpm, now()]).await?;
    get_song(conn, id).await?.ok_or_else(|| anyhow!("song not found after update"))
}
pub async fn set_song_current_stage(conn: &Connection, id: &str, stage_type: &str) -> Result<()> {
    conn.execute("UPDATE song SET current_stage=?2, updated_at=?3 WHERE id=?1", params![id, stage_type, now()]).await?;
    Ok(())
}
pub async fn delete_song(conn: &Connection, id: &str) -> Result<()> {
    // all-or-nothing — never leave orphaned stages/artifacts behind
    let tx = conn.transaction().await?;
    tx.execute("DELETE FROM stage_draft WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM artifact WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM stage WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM render WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM section WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM song WHERE id = ?1", params![id]).await?;
    tx.commit().await?;
    Ok(())
}

// ---- Section spine (docs/SECTION-SPINE-SPEC.md — Phase 1: CRUD only) --------

const SECTION_COLS: &str = "id, song_id, position, label, type, bars, role, created_at, updated_at";
fn map_section(r: &libsql::Row) -> Section {
    Section {
        id: s(r, 0), song_id: s(r, 1), position: i(r, 2), label: s(r, 3), r#type: s(r, 4),
        bars: i(r, 5), role: s(r, 6), created_at: s(r, 7), updated_at: s(r, 8),
    }
}

pub async fn list_sections(conn: &Connection, song_id: &str) -> Result<Vec<Section>> {
    let mut rows = conn
        .query(&format!("SELECT {SECTION_COLS} FROM section WHERE song_id = ?1 ORDER BY position"), params![song_id])
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_section(&r)); }
    Ok(out)
}
pub async fn get_section(conn: &Connection, id: &str) -> Result<Option<Section>> {
    let mut rows = conn.query(&format!("SELECT {SECTION_COLS} FROM section WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_section))
}
/// Add a spine row. `position: None` appends at the end; `Some(p)` inserts at
/// `p` (clamped), shifting later sections down — shift + insert land together.
pub async fn create_section(
    conn: &Connection, song_id: &str, label: &str, r#type: &str, bars: i64, role: &str, position: Option<i64>,
) -> Result<Section> {
    let id = new_id();
    let ts = now();
    let end: i64 = {
        let mut rows = conn
            .query("SELECT COALESCE(MAX(position) + 1, 0) FROM section WHERE song_id = ?1", params![song_id])
            .await?;
        rows.next().await?.as_ref().map(|r| i(r, 0)).unwrap_or(0)
    };
    let pos = position.map(|p| p.clamp(0, end)).unwrap_or(end);
    let tx = conn.transaction().await?;
    tx.execute("UPDATE section SET position = position + 1 WHERE song_id = ?1 AND position >= ?2", params![song_id, pos]).await?;
    tx.execute(
        "INSERT INTO section (id, song_id, position, label, type, bars, role, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![id.clone(), song_id, pos, label, r#type, bars.max(1), role, ts],
    ).await?;
    tx.commit().await?;
    get_section(conn, &id).await?.ok_or_else(|| anyhow!("section not found after create"))
}
/// Re-insert a spine row with a KNOWN id — the snapshot-based restore
/// (docs/SECTION-SPINE-SPEC.md §Snapshots): a restored artifact's entries
/// reference this exact id, so a freshly-minted one would not reattach them.
/// Position is clamped into the current spine; later rows shift down. The
/// form takes defaults (type ""/bars 8/role "") — the snapshot is light.
pub(crate) async fn restore_section_row(conn: &Connection, id: &str, song_id: &str, position: i64, label: &str) -> Result<Section> {
    let ts = now();
    let end: i64 = {
        let mut rows = conn
            .query("SELECT COALESCE(MAX(position) + 1, 0) FROM section WHERE song_id = ?1", params![song_id])
            .await?;
        rows.next().await?.as_ref().map(|r| i(r, 0)).unwrap_or(0)
    };
    let pos = position.clamp(0, end);
    let tx = conn.transaction().await?;
    tx.execute("UPDATE section SET position = position + 1 WHERE song_id = ?1 AND position >= ?2", params![song_id, pos]).await?;
    tx.execute(
        "INSERT INTO section (id, song_id, position, label, type, bars, role, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, '', 8, '', ?5, ?5)",
        params![id, song_id, pos, label, ts],
    ).await?;
    tx.commit().await?;
    get_section(conn, id).await?.ok_or_else(|| anyhow!("section not found after restore"))
}
/// Update a section's FORM (label/type/bars/role). Order changes go through
/// `reorder_sections`; identity (`id`) and `song_id` never change.
pub async fn update_section(conn: &Connection, id: &str, label: &str, r#type: &str, bars: i64, role: &str) -> Result<Section> {
    conn.execute(
        "UPDATE section SET label=?2, type=?3, bars=?4, role=?5, updated_at=?6 WHERE id=?1",
        params![id, label, r#type, bars.max(1), role, now()],
    ).await?;
    get_section(conn, id).await?.ok_or_else(|| anyhow!("section not found after update"))
}
/// Delete a spine row and close the position gap — both or neither.
pub async fn delete_section(conn: &Connection, id: &str) -> Result<()> {
    let sec = get_section(conn, id).await?.ok_or_else(|| anyhow!("section not found"))?;
    let tx = conn.transaction().await?;
    tx.execute("DELETE FROM section WHERE id = ?1", params![id]).await?;
    tx.execute(
        "UPDATE section SET position = position - 1 WHERE song_id = ?1 AND position > ?2",
        params![sec.song_id, sec.position],
    ).await?;
    tx.commit().await?;
    Ok(())
}
/// Reorder a song's spine: `ids` must be exactly the song's section ids (each
/// once — a full permutation), in the new order. All positions land together.
pub async fn reorder_sections(conn: &Connection, song_id: &str, ids: &[String]) -> Result<Vec<Section>> {
    let existing = list_sections(conn, song_id).await?;
    let have: std::collections::HashSet<&str> = existing.iter().map(|x| x.id.as_str()).collect();
    let given: std::collections::HashSet<&str> = ids.iter().map(|x| x.as_str()).collect();
    if ids.len() != existing.len() || have != given {
        return Err(anyhow!("reorder_sections needs every section id of the song exactly once ({} sections)", existing.len()));
    }
    let ts = now();
    let tx = conn.transaction().await?;
    for (pos, id) in ids.iter().enumerate() {
        tx.execute("UPDATE section SET position = ?2, updated_at = ?3 WHERE id = ?1", params![id.as_str(), pos as i64, ts.clone()]).await?;
    }
    tx.commit().await?;
    list_sections(conn, song_id).await
}

// ---- Final renders (audio versions referenced on disk) ---------------------

pub async fn list_renders(conn: &Connection, song_id: &str) -> Result<Vec<Render>> {
    let mut rows = conn.query(
        "SELECT id, song_id, label, file_path, source, notes, is_pick, analysis, created_at FROM render WHERE song_id = ?1 ORDER BY created_at DESC",
        params![song_id],
    ).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(Render {
            id: s(&r, 0), song_id: s(&r, 1), label: s(&r, 2), file_path: s(&r, 3),
            source: s(&r, 4), notes: s(&r, 5), is_pick: i(&r, 6) != 0, analysis: s(&r, 7), created_at: s(&r, 8),
        });
    }
    Ok(out)
}
pub async fn create_render(conn: &Connection, song_id: &str, label: &str, file_path: &str, source: &str, notes: &str) -> Result<Render> {
    let id = new_id();
    let ts = now();
    conn.execute(
        "INSERT INTO render (id, song_id, label, file_path, source, notes, is_pick, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![id.clone(), song_id, label, file_path, source, notes, ts.clone()],
    ).await?;
    Ok(Render { id, song_id: song_id.into(), label: label.into(), file_path: file_path.into(), source: source.into(), notes: notes.into(), is_pick: false, analysis: String::new(), created_at: ts })
}
// ---- song melody (Melodist skill → the Ableton build's lead) ----------------
pub async fn set_song_melody(conn: &Connection, song_id: &str, data: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO song_melody (song_id, data, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(song_id) DO UPDATE SET data = ?2, updated_at = ?3",
        params![song_id, data, now()],
    ).await?;
    Ok(())
}
pub async fn get_song_melody(conn: &Connection, song_id: &str) -> Result<Option<String>> {
    let mut rows = conn.query("SELECT data FROM song_melody WHERE song_id = ?1", params![song_id]).await?;
    Ok(rows.next().await?.as_ref().map(|r| s(r, 0)))
}
pub async fn delete_song_melody(conn: &Connection, song_id: &str) -> Result<()> {
    conn.execute("DELETE FROM song_melody WHERE song_id = ?1", params![song_id]).await?;
    Ok(())
}
pub async fn set_song_part(conn: &Connection, song_id: &str, part: &str, data: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO song_part (song_id, part, data, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(song_id, part) DO UPDATE SET data = ?3, updated_at = ?4",
        params![song_id, part, data, now()],
    ).await?;
    Ok(())
}
pub async fn get_song_part(conn: &Connection, song_id: &str, part: &str) -> Result<Option<String>> {
    let mut rows = conn.query("SELECT data FROM song_part WHERE song_id = ?1 AND part = ?2", params![song_id, part]).await?;
    Ok(rows.next().await?.as_ref().map(|r| s(r, 0)))
}
pub async fn list_song_parts(conn: &Connection, song_id: &str) -> Result<Vec<(String, String)>> {
    let mut rows = conn.query("SELECT part, data FROM song_part WHERE song_id = ?1", params![song_id]).await?;
    let mut out = vec![];
    while let Some(r) = rows.next().await? {
        out.push((s(&r, 0), s(&r, 1)));
    }
    Ok(out)
}

pub async fn set_render_analysis(conn: &Connection, id: &str, analysis: &str) -> Result<()> {
    conn.execute("UPDATE render SET analysis = ?2 WHERE id = ?1", params![id, analysis]).await?;
    Ok(())
}
pub async fn set_render_pick(conn: &Connection, id: &str, pick: bool) -> Result<()> {
    if pick {
        // one pick per song
        if let Some(r) = conn.query("SELECT song_id FROM render WHERE id = ?1", params![id]).await?.next().await? {
            conn.execute("UPDATE render SET is_pick = 0 WHERE song_id = ?1", params![s(&r, 0)]).await?;
        }
    }
    conn.execute("UPDATE render SET is_pick = ?2 WHERE id = ?1", params![id, if pick { 1i64 } else { 0 }]).await?;
    Ok(())
}
pub async fn delete_render(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM render WHERE id = ?1", params![id]).await?;
    Ok(())
}

pub async fn list_stages(conn: &Connection, song_id: &str) -> Result<Vec<Stage>> {
    let mut rows = conn.query(&format!("{STAGE_SELECT} WHERE song_id = ?1 ORDER BY ordinal"), params![song_id]).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_stage(&r)); }
    Ok(out)
}
pub async fn get_stage(conn: &Connection, id: &str) -> Result<Option<Stage>> {
    let mut rows = conn.query(&format!("{STAGE_SELECT} WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_stage))
}
pub async fn set_stage_status(conn: &Connection, id: &str, status: &str) -> Result<()> {
    conn.execute("UPDATE stage SET status=?2, updated_at=?3 WHERE id=?1", params![id, status, now()]).await?;
    Ok(())
}
pub async fn set_stage_skill(conn: &Connection, id: &str, skill_id: &str) -> Result<()> {
    conn.execute("UPDATE stage SET skill_id=?2 WHERE id=?1", params![id, skill_id]).await?;
    Ok(())
}
pub async fn get_stage_detail(conn: &Connection, id: &str) -> Result<Option<StageDetail>> {
    let Some(stage) = get_stage(conn, id).await? else { return Ok(None) };
    let artifact = current_artifact(conn, id).await?;
    let skill = get_active_skill_for_stage(conn, &stage.r#type).await?;
    let draft = get_stage_draft(conn, id).await?;
    Ok(Some(StageDetail { stage, artifact, skill, draft }))
}

// ---- Regeneration drafts (regenerate-as-draft) ------------------------------

const DRAFT_COLS: &str = "stage_id, song_id, kind, content, created_at";
fn map_draft(r: &libsql::Row) -> StageDraft {
    StageDraft { stage_id: s(r, 0), song_id: s(r, 1), kind: s(r, 2), content: s(r, 3), created_at: s(r, 4) }
}

/// Upsert THE pending draft for a stage (a newer re-run replaces an
/// unreviewed older draft — there is never a queue of them).
pub async fn set_stage_draft(conn: &Connection, stage_id: &str, song_id: &str, kind: &str, content: &str) -> Result<StageDraft> {
    conn.execute(
        "INSERT INTO stage_draft (stage_id, song_id, kind, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(stage_id) DO UPDATE SET song_id=?2, kind=?3, content=?4, created_at=?5",
        params![stage_id, song_id, kind, content, now()],
    ).await?;
    get_stage_draft(conn, stage_id).await?.ok_or_else(|| anyhow!("draft not found after upsert"))
}

pub async fn get_stage_draft(conn: &Connection, stage_id: &str) -> Result<Option<StageDraft>> {
    let mut rows = conn.query(&format!("SELECT {DRAFT_COLS} FROM stage_draft WHERE stage_id = ?1"), params![stage_id]).await?;
    Ok(rows.next().await?.as_ref().map(map_draft))
}

pub async fn delete_stage_draft(conn: &Connection, stage_id: &str) -> Result<()> {
    conn.execute("DELETE FROM stage_draft WHERE stage_id = ?1", params![stage_id]).await?;
    Ok(())
}

// ---- Artifacts (journaled) -------------------------------------------------

const ARTIFACT_COLS: &str = "id, song_id, stage_id, kind, content, version, approved, created_at, label";
fn map_artifact(r: &libsql::Row) -> Artifact {
    Artifact {
        id: s(r, 0), song_id: s(r, 1), stage_id: so(r, 2), kind: s(r, 3), content: s(r, 4),
        version: i(r, 5), approved: i(r, 6) != 0, created_at: s(r, 7), label: so(r, 8),
    }
}

pub async fn current_artifact(conn: &Connection, stage_id: &str) -> Result<Option<Artifact>> {
    let mut rows = conn.query(
        &format!("SELECT {ARTIFACT_COLS} FROM artifact WHERE stage_id = ?1 ORDER BY version DESC LIMIT 1"),
        params![stage_id],
    ).await?;
    Ok(rows.next().await?.as_ref().map(map_artifact))
}
pub async fn get_artifact(conn: &Connection, id: &str) -> Result<Option<Artifact>> {
    let mut rows = conn.query(&format!("SELECT {ARTIFACT_COLS} FROM artifact WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_artifact))
}
pub async fn save_artifact(conn: &Connection, song_id: &str, stage_id: Option<&str>, kind: &str, content: &str) -> Result<Artifact> {
    let id = new_id();
    match stage_id {
        // version computed INSIDE the insert — one atomic statement, so concurrent
        // app/shim saves can't both read the same max and collide (and the UNIQUE
        // index on (stage_id, version) backstops it)
        Some(sid) => {
            conn.execute(
                "INSERT INTO artifact (id, song_id, stage_id, kind, content, version, approved, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, COALESCE(MAX(version), 0) + 1, 0, ?6 FROM artifact WHERE stage_id = ?3",
                params![id.clone(), song_id, sid, kind, content, now()],
            ).await?;
        }
        // song-level artifacts (no stage) keep today's behavior: always version 1
        None => {
            conn.execute(
                "INSERT INTO artifact (id, song_id, stage_id, kind, content, version, approved, created_at)
                 VALUES (?1, ?2, NULL, ?3, ?4, 1, 0, ?5)",
                params![id.clone(), song_id, kind, content, now()],
            ).await?;
        }
    }
    get_artifact(conn, &id).await?.ok_or_else(|| anyhow!("artifact not found after save"))
}
pub async fn list_artifact_revisions(conn: &Connection, stage_id: &str) -> Result<Vec<Artifact>> {
    let mut rows = conn.query(
        &format!("SELECT {ARTIFACT_COLS} FROM artifact WHERE stage_id = ?1 ORDER BY version DESC"),
        params![stage_id],
    ).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_artifact(&r)); }
    Ok(out)
}
// (revert lives in `spine::revert_artifact` / `freeze::revert_artifact_guarded`
// since Phase 4 — both are snapshot-based; see docs/SECTION-SPINE-SPEC.md.)
/// Name (or clear — `None`) a revision's label. Non-destructive metadata: the
/// content journal is untouched; new revisions always start unlabeled (the
/// explicit `save_artifact` column lists leave `label` NULL).
pub async fn set_artifact_label(conn: &Connection, artifact_id: &str, label: Option<&str>) -> Result<()> {
    conn.execute("UPDATE artifact SET label=?2 WHERE id=?1", params![artifact_id, label]).await?;
    Ok(())
}
pub async fn set_artifact_approved(conn: &Connection, artifact_id: &str, approved: bool) -> Result<()> {
    conn.execute("UPDATE artifact SET approved=?2 WHERE id=?1", params![artifact_id, if approved { 1i64 } else { 0 }]).await?;
    Ok(())
}

// ---- Skills ----------------------------------------------------------------

const SKILL_COLS: &str = "id, key, name, stage_type, instructions, source, enabled, created_at, updated_at";
fn map_skill(r: &libsql::Row) -> Skill {
    Skill {
        id: s(r, 0), key: s(r, 1), name: s(r, 2), stage_type: s(r, 3), instructions: s(r, 4),
        source: s(r, 5), enabled: i(r, 6) != 0, created_at: s(r, 7), updated_at: s(r, 8),
    }
}
pub async fn list_skills(conn: &Connection) -> Result<Vec<Skill>> {
    let mut rows = conn.query(&format!("SELECT {SKILL_COLS} FROM skill ORDER BY stage_type, name"), ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? { out.push(map_skill(&r)); }
    Ok(out)
}
pub async fn get_skill(conn: &Connection, id: &str) -> Result<Option<Skill>> {
    let mut rows = conn.query(&format!("SELECT {SKILL_COLS} FROM skill WHERE id = ?1"), params![id]).await?;
    Ok(rows.next().await?.as_ref().map(map_skill))
}
pub async fn get_active_skill_for_stage(conn: &Connection, stage_type: &str) -> Result<Option<Skill>> {
    // User skills (created or edited) outrank builtins; recency breaks ties within a tier.
    // Without the tier, a launch-time builtin reseed could outrank a user's own skill.
    let mut rows = conn.query(
        &format!(
            "SELECT {SKILL_COLS} FROM skill WHERE stage_type = ?1 AND enabled = 1 \
             ORDER BY CASE source WHEN 'user' THEN 0 ELSE 1 END, updated_at DESC LIMIT 1"
        ),
        params![stage_type],
    ).await?;
    Ok(rows.next().await?.as_ref().map(map_skill))
}
pub async fn create_skill(conn: &Connection, input: SkillInput) -> Result<Skill> {
    let id = new_id();
    let ts = now();
    conn.execute(
        "INSERT INTO skill (id, key, name, stage_type, instructions, source, enabled, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'user', 1, ?6, ?6)",
        params![id.clone(), input.key, input.name, input.stage_type, input.instructions, ts],
    ).await?;
    get_skill(conn, &id).await?.ok_or_else(|| anyhow!("skill not found after create"))
}
pub async fn update_skill(conn: &Connection, id: &str, input: SkillInput) -> Result<Skill> {
    // mark as user-owned so the builtin refresh on startup won't overwrite the edit
    conn.execute(
        "UPDATE skill SET key=?2, name=?3, stage_type=?4, instructions=?5, source='user', updated_at=?6 WHERE id=?1",
        params![id, input.key, input.name, input.stage_type, input.instructions, now()],
    ).await?;
    get_skill(conn, id).await?.ok_or_else(|| anyhow!("skill not found after update"))
}
pub async fn set_skill_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<Skill> {
    conn.execute("UPDATE skill SET enabled=?2, updated_at=?3 WHERE id=?1", params![id, if enabled { 1i64 } else { 0 }, now()]).await?;
    get_skill(conn, id).await?.ok_or_else(|| anyhow!("skill not found after update"))
}

// ---- Saved outlines ---------------------------------------------------------

pub async fn list_outlines(conn: &Connection) -> Result<Vec<Outline>> {
    let mut rows = conn.query("SELECT id, name, bpm, sections, created_at FROM outline ORDER BY created_at DESC", ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(Outline {
            id: s(&r, 0), name: s(&r, 1), bpm: i(&r, 2),
            sections: serde_json::from_str(&s(&r, 3)).unwrap_or_default(),
            created_at: s(&r, 4),
        });
    }
    Ok(out)
}
pub async fn create_outline(conn: &Connection, name: &str, bpm: i64, sections: &[(String, i64)]) -> Result<Outline> {
    let id = new_id();
    let ts = now();
    let json = serde_json::to_string(sections).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO outline (id, name, bpm, sections, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id.clone(), name, bpm, json, ts.clone()],
    ).await?;
    Ok(Outline { id, name: name.to_string(), bpm, sections: sections.to_vec(), created_at: ts })
}
pub async fn delete_outline(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM outline WHERE id = ?1", params![id]).await?;
    Ok(())
}

// ---- Saved progressions ----------------------------------------------------

pub async fn list_progressions(conn: &Connection) -> Result<Vec<Progression>> {
    let mut rows = conn.query("SELECT id, name, chords, picks, created_at FROM progression ORDER BY created_at DESC", ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(Progression {
            id: s(&r, 0), name: s(&r, 1),
            chords: serde_json::from_str(&s(&r, 2)).unwrap_or_default(),
            picks: s(&r, 3),
            created_at: s(&r, 4),
        });
    }
    Ok(out)
}
pub async fn create_progression(conn: &Connection, name: &str, chords: &[String], picks: &str) -> Result<Progression> {
    // names are the library's identity for humans — refuse silent duplicates
    let mut dup = conn.query("SELECT id FROM progression WHERE lower(name) = lower(?1)", params![name]).await?;
    if dup.next().await?.is_some() {
        return Err(anyhow!("a progression named \"{name}\" already exists — load it and use Update, or pick another name"));
    }
    let id = new_id();
    let ts = now();
    let json = serde_json::to_string(chords).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO progression (id, name, chords, picks, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id.clone(), name, json, picks, ts.clone()],
    ).await?;
    Ok(Progression { id, name: name.to_string(), chords: chords.to_vec(), picks: picks.to_string(), created_at: ts })
}
pub async fn update_progression(conn: &Connection, id: &str, name: &str, chords: &[String], picks: &str) -> Result<Progression> {
    // renaming onto ANOTHER row's name is the same silent-duplicate trap
    let mut dup = conn.query("SELECT id FROM progression WHERE lower(name) = lower(?1) AND id != ?2", params![name, id]).await?;
    if dup.next().await?.is_some() {
        return Err(anyhow!("a different progression named \"{name}\" already exists"));
    }
    let json = serde_json::to_string(chords).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "UPDATE progression SET name=?2, chords=?3, picks=?4 WHERE id=?1",
        params![id, name, json, picks],
    ).await?;
    let mut rows = conn.query("SELECT id, name, chords, picks, created_at FROM progression WHERE id = ?1", params![id]).await?;
    let r = rows.next().await?.ok_or_else(|| anyhow!("progression not found"))?;
    Ok(Progression { id: s(&r, 0), name: s(&r, 1), chords: serde_json::from_str(&s(&r, 2)).unwrap_or_default(), picks: s(&r, 3), created_at: s(&r, 4) })
}
pub async fn delete_progression(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM progression WHERE id = ?1", params![id]).await?;
    Ok(())
}

// ---- Saved compositions (Composer sketches / full-song exports) -------------

const COMPOSITION_COLS: &str = "id, name, song_id, data, created_at, updated_at";
fn map_composition(r: &libsql::Row) -> CompositionRow {
    CompositionRow {
        id: s(r, 0), name: s(r, 1), song_id: so(r, 2), data: s(r, 3),
        created_at: s(r, 4), updated_at: s(r, 5),
    }
}

/// Light listing (no `data` blob) for the library panel / MCP, newest first.
pub async fn list_compositions(conn: &Connection) -> Result<Vec<CompositionMeta>> {
    let mut rows = conn
        .query("SELECT id, name, song_id, created_at, updated_at FROM composition ORDER BY updated_at DESC", ())
        .await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(CompositionMeta {
            id: s(&r, 0), name: s(&r, 1), song_id: so(&r, 2), created_at: s(&r, 3), updated_at: s(&r, 4),
        });
    }
    Ok(out)
}
pub async fn get_composition(conn: &Connection, id: &str) -> Result<Option<CompositionRow>> {
    let mut rows = conn
        .query(&format!("SELECT {COMPOSITION_COLS} FROM composition WHERE id = ?1"), params![id])
        .await?;
    Ok(rows.next().await?.as_ref().map(map_composition))
}
/// Upsert semantics: `id: None` inserts a new row (the caller adopts the
/// minted id for subsequent saves); `Some(id)` updates that row in place and
/// bumps `updated_at`. `data` must at least parse as JSON — the frontend
/// sends the zod-validated Composition blob; garbage is rejected here so a
/// bad MCP write can't poison the library.
pub async fn save_composition(
    conn: &Connection, id: Option<&str>, name: &str, song_id: Option<&str>, data: &str,
) -> Result<CompositionRow> {
    serde_json::from_str::<serde_json::Value>(data)
        .map_err(|e| anyhow!("composition data is not valid JSON: {e}"))?;
    let ts = now();
    match id {
        Some(id) => {
            let n = conn
                .execute(
                    "UPDATE composition SET name=?2, song_id=?3, data=?4, updated_at=?5 WHERE id=?1",
                    params![id, name, song_id, data, ts],
                )
                .await?;
            if n == 0 {
                return Err(anyhow!("composition not found"));
            }
            get_composition(conn, id).await?.ok_or_else(|| anyhow!("composition not found after update"))
        }
        None => {
            let id = new_id();
            conn.execute(
                "INSERT INTO composition (id, name, song_id, data, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![id.clone(), name, song_id, data, ts],
            )
            .await?;
            get_composition(conn, &id).await?.ok_or_else(|| anyhow!("composition not found after save"))
        }
    }
}
pub async fn delete_composition(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM composition WHERE id = ?1", params![id]).await?;
    Ok(())
}

// ---- Settings --------------------------------------------------------------

pub async fn get_settings(conn: &Connection) -> Result<Settings> {
    let mut st = Settings::default();
    let mut rows = conn.query("SELECT key, value FROM setting", ()).await?;
    while let Some(r) = rows.next().await? {
        match s(&r, 0).as_str() {
            "claude_model" => st.claude_model = s(&r, 1),
            "claude_bin" => st.claude_bin = s(&r, 1),
            "ableton_mcp" => st.ableton_mcp = s(&r, 1),
            "music_folder" => st.music_folder = s(&r, 1),
            "analyzer_cmd" => st.analyzer_cmd = s(&r, 1),
            "musicai_api_key" => st.musicai_api_key = s(&r, 1),
            "musicai_workflow" => st.musicai_workflow = s(&r, 1),
            _ => {}
        }
    }
    Ok(st)
}
pub async fn set_settings(conn: &Connection, st: &Settings) -> Result<()> {
    // one transaction: settings change as a unit, never a half-applied mix
    let tx = conn.transaction().await?;
    for (k, v) in [("claude_model", &st.claude_model), ("claude_bin", &st.claude_bin), ("ableton_mcp", &st.ableton_mcp), ("music_folder", &st.music_folder), ("analyzer_cmd", &st.analyzer_cmd), ("musicai_api_key", &st.musicai_api_key), ("musicai_workflow", &st.musicai_workflow)] {
        tx.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![k, v.as_str()],
        ).await?;
    }
    tx.commit().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use libsql::Builder;

    // libSQL `:memory:` gives each connection its OWN database, so return the
    // single connection migrated here and reuse it for the whole test.
    async fn mem_conn() -> (libsql::Database, Connection) {
        let db = Builder::new_local(":memory:").build().await.unwrap();
        let conn = db.connect().unwrap();
        migrate(&conn).await.unwrap();
        (db, conn)
    }

    /// REGRESSION (audit 2026-07-28): EVERY Settings field must round-trip.
    /// set_settings writes an explicit key list, so a field added to the
    /// struct without touching db.rs silently never persists — that is
    /// exactly how the Music.AI add-on shipped dead (you paste the key, hit
    /// Save, and it vanishes on reload).
    #[tokio::test]
    async fn settings_round_trip_every_field() {
        let (_db, conn) = mem_conn().await;
        let want = Settings {
            claude_model: "m".into(), claude_bin: "/bin/claude".into(), ableton_mcp: "{}".into(),
            music_folder: "/music".into(), analyzer_cmd: "/py /a.py".into(),
            musicai_api_key: "sk-test".into(), musicai_workflow: "lyric-transcription".into(),
        };
        set_settings(&conn, &want).await.unwrap();
        let got = get_settings(&conn).await.unwrap();
        // serde round-trip compares EVERY field — a new field fails this the
        // moment it is added to the struct without the two db.rs lists
        assert_eq!(
            serde_json::to_value(&got).unwrap(),
            serde_json::to_value(&want).unwrap(),
            "a Settings field did not persist — add it to BOTH get_settings and set_settings",
        );
    }

    // ---- Preset key/BPM seeding (BACKLOG: seed new songs from the preset) ----

    /// The brief's real-world strings: first explicit key mention wins, BPM
    /// ranges take the rounded midpoint, unparseable prose yields None.
    #[test]
    fn parse_key_tempo_real_world_strings() {
        assert_eq!(
            parse_key_tempo("Dark minor key (F minor / cowbell-friendly), ~135–145 BPM with a half-time trap feel"),
            (Some(("F".into(), "minor".into())), Some(140)),
        );
        assert_eq!(
            parse_key_tempo("60–75 BPM half-time crawl in minor keys (A minor, D minor), swung trap hi-hats"),
            (Some(("A".into(), "minor".into())), Some(68)),
            "the first key LISTED wins; the midpoint of 60–75 rounds to 68",
        );
        assert_eq!(parse_key_tempo("A minor, ~120 BPM"), (Some(("A".into(), "minor".into())), Some(120)));
        assert_eq!(parse_key_tempo("A min"), (Some(("A".into(), "minor".into())), None));
        assert_eq!(parse_key_tempo("F# major at 98 BPM"), (Some(("F#".into(), "major".into())), Some(98)));
        assert_eq!(parse_key_tempo(""), (None, None));
        assert_eq!(parse_key_tempo("dreamy, slow, cinematic"), (None, None));
        // "in a minor key" is English, not A minor; nonsense BPMs are ignored
        assert_eq!(parse_key_tempo("in a minor key, 9000 BPM"), (None, None));
    }

    /// New songs seed key/BPM from the preset's prose; unparseable prose keeps
    /// the old A-minor/120 defaults.
    #[tokio::test]
    async fn create_song_seeds_key_bpm_from_preset_feel() {
        let (_db, conn) = mem_conn().await;
        let phonk = create_preset(&conn, StyleInput {
            name: "Sinister Memphis Phonk".into(), genre: "phonk".into(), mood: String::new(),
            influences: String::new(),
            key_tempo_feel: "Dark minor key (F minor / cowbell-friendly), ~135–145 BPM with a half-time trap feel".into(),
            vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &phonk.id, "Seeded").await.unwrap();
        assert_eq!((song.key_root.as_str(), song.key_mode.as_str(), song.bpm), ("F", "minor", 140));

        let vague = create_preset(&conn, StyleInput {
            name: "Vibes".into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: "dreamy and slow".into(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &vague.id, "Default").await.unwrap();
        assert_eq!((song.key_root.as_str(), song.key_mode.as_str(), song.bpm), ("A", "minor", 120));
    }

    /// North Star: new songs start with an empty intent; `update_song_intent`
    /// round-trips through get/list and bumps `updated_at`.
    #[tokio::test]
    async fn update_song_intent_round_trips() {
        let (_db, conn) = mem_conn().await;
        let preset = create_preset(&conn, StyleInput {
            name: "P".into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &preset.id, "Finding you in the sand of time").await.unwrap();
        assert_eq!(song.intent, "", "a new song has no intent yet");

        let updated = update_song_intent(&conn, &song.id, "searching for love in desert").await.unwrap();
        assert_eq!(updated.intent, "searching for love in desert");
        assert!(updated.updated_at >= song.updated_at);
        let got = get_song(&conn, &song.id).await.unwrap().unwrap();
        assert_eq!(got.intent, "searching for love in desert");
        let listed = list_songs(&conn).await.unwrap();
        assert_eq!(listed.iter().find(|v| v.id == song.id).unwrap().intent, "searching for love in desert");
    }

    /// Revision labels (History UX): new revisions start unlabeled, a set
    /// label round-trips through get/list, sticks to ITS revision when newer
    /// ones land, and `None` clears it.
    #[tokio::test]
    async fn artifact_label_round_trips() {
        let (_db, conn) = mem_conn().await;
        let preset = create_preset(&conn, StyleInput {
            name: "P".into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &preset.id, "Labeled").await.unwrap();
        let stage = list_stages(&conn, &song.id).await.unwrap().into_iter().find(|s| s.r#type == "lyrics").unwrap();

        let v1 = save_artifact(&conn, &song.id, Some(&stage.id), "lyrics", r#"{"kind":"lyrics","text":"v1","data":null}"#).await.unwrap();
        assert_eq!(v1.label, None, "a new revision starts unlabeled");

        set_artifact_label(&conn, &v1.id, Some("first draft")).await.unwrap();
        assert_eq!(get_artifact(&conn, &v1.id).await.unwrap().unwrap().label.as_deref(), Some("first draft"));

        // a newer revision doesn't inherit the label; the list carries both correctly
        save_artifact(&conn, &song.id, Some(&stage.id), "lyrics", r#"{"kind":"lyrics","text":"v2","data":null}"#).await.unwrap();
        let revs = list_artifact_revisions(&conn, &stage.id).await.unwrap();
        assert_eq!(revs.len(), 2);
        assert_eq!(revs[0].label, None, "newest first, unlabeled");
        assert_eq!(revs[1].label.as_deref(), Some("first draft"));

        // None clears
        set_artifact_label(&conn, &v1.id, None).await.unwrap();
        assert_eq!(get_artifact(&conn, &v1.id).await.unwrap().unwrap().label, None);
    }

    /// The stored blob comes back byte-for-byte (the Composer round-trips
    /// sections/lyrics through it), and the nullable song link is kept.
    #[tokio::test]
    async fn composition_save_get_round_trips_blob_byte_for_byte() {
        let (_db, conn) = mem_conn().await;
        // key order + unicode + nesting all preserved exactly as sent
        let data = r#"{"id":"comp-1","version":3,"name":"Sketch — épreuve","key":{"root":"C","mode":"major"},"bpm":100,"bars":8,"totalTicks":128,"chords":[{"id":"s1","degree":1,"seventh":false,"start":0,"length":16,"name":"Am"}],"melody":[],"bass":[],"sections":[{"id":"sec1","name":"Verse 1","startTick":0,"lengthTicks":64}],"lyrics":[{"tick":0,"text":"city lights","words":[{"text":"city","chord":"Am"},{"text":"lights"}]}]}"#;
        let saved = save_composition(&conn, None, "Sketch", Some("song-42"), data).await.unwrap();
        assert_eq!(saved.data, data);
        assert_eq!(saved.song_id.as_deref(), Some("song-42"));

        let got = get_composition(&conn, &saved.id).await.unwrap().unwrap();
        assert_eq!(got.data, data, "blob must round-trip byte-for-byte");
        assert_eq!(got.name, "Sketch");
        assert_eq!(got.song_id.as_deref(), Some("song-42"));

        // a blank sketch has no song link — None round-trips as NULL
        let blank = save_composition(&conn, None, "Blank", None, r#"{"a":1}"#).await.unwrap();
        assert_eq!(get_composition(&conn, &blank.id).await.unwrap().unwrap().song_id, None);
    }

    /// `Some(id)` updates in place: same row (no duplicate), `updated_at`
    /// bumped, `created_at` untouched. Unknown ids are rejected.
    #[tokio::test]
    async fn composition_update_in_place_bumps_updated_at_without_duplicating() {
        let (_db, conn) = mem_conn().await;
        let first = save_composition(&conn, None, "v1", None, r#"{"n":1}"#).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;

        let second = save_composition(&conn, Some(&first.id), "v2", None, r#"{"n":2}"#).await.unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(second.name, "v2");
        assert_eq!(second.data, r#"{"n":2}"#);
        assert_eq!(second.created_at, first.created_at, "created_at is stable across updates");
        assert!(second.updated_at > first.updated_at, "updated_at must bump on update");

        let all = list_compositions(&conn).await.unwrap();
        assert_eq!(all.len(), 1, "update must not duplicate the row");

        // updating a row that doesn't exist is an error, not a silent insert
        assert!(save_composition(&conn, Some("nope"), "x", None, "{}").await.is_err());
    }

    #[tokio::test]
    async fn composition_delete_removes_row() {
        let (_db, conn) = mem_conn().await;
        let saved = save_composition(&conn, None, "gone soon", None, "{}").await.unwrap();
        delete_composition(&conn, &saved.id).await.unwrap();
        assert!(get_composition(&conn, &saved.id).await.unwrap().is_none());
        assert!(list_compositions(&conn).await.unwrap().is_empty());
    }

    /// Garbage `data` never lands in the table (insert or update path).
    #[tokio::test]
    async fn composition_rejects_invalid_json_data() {
        let (_db, conn) = mem_conn().await;
        assert!(save_composition(&conn, None, "bad", None, "not json {{").await.is_err());
        assert!(list_compositions(&conn).await.unwrap().is_empty(), "rejected saves leave no row");

        let ok = save_composition(&conn, None, "good", None, r#"{"ok":true}"#).await.unwrap();
        assert!(save_composition(&conn, Some(&ok.id), "good", None, "],garbage").await.is_err());
        let kept = get_composition(&conn, &ok.id).await.unwrap().unwrap();
        assert_eq!(kept.data, r#"{"ok":true}"#, "rejected update must not touch the row");
    }

    /// Listing is newest-first by `updated_at` — an in-place save floats the
    /// row to the top — and carries the song badge (song_id) without the blob.
    // ---- Section spine (docs/SECTION-SPINE-SPEC.md — Phase 1) ---------------

    fn blank_style(name: &str) -> StyleInput {
        StyleInput {
            name: name.into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
        }
    }
    async fn spine_song(conn: &Connection, title: &str) -> Song {
        let preset = create_preset(conn, blank_style("P")).await.unwrap();
        create_song(conn, &preset.id, title).await.unwrap()
    }
    async fn stage_of(conn: &Connection, song_id: &str, t: &str) -> Stage {
        list_stages(conn, song_id).await.unwrap().into_iter().find(|s| s.r#type == t).unwrap()
    }
    async fn save_stage_data(conn: &Connection, song_id: &str, t: &str, data: serde_json::Value) {
        let stage = stage_of(conn, song_id, t).await;
        let content = serde_json::json!({ "kind": t, "text": "", "data": data }).to_string();
        save_artifact(conn, song_id, Some(&stage.id), t, &content).await.unwrap();
    }
    async fn current_content(conn: &Connection, song_id: &str, t: &str) -> serde_json::Value {
        let stage = stage_of(conn, song_id, t).await;
        let art = current_artifact(conn, &stage.id).await.unwrap().unwrap();
        serde_json::from_str(&art.content).unwrap()
    }
    fn section_ids_of(content: &serde_json::Value, arr_key: &str) -> Vec<String> {
        content["data"][arr_key]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["section_id"].as_str().unwrap_or("<missing>").to_string())
            .collect()
    }

    /// Spec test (a): a 3-stage fixture (plus lyric_spec beats) whose labels
    /// disagree in case/spacing yields ONE spine row per section; ids are
    /// attached in every artifact (labels kept), snapshots added beside `data`;
    /// legacy revisions stay untouched and no new revision is created.
    #[tokio::test]
    async fn migrate_sections_unifies_mismatched_labels_across_stages() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "Spine A").await;
        save_stage_data(&conn, &song.id, "structure", serde_json::json!({
            "keyNote": "", "tempoNote": "",
            "sections": [
                { "type": "verse", "label": "Verse 1", "bars": 16, "role": "opens the story" },
                { "type": "chorus", "label": "Chorus 1", "bars": 8, "role": "payoff", "frozen": true },
            ],
        })).await;
        save_stage_data(&conn, &song.id, "chords", serde_json::json!({
            "sections": [
                { "label": "VERSE 1", "chords": [{ "name": "Am", "beats": 4 }] },
                { "label": "chorus  1", "chords": [{ "name": "F", "beats": 4 }] },
            ],
        })).await;
        save_stage_data(&conn, &song.id, "lyric_spec", serde_json::json!({
            "hook": "H",
            "beats": [
                { "section": "verse 1", "beat": "set the scene" },
                { "section": "Chorus 1", "beat": "the payoff" },
            ],
        })).await;
        // two lyrics revisions — the migration must rewrite ONLY the current one
        save_stage_data(&conn, &song.id, "lyrics", serde_json::json!({
            "sections": [{ "label": "Verse 1", "lines": ["old draft"] }],
        })).await;
        save_stage_data(&conn, &song.id, "lyrics", serde_json::json!({
            "sections": [
                { "label": "verse 1", "lines": ["la la"] },
                { "label": "CHORUS 1", "lines": ["oh oh"] },
            ],
        })).await;

        migrate_sections(&conn).await.unwrap();

        // one spine row per section, in structure order, form fields carried
        let spine = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(spine.len(), 2, "mismatched-case labels must collapse to one row each");
        assert_eq!(
            (spine[0].position, spine[0].label.as_str(), spine[0].r#type.as_str(), spine[0].bars, spine[0].role.as_str()),
            (0, "Verse 1", "verse", 16, "opens the story"),
        );
        assert_eq!(
            (spine[1].position, spine[1].label.as_str(), spine[1].r#type.as_str(), spine[1].bars, spine[1].role.as_str()),
            (1, "Chorus 1", "chorus", 8, "payoff"),
        );

        // ids attached in all four artifacts; labels + frozen flags untouched
        let expect = vec![spine[0].id.clone(), spine[1].id.clone()];
        for (stage, arr_key) in [("structure", "sections"), ("chords", "sections"), ("lyric_spec", "beats"), ("lyrics", "sections")] {
            let content = current_content(&conn, &song.id, stage).await;
            assert_eq!(section_ids_of(&content, arr_key), expect, "{stage} must key every entry to the spine");
            // the snapshot sits BESIDE data and mirrors the spine
            let snap = content["spine_snapshot"].as_array().unwrap();
            assert_eq!(snap.len(), 2);
            assert_eq!(snap[0]["section_id"], serde_json::json!(spine[0].id));
            assert_eq!(snap[1]["label"], serde_json::json!("Chorus 1"));
            assert_eq!(snap[1]["position"], serde_json::json!(1));
        }
        let structure = current_content(&conn, &song.id, "structure").await;
        assert_eq!(structure["data"]["sections"][0]["label"], serde_json::json!("Verse 1"), "labels stay (Phase-1 readers use them)");
        assert_eq!(structure["data"]["sections"][1]["frozen"], serde_json::json!(true), "freeze flags stay in stage data");
        let chords = current_content(&conn, &song.id, "chords").await;
        assert_eq!(chords["data"]["sections"][0]["label"], serde_json::json!("VERSE 1"), "stage labels are not rewritten");
        let spec = current_content(&conn, &song.id, "lyric_spec").await;
        assert_eq!(spec["data"]["beats"][0]["section"], serde_json::json!("verse 1"));

        // in-place rewrite: still 2 lyrics revisions; the legacy v1 untouched
        let lyrics_stage = stage_of(&conn, &song.id, "lyrics").await;
        let revs = list_artifact_revisions(&conn, &lyrics_stage.id).await.unwrap();
        assert_eq!(revs.len(), 2, "migration must not create a new revision");
        let v1: serde_json::Value = serde_json::from_str(&revs[1].content).unwrap();
        assert!(v1.get("spine_snapshot").is_none(), "legacy revisions are NOT rewritten");
    }

    /// Spec test (b): a section that exists only in a later stage (the classic
    /// lyrics-only Bridge) is unioned into the spine, appended at the end with
    /// defaults, and its lyrics entry gets the new id.
    #[tokio::test]
    async fn migrate_sections_unions_lyrics_only_bridge_at_end() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "Spine B").await;
        save_stage_data(&conn, &song.id, "structure", serde_json::json!({
            "sections": [
                { "type": "verse", "label": "Verse 1", "bars": 12, "role": "" },
                { "type": "chorus", "label": "Chorus", "bars": 8, "role": "" },
            ],
        })).await;
        save_stage_data(&conn, &song.id, "lyrics", serde_json::json!({
            "sections": [
                { "label": "Verse 1", "lines": ["a"] },
                { "label": "Chorus", "lines": ["b"] },
                { "label": "Bridge", "lines": ["c"] },
            ],
        })).await;

        migrate_sections(&conn).await.unwrap();

        let spine = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(
            spine.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(),
            vec!["Verse 1", "Chorus", "Bridge"],
        );
        let bridge = &spine[2];
        assert_eq!((bridge.position, bridge.bars, bridge.r#type.as_str(), bridge.role.as_str()), (2, 8, "", ""), "unioned sections take defaults");
        let lyrics = current_content(&conn, &song.id, "lyrics").await;
        assert_eq!(lyrics["data"]["sections"][2]["section_id"], serde_json::json!(bridge.id));
        // the structure snapshot still mirrors the WHOLE spine (incl. Bridge)
        let structure = current_content(&conn, &song.id, "structure").await;
        assert_eq!(structure["spine_snapshot"].as_array().unwrap().len(), 3);
    }

    /// Spec test (c): with no structure artifact the spine seeds from the
    /// Chords artifact's order; lyrics-only sections still union in after.
    #[tokio::test]
    async fn migrate_sections_without_structure_falls_back_to_chords_order() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "Spine C").await;
        save_stage_data(&conn, &song.id, "chords", serde_json::json!({
            "sections": [
                { "label": "Intro", "chords": [{ "name": "Am", "beats": 4 }] },
                { "label": "Verse 1", "chords": [{ "name": "F", "beats": 4 }] },
            ],
        })).await;
        save_stage_data(&conn, &song.id, "lyrics", serde_json::json!({
            "sections": [
                { "label": "Verse 1", "lines": ["a"] },
                { "label": "Outro", "lines": ["b"] },
            ],
        })).await;

        migrate_sections(&conn).await.unwrap();

        let spine = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(
            spine.iter().map(|x| (x.label.as_str(), x.bars)).collect::<Vec<_>>(),
            vec![("Intro", 8), ("Verse 1", 8), ("Outro", 8)],
            "chords order seeds the spine; lyrics-only Outro unions in at the end",
        );
        let chords = current_content(&conn, &song.id, "chords").await;
        assert_eq!(section_ids_of(&chords, "sections"), vec![spine[0].id.clone(), spine[1].id.clone()]);
        let lyrics = current_content(&conn, &song.id, "lyrics").await;
        assert_eq!(section_ids_of(&lyrics, "sections"), vec![spine[1].id.clone(), spine[2].id.clone()]);
    }

    /// Spec test (d): the migration is idempotent — a second run leaves rows
    /// and artifact contents byte-identical (songs with spine rows are skipped).
    #[tokio::test]
    async fn migrate_sections_second_run_is_a_no_op() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "Spine D").await;
        save_stage_data(&conn, &song.id, "structure", serde_json::json!({
            "sections": [{ "type": "verse", "label": "Verse 1", "bars": 8, "role": "" }],
        })).await;
        save_stage_data(&conn, &song.id, "lyrics", serde_json::json!({
            "sections": [{ "label": "Verse 1", "lines": ["a"] }, { "label": "Bridge", "lines": ["b"] }],
        })).await;

        migrate_sections(&conn).await.unwrap();
        let spine1 = list_sections(&conn, &song.id).await.unwrap();
        let structure1 = current_content(&conn, &song.id, "structure").await;
        let lyrics1 = current_content(&conn, &song.id, "lyrics").await;

        migrate_sections(&conn).await.unwrap();
        let spine2 = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(spine2.len(), spine1.len());
        assert_eq!(
            spine2.iter().map(|x| (x.id.clone(), x.position, x.label.clone())).collect::<Vec<_>>(),
            spine1.iter().map(|x| (x.id.clone(), x.position, x.label.clone())).collect::<Vec<_>>(),
            "row identity must survive re-runs",
        );
        assert_eq!(current_content(&conn, &song.id, "structure").await, structure1);
        assert_eq!(current_content(&conn, &song.id, "lyrics").await, lyrics1);
    }

    /// Spec test (e): spine CRUD round-trips — append + positioned insert,
    /// form update, delete compacts positions.
    #[tokio::test]
    async fn section_crud_round_trips() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "CRUD").await;

        let verse = create_section(&conn, &song.id, "Verse 1", "verse", 16, "opens", None).await.unwrap();
        let chorus = create_section(&conn, &song.id, "Chorus", "chorus", 8, "payoff", None).await.unwrap();
        assert_eq!((verse.position, chorus.position), (0, 1), "None appends at the end");
        // positioned insert shifts later sections down; bars clamp to >= 1
        let intro = create_section(&conn, &song.id, "Intro", "", 0, "", Some(0)).await.unwrap();
        assert_eq!((intro.position, intro.bars), (0, 1));
        let labels = |secs: &[Section]| secs.iter().map(|x| x.label.clone()).collect::<Vec<_>>();
        let spine = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(labels(&spine), vec!["Intro", "Verse 1", "Chorus"]);
        assert_eq!(spine.iter().map(|x| x.position).collect::<Vec<_>>(), vec![0, 1, 2]);

        // update touches FORM only (label/type/bars/role) and bumps updated_at
        let updated = update_section(&conn, &verse.id, "Verse One", "verse", 12, "sets the scene").await.unwrap();
        assert_eq!(
            (updated.label.as_str(), updated.r#type.as_str(), updated.bars, updated.role.as_str(), updated.position),
            ("Verse One", "verse", 12, "sets the scene", 1),
        );
        assert!(updated.updated_at >= verse.updated_at);
        assert_eq!(get_section(&conn, &verse.id).await.unwrap().unwrap().label, "Verse One");

        // delete closes the position gap
        delete_section(&conn, &verse.id).await.unwrap();
        let spine = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(labels(&spine), vec!["Intro", "Chorus"]);
        assert_eq!(spine.iter().map(|x| x.position).collect::<Vec<_>>(), vec![0, 1]);
        assert!(get_section(&conn, &verse.id).await.unwrap().is_none());
        assert!(delete_section(&conn, &verse.id).await.is_err(), "deleting a missing section is an error");
    }

    /// Spec test (e, reorder): a full permutation reorders; anything else
    /// (wrong count, duplicate, foreign id) is rejected without changes.
    #[tokio::test]
    async fn reorder_sections_requires_a_full_permutation() {
        let (_db, conn) = mem_conn().await;
        let song = spine_song(&conn, "Reorder").await;
        let a = create_section(&conn, &song.id, "A", "", 8, "", None).await.unwrap();
        let b = create_section(&conn, &song.id, "B", "", 8, "", None).await.unwrap();
        let c = create_section(&conn, &song.id, "C", "", 8, "", None).await.unwrap();

        let spine = reorder_sections(&conn, &song.id, &[c.id.clone(), a.id.clone(), b.id.clone()]).await.unwrap();
        assert_eq!(spine.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["C", "A", "B"]);
        assert_eq!(spine.iter().map(|x| x.position).collect::<Vec<_>>(), vec![0, 1, 2]);

        assert!(reorder_sections(&conn, &song.id, &[a.id.clone(), b.id.clone()]).await.is_err(), "missing an id");
        assert!(reorder_sections(&conn, &song.id, &[a.id.clone(), a.id.clone(), b.id.clone()]).await.is_err(), "duplicate id");
        assert!(reorder_sections(&conn, &song.id, &[a.id.clone(), b.id.clone(), "nope".into()]).await.is_err(), "foreign id");
        let unchanged = list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(unchanged.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["C", "A", "B"], "rejected reorders change nothing");
    }

    #[tokio::test]
    async fn composition_list_orders_newest_first() {
        let (_db, conn) = mem_conn().await;
        let a = save_composition(&conn, None, "older", None, "{}").await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        let b = save_composition(&conn, None, "newer", Some("song-1"), "{}").await.unwrap();

        let list = list_compositions(&conn).await.unwrap();
        assert_eq!(list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec![b.id.as_str(), a.id.as_str()]);
        assert_eq!(list[0].song_id.as_deref(), Some("song-1"));

        // updating the older one floats it to the top
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        save_composition(&conn, Some(&a.id), "older", None, "{}").await.unwrap();
        let list = list_compositions(&conn).await.unwrap();
        assert_eq!(list[0].id, a.id);
    }
}
