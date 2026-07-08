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
        "#,
    )
    .await?;
    // columns added after v0.1 — idempotent (errors if already present, ignored)
    let _ = conn.execute("ALTER TABLE song ADD COLUMN voicings TEXT NOT NULL DEFAULT '{}'", ()).await;
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
    get_preset(conn, &id).await?.ok_or_else(|| anyhow!("preset not found after create"))
}
pub async fn update_preset(conn: &Connection, id: &str, p: StyleInput) -> Result<StylePreset> {
    conn.execute(
        "UPDATE style_preset SET name=?2, genre=?3, mood=?4, influences=?5, key_tempo_feel=?6, vocal_range=?7, themes=?8, updated_at=?9 WHERE id=?1",
        params![id, p.name, p.genre, p.mood, p.influences, p.key_tempo_feel, p.vocal_range, p.themes, now()],
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
        "INSERT INTO song (id, style_preset_id, title, status, current_stage, key_root, key_mode, bpm, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'in_progress', 'concept', ?4, ?5, ?6, ?7, ?7)",
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
    tx.execute("DELETE FROM artifact WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM stage WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM render WHERE song_id = ?1", params![id]).await?;
    tx.execute("DELETE FROM song WHERE id = ?1", params![id]).await?;
    tx.commit().await?;
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
pub async fn revert_artifact(conn: &Connection, artifact_id: &str) -> Result<Artifact> {
    let t = get_artifact(conn, artifact_id).await?.ok_or_else(|| anyhow!("artifact not found"))?;
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
            _ => {}
        }
    }
    Ok(st)
}
pub async fn set_settings(conn: &Connection, st: &Settings) -> Result<()> {
    // one transaction: settings change as a unit, never a half-applied mix
    let tx = conn.transaction().await?;
    for (k, v) in [("claude_model", &st.claude_model), ("claude_bin", &st.claude_bin), ("ableton_mcp", &st.ableton_mcp), ("music_folder", &st.music_folder), ("analyzer_cmd", &st.analyzer_cmd)] {
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
            vocal_range: String::new(), themes: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &phonk.id, "Seeded").await.unwrap();
        assert_eq!((song.key_root.as_str(), song.key_mode.as_str(), song.bpm), ("F", "minor", 140));

        let vague = create_preset(&conn, StyleInput {
            name: "Vibes".into(), genre: String::new(), mood: String::new(), influences: String::new(),
            key_tempo_feel: "dreamy and slow".into(), vocal_range: String::new(), themes: String::new(),
        }).await.unwrap();
        let song = create_song(&conn, &vague.id, "Default").await.unwrap();
        assert_eq!((song.key_root.as_str(), song.key_mode.as_str(), song.bpm), ("A", "minor", 120));
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
