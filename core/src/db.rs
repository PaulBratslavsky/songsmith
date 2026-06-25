//! libSQL (SQLite-compatible) persistence: schema, migrations, seed, and CRUD.
//!
//! The Rust core owns all state. Claude (via the agent loop) and the UI both go
//! through these functions.

use crate::models::*;
use anyhow::Result;
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
    let conn = db.connect()?;
    migrate(&conn).await?;
    seed_skills(&conn).await?;
    Ok(db)
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
        CREATE TABLE IF NOT EXISTS render (
          id TEXT PRIMARY KEY, song_id TEXT NOT NULL REFERENCES song(id),
          label TEXT, file_path TEXT NOT NULL, source TEXT, notes TEXT,
          is_pick INTEGER NOT NULL DEFAULT 0, created_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_render_song ON render(song_id);
        CREATE TABLE IF NOT EXISTS setting (key TEXT PRIMARY KEY, value TEXT);
        "#,
    )
    .await?;
    // columns added after v0.1 — idempotent (errors if already present, ignored)
    let _ = conn.execute("ALTER TABLE song ADD COLUMN voicings TEXT NOT NULL DEFAULT '{}'", ()).await;
    // retrofit the Lyric Spec stage (added between Chords and Lyrics) into existing
    // songs that predate it — make room by shifting Lyrics/Prompt, then insert. Idempotent.
    let _ = conn.execute(
        "UPDATE stage SET ordinal = ordinal + 1 WHERE type IN ('lyrics','prompt') \
         AND song_id NOT IN (SELECT song_id FROM stage WHERE type='lyric_spec')", ()).await;
    let _ = conn.execute(
        "INSERT INTO stage (id, song_id, type, ordinal, status, created_at, updated_at) \
         SELECT lower(hex(randomblob(16))), s.id, 'lyric_spec', 3, 'pending', \
                strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now') \
         FROM song s WHERE s.id NOT IN (SELECT song_id FROM stage WHERE type='lyric_spec')", ()).await;
    Ok(())
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
        let mut rows = conn.query("SELECT source FROM skill WHERE key = ?1", params![*key]).await?;
        if let Some(r) = rows.next().await? {
            // refresh untouched builtins to the latest embedded version; leave user-edited ones
            if s(&r, 0) == "builtin" {
                conn.execute(
                    "UPDATE skill SET name=?2, stage_type=?3, instructions=?4, updated_at=?5 WHERE key=?1 AND source='builtin'",
                    params![*key, *name, *stage_type, strip_frontmatter(body), ts],
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
    "id, name, genre, mood, influences, key_tempo_feel, vocal_range, themes, created_at, updated_at";
fn map_preset(r: &libsql::Row) -> StylePreset {
    StylePreset {
        id: s(r, 0), name: s(r, 1), genre: s(r, 2), mood: s(r, 3), influences: s(r, 4),
        key_tempo_feel: s(r, 5), vocal_range: s(r, 6), themes: s(r, 7), created_at: s(r, 8), updated_at: s(r, 9),
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
        "INSERT INTO style_preset (id, name, genre, mood, influences, key_tempo_feel, vocal_range, themes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![id.clone(), p.name, p.genre, p.mood, p.influences, p.key_tempo_feel, p.vocal_range, p.themes, ts],
    ).await?;
    Ok(get_preset(conn, &id).await?.unwrap())
}
pub async fn update_preset(conn: &Connection, id: &str, p: StyleInput) -> Result<StylePreset> {
    conn.execute(
        "UPDATE style_preset SET name=?2, genre=?3, mood=?4, influences=?5, key_tempo_feel=?6, vocal_range=?7, themes=?8, updated_at=?9 WHERE id=?1",
        params![id, p.name, p.genre, p.mood, p.influences, p.key_tempo_feel, p.vocal_range, p.themes, now()],
    ).await?;
    Ok(get_preset(conn, id).await?.unwrap())
}

// ---- Songs & stages --------------------------------------------------------

const SONG_COLS: &str =
    "id, style_preset_id, title, status, current_stage, key_root, key_mode, bpm, created_at, updated_at, voicings";
fn map_song(r: &libsql::Row) -> Song {
    let voicings = so(r, 10).filter(|v| !v.is_empty()).unwrap_or_else(|| "{}".into());
    Song {
        id: s(r, 0), style_preset_id: s(r, 1), title: s(r, 2), status: s(r, 3), current_stage: s(r, 4),
        key_root: s(r, 5), key_mode: s(r, 6), bpm: i(r, 7), created_at: s(r, 8), updated_at: s(r, 9), voicings,
    }
}
// includes the current artifact's timestamp (max-version) as a correlated subquery
const STAGE_SELECT: &str = "SELECT id, song_id, type, ordinal, status, skill_id, created_at, updated_at, \
    (SELECT created_at FROM artifact WHERE stage_id = stage.id ORDER BY version DESC LIMIT 1) AS artifact_at FROM stage";
fn map_stage(r: &libsql::Row) -> Stage {
    Stage {
        id: s(r, 0), song_id: s(r, 1), r#type: s(r, 2), ordinal: i(r, 3), status: s(r, 4),
        skill_id: so(r, 5), created_at: s(r, 6), updated_at: s(r, 7), artifact_at: so(r, 8),
    }
}

pub async fn create_song(conn: &Connection, preset_id: &str, title: &str) -> Result<Song> {
    let id = new_id();
    let ts = now();
    conn.execute(
        "INSERT INTO song (id, style_preset_id, title, status, current_stage, key_root, key_mode, bpm, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'in_progress', 'concept', 'A', 'minor', 120, ?4, ?4)",
        params![id.clone(), preset_id, title, ts.clone()],
    ).await?;
    for (ordinal, stage_type) in STAGE_ORDER.iter().enumerate() {
        conn.execute(
            "INSERT INTO stage (id, song_id, type, ordinal, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?5)",
            params![new_id(), id.clone(), *stage_type, ordinal as i64, ts.clone()],
        ).await?;
    }
    Ok(get_song(conn, &id).await?.unwrap())
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
    Ok(get_song(conn, id).await?.unwrap())
}
pub async fn update_song_title(conn: &Connection, id: &str, title: &str) -> Result<Song> {
    conn.execute("UPDATE song SET title=?2, updated_at=?3 WHERE id=?1", params![id, title, now()]).await?;
    Ok(get_song(conn, id).await?.unwrap())
}
pub async fn update_song_voicings(conn: &Connection, id: &str, voicings: &str) -> Result<Song> {
    conn.execute("UPDATE song SET voicings=?2, updated_at=?3 WHERE id=?1", params![id, voicings, now()]).await?;
    Ok(get_song(conn, id).await?.unwrap())
}
pub async fn update_song_key(conn: &Connection, id: &str, root: &str, mode: &str, bpm: i64) -> Result<Song> {
    conn.execute("UPDATE song SET key_root=?2, key_mode=?3, bpm=?4, updated_at=?5 WHERE id=?1", params![id, root, mode, bpm, now()]).await?;
    Ok(get_song(conn, id).await?.unwrap())
}
pub async fn set_song_current_stage(conn: &Connection, id: &str, stage_type: &str) -> Result<()> {
    conn.execute("UPDATE song SET current_stage=?2, updated_at=?3 WHERE id=?1", params![id, stage_type, now()]).await?;
    Ok(())
}
pub async fn delete_song(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM artifact WHERE song_id = ?1", params![id]).await?;
    conn.execute("DELETE FROM stage WHERE song_id = ?1", params![id]).await?;
    conn.execute("DELETE FROM render WHERE song_id = ?1", params![id]).await?;
    conn.execute("DELETE FROM song WHERE id = ?1", params![id]).await?;
    Ok(())
}

// ---- Final renders (audio versions referenced on disk) ---------------------

pub async fn list_renders(conn: &Connection, song_id: &str) -> Result<Vec<Render>> {
    let mut rows = conn.query(
        "SELECT id, song_id, label, file_path, source, notes, is_pick, created_at FROM render WHERE song_id = ?1 ORDER BY created_at DESC",
        params![song_id],
    ).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(Render {
            id: s(&r, 0), song_id: s(&r, 1), label: s(&r, 2), file_path: s(&r, 3),
            source: s(&r, 4), notes: s(&r, 5), is_pick: i(&r, 6) != 0, created_at: s(&r, 7),
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
    Ok(Render { id, song_id: song_id.into(), label: label.into(), file_path: file_path.into(), source: source.into(), notes: notes.into(), is_pick: false, created_at: ts })
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
    Ok(Some(StageDetail { stage, artifact, skill }))
}

// ---- Artifacts (journaled) -------------------------------------------------

const ARTIFACT_COLS: &str = "id, song_id, stage_id, kind, content, version, approved, created_at";
fn map_artifact(r: &libsql::Row) -> Artifact {
    Artifact {
        id: s(r, 0), song_id: s(r, 1), stage_id: so(r, 2), kind: s(r, 3), content: s(r, 4),
        version: i(r, 5), approved: i(r, 6) != 0, created_at: s(r, 7),
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
    let next_version = match stage_id {
        Some(sid) => current_artifact(conn, sid).await?.map(|a| a.version + 1).unwrap_or(1),
        None => 1,
    };
    let id = new_id();
    conn.execute(
        "INSERT INTO artifact (id, song_id, stage_id, kind, content, version, approved, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![id.clone(), song_id, stage_id, kind, content, next_version, now()],
    ).await?;
    Ok(get_artifact(conn, &id).await?.unwrap())
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
pub async fn revert_artifact(conn: &Connection, artifact_id: &str) -> Result<Artifact> {
    let t = get_artifact(conn, artifact_id).await?.ok_or_else(|| anyhow::anyhow!("artifact not found"))?;
    save_artifact(conn, &t.song_id, t.stage_id.as_deref(), &t.kind, &t.content).await
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
    let mut rows = conn.query(
        &format!("SELECT {SKILL_COLS} FROM skill WHERE stage_type = ?1 AND enabled = 1 ORDER BY updated_at DESC LIMIT 1"),
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
    Ok(get_skill(conn, &id).await?.unwrap())
}
pub async fn update_skill(conn: &Connection, id: &str, input: SkillInput) -> Result<Skill> {
    // mark as user-owned so the builtin refresh on startup won't overwrite the edit
    conn.execute(
        "UPDATE skill SET key=?2, name=?3, stage_type=?4, instructions=?5, source='user', updated_at=?6 WHERE id=?1",
        params![id, input.key, input.name, input.stage_type, input.instructions, now()],
    ).await?;
    Ok(get_skill(conn, id).await?.unwrap())
}
pub async fn set_skill_enabled(conn: &Connection, id: &str, enabled: bool) -> Result<Skill> {
    conn.execute("UPDATE skill SET enabled=?2, updated_at=?3 WHERE id=?1", params![id, if enabled { 1i64 } else { 0 }, now()]).await?;
    Ok(get_skill(conn, id).await?.unwrap())
}

// ---- Saved progressions ----------------------------------------------------

pub async fn list_progressions(conn: &Connection) -> Result<Vec<Progression>> {
    let mut rows = conn.query("SELECT id, name, chords, created_at FROM progression ORDER BY created_at DESC", ()).await?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().await? {
        out.push(Progression {
            id: s(&r, 0), name: s(&r, 1),
            chords: serde_json::from_str(&s(&r, 2)).unwrap_or_default(),
            created_at: s(&r, 3),
        });
    }
    Ok(out)
}
pub async fn create_progression(conn: &Connection, name: &str, chords: &[String]) -> Result<Progression> {
    let id = new_id();
    let ts = now();
    let json = serde_json::to_string(chords).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO progression (id, name, chords, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![id.clone(), name, json, ts.clone()],
    ).await?;
    Ok(Progression { id, name: name.to_string(), chords: chords.to_vec(), created_at: ts })
}
pub async fn delete_progression(conn: &Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM progression WHERE id = ?1", params![id]).await?;
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
            "mcp_token" => st.mcp_token = s(&r, 1),
            "ableton_mcp" => st.ableton_mcp = s(&r, 1),
            "music_folder" => st.music_folder = s(&r, 1),
            "analyzer_cmd" => st.analyzer_cmd = s(&r, 1),
            _ => {}
        }
    }
    Ok(st)
}
pub async fn set_settings(conn: &Connection, st: &Settings) -> Result<()> {
    for (k, v) in [("claude_model", &st.claude_model), ("claude_bin", &st.claude_bin), ("mcp_token", &st.mcp_token), ("ableton_mcp", &st.ableton_mcp), ("music_folder", &st.music_folder), ("analyzer_cmd", &st.analyzer_cmd)] {
        conn.execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=?2",
            params![k, v.as_str()],
        ).await?;
    }
    Ok(())
}
pub async fn ensure_mcp_token(conn: &Connection) -> Result<String> {
    let st = get_settings(conn).await?;
    if !st.mcp_token.is_empty() { return Ok(st.mcp_token); }
    let token = new_id().replace('-', "");
    conn.execute(
        "INSERT INTO setting (key, value) VALUES ('mcp_token', ?1) ON CONFLICT(key) DO UPDATE SET value=?1",
        params![token.clone()],
    ).await?;
    Ok(token)
}
