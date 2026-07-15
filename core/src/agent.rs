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

// Split modules (audit Tier-2 #11): `engine` drives the Claude CLI, `render`
// holds the per-stage text renderers, `freeze` owns the per-section lock and
// its write boundary. Re-exported here so `agent::X` call sites keep compiling.
pub use crate::engine::{extract_json, CancelToken};
pub use crate::freeze::{merge_frozen_sections, revert_artifact_guarded, save_artifact_guarded};
use crate::engine::call_claude;
use crate::freeze::{frozen_labels, frozen_prompt_block, norm_label, section_label};
use crate::render::{chords_editor_text, lyrics_text, render_stage_text};
#[cfg(test)]
use crate::render::structure_editor_text; // legacy-shaped test fixtures still render with it

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
    let mut song = db::get_song(conn, &stage.song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let preset = db::get_preset(conn, &song.style_preset_id).await?.ok_or_else(|| anyhow!("style preset not found"))?;
    let skill = db::get_active_skill_for_stage(conn, &stage.r#type)
        .await?
        .ok_or_else(|| anyhow!("no enabled skill for stage '{}'", stage.r#type))?;

    // North Star: a Concept-stage seed IS the producer's intent — persist it on
    // the song BEFORE building prompts so every later stage and regeneration
    // sees it. Concept only; only when empty — never overwrite a user-set intent.
    if stage.r#type == "concept" && song.intent.trim().is_empty() {
        if let Some(seed) = user_input.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            song = db::update_song_intent(conn, &song.id, seed).await?;
        }
    }

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

        let system = build_system_prompt(&skill, &preset, &song);
        let mut user = stage_user_prompt(conn, &stage, user_input.as_deref()).await?;
        if let Some(pd) = &prior_data {
            if let Some(block) = frozen_prompt_block(&stage.r#type, pd) {
                user.push_str("\n\n");
                user.push_str(&block);
            }
        }

        let text = call_claude(settings, &system, &user, &on_token, cancel.as_ref()).await?;
        // Phase-3 write pipeline (docs/SECTION-SPINE-SPEC.md): frozen merge
        // (id-first) + spine reconciliation (structure creates/renames/deletes;
        // other stages map labels → section ids, dropping invented ones) +
        // key/tempo enforcement. Byte-identical to the legacy path for songs
        // without spine rows.
        let content = crate::spine::build_run_content(conn, &song, &stage.r#type, &text, prior_artifact.as_ref().map(|a| a.content.as_str())).await?;

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
        // the canonical SECTIONS block (spine) — "" for legacy songs, keeping
        // the self-check prompt byte-identical without spine rows
        let sections = spine_sections_block(conn, &stage.song_id).await?;
        let sections_block = if sections.is_empty() { String::new() } else { format!("{sections}\n\n") };
        let prior = gather_prior_context(conn, &stage.song_id, stage.ordinal).await?;
        // Reverse context (Feature B2 #1) — empty for a normal forward song, so
        // this prompt stays byte-identical when no later artifacts exist.
        let later = gather_later_context(conn, &stage.song_id, stage.ordinal).await?;
        // a self-check is by definition a regeneration — later stages are reference only
        let later_block = if later.is_empty() { String::new() } else { format!("{LATER_STAGES_REGEN_BANNER}\n\n{later}\n\n") };
        // lyrics stage: the same computed TECHNICAL BRIEF the run saw ("" otherwise)
        let tech_brief = lyrics_brief_for_stage(conn, &stage).await?;
        let brief_block = if tech_brief.is_empty() { String::new() } else { format!("{tech_brief}\n\n") };
        let system = build_system_prompt(&skill, &preset, &song);
        let checks = "SELF-TEST then REVISE. You wrote the output below. Audit it hard and rewrite it, fixing every issue you find:\n\
0. INTENT: does the output serve the song's title and the producer's intent (THE SONG block above)? If it drifted into a different song, rewrite toward the brief.\n\
1. TITLE/HOOK: does the song's title (from the Concept) actually land as the chorus hook line? If not, work it in so the chorus sings the title. ONLY if the song clearly found a stronger, more specific hook, build the chorus around that instead and keep it consistent across every chorus.\n\
2. COHERENCE: every section must fit the Lyric Spec's beat sheet and the concept — no section that drifts off-theme, contradicts the story, or repeats instead of develops.\n\
3. CRAFT: cut clichés, weak \"to be\" verbs, abstract emotion-words, forced rhymes, and over-written \"poetic\" lines that no one would actually sing; keep it human and singable. Hunt UNEARNED images: personified objects/weather, chained aphorisms (X's got Y and Y's got Z), abstractions doing physical verbs — an image must grow from the song's established world AND sound like a person; a line that only works as isolated poetry gets rewritten plain.\n\
4. THE STRANGER TEST (for lyric content): reading ONLY the words, a first-time listener must be able to follow one clear story, section by section. Rewrite any line that depends on outside knowledge, any motif used before it's established in plain language, and any pronoun without an obvious referent.\n\
5. WORDS, NOT ARRANGEMENT (for lyric content): parenthetical cues are capped at 2-3 in the whole song and must be short vocal-delivery cues only — a parenthetical is NEVER a substitute for a sung line, and production/FX directions ((melisma), (pitched down), (static)…) do not belong in lyrics; move that intent to the Generation Prompt stage's notes instead.\n\
6. Preserve the inline [chord] tags and the section labels exactly.\n\
Return ONLY the revised result as the single fenced ```json block your skill specifies — no commentary.";
        let mut user = format!(
            "{sections_block}Prior stages (context):\n\n{prior}\n\n{later_block}{brief_block}---\nYOUR CURRENT {} OUTPUT TO SELF-TEST AND REVISE:\n\n{cur_text}\n\n{checks}",
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
        // same Phase-3 write pipeline as run_stage (spine reconciliation + frozen merge)
        let content = crate::spine::build_run_content(conn, &song, &stage.r#type, &text, Some(current.content.as_str())).await?;
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
        // deliberately left empty — exemplars are the USER's taste lever, not
        // something the generator should invent
        lyric_exemplars: String::new(),
    })
}

/// A stage artifact's context body. Prefer rendering the structured `data` over
/// trusting the stored `text`: Claude-originated saves historically wrote
/// commentary into `text` while the real content lived in `data` — feeding a
/// changelog to downstream stages (e.g. the Generation Prompt saw "v4 —
/// reconciled labels…" instead of the lyrics). render(data) is the ground
/// truth when it exists.
fn artifact_context_body(stage_type: &str, content: &str) -> String {
    let parsed = serde_json::from_str::<Value>(content).ok();
    parsed
        .as_ref()
        .and_then(|v| v.get("data"))
        .and_then(|d| render_stage_text(stage_type, d))
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|v| v.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
        })
        .unwrap_or_else(|| content.to_string())
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
            blocks.push(format!("### {} output\n{}", stage_label(&s.r#type), artifact_context_body(&s.r#type, &art.content)));
        }
    }
    Ok(blocks.join("\n\n"))
}

/// Reverse context (spec Feature B2 #1): already-written LATER stages (ordinal
/// > current, non-empty content) — a lyrics-first import fills Lyrics/Structure
/// before Concept ever runs, and without this the pipeline is forward-only and
/// Concept INVENTS an unrelated song. Empty for a normal forward song, so
/// prompts stay byte-identical when no later artifacts exist.
async fn gather_later_context(conn: &Connection, song_id: &str, ordinal: i64) -> Result<String> {
    let stages = db::list_stages(conn, song_id).await?;
    let mut blocks = Vec::new();
    for s in stages.into_iter().filter(|s| s.ordinal > ordinal) {
        if let Some(art) = db::current_artifact(conn, &s.id).await? {
            let body = artifact_context_body(&s.r#type, &art.content);
            if !body.trim().is_empty() {
                blocks.push(format!("### {} output\n{}", stage_label(&s.r#type), body));
            }
        }
    }
    Ok(blocks.join("\n\n"))
}

/// The banner over reverse (later-stage) context in stage prompts.
const LATER_STAGES_BANNER: &str = "ALREADY-WRITTEN LATER STAGES (this song was imported lyrics-first) — DERIVE this stage FROM them; stay consistent; do not contradict or invent a different song.";

/// Used instead of `LATER_STAGES_BANNER` when the stage being run ALREADY HAS an
/// artifact (a regeneration). Derive-from-later is right when filling an empty
/// stage, but on a regeneration it locks in exactly what the user is trying to
/// improve — real Claude dutifully copied a stale Generation Prompt's old lyrics
/// back verbatim under the derive banner (caught in live skill testing).
const LATER_STAGES_REGEN_BANNER: &str = "LATER STAGES ALREADY EXIST (reference only — they may be OUTDATED). You are REGENERATING this stage: write a fresh, improved version per your skill and the earlier stages. Do NOT copy this content back from the later stages' rendition of it; downstream stages will be re-run afterwards. Only borrow from them what is genuinely settled (the song's identity, story, and hook).";

/// The computed TECHNICAL BRIEF for a song's LYRICS stage (run + self-check):
/// the section SPINE (bars/roles, when the song has one) + real Structure/
/// Chords `data` + the song's BPM → per-section bar / chord-change / line
/// budgets (see `render::lyrics_technical_brief`). Empty when the stage isn't
/// lyrics or the song has no spine AND no structure yet — prompt unchanged.
async fn lyrics_brief_for_stage(conn: &Connection, stage: &Stage) -> Result<String> {
    if stage.r#type != "lyrics" {
        return Ok(String::new());
    }
    let Some(song) = db::get_song(conn, &stage.song_id).await? else { return Ok(String::new()) };
    let spine = db::list_sections(conn, &stage.song_id).await?;
    let stages = db::list_stages(conn, &stage.song_id).await?;
    let mut structure = None;
    let mut chords = None;
    for s in &stages {
        if s.r#type != "structure" && s.r#type != "chords" {
            continue;
        }
        if let Some(a) = db::current_artifact(conn, &s.id).await? {
            let data = serde_json::from_str::<Value>(&a.content).ok().and_then(|v| v.get("data").cloned());
            if s.r#type == "structure" { structure = data; } else { chords = data; }
        }
    }
    Ok(crate::render::lyrics_technical_brief(&song, &spine, structure.as_ref(), chords.as_ref()))
}

/// The canonical SECTIONS block (docs/SECTION-SPINE-SPEC.md, Phase 2): the
/// song's section SPINE rendered once — label · bars · role, in spine order —
/// so every stage sees the same section list regardless of which stage
/// artifact it reads. Empty ("") when the song has no spine rows: legacy /
/// unmigrated songs keep their prompts byte-identical.
async fn spine_sections_block(conn: &Connection, song_id: &str) -> Result<String> {
    let rows = db::list_sections(conn, song_id).await?;
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut lines = vec![
        "----- SECTIONS (canonical — the song's section spine; use exactly these sections, labels, and order) -----".to_string(),
    ];
    for (i, r) in rows.iter().enumerate() {
        lines.push(format!(
            "{}. {} ({} bars){}",
            i + 1,
            r.label,
            r.bars,
            if r.role.trim().is_empty() { String::new() } else { format!(" — {}", r.role.trim()) },
        ));
    }
    lines.push("-----------------------------------------------------------------------------------------------------".to_string());
    Ok(lines.join("\n"))
}

/// The user prompt for a stage run: the canonical SECTIONS block (when the
/// song has a spine), earlier-stage context, reverse (later-stage) context
/// when it exists, the lyrics-stage TECHNICAL BRIEF, and the producer's seed.
/// Factored out of `run_stage` so tests can assert the exact prompt without a
/// Claude call.
pub(crate) async fn stage_user_prompt(conn: &Connection, stage: &Stage, user_input: Option<&str>) -> Result<String> {
    let sections = spine_sections_block(conn, &stage.song_id).await?;
    let prior = gather_prior_context(conn, &stage.song_id, stage.ordinal).await?;
    let later = gather_later_context(conn, &stage.song_id, stage.ordinal).await?;
    let tech_brief = lyrics_brief_for_stage(conn, stage).await?;
    // Empty stage → later content is the source of truth (derive). Regeneration
    // → later content is reference only (see LATER_STAGES_REGEN_BANNER).
    let regenerating = db::current_artifact(conn, &stage.id).await?.is_some();
    Ok(build_user_prompt(&stage.r#type, &sections, &prior, &later, &tech_brief, user_input, regenerating))
}

fn build_system_prompt(skill: &Skill, preset: &StylePreset, song: &Song) -> String {
    let mut p = format!(
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
         PRECEDENCE — the preset vs the song: the preset defines the SOUND (genre, mood, instrumentation, tempo feel, vocal). Its 'Recurring themes' are project defaults ONLY — wherever they conflict with THE SONG's title or the producer's intent below, THE SONG WINS. Do not import scenarios, settings, or props from the preset themes (e.g. driving, cities, roads) into a song whose brief says otherwise. Never name real artists to imitate or quote their lyrics.",
        instructions = skill.instructions,
        name = preset.name, genre = preset.genre, mood = preset.mood, influences = preset.influences,
        ktf = preset.key_tempo_feel, vr = preset.vocal_range, themes = preset.themes,
        root = song.key_root, mode = song.key_mode, bpm = song.bpm,
    );
    // THE SONG — the producer's brief. EVERY stage sees the title + intent so
    // no stage can drift into a different song than the one the user asked for.
    let intent = song.intent.trim();
    p.push_str(&format!(
        "\n\n----- THE SONG (the producer's brief — the north star; NEVER drift from it) -----\n\
         Title: {title}   (the title's plain meaning is part of the brief)\n\
         Producer's intent: {intent}\n\
         ---------------------------------------------------------------------------------",
        title = song.title,
        intent = if intent.is_empty() { "(none stated — honor the title)" } else { intent },
    ));
    // The user's taste lever: lyric-stage runs (and self-checks) see the
    // preset's exemplar lines as voice calibration — never as content.
    if skill.stage_type == "lyrics" && !preset.lyric_exemplars.trim().is_empty() {
        p.push_str(&format!(
            "\n\n----- LYRIC EXEMPLARS (calibrate voice/diction/line-length to these; NEVER copy or lightly rework them) -----\n\
             {}\n\
             -----------------------------------------------------------------------",
            preset.lyric_exemplars.trim()
        ));
    }
    p
}

/// The SONG owns key/tempo (header pickers, preset-seeded) — stages never do.
/// A structure regeneration used to overwrite the user's picked key/BPM with the
/// skill's own choice (user-reported: pickers said F# minor / 100, the regen
/// reset them to A minor / 138). Deterministic guarantee, freeze-style: splice
/// the song's current key/bpm into any structure artifact before save and
/// re-render its text. Skills may only SUGGEST changes in prose notes.
/// (Called by `spine::build_run_content`'s legacy path; the spine path splices
/// the same values directly into the reconciled data.)
pub(crate) fn enforce_song_key_tempo(song: &Song, stage_type: &str, content: &str) -> String {
    if stage_type != "structure" {
        return content.to_string();
    }
    let Ok(mut v) = serde_json::from_str::<Value>(content) else { return content.to_string() };
    let Some(data) = v.get_mut("data").filter(|d| d.is_object()) else { return content.to_string() };
    data["key"] = json!({ "root": song.key_root, "mode": song.key_mode });
    data["bpm"] = json!(song.bpm);
    let data_owned = data.clone();
    if let Some(text) = render_stage_text(stage_type, &data_owned) {
        v["text"] = json!(text);
    }
    v.to_string()
}

fn build_user_prompt(stage_type: &str, sections: &str, prior: &str, later: &str, tech_brief: &str, user_input: Option<&str>, regenerating: bool) -> String {
    let mut p = String::new();
    // the canonical section list leads the prompt — stage artifacts below may
    // carry their own (possibly stale) section renditions; the spine wins
    if !sections.is_empty() {
        p.push_str(sections);
        p.push_str("\n\n");
    }
    if !prior.is_empty() {
        p.push_str("Approved outputs from earlier stages (carry these forward):\n\n");
        p.push_str(prior);
        p.push_str("\n\n");
    }
    if !later.is_empty() {
        p.push_str(if regenerating { LATER_STAGES_REGEN_BANNER } else { LATER_STAGES_BANNER });
        p.push_str("\n\n");
        p.push_str(later);
        p.push_str("\n\n");
    }
    if !tech_brief.is_empty() {
        p.push_str(tech_brief);
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
            key_tempo_feel: String::new(), vocal_range: String::new(), themes: String::new(), lyric_exemplars: String::new(),
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
    // Phase 4 (docs/SECTION-SPINE-SPEC.md): the analysis' sections become the
    // NEW song's SPINE (user-authority import); the structure artifact keeps
    // the notes only, and the chords entries attach by section_id.
    let mut entries: Vec<Value> = structure.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    crate::spine::sync_spine(conn, &song.id, &mut entries).await?;
    let rows = db::list_sections(conn, &song.id).await?;
    let snapshot = crate::spine::snapshot_of(&rows);
    if let Some(sid) = stage_id("structure") {
        let s_data = json!({
            "keyNote": structure.get("keyNote").and_then(|v| v.as_str()).unwrap_or(""),
            "tempoNote": structure.get("tempoNote").and_then(|v| v.as_str()).unwrap_or(""),
        });
        let content = json!({ "kind": "structure", "text": crate::render::structure_spine_text(&s_data, &rows), "data": s_data, "spine_snapshot": snapshot }).to_string();
        db::save_artifact(conn, &song.id, Some(&sid), "structure", &content).await?;
        let _ = db::set_stage_status(conn, &sid, "done").await;
    }
    if let Some(cid) = stage_id("chords") {
        let mut chords = chords;
        if let Some(arr) = chords.get_mut("sections").and_then(|v| v.as_array_mut()) {
            for sec in arr.iter_mut() {
                let label = norm_label(&section_label("chords", sec));
                if let Some(row) = rows.iter().find(|r| norm_label(&r.label) == label) {
                    sec["section_id"] = json!(row.id);
                }
            }
        }
        let content = json!({ "kind": "chords", "text": chords_editor_text(&chords), "data": chords, "spine_snapshot": snapshot }).to_string();
        db::save_artifact(conn, &song.id, Some(&cid), "chords", &content).await?;
        let _ = db::set_stage_status(conn, &cid, "done").await;
    }
    Ok(song.id)
}

/// The style/song context block injected into field refinement so 💬 edits stay
/// on-style. Without it, refined fields drift: Claude rewrites a Memphis-phonk
/// style line with no idea the project IS Memphis phonk (audit Tier-2 #9).
pub fn field_refine_context(song: &Song, preset: &StylePreset) -> String {
    format!(
        "----- SONG & STYLE CONTEXT (stay faithful to this) -----\n\
         Song: {title} — {root} {mode} · {bpm} BPM\n\
         Genre: {genre}\nMood: {mood}\nInfluences: {influences}\n\
         Key/tempo feel: {ktf}\nVocal: {vocal}\nThemes: {themes}\n\
         --------------------------------------------------------",
        title = song.title,
        root = song.key_root,
        mode = song.key_mode,
        bpm = song.bpm,
        genre = preset.genre,
        mood = preset.mood,
        influences = preset.influences,
        ktf = preset.key_tempo_feel,
        vocal = preset.vocal_range,
        themes = preset.themes,
    )
}

/// Refine a single field of a song spec via Claude — returns ONLY the new value
/// for that field, so the caller can drop it straight into the structured object.
/// When `song_id` is given, the song's key/BPM + full style preset are injected
/// so the rewrite stays on-style.
pub async fn refine_field(
    conn: &Connection,
    settings: &Settings,
    song_id: Option<&str>,
    stage_label: &str,
    field_label: &str,
    current: &str,
    instruction: &str,
) -> Result<String> {
    let mut system = String::from(
        "You refine exactly ONE field of a song's structured spec. Return ONLY the new value for that field — no preamble, no explanation, no markdown code fences, no surrounding quotes. If the field is a list, separate items with ' · '. Keep the same voice and length unless the request says otherwise.",
    );
    if let Some(sid) = song_id {
        if let Some(song) = db::get_song(conn, sid).await? {
            if let Some(preset) = db::get_preset(conn, &song.style_preset_id).await? {
                system.push_str("\n\n");
                system.push_str(&field_refine_context(&song, &preset));
                system.push_str("\nEvery rewrite must stay inside this sonic world unless the producer's request explicitly changes it.");
            }
        }
    }
    let user = format!(
        "Stage: {stage_label}\nField: {field_label}\n\nCurrent value:\n{current}\n\nProducer's request: {instruction}\n\nReturn ONLY the new {field_label} value.",
    );
    let text = call_claude(settings, &system, &user, &|_: String| {}, None).await?;
    let t = text.trim();
    // strip an accidental ```fence``` or wrapping quotes if the model added them
    let t = t.strip_prefix("```").map(|s| s.trim_start_matches(|c: char| c.is_alphanumeric()).trim()).unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t).trim();
    let t = t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t);
    Ok(t.trim().to_string())
}

// ---- Paste-lyrics import (spec Feature B) ----------------------------------
//
// The user pastes FINISHED lyrics instead of generating. We only parse/tag —
// the words are kept VERBATIM everywhere. Deterministic header split first
// (`[Verse 1]` / Suno-style `[verse]` / line-style `Verse 1:`); Claude is asked
// ONLY for section boundaries on unlabeled text, and a validator rebuilds every
// line from the ORIGINAL input (whitespace-normalized match, every input line
// used exactly once, in order) — any alteration discards the segmentation and
// falls back to one section. The validator is the guarantee, not the prompt.

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ParsedSection {
    pub label: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParsedLyrics {
    pub sections: Vec<ParsedSection>,
    pub used_claude: bool,
}

/// First words that make a `Something:` line a section header. Kept tight so a
/// lyric line that happens to end with ':' is never eaten as a header.
const HEADER_WORDS: &[&str] = &[
    "verse", "chorus", "prechorus", "pre", "post", "postchorus", "bridge", "intro", "outro",
    "hook", "refrain", "drop", "build", "buildup", "breakdown", "break", "interlude",
    "instrumental", "solo", "tag", "coda", "vamp", "middle", "part", "section", "ending",
];

/// The section label when `line` is a header, else None. Two styles:
/// `[Verse 1]` (any bracket-only line, Suno-style included) and `Verse 1:`
/// (line-style, only when the first word is a known section word).
fn header_label(line: &str) -> Option<String> {
    let t = line.trim();
    if t.len() >= 3 && t.starts_with('[') && t.ends_with(']') {
        let inner = t[1..t.len() - 1].trim();
        if !inner.is_empty() && !inner.contains('[') && !inner.contains(']') {
            return Some(inner.to_string());
        }
    }
    if let Some(name) = t.strip_suffix(':') {
        let name = name.trim();
        if !name.is_empty() && name.len() <= 40 && !name.contains(':') {
            let first: String = name.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
            if HEADER_WORDS.contains(&first.as_str()) {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Trim leading/trailing blank lines; interior blanks (stanza breaks) stay.
fn trim_blank_edges(mut lines: Vec<String>) -> Vec<String> {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// Whitespace-normalized form of a lyric line (the verbatim compare unit).
fn norm_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Deterministic split on section headers — the safe path, no model can touch
/// the words. Returns None when the text has no headers at all. Non-header
/// lines are kept verbatim (only `\r` stripped, blank edges trimmed per section).
fn split_labeled_lyrics(text: &str) -> Option<Vec<ParsedSection>> {
    let mut sections: Vec<ParsedSection> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut preamble: Vec<String> = Vec::new();
    let mut found = false;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(label) = header_label(line) {
            found = true;
            if let Some((lbl, lines)) = current.take() {
                sections.push(ParsedSection { label: lbl, lines: trim_blank_edges(lines) });
            }
            current = Some((label, Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line.to_string());
        } else {
            preamble.push(line.to_string());
        }
    }
    if !found {
        return None;
    }
    if let Some((lbl, lines)) = current.take() {
        sections.push(ParsedSection { label: lbl, lines: trim_blank_edges(lines) });
    }
    let pre = trim_blank_edges(preamble);
    if !pre.is_empty() {
        sections.insert(0, ParsedSection { label: "Lyrics".into(), lines: pre });
    }
    Some(sections)
}

// ---- Inline [chord] tags (spec Feature B2 #2/#3) -----------------------------
//
// Rust mirror of frontend/src/lib/music/chordpro.ts's tag handling, scoped to
// what the paste-import needs: extract the inline `[C]word` tags a lyric line
// carries so the import can back-fill the Chords stage and infer the key.
// Names are preserved exactly as authored (no auto-sharpening — Bb is often the
// musically-correct spelling).

/// Split a chord tag into (root, quality) — `"D#m"` → `("D#", "m")` — or None
/// when the bracketed text is not chord-shaped (annotations like "[x2]" or
/// "[whispered]" must never become chords or vote on the key).
fn parse_chord_tag(name: &str) -> Option<(&str, &str)> {
    let b = name.as_bytes();
    if b.is_empty() || !(b'A'..=b'G').contains(&b[0]) {
        return None;
    }
    let mut i = 1;
    if b.len() > i && (b[i] == b'#' || b[i] == b'b') {
        i += 1;
    }
    let quality = &name[i..];
    if quality.contains(char::is_whitespace) {
        return None; // "[Bridge out]" is a direction, not a chord
    }
    let chord_shaped = quality.is_empty()
        || quality.starts_with('m') // m, m7, min, maj7 …
        || quality.starts_with("dim")
        || quality.starts_with("aug")
        || quality.starts_with("sus")
        || quality.starts_with("add")
        || quality.starts_with('(')
        || quality.starts_with('/')
        || quality.starts_with('+')
        || quality.chars().next().is_some_and(|c| c.is_ascii_digit());
    if chord_shaped { Some((&name[..i], quality)) } else { None }
}

/// A lyric line's inline chord tags, in order (chord-shaped tags only).
fn line_chord_tags(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else { break };
        let name = after[..close].trim();
        if !name.is_empty() && parse_chord_tag(name).is_some() {
            out.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    out
}

/// A section's chord-tag sequence collapsed to ONE progression pass when it
/// repeats exactly (Bb C Dm C Bb C Dm C → Bb C Dm C), else the full sequence.
/// The Chords stage semantically holds the PROGRESSION — the Composer's v3
/// layout re-expands it from the lyric placements.
fn collapse_progression(seq: &[String]) -> Vec<String> {
    let n = seq.len();
    for d in 1..=n {
        if n % d == 0 && (d..n).all(|i| seq[i] == seq[i % d]) {
            return seq[..d].to_vec();
        }
    }
    seq.to_vec()
}

/// Infer the song key from the paste's chord tags (spec Feature B2 #3): tonic
/// = the most frequent chord root (ties → the earliest-seen root, i.e. the
/// first tag's), minor when that tonic's tags are predominantly minor
/// (quality starts with 'm' but not "maj"). None when no chord tags exist.
fn infer_key_from_tags(tags: &[String]) -> Option<(String, String)> {
    let mut order: Vec<&str> = Vec::new(); // roots in first-seen order
    let mut stats: std::collections::HashMap<&str, (usize, usize)> = std::collections::HashMap::new(); // root → (total, minor)
    for t in tags {
        let Some((root, quality)) = parse_chord_tag(t) else { continue };
        let e = stats.entry(root).or_insert_with(|| {
            order.push(root);
            (0, 0)
        });
        e.0 += 1;
        if quality.starts_with('m') && !quality.starts_with("maj") {
            e.1 += 1;
        }
    }
    let mut best: Option<&str> = None;
    for r in &order {
        if best.is_none_or(|b| stats[r].0 > stats[b].0) {
            best = Some(r); // strict > keeps the FIRST root on ties
        }
    }
    let root = best?;
    let (total, minor) = stats[root];
    let mode = if minor * 2 > total { "minor" } else { "major" };
    Some((root.to_string(), mode.to_string()))
}

/// Per-section chord-tag sequences for a parsed paste (untagged sections are
/// empty; the outer Vec always matches `parsed.sections` 1:1).
fn section_chord_tags(parsed: &ParsedLyrics) -> Vec<Vec<String>> {
    parsed
        .sections
        .iter()
        .map(|s| s.lines.iter().flat_map(|l| line_chord_tags(l)).collect())
        .collect()
}

/// The no-segmentation fallback: everything as ONE section, words verbatim.
fn single_section(text: &str) -> Vec<ParsedSection> {
    let lines = trim_blank_edges(text.lines().map(|l| l.trim_end_matches('\r').to_string()).collect());
    vec![ParsedSection { label: "Lyrics".into(), lines }]
}

/// THE VERBATIM VALIDATOR. Rebuild the sections from the ORIGINAL input using
/// only Claude's boundaries: each returned non-empty line must match the next
/// input line (whitespace-normalized) and every input line must be consumed —
/// the kept text is the input's bytes, never the model's. Any altered, dropped,
/// reordered, or invented line rejects the whole segmentation (None).
fn rebuild_from_input(input: &str, parsed: &Value) -> Option<Vec<ParsedSection>> {
    let secs = parsed.get("sections")?.as_array()?;
    let input_lines: Vec<&str> = input
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .collect();
    let mut idx = 0usize;
    let mut out = Vec::new();
    for (i, sec) in secs.iter().enumerate() {
        let label = sec
            .get("label")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .unwrap_or_else(|| format!("Section {}", i + 1));
        let lines = sec.get("lines")?.as_array()?;
        let mut kept = Vec::new();
        for l in lines {
            let l = l.as_str()?;
            if l.trim().is_empty() {
                continue; // blank separators carry no words
            }
            let orig = input_lines.get(idx)?;
            if norm_line(l) != norm_line(orig) {
                return None; // altered or reordered → reject everything
            }
            kept.push((*orig).to_string()); // keep the INPUT text, not the model's
            idx += 1;
        }
        if !kept.is_empty() {
            out.push(ParsedSection { label, lines: kept });
        }
    }
    // leftover input lines mean the model dropped words — reject
    if idx != input_lines.len() || out.is_empty() {
        return None;
    }
    Some(out)
}

/// Parse pasted lyrics into sections (dry-run — drives the preview, saves
/// nothing). Deterministic header split first; Claude fallback ONLY for
/// unlabeled multi-line text, validated by `rebuild_from_input`. Never errors
/// on parse trouble — the worst case is one "Lyrics" section, words verbatim.
pub async fn parse_pasted_lyrics(settings: &Settings, text: &str) -> Result<ParsedLyrics> {
    if let Some(sections) = split_labeled_lyrics(text) {
        return Ok(ParsedLyrics { sections, used_claude: false });
    }
    let non_empty = text.lines().filter(|l| !l.trim().is_empty()).count();
    if non_empty > 1 {
        let system = "You mark SECTION BOUNDARIES in finished song lyrics. You never write, rewrite, add, remove, or reorder lyrics — you only decide where each section starts and what it is called (Verse 1, Chorus, Bridge, …).\nRespond with ONLY a single fenced ```json block of the shape {\"sections\":[{\"label\":\"Verse 1\",\"lines\":[\"…\"]}]} where every entry of every `lines` array is a line COPIED VERBATIM from the input, in the original order, with every input line used exactly once.";
        let user = format!("Segment these lyrics into sections (copy every line verbatim):\n\n{text}\n\nReturn the JSON now.");
        if let Ok(out) = call_claude(settings, system, &user, &|_| {}, None).await {
            if let Some(v) = extract_json(&out) {
                if let Some(sections) = rebuild_from_input(text, &v) {
                    return Ok(ParsedLyrics { sections, used_claude: true });
                }
            }
        }
    }
    Ok(ParsedLyrics { sections: single_section(text), used_claude: false })
}

/// Write the parsed paste into a song: replace the Lyrics artifact and
/// back-fill the Structure stage's section list to match (labels/order), then
/// mark both stages done. Pasting is the USER's deliberate action, so it
/// replaces everything including locked sections (the UI warns first) and
/// carries no frozen flags into the new Lyrics artifact.
///
/// Phase 3 (docs/SECTION-SPINE-SPEC.md): the paste is a user-authority spine
/// writer — the SPINE is replaced to the pasted labels/order (existing rows
/// matched by norm-label keep their ids + bars/role/type; new sections get
/// rows) and every saved artifact entry carries its `section_id`.
async fn apply_parsed_lyrics(conn: &Connection, song: &Song, parsed: &ParsedLyrics) -> Result<()> {
    let stages = db::list_stages(conn, &song.id).await?;
    let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").ok_or_else(|| anyhow!("song has no lyrics stage"))?;
    let structure_stage = stages.iter().find(|s| s.r#type == "structure").ok_or_else(|| anyhow!("song has no structure stage"))?;

    // Section forms for the SPINE replace — the pasted labels/order become the
    // section map; bars/role/type (and an existing lock, which keeps its row's
    // form verbatim through the sync) survive where a label matches — the
    // SPINE row is the preferred form source, the prior artifact entry the
    // legacy fallback; new sections get the editor defaults (bars=8, role="").
    let prior_data = match db::current_artifact(conn, &structure_stage.id).await? {
        Some(a) => serde_json::from_str::<Value>(&a.content).ok().and_then(|v| v.get("data").cloned()).unwrap_or(Value::Null),
        None => Value::Null,
    };
    let prior_secs: Vec<Value> = prior_data.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let spine = db::list_sections(conn, &song.id).await?;
    let mut new_secs: Vec<Value> = parsed
        .sections
        .iter()
        .map(|p| {
            let old = prior_secs.iter().find(|s| norm_label(&section_label("structure", s)) == norm_label(&p.label));
            let frozen = old.is_some_and(|o| o.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false));
            let row = spine.iter().find(|r| norm_label(&r.label) == norm_label(&p.label));
            let mut o = match (row, old) {
                // the spine owns the form when the song has a row for this label
                (Some(row), _) => json!({
                    "section_id": row.id, "type": row.r#type, "label": p.label, "bars": row.bars, "role": row.role,
                }),
                (None, Some(old)) => json!({
                    "type": old.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                    "label": p.label,
                    "bars": old.get("bars").and_then(|b| b.as_i64()).unwrap_or(8),
                    "role": old.get("role").and_then(|v| v.as_str()).unwrap_or(""),
                }),
                (None, None) => json!({ "type": "", "label": p.label, "bars": 8, "role": "" }),
            };
            if frozen {
                o["frozen"] = json!(true);
            }
            o
        })
        .collect();

    // SPINE replace (user authority): pasted labels/order become the spine;
    // matched rows keep their ids, the rest are created/deleted. `ids` aligns
    // 1:1 with `parsed.sections`, keying every artifact entry below.
    let ids = crate::spine::sync_spine(conn, &song.id, &mut new_secs).await?;
    let snapshot = crate::spine::spine_snapshot(conn, &song.id).await?;

    // Lyrics artifact — the pasted words verbatim, text rendered like the editor
    let lyrics_data = json!({
        "sections": parsed.sections.iter().zip(&ids).map(|(s, id)| json!({ "section_id": id, "label": s.label, "lines": s.lines })).collect::<Vec<_>>()
    });
    let content = json!({ "kind": "lyrics", "text": lyrics_text(&lyrics_data), "data": lyrics_data, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&lyrics_stage.id), "lyrics", &content).await?;
    db::set_stage_status(conn, &lyrics_stage.id, "done").await?;

    // The SONG owns key/tempo (docs/SONG-FACTS.md) and the SPINE owns the
    // sections (docs/SECTION-SPINE-SPEC.md Phase 4) — the structure artifact
    // keeps only the prose notes; its text renders the map from the spine.
    let structure_data = json!({
        "keyNote": prior_data.get("keyNote").and_then(|v| v.as_str()).unwrap_or(""),
        "tempoNote": prior_data.get("tempoNote").and_then(|v| v.as_str()).unwrap_or(""),
    });
    let rows = db::list_sections(conn, &song.id).await?;
    let s_content = json!({ "kind": "structure", "text": crate::render::structure_spine_text(&structure_data, &rows), "data": structure_data, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await?;
    db::set_stage_status(conn, &structure_stage.id, "done").await?;

    // Chords back-fill from inline [chord] tags (spec Feature B2 #2): when the
    // paste carries ChordPro tags, the Chords stage gets each section's tag
    // sequence — collapsed to one progression pass when it repeats exactly —
    // as {name, beats: 4}. Untagged sections get an empty chords list; a paste
    // with NO tags anywhere leaves the Chords stage untouched (today's
    // behavior). Like the lyrics replace above, pasting is the user's
    // deliberate action, so it replaces the whole Chords artifact.
    let tag_seqs = section_chord_tags(parsed);
    if tag_seqs.iter().any(|t| !t.is_empty()) {
        if let Some(chords_stage) = stages.iter().find(|s| s.r#type == "chords") {
            let chords_data = json!({
                "sections": parsed.sections.iter().zip(&tag_seqs).zip(&ids).map(|((p, tags), id)| json!({
                    "section_id": id,
                    "label": p.label,
                    "chords": collapse_progression(tags).iter().map(|n| json!({ "name": n, "beats": 4 })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>()
            });
            let c_content = json!({ "kind": "chords", "text": chords_editor_text(&chords_data), "data": chords_data, "spine_snapshot": snapshot }).to_string();
            db::save_artifact(conn, &song.id, Some(&chords_stage.id), "chords", &c_content).await?;
            db::set_stage_status(conn, &chords_stage.id, "done").await?;
        }
    }
    Ok(())
}

fn ensure_has_words(parsed: &ParsedLyrics) -> Result<()> {
    if parsed.sections.iter().all(|s| s.lines.iter().all(|l| l.trim().is_empty())) {
        return Err(anyhow!("no lyrics to import — paste some text first"));
    }
    Ok(())
}

/// Import pasted lyrics into an existing song (entry point 1): parse, replace
/// the Lyrics artifact, back-fill Structure (and Chords when the paste carries
/// inline [chord] tags). The song's key is left alone — only
/// `create_song_from_lyrics` infers it. Returns the parse for callers.
pub async fn import_lyrics(conn: &Connection, settings: &Settings, song_id: &str, text: &str) -> Result<ParsedLyrics> {
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let parsed = parse_pasted_lyrics(settings, text).await?;
    ensure_has_words(&parsed)?;
    apply_parsed_lyrics(conn, &song, &parsed).await?;
    Ok(parsed)
}

/// New song from pasted lyrics (entry point 2): mirror the create flow's inputs
/// (preset + working title), then run the same import. Concept stays blank —
/// the user adds it next. When the paste carries inline [chord] tags the song
/// key is INFERRED from them (spec Feature B2 #3 — a [D#m]-heavy paste makes a
/// D#-minor song, not the preset's default); BPM keeps the preset seeding.
/// No tags → key keeps the preset seeding too. (`import_lyrics` into an
/// existing song never touches the key — the user may have set it on purpose.)
pub async fn create_song_from_lyrics(conn: &Connection, settings: &Settings, style_preset_id: &str, title: &str, text: &str) -> Result<Song> {
    // parse (and validate) BEFORE creating, so a bad paste never leaves an empty song
    let parsed = parse_pasted_lyrics(settings, text).await?;
    ensure_has_words(&parsed)?;
    let song = db::create_song(conn, style_preset_id, title).await?;
    // key inference BEFORE apply, so the Structure back-fill renders the inferred key
    let all_tags: Vec<String> = section_chord_tags(&parsed).into_iter().flatten().collect();
    let song = match infer_key_from_tags(&all_tags) {
        Some((root, mode)) => db::update_song_key(conn, &song.id, &root, &mode, song.bpm).await?,
        None => song,
    };
    apply_parsed_lyrics(conn, &song, &parsed).await?;
    db::get_song(conn, &song.id).await?.ok_or_else(|| anyhow!("song not found after import"))
}

// ---- Composer export (composition → song) ----------------------------------
//
// The Composer's export-back-to-song (COMPOSER-SPEC.md #2/#3). The FRONTEND
// resolves scale degrees to absolute chord names (lib/music/compose/
// compositionToSong.ts — the theory engine lives there), so the backend takes
// fully RESOLVED sections and stays theory-free: it validates the shape,
// respects 🔒 frozen sections, renders text with the existing per-stage
// renderers, and saves through the normal artifact conventions.

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ResolvedChord {
    pub name: String,
    #[serde(default = "default_beats")]
    pub beats: i64,
}
fn default_beats() -> i64 {
    4
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ResolvedSection {
    pub label: String,
    #[serde(default = "default_bars")]
    pub bars: i64,
    #[serde(default)]
    pub chords: Vec<ResolvedChord>,
    /// The song's spine row this section came from (docs/SECTION-SPINE-SPEC.md
    /// Phase 3 — `Composition.sections` carries it since Phase 2), so the
    /// export maps back losslessly even across renames. Absent on sketches.
    #[serde(default)]
    pub section_id: Option<String>,
}
fn default_bars() -> i64 {
    8
}

fn parse_resolved_sections(sections_json: &str) -> Result<Vec<ResolvedSection>> {
    let sections: Vec<ResolvedSection> = serde_json::from_str(sections_json)
        .map_err(|e| anyhow!("could not read the exported sections JSON: {e}"))?;
    if sections.is_empty() {
        return Err(anyhow!("nothing to export — the composition has no sections"));
    }
    Ok(sections)
}

/// The Chords-stage `data` for a set of resolved sections (each entry carries
/// its `section_id` when the resolved section has one).
fn chords_data_from_resolved(sections: &[ResolvedSection]) -> Value {
    json!({
        "sections": sections.iter().map(|s| {
            let mut o = json!({
                "label": s.label,
                "chords": s.chords.iter().map(|c| json!({ "name": c.name, "beats": c.beats.max(1) })).collect::<Vec<_>>(),
            });
            if let Some(id) = &s.section_id {
                o["section_id"] = json!(id);
            }
            o
        }).collect::<Vec<_>>()
    })
}

/// The Structure-stage `data` for resolved sections, back-filled against the
/// prior structure exactly like `apply_parsed_lyrics`: labels/order come from
/// the export; `type`/`role` survive where a section matches — the SPINE row
/// (by `section_id`) is the preferred form source, the prior artifact entry
/// (id first, label fallback) the legacy one; a 🔒 lock carries over from the
/// prior entry. BARS: when the song has a spine the EXPORTED bar counts win
/// (D4 — the dialog shows the changes before confirm); the legacy (spineless)
/// path keeps the prior bars on label match, byte-identical to before. NEW
/// sections take the exported bar count.
fn structure_data_from_resolved(sections: &[ResolvedSection], prior_data: &Value, spine: &[Section], export_bars_win: bool) -> Value {
    let prior_secs: Vec<Value> = prior_data.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let new_secs: Vec<Value> = sections
        .iter()
        .map(|p| {
            let row = p.section_id.as_deref().and_then(|id| spine.iter().find(|r| r.id == id));
            let old = prior_secs.iter().find(|s| {
                let by_id = p.section_id.as_deref().is_some_and(|id| s.get("section_id").and_then(|v| v.as_str()) == Some(id));
                by_id || norm_label(&section_label("structure", s)) == norm_label(&p.label)
            });
            let ty = row
                .map(|r| r.r#type.clone())
                .or_else(|| old.and_then(|o| o.get("type").and_then(|v| v.as_str()).map(String::from)))
                .unwrap_or_default();
            let role = row
                .map(|r| r.role.clone())
                .or_else(|| old.and_then(|o| o.get("role").and_then(|v| v.as_str()).map(String::from)))
                .unwrap_or_default();
            let bars = if export_bars_win {
                p.bars.max(1)
            } else {
                old.and_then(|o| o.get("bars").and_then(|b| b.as_i64())).unwrap_or(p.bars.max(1))
            };
            let mut o = json!({ "type": ty, "label": p.label, "bars": bars, "role": role });
            if let Some(id) = &p.section_id {
                o["section_id"] = json!(id);
            }
            if old.is_some_and(|o| o.get("frozen").and_then(|f| f.as_bool()).unwrap_or(false)) {
                o["frozen"] = json!(true);
            }
            o
        })
        .collect();
    // The SONG owns key/tempo (docs/SONG-FACTS.md) — no embedded copies; only
    // the prose notes carry over (legacy embedded values migrate away here).
    json!({
        "keyNote": prior_data.get("keyNote").and_then(|v| v.as_str()).unwrap_or(""),
        "tempoNote": prior_data.get("tempoNote").and_then(|v| v.as_str()).unwrap_or(""),
        "sections": new_secs,
    })
}

/// The `data` of a stage's current artifact (Null when there is none).
async fn current_stage_data(conn: &Connection, stage_id: &str) -> Result<Value> {
    Ok(match db::current_artifact(conn, stage_id).await? {
        Some(a) => serde_json::from_str::<Value>(&a.content)
            .ok()
            .and_then(|v| v.get("data").cloned())
            .unwrap_or(Value::Null),
        None => Value::Null,
    })
}

/// Export a composition into its source song (destination 1): overwrite the
/// Chords stage's sections and back-fill Structure like `import_lyrics`.
/// Exporting is the USER's action, but 🔒 FROZEN sections are still respected
/// deterministically: `merge_frozen_sections` keeps every frozen chord/
/// structure section byte-identical (and re-inserts dropped ones) — the
/// export only lands on unlocked sections. Returns
/// `{ ok, skipped_frozen: [labels] }` so the UI can report what was kept.
///
/// Phase 3 (docs/SECTION-SPINE-SPEC.md): the export is a user-authority spine
/// writer — resolved sections map back through `section_id` (Phase 2 carries
/// it on `Composition.sections`; label fallback otherwise), matched rows take
/// the exported BARS (D4 — the dialog shows the changes first), genuinely new
/// sections get rows, and both saved artifacts carry `section_id`s.
pub async fn export_composition_to_song(conn: &Connection, song_id: &str, sections_json: &str) -> Result<Value> {
    let song = db::get_song(conn, song_id).await?.ok_or_else(|| anyhow!("song not found"))?;
    let mut sections = parse_resolved_sections(sections_json)?;
    let stages = db::list_stages(conn, &song.id).await?;
    let chords_stage = stages.iter().find(|s| s.r#type == "chords").ok_or_else(|| anyhow!("song has no chords stage"))?;
    let structure_stage = stages.iter().find(|s| s.r#type == "structure").ok_or_else(|| anyhow!("song has no structure stage"))?;

    // attach existing spine ids to resolved sections that lack one (norm-label,
    // consume-once) — renamed rows still match through the id the Composer kept
    let spine = db::list_sections(conn, &song.id).await?;
    let had_spine = !spine.is_empty();
    let mut consumed: Vec<bool> = spine.iter().map(|r| sections.iter().any(|s| s.section_id.as_deref() == Some(r.id.as_str()))).collect();
    for s in sections.iter_mut().filter(|s| s.section_id.is_none()) {
        if let Some(ri) = spine
            .iter()
            .enumerate()
            .find(|(ri, r)| !consumed[*ri] && norm_label(&r.label) == norm_label(&s.label))
            .map(|(ri, _)| ri)
        {
            consumed[ri] = true;
            s.section_id = Some(spine[ri].id.clone());
        }
    }

    // Structure FIRST (it drives the spine): back-fill (labels/order from the
    // export; type/role/🔒 preserved on match; exported bars win once the song
    // has a spine — D4), frozen guard re-inserts dropped locked sections, then
    // the SPINE is replaced to the merged list (matched rows keep ids).
    let prior_structure = current_stage_data(conn, &structure_stage.id).await?;
    let new_structure = structure_data_from_resolved(&sections, &prior_structure, &spine, had_spine);
    let merged_structure = merge_frozen_sections("structure", &prior_structure, &new_structure);
    let mut s_entries = merged_structure.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    crate::spine::sync_spine(conn, &song.id, &mut s_entries).await?;
    let snapshot = crate::spine::spine_snapshot(conn, &song.id).await?;

    // Chords: the export (entries keyed to the fresh spine — new rows included),
    // with prior frozen sections spliced back verbatim (id-first).
    let fresh = db::list_sections(conn, &song.id).await?;
    for s in sections.iter_mut().filter(|s| s.section_id.is_none()) {
        s.section_id = fresh.iter().find(|r| norm_label(&r.label) == norm_label(&s.label)).map(|r| r.id.clone());
    }
    let prior_chords = current_stage_data(conn, &chords_stage.id).await?;
    let skipped = frozen_labels("chords", &prior_chords);
    let merged_chords = merge_frozen_sections("chords", &prior_chords, &chords_data_from_resolved(&sections));
    let c_content = json!({ "kind": "chords", "text": chords_editor_text(&merged_chords), "data": merged_chords, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&chords_stage.id), "chords", &c_content).await?;
    db::set_stage_status(conn, &chords_stage.id, "done").await?;

    // Phase 4 (docs/SECTION-SPINE-SPEC.md): the SPINE owns the sections — the
    // structure artifact keeps the prose notes; text renders from the spine.
    let s_data = json!({
        "keyNote": merged_structure.get("keyNote").and_then(|v| v.as_str()).unwrap_or(""),
        "tempoNote": merged_structure.get("tempoNote").and_then(|v| v.as_str()).unwrap_or(""),
    });
    let s_content = json!({ "kind": "structure", "text": crate::render::structure_spine_text(&s_data, &fresh), "data": s_data, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await?;
    db::set_stage_status(conn, &structure_stage.id, "done").await?;

    Ok(json!({ "ok": true, "song_id": song.id, "skipped_frozen": skipped }))
}

/// Create a NEW song from a composition (destination 2 — works for blank
/// sketches too): mirrors `create_song_from_lyrics`'s preset/title inputs,
/// carries the composition's key/bpm onto the song, and populates Structure +
/// Chords from the resolved sections. Lyrics stay empty (melody/bass live in
/// the saved composition, not in a stage).
pub async fn create_song_from_composition(
    conn: &Connection,
    style_preset_id: &str,
    title: &str,
    key_root: &str,
    key_mode: &str,
    bpm: i64,
    sections_json: &str,
) -> Result<Song> {
    // validate BEFORE creating, so a bad export never leaves an empty song
    let mut sections = parse_resolved_sections(sections_json)?;
    // a NEW song gets fresh spine rows — ids from the source composition (if
    // any) belong to another song and must not leak in
    for s in sections.iter_mut() {
        s.section_id = None;
    }
    let song = db::create_song(conn, style_preset_id, title).await?;
    let song = db::update_song_key(conn, &song.id, key_root, key_mode, bpm).await?;
    let stages = db::list_stages(conn, &song.id).await?;
    let chords_stage = stages.iter().find(|s| s.r#type == "chords").ok_or_else(|| anyhow!("song has no chords stage"))?;
    let structure_stage = stages.iter().find(|s| s.r#type == "structure").ok_or_else(|| anyhow!("song has no structure stage"))?;

    // the resolved sections BECOME the new song's spine (user authority)
    let structure_data = structure_data_from_resolved(&sections, &Value::Null, &[], true);
    let mut s_entries = structure_data.get("sections").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let ids = crate::spine::sync_spine(conn, &song.id, &mut s_entries).await?;
    for (s, id) in sections.iter_mut().zip(&ids) {
        s.section_id = Some(id.clone());
    }
    let rows = db::list_sections(conn, &song.id).await?;
    let snapshot = crate::spine::snapshot_of(&rows);

    let chords_data = chords_data_from_resolved(&sections);
    let c_content = json!({ "kind": "chords", "text": chords_editor_text(&chords_data), "data": chords_data, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&chords_stage.id), "chords", &c_content).await?;
    db::set_stage_status(conn, &chords_stage.id, "done").await?;

    // Phase 4: the SPINE owns the sections — the structure artifact keeps the
    // prose notes only (empty on a fresh export); text renders from the spine.
    let s_data = json!({ "keyNote": "", "tempoNote": "" });
    let s_content = json!({ "kind": "structure", "text": crate::render::structure_spine_text(&s_data, &rows), "data": s_data, "spine_snapshot": snapshot }).to_string();
    db::save_artifact(conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await?;
    db::set_stage_status(conn, &structure_stage.id, "done").await?;

    db::get_song(conn, &song.id).await?.ok_or_else(|| anyhow!("song not found after export"))
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

    /// Phase 3 (docs/SECTION-SPINE-SPEC.md): frozen sections match by
    /// `section_id` BEFORE the label — a renamed section (spine rename → the
    /// model outputs the new label) still merges onto its own entry instead of
    /// being duplicated by the label mismatch.
    #[test]
    fn merge_frozen_matches_by_section_id_before_label() {
        let prior = json!({ "sections": [
            { "section_id": "sec-x", "label": "Verse 1", "chords": [{"name":"Am","beats":4}], "frozen": true }
        ]});
        let new = json!({ "sections": [
            { "section_id": "sec-x", "label": "Verso Uno", "chords": [{"name":"G","beats":4}] }
        ]});
        let merged = merge_frozen_sections("chords", &prior, &new);
        let secs = merged["sections"].as_array().unwrap();
        assert_eq!(secs.len(), 1, "id match must not duplicate the renamed section");
        assert_eq!(secs[0]["chords"][0]["name"], "Am", "frozen content wins");
        assert_eq!(secs[0]["label"], "Verse 1", "frozen entry stays byte-verbatim");
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
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
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
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
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
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
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
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
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

    // ---- Paste-lyrics import (Feature B) ------------------------------------

    /// (a) Deterministic header split: `[x]` and `x:` styles, labels/order kept,
    /// every word verbatim, no Claude involved.
    #[tokio::test]
    async fn paste_split_on_headers_is_deterministic_and_verbatim() {
        let text = "[Verse 1]\nCity lights are calling me home\nEvery street I know by heart\n\n[Chorus]\nWe run until the morning finds us\n\nBridge:\nHold on to the static in the air";
        let p = parse_pasted_lyrics(&Settings::default(), text).await.unwrap();
        assert!(!p.used_claude);
        let labels: Vec<&str> = p.sections.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"]);
        assert_eq!(p.sections[0].lines, ["City lights are calling me home", "Every street I know by heart"]);
        assert_eq!(p.sections[1].lines, ["We run until the morning finds us"]);
        assert_eq!(p.sections[2].lines, ["Hold on to the static in the air"]);
    }

    /// (b) Unlabeled text: Claude (mocked) returns a VALID segmentation — the
    /// boundaries are used, `used_claude` is true, and the kept lines are the
    /// input's own bytes.
    #[tokio::test]
    async fn paste_unlabeled_uses_claude_segmentation_when_valid() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let text = "City lights are calling me home\nEvery street I know by heart\nWe run until the morning finds us";
        let seg = json!({ "sections": [
            { "label": "Verse 1", "lines": ["City lights are calling me home", "Every street I know by heart"] },
            { "label": "Chorus", "lines": ["We run until the morning finds us"] }
        ]});
        std::env::set_var("SONGSMITH_MOCK_CLAUDE", format!("```json\n{seg}\n```"));
        let p = parse_pasted_lyrics(&Settings::default(), text).await.unwrap();
        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");
        assert!(p.used_claude);
        assert_eq!(p.sections.len(), 2);
        assert_eq!(p.sections[0].label, "Verse 1");
        assert_eq!(p.sections[0].lines, ["City lights are calling me home", "Every street I know by heart"]);
        assert_eq!(p.sections[1].label, "Chorus");
        assert_eq!(p.sections[1].lines, ["We run until the morning finds us"]);
    }

    /// (c) THE VERBATIM GUARANTEE: Claude (mocked) alters one word — the
    /// validator rejects the whole segmentation and the parse falls back to a
    /// single section whose lines are the INPUT text, untouched.
    #[tokio::test]
    async fn paste_claude_altering_a_word_falls_back_to_single_section() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let text = "City lights are calling me home\nEvery street I know by heart";
        // the model "improved" home → homeward: every line must match or nothing does
        let seg = json!({ "sections": [
            { "label": "Verse 1", "lines": ["City lights are calling me homeward", "Every street I know by heart"] }
        ]});
        std::env::set_var("SONGSMITH_MOCK_CLAUDE", format!("```json\n{seg}\n```"));
        let p = parse_pasted_lyrics(&Settings::default(), text).await.unwrap();
        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");
        assert!(!p.used_claude, "a rejected segmentation is not Claude's");
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.sections[0].label, "Lyrics");
        assert_eq!(p.sections[0].lines, ["City lights are calling me home", "Every street I know by heart"]);
    }

    /// (d) `import_lyrics` replaces the Lyrics artifact (dropping frozen flags —
    /// pasting is the user's authority) and back-fills Structure to the pasted
    /// labels/order, keeping bars/role where a label matches.
    #[tokio::test]
    async fn import_lyrics_backfills_structure_preserving_bars_and_role() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();
        let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").unwrap();

        // prior Structure: Verse 1 + Chorus carry bars/role to preserve; Outro will be dropped
        let s_data = json!({ "key": {"root":"A","mode":"minor"}, "bpm": 120, "keyNote": "", "tempoNote": "", "sections": [
            { "type": "verse", "label": "Verse 1", "bars": 16, "role": "story" },
            { "label": "Chorus", "bars": 8, "role": "lift" },
            { "label": "Outro", "bars": 4, "role": "fade" }
        ]});
        let s_content = json!({ "kind": "structure", "text": structure_editor_text(&s_data), "data": s_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await.unwrap();
        // prior Lyrics with a FROZEN section — the paste replaces it (user authority)
        let l_data = json!({ "sections": [ { "label": "Verse 1", "lines": ["old words"], "frozen": true } ] });
        let l_content = json!({ "kind": "lyrics", "text": lyrics_text(&l_data), "data": l_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&lyrics_stage.id), "lyrics", &l_content).await.unwrap();

        let text = "[Verse 1]\nNew words all the way down\n\n[Chorus]\nSing it loud\n\n[Bridge]\nSomething new";
        import_lyrics(&conn, &settings, &song.id, text).await.unwrap();

        // Lyrics: pasted words verbatim, editor-style text, no frozen flags carried
        let lyr = db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().unwrap();
        let lv = serde_json::from_str::<Value>(&lyr.content).unwrap();
        assert_eq!(lv["text"].as_str().unwrap(), "[Verse 1]\nNew words all the way down\n\n[Chorus]\nSing it loud\n\n[Bridge]\nSomething new");
        let lsecs = lv["data"]["sections"].as_array().unwrap();
        assert_eq!(lsecs[0]["lines"][0], "New words all the way down");
        assert!(lsecs.iter().all(|s| s.get("frozen").is_none()), "paste carries no frozen flags");

        // The SPINE is back-filled to the pasted labels/order, bars/role
        // preserved on match (Phase 4: the structure artifact keeps no copy)
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"]);
        assert_eq!((rows[0].bars, rows[0].role.as_str(), rows[0].r#type.as_str()), (16, "story", "verse"));
        assert_eq!((rows[1].bars, rows[1].role.as_str()), (8, "lift"));
        assert_eq!((rows[2].bars, rows[2].role.as_str()), (8, ""), "new section gets the editor default");
        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        // song-facts + spine contracts: no embedded key/bpm (the SONG owns
        // them — docs/SONG-FACTS.md) and no section copy (the SPINE owns them
        // — docs/SECTION-SPINE-SPEC.md); only the prose notes carry over
        assert!(sv["data"].get("key").is_none(), "back-fill must not copy the key into the artifact");
        assert!(sv["data"].get("bpm").is_none(), "back-fill must not copy the bpm into the artifact");
        assert!(sv["data"].get("sections").is_none(), "the structure artifact keeps no section copy");
        assert!(sv["spine_snapshot"].is_array(), "the save embeds the spine snapshot");
        assert!(sv["text"].as_str().unwrap().contains("1. **Verse 1** (16 bars) — story"), "text renders the map from the spine, got: {}", sv["text"]);
        // both stages marked done
        assert_eq!(db::get_stage(&conn, &lyrics_stage.id).await.unwrap().unwrap().status, "done");
        assert_eq!(db::get_stage(&conn, &structure_stage.id).await.unwrap().unwrap().status, "done");
    }

    /// (e) `create_song_from_lyrics` end-to-end: a new song whose Lyrics AND
    /// Structure match the paste, ready for Concept + Chords.
    #[tokio::test]
    async fn create_song_from_lyrics_populates_structure_and_lyrics() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();

        let text = "[Verse 1]\nFirst line here\nSecond line here\n\n[Chorus]\nHook line";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Pasted Song", text).await.unwrap();
        assert_eq!(song.title, "Pasted Song");
        assert_eq!(song.status, "in_progress");

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();
        let lyr = db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().unwrap();
        let lv = serde_json::from_str::<Value>(&lyr.content).unwrap();
        assert_eq!(lv["data"]["sections"][0]["label"], "Verse 1");
        assert_eq!(lv["data"]["sections"][0]["lines"], json!(["First line here", "Second line here"]));
        assert_eq!(lv["data"]["sections"][1]["lines"], json!(["Hook line"]));
        // the pasted labels/order become the SPINE (the structure artifact keeps no copy)
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus"]);
        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        // song-facts contract: the artifact carries no key/bpm — the SONG does
        assert!(sv["data"].get("key").is_none(), "structure data must not embed the key");
        assert!(sv["data"].get("bpm").is_none(), "structure data must not embed the bpm");
        assert!(sv["data"].get("sections").is_none(), "the SPINE owns the sections");
        assert!(sv["text"].as_str().unwrap().contains("2. **Chorus** (8 bars)"), "text renders the spine map, got: {}", sv["text"]);
        assert_eq!(song.key_root, "A"); // create-flow defaults live on the song
        assert_eq!(song.bpm, 120);
        assert_eq!(db::get_stage(&conn, &lyrics_stage.id).await.unwrap().unwrap().status, "done");
        assert_eq!(db::get_stage(&conn, &structure_stage.id).await.unwrap().unwrap().status, "done");
        // Concept stays pending — the user adds it and runs Chords next
        let concept = stages.iter().find(|s| s.r#type == "concept").unwrap();
        assert_eq!(db::get_stage(&conn, &concept.id).await.unwrap().unwrap().status, "pending");
    }

    // Audit Tier-2 #6: user skills must outrank builtins, and reseeding must not
    // bump an unchanged builtin's updated_at (which used to re-win the recency sort).
    #[tokio::test]
    async fn user_skill_outranks_builtin_and_reseed_is_noop() {
        let (_db, conn) = mem_conn().await;

        // the builtin is active before any user skill exists
        let before = db::get_active_skill_for_stage(&conn, "chords").await.unwrap().unwrap();
        assert_eq!(before.source, "builtin");
        let builtin_ts = before.updated_at.clone();

        // reseeding with unchanged embedded content must not touch updated_at
        db::seed_skills(&conn).await.unwrap();
        let reseeded = db::get_active_skill_for_stage(&conn, "chords").await.unwrap().unwrap();
        assert_eq!(reseeded.updated_at, builtin_ts, "no-op reseed must not bump updated_at");

        // a user-created skill for the same stage wins…
        let user = db::create_skill(&conn, crate::models::SkillInput {
            key: "my-chords".into(),
            name: "My Chords".into(),
            stage_type: "chords".into(),
            instructions: "Always write jazz voicings.".into(),
        }).await.unwrap();
        assert_eq!(user.source, "user");

        // …including after another reseed (the old bug: builtins re-timestamped each launch)
        db::seed_skills(&conn).await.unwrap();
        let active = db::get_active_skill_for_stage(&conn, "chords").await.unwrap().unwrap();
        assert_eq!(active.id, user.id, "user skill must outrank the builtin after reseed");
    }

    // ---- Composer export (composition → song) --------------------------------

    /// (a) Export into an existing song: the Chords stage takes the exported
    /// sections EXCEPT the 🔒 frozen one, which is skipped and preserved
    /// byte-identical; Structure is back-filled to the exported labels/order
    /// with bars/role preserved on label match; both stages land "done"; the
    /// skipped labels are reported.
    #[tokio::test]
    async fn export_composition_updates_song_skipping_frozen_chords() {
        let (_db, conn) = mem_conn().await;
        let (song, chords_stage, _prior, prior_frozen) = frozen_chords_fixture(&conn).await;
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();

        // prior Structure: Verse 1 carries bars/role to preserve on label match
        let s_data = json!({ "key": {"root":"A","mode":"minor"}, "bpm": 120, "keyNote": "", "tempoNote": "", "sections": [
            { "type": "verse", "label": "Verse 1", "bars": 16, "role": "story" }
        ]});
        let s_content = json!({ "kind": "structure", "text": structure_editor_text(&s_data), "data": s_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await.unwrap();

        // the Composer exports resolved sections that rewrite EVERYTHING —
        // including the frozen Verse 1 (which must be skipped)
        let resolved = json!([
            { "label": "Verse 1", "bars": 8, "chords": [{"name":"Dm","beats":4},{"name":"Bb","beats":4}] },
            { "label": "Chorus", "bars": 8, "chords": [{"name":"G","beats":4},{"name":"Em","beats":4}] },
            { "label": "Bridge", "bars": 4, "chords": [{"name":"F","beats":2},{"name":"G","beats":2}] }
        ])
        .to_string();
        let out = export_composition_to_song(&conn, &song.id, &resolved).await.unwrap();
        assert_eq!(out["skipped_frozen"], json!(["Verse 1"]), "the frozen section is surfaced as skipped");

        // Chords: frozen Verse 1 byte-identical; Chorus/Bridge take the export
        let cur = db::current_artifact(&conn, &chords_stage.id).await.unwrap().unwrap();
        let cv = serde_json::from_str::<Value>(&cur.content).unwrap();
        let secs = cv["data"]["sections"].as_array().unwrap();
        let labels: Vec<&str> = secs.iter().map(|s| s["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"]);
        let verse = secs.iter().find(|s| s["label"] == "Verse 1").unwrap();
        assert_eq!(*verse, prior_frozen, "frozen chords section must survive the export byte-identical");
        assert_eq!(verse["frozen"], json!(true));
        let chorus = secs.iter().find(|s| s["label"] == "Chorus").unwrap();
        assert_eq!(chorus["chords"][0]["name"], "G");
        assert_eq!(chorus["chords"][0]["beats"], json!(4));
        // text is the chords editor's own rendering of the merged data
        let text = cv["text"].as_str().unwrap();
        assert!(text.contains("Verse 1: Am F"), "rendered text keeps the frozen chords, got: {text}");
        assert!(text.contains("Chorus: G Em"), "rendered text has the exported chorus, got: {text}");
        assert!(text.contains("Bridge: F G"), "rendered text has the new bridge, got: {text}");

        // The SPINE takes the exported labels/order; bars/role/type preserved
        // on match; new sections take the exported bar counts (the structure
        // artifact keeps no section copy — Phase 4)
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        let slabels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(slabels, ["Verse 1", "Chorus", "Bridge"]);
        assert_eq!((rows[0].bars, rows[0].role.as_str(), rows[0].r#type.as_str()), (16, "story", "verse"), "matched section keeps its form");
        assert_eq!(rows[1].bars, 8, "new section takes the exported bars");
        assert_eq!(rows[2].bars, 4);
        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        assert!(sv["data"].get("sections").is_none(), "the SPINE owns the sections");
        assert!(sv["text"].as_str().unwrap().contains("1. **Verse 1** (16 bars) — story"), "text renders the spine map, got: {}", sv["text"]);
        // both stages marked done
        assert_eq!(db::get_stage(&conn, &chords_stage.id).await.unwrap().unwrap().status, "done");
        assert_eq!(db::get_stage(&conn, &structure_stage.id).await.unwrap().unwrap().status, "done");
    }

    /// (b) A frozen STRUCTURE section dropped by the export is re-inserted
    /// verbatim (skip-and-preserve applies to structure too).
    #[tokio::test]
    async fn export_composition_preserves_dropped_frozen_structure_section() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Test Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();

        let s_data = json!({ "key": {"root":"A","mode":"minor"}, "bpm": 120, "keyNote": "", "tempoNote": "", "sections": [
            { "type": "intro", "label": "Intro", "bars": 4, "role": "set the scene", "frozen": true },
            { "label": "Verse 1", "bars": 8, "role": "" }
        ]});
        let s_content = json!({ "kind": "structure", "text": structure_editor_text(&s_data), "data": s_data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&structure_stage.id), "structure", &s_content).await.unwrap();

        // the export has no Intro at all
        let resolved = json!([
            { "label": "Verse 1", "bars": 8, "chords": [{"name":"Am","beats":4}] }
        ])
        .to_string();
        export_composition_to_song(&conn, &song.id, &resolved).await.unwrap();

        // the frozen Intro survives (re-inserted by the merge) as a SPINE row
        // with its form verbatim; the artifact keeps no section copy (Phase 4)
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].label.as_str(), rows[0].r#type.as_str(), rows[0].bars, rows[0].role.as_str()),
            ("Intro", "intro", 4, "set the scene"),
            "dropped frozen structure section survives at its index, form verbatim"
        );
        assert_eq!(rows[1].label, "Verse 1");
        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        assert!(sv["data"].get("sections").is_none(), "the SPINE owns the sections");
        assert!(sv["text"].as_str().unwrap().contains("1. **Intro** (4 bars) — set the scene"), "got: {}", sv["text"]);
    }

    /// (c) Create a NEW song from a composition: stages populated, key/bpm
    /// carried from the composition, lyrics left empty. The resolved JSON here
    /// is exactly what compositionToSong.ts produces for a C-major blank
    /// sketch with degree chords 1–5–6–4 (one "Sketch" section) — the Rust
    /// half of the degree→name round-trip contract.
    #[tokio::test]
    async fn sketch_resolved_sections_round_trip_into_new_song() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();

        // compositionToSong.ts: C major, degrees 1,5,6,4 (one bar each) →
        // one "Sketch" section, chords C G Am F @ 4 beats, bars = 8.
        let resolved = json!([
            { "label": "Sketch", "bars": 8, "chords": [
                {"name":"C","beats":4},{"name":"G","beats":4},{"name":"Am","beats":4},{"name":"F","beats":4}
            ]}
        ])
        .to_string();
        let song = create_song_from_composition(&conn, &preset.id, "Neon idea", "C", "major", 112, &resolved).await.unwrap();
        assert_eq!(song.title, "Neon idea");
        assert_eq!(song.key_root, "C", "key root carried from the composition");
        assert_eq!(song.key_mode, "major");
        assert_eq!(song.bpm, 112, "bpm carried from the composition");

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();
        let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").unwrap();

        let cur = db::current_artifact(&conn, &chords_stage.id).await.unwrap().unwrap();
        let cv = serde_json::from_str::<Value>(&cur.content).unwrap();
        assert_eq!(cv["data"]["sections"][0]["label"], "Sketch");
        let names: Vec<&str> = cv["data"]["sections"][0]["chords"].as_array().unwrap()
            .iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["C", "G", "Am", "F"], "degree 1/5/6/4 in C major resolve to C G Am F");
        assert_eq!(cv["text"].as_str().unwrap(), "Sketch: C G Am F");

        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        // song-facts contract: key/bpm live on the SONG (asserted above), never in the artifact
        assert!(sv["data"].get("key").is_none(), "structure data must not embed the key");
        assert!(sv["data"].get("bpm").is_none(), "structure data must not embed the bpm");
        // spine contract (Phase 4): the section lives on the SPINE, not in the artifact
        assert!(sv["data"].get("sections").is_none(), "the SPINE owns the sections");
        let rows = db::list_sections(&conn, &song.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].label.as_str(), rows[0].bars), ("Sketch", 8));
        assert!(sv["text"].as_str().unwrap().contains("1. **Sketch** (8 bars)"), "got: {}", sv["text"]);

        assert_eq!(db::get_stage(&conn, &chords_stage.id).await.unwrap().unwrap().status, "done");
        assert_eq!(db::get_stage(&conn, &structure_stage.id).await.unwrap().unwrap().status, "done");
        // lyrics stay empty/pending — melody/bass live in the composition
        assert_eq!(db::get_stage(&conn, &lyrics_stage.id).await.unwrap().unwrap().status, "pending");
        assert!(db::current_artifact(&conn, &lyrics_stage.id).await.unwrap().is_none());
    }

    /// (d) Invalid/empty export payloads never create a song or touch stages.
    #[tokio::test]
    async fn export_rejects_bad_sections_json() {
        let (_db, conn) = mem_conn().await;
        let (song, chords_stage, prior_content, _f) = frozen_chords_fixture(&conn).await;

        assert!(export_composition_to_song(&conn, &song.id, "not json").await.is_err());
        assert!(export_composition_to_song(&conn, &song.id, "[]").await.is_err());
        let cur = db::current_artifact(&conn, &chords_stage.id).await.unwrap().unwrap();
        assert_eq!(cur.content, prior_content, "a rejected export leaves the chords artifact untouched");

        let preset_id = song.style_preset_id.clone();
        let songs_before = db::list_songs(&conn).await.unwrap().len();
        assert!(create_song_from_composition(&conn, &preset_id, "X", "C", "major", 100, "[]").await.is_err());
        assert_eq!(db::list_songs(&conn).await.unwrap().len(), songs_before, "no empty song is created on a bad export");
    }

    /// text/data divergence (found in the wild — "Black Eucharist"): a Claude
    /// stage-chat save stored its CHANGELOG as `text` while the lyrics lived in
    /// `data`, so the Generation Prompt received a changelog instead of lyrics.
    /// (1) read side: gather_prior_context must render from `data`, healing
    /// legacy artifacts; (2) write side: save_artifact_guarded must rebuild
    /// `text` from `data` on every section-stage save, not only frozen merges.
    #[tokio::test]
    async fn prior_context_and_guarded_save_prefer_data_over_commentary_text() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "phonk".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Black Eucharist").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").unwrap();
        let prompt_ordinal = stages.iter().find(|s| s.r#type == "prompt").unwrap().ordinal;

        let data = json!({ "sections": [
            { "label": "Intro", "lines": ["[Am] kneel…", "the candle keeps my name"] }
        ]});

        // (1) legacy broken artifact written straight to the DB: changelog in `text`
        let broken = json!({ "kind": "lyrics", "text": "v4 — reconciled section labels (changelog)", "data": data }).to_string();
        db::save_artifact(&conn, &song.id, Some(&lyrics_stage.id), "lyrics", &broken).await.unwrap();
        let ctx = gather_prior_context(&conn, &song.id, prompt_ordinal).await.unwrap();
        assert!(ctx.contains("[Am] kneel…"), "prior context must carry the rendered lyrics, got: {ctx}");
        assert!(!ctx.contains("changelog"), "prior context must not carry the commentary text, got: {ctx}");

        // (2) the guarded (Claude/MCP) write path normalizes text from data
        let incoming = json!({ "kind": "lyrics", "text": "v5 — more commentary, not lyrics", "data": data }).to_string();
        let saved = save_artifact_guarded(&conn, &song.id, Some(&lyrics_stage.id), "lyrics", &incoming).await.unwrap();
        let v: Value = serde_json::from_str(&saved.content).unwrap();
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("[Am] kneel…"), "guarded save must rebuild text from data, got: {text}");
        assert!(!text.contains("commentary"), "commentary must not survive as text, got: {text}");
    }

    // ---- Feature B2: lyrics-first flow completion ----------------------------

    /// (1) Reverse context: after a lyrics-first import, the Concept run's
    /// prompt carries the already-written later stages under the banner, with
    /// the pasted lyrics rendered in.
    #[tokio::test]
    async fn stage_prompt_carries_later_stages_after_lyrics_import() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let text = "[Verse 1]\nCity lights are calling me home\n\n[Chorus]\nWe run until the morning finds us";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Imported", text).await.unwrap();

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let concept = stages.iter().find(|s| s.r#type == "concept").unwrap();
        let prompt = stage_user_prompt(&conn, concept, None).await.unwrap();

        assert!(prompt.contains(LATER_STAGES_BANNER), "prompt must carry the reverse-context banner, got: {prompt}");
        assert!(prompt.contains("### Lyrics output"), "prompt must carry the Lyrics stage block, got: {prompt}");
        assert!(prompt.contains("City lights are calling me home"), "prompt must carry the pasted words, got: {prompt}");
        // the back-filled sections arrive as the canonical SECTIONS block (the
        // import built the SPINE; the structure artifact is notes-only now)
        assert!(prompt.contains("SECTIONS (canonical"), "prompt must lead with the spine's section block, got: {prompt}");
        assert!(prompt.contains("2. Chorus (8 bars)"), "the imported sections are in the block, got: {prompt}");
        // and the banner comes BEFORE the later-stage blocks
        assert!(prompt.find(LATER_STAGES_BANNER).unwrap() < prompt.find("### Lyrics output").unwrap());
    }

    /// (1c — user-requested) The NO-CHORDS paste, end to end: a tagless import
    /// leaves the Chords stage empty/pending, and running the CHORDS stage then
    /// sees the imported lyrics via reverse context — so generated chords derive
    /// from the words instead of being invented blind.
    #[tokio::test]
    async fn chords_prompt_derives_from_tagless_imported_lyrics() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let text = "[Verse 1]\nMidnight, the room gone quiet\n\n[Chorus]\nPull me under, make me clean";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Tagless", text).await.unwrap();

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords = stages.iter().find(|s| s.r#type == "chords").unwrap();
        // the tagless import must NOT have back-filled chords
        assert!(db::current_artifact(&conn, &chords.id).await.unwrap().is_none(), "tagless paste must leave Chords empty");
        assert_eq!(chords.status, "pending");

        // running the Chords stage sees the pasted words as later-stage context
        let prompt = stage_user_prompt(&conn, chords, None).await.unwrap();
        assert!(prompt.contains(LATER_STAGES_BANNER), "chords prompt must carry the reverse-context banner");
        assert!(prompt.contains("Pull me under, make me clean"), "chords prompt must carry the imported lyrics, got: {prompt}");
        // Structure (ordinal < chords) still arrives as normal prior context
        assert!(prompt.contains("### Structure output"));
    }

    /// Regenerating a stage that already has an artifact must get the REFERENCE
    /// banner, not the derive banner — real Claude copied a stale Generation
    /// Prompt's old lyrics back verbatim under "DERIVE FROM them" (caught live).
    #[tokio::test]
    async fn regenerating_stage_gets_reference_banner_not_derive() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let text = "[Verse 1]\nMidnight, the room gone quiet\n\n[Chorus]\nPull me under, make me clean";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Imported", text).await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let lyrics_stage = stages.iter().find(|s| s.r#type == "lyrics").unwrap();

        // give a LATER stage (prompt) an artifact, like a generated Generation Prompt
        let prompt_stage = stages.iter().find(|s| s.r#type == "prompt").unwrap();
        db::save_artifact(&conn, &song.id, Some(&prompt_stage.id), "prompt",
            &json!({ "kind": "prompt", "text": "style + tagged lyrics …", "data": null }).to_string(),
        ).await.unwrap();

        // the lyrics stage HAS an artifact (the import) → regenerating it must
        // use the reference banner so it doesn't copy the later stage back
        let prompt = stage_user_prompt(&conn, lyrics_stage, None).await.unwrap();
        assert!(prompt.contains(LATER_STAGES_REGEN_BANNER), "regen must use the reference banner, got: {prompt}");
        assert!(!prompt.contains(LATER_STAGES_BANNER), "regen must NOT use the derive banner");

        // while an EMPTY stage (concept) still derives
        let concept = stages.iter().find(|s| s.r#type == "concept").unwrap();
        let cprompt = stage_user_prompt(&conn, concept, None).await.unwrap();
        assert!(cprompt.contains(LATER_STAGES_BANNER), "empty stage keeps the derive banner");
    }

    /// The SONG owns key/tempo: a structure regeneration whose model output
    /// carries a different key/BPM gets the song's current values spliced back
    /// (user-reported: pickers said F# minor / 100, regen reset to A minor / 138).
    #[tokio::test]
    async fn structure_regen_cannot_override_song_key_tempo() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Key Test").await.unwrap();
        db::update_song_key(&conn, &song.id, "F#", "minor", 100).await.unwrap();
        let song = db::get_song(&conn, &song.id).await.unwrap().unwrap();

        // the model proposed its own key/tempo (A minor / 138)
        let model_out = json!({
            "kind": "structure",
            "text": "whatever the model rendered",
            "data": { "key": {"root": "A", "mode": "minor"}, "bpm": 138,
                       "keyNote": "A minor carries...", "tempoNote": "138 BPM phonk...",
                       "sections": [{"type": "verse", "label": "Verse 1", "bars": 8, "role": "open"}] }
        }).to_string();

        let enforced = enforce_song_key_tempo(&song, "structure", &model_out);
        let v: Value = serde_json::from_str(&enforced).unwrap();
        assert_eq!(v["data"]["key"]["root"], "F#", "song key root wins");
        assert_eq!(v["data"]["key"]["mode"], "minor");
        assert_eq!(v["data"]["bpm"], 100, "song bpm wins");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("F#") && text.contains("100"), "re-rendered text reflects the song's key/tempo, got: {text}");
        // notes stay as prose (suggestions allowed there)
        assert_eq!(v["data"]["keyNote"], "A minor carries...");
        // non-structure stages pass through untouched
        assert_eq!(enforce_song_key_tempo(&song, "lyrics", &model_out), model_out);
    }

    /// Advance stops at a done-but-STALE stage (user-reported: after generating
    /// Concept on an imported song, advance skipped the ⚠-flagged Structure
    /// straight to Chords — the UI warned "out of date" while advance hopped it).
    #[tokio::test]
    async fn advance_stops_at_stale_backfilled_stage() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let text = "[Verse 1]\nMidnight, the room gone quiet\n\n[Chorus]\nPull me under, make me clean";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Imported", text).await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let concept = stages.iter().find(|s| s.r#type == "concept").unwrap();

        // simulate generating + approving Concept AFTER the import: its artifact
        // is now newer than the back-filled Structure/Lyrics ones
        db::save_artifact(&conn, &song.id, Some(&concept.id), "concept",
            &json!({ "kind": "concept", "text": "A drowning-sacrament song.", "data": null }).to_string(),
        ).await.unwrap();
        db::set_stage_status(&conn, &concept.id, "done").await.unwrap();

        let out = crate::tools::advance_song(&conn, &song.id).await.unwrap();
        assert_eq!(
            out["current_stage"], "structure",
            "advance must stop at the stale back-filled Structure, not skip to a pending later stage; got: {out}"
        );
    }

    /// (1b) A normal forward song's prompt is byte-identical to today: no later
    /// artifacts → no banner, exactly the legacy prompt.
    #[tokio::test]
    async fn stage_prompt_unchanged_for_forward_song() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Forward Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let concept = stages.iter().find(|s| s.r#type == "concept").unwrap();

        let prompt = stage_user_prompt(&conn, concept, Some("a song about rain")).await.unwrap();
        // byte-identical to the pre-B2 prompt builder (empty later block)
        assert_eq!(prompt, build_user_prompt("concept", "", "", "", "", Some("a song about rain"), false));
        assert!(!prompt.contains("ALREADY-WRITTEN LATER STAGES"));
    }

    /// Phase 2 (docs/SECTION-SPINE-SPEC.md): every stage's prompt leads with
    /// the canonical SECTIONS block when the song has spine rows — and is
    /// BYTE-IDENTICAL to the spineless prompt when it doesn't (the block is
    /// prepended verbatim, nothing else moves).
    #[tokio::test]
    async fn prompt_carries_canonical_sections_block_only_with_spine() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Spine Song").await.unwrap();
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let stage = |t: &str| stages.iter().find(|s| s.r#type == t).unwrap();

        // some prior context so the prompt isn't trivially empty
        db::save_artifact(&conn, &song.id, Some(&stage("concept").id), "concept",
            &json!({ "kind": "concept", "text": "A night-drive song.", "data": null }).to_string()).await.unwrap();

        // no spine rows → no block (byte-identical legacy prompt)
        let without: std::collections::HashMap<&str, String> = {
            let mut m = std::collections::HashMap::new();
            for t in ["structure", "chords", "lyric_spec", "lyrics", "prompt"] {
                m.insert(t, stage_user_prompt(&conn, stage(t), None).await.unwrap());
            }
            m
        };
        for (t, p) in &without {
            assert!(!p.contains("SECTIONS (canonical"), "{t}: no spine → no block, got: {p}");
        }

        // spine rows exist → EVERY stage's prompt leads with the block, and the
        // rest of the prompt is unchanged (block + "\n\n" prepended verbatim)
        db::create_section(&conn, &song.id, "Verse 1", "verse", 8, "set the scene", None).await.unwrap();
        db::create_section(&conn, &song.id, "Chorus", "", 12, "", None).await.unwrap();
        let block = "----- SECTIONS (canonical — the song's section spine; use exactly these sections, labels, and order) -----\n\
                     1. Verse 1 (8 bars) — set the scene\n\
                     2. Chorus (12 bars)\n\
                     -----------------------------------------------------------------------------------------------------";
        for t in ["structure", "chords", "lyric_spec", "prompt"] {
            let p = stage_user_prompt(&conn, stage(t), None).await.unwrap();
            assert_eq!(p, format!("{block}\n\n{}", without[t]), "{t}: block must be prepended verbatim");
        }
        // the LYRICS prompt additionally gains the TECHNICAL BRIEF — the spine
        // now supplies its section list even without a structure artifact
        let lp = stage_user_prompt(&conn, stage("lyrics"), None).await.unwrap();
        assert!(lp.starts_with(&format!("{block}\n\n")), "lyrics prompt leads with the block, got: {lp}");
        assert!(lp.contains("----- TECHNICAL BRIEF"), "spine feeds the brief, got: {lp}");
        assert!(lp.contains("- Chorus: 12 bars"), "brief bars come from the spine, got: {lp}");
    }

    /// The computed TECHNICAL BRIEF lands in the LYRICS stage prompt only —
    /// and only once real Structure data exists (missing data → no brief).
    #[tokio::test]
    async fn technical_brief_injected_into_lyrics_prompt_only() {
        let (_db, conn) = mem_conn().await;
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Brief Song").await.unwrap(); // 120 BPM default
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let stage = |t: &str| stages.iter().find(|s| s.r#type == t).unwrap();

        // no structure yet → the lyrics prompt is unchanged (no brief)
        let bare = stage_user_prompt(&conn, stage("lyrics"), None).await.unwrap();
        assert!(!bare.contains("TECHNICAL BRIEF"), "no data → no brief, got: {bare}");

        let s_data = json!({ "sections": [
            { "label": "Intro", "bars": 4 },
            { "label": "Verse 1", "bars": 8 },
            { "label": "Chorus 1", "bars": 8 }
        ]});
        db::save_artifact(&conn, &song.id, Some(&stage("structure").id), "structure",
            &json!({ "kind": "structure", "text": structure_editor_text(&s_data), "data": s_data }).to_string()).await.unwrap();
        let c_data = json!({ "sections": [
            { "label": "Chorus 1", "chords": [{"name":"F","beats":4},{"name":"C","beats":4},{"name":"G","beats":4},{"name":"Am","beats":4}] }
        ]});
        db::save_artifact(&conn, &song.id, Some(&stage("chords").id), "chords",
            &json!({ "kind": "chords", "text": chords_editor_text(&c_data), "data": c_data }).to_string()).await.unwrap();

        let prompt = stage_user_prompt(&conn, stage("lyrics"), None).await.unwrap();
        assert!(prompt.contains("----- TECHNICAL BRIEF"), "lyrics prompt must carry the brief, got: {prompt}");
        assert!(prompt.contains("Tempo: 120 BPM → a comfortable sung line is roughly 6-10 syllables."), "got: {prompt}");
        assert!(prompt.contains("- Intro: 4 bars, instrumental"), "got: {prompt}");
        assert!(prompt.contains("- Chorus 1: 8 bars, 4 chord changes (16 beats) → aim for 4-8 lines, roughly one chord change per line; land the hook on line 1."), "got: {prompt}");

        // other stages never see it — even with the same data present
        for t in ["concept", "structure", "chords", "lyric_spec", "prompt"] {
            let p = stage_user_prompt(&conn, stage(t), None).await.unwrap();
            assert!(!p.contains("TECHNICAL BRIEF"), "{t} prompt must not carry the brief");
        }
    }

    /// Lyric exemplars: appended to the LYRICS system prompt only, and only
    /// when non-empty — never copied, always fenced as calibration.
    #[test]
    fn lyric_exemplars_in_lyrics_system_prompt_only() {
        let mk_skill = |stage_type: &str| Skill {
            id: "sk".into(), key: "k".into(), name: "N".into(), stage_type: stage_type.into(),
            instructions: "INSTR".into(), source: "builtin".into(), enabled: true,
            created_at: String::new(), updated_at: String::new(),
        };
        let mut preset = StylePreset {
            id: "p".into(), name: "P".into(), genre: "".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(),
            lyric_exemplars: "I left the porch light on again\nNobody's coming home".into(),
            created_at: String::new(), updated_at: String::new(),
        };
        let song = Song {
            id: "s".into(), style_preset_id: "p".into(), title: "T".into(), intent: String::new(), status: "in_progress".into(),
            current_stage: "lyrics".into(), key_root: "A".into(), key_mode: "minor".into(), bpm: 120,
            voicings: "{}".into(), created_at: String::new(), updated_at: String::new(),
        };

        let lyr = build_system_prompt(&mk_skill("lyrics"), &preset, &song);
        assert!(lyr.contains("----- LYRIC EXEMPLARS (calibrate voice/diction/line-length to these; NEVER copy or lightly rework them) -----"), "got: {lyr}");
        assert!(lyr.contains("I left the porch light on again"));

        // other stages never see the exemplars
        let con = build_system_prompt(&mk_skill("concept"), &preset, &song);
        assert!(!con.contains("LYRIC EXEMPLARS"));

        // empty/whitespace exemplars → the lyrics system prompt is unchanged
        preset.lyric_exemplars = "  \n ".into();
        let plain = build_system_prompt(&mk_skill("lyrics"), &preset, &song);
        assert!(!plain.contains("LYRIC EXEMPLARS"));
    }

    /// North Star: EVERY stage's system prompt carries THE SONG block with the
    /// title + producer's intent; an empty intent shows the "(none stated —
    /// honor the title)" fallback instead of a blank line.
    #[test]
    fn north_star_title_and_intent_in_every_stage_system_prompt() {
        let mk_skill = |stage_type: &str| Skill {
            id: "sk".into(), key: "k".into(), name: "N".into(), stage_type: stage_type.into(),
            instructions: "INSTR".into(), source: "builtin".into(), enabled: true,
            created_at: String::new(), updated_at: String::new(),
        };
        let preset = StylePreset {
            id: "p".into(), name: "P".into(), genre: "".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
            created_at: String::new(), updated_at: String::new(),
        };
        let mut song = Song {
            id: "s".into(), style_preset_id: "p".into(), title: "Finding you in the sand of time".into(),
            intent: "searching for love in desert".into(), status: "in_progress".into(),
            current_stage: "concept".into(), key_root: "A".into(), key_mode: "minor".into(), bpm: 120,
            voicings: "{}".into(), created_at: String::new(), updated_at: String::new(),
        };

        for t in ["concept", "structure", "chords", "lyric_spec", "lyrics", "prompt"] {
            let p = build_system_prompt(&mk_skill(t), &preset, &song);
            assert!(p.contains("----- THE SONG (the producer's brief — the north star; NEVER drift from it) -----"), "{t} prompt must carry THE SONG block");
            assert!(p.contains("Title: Finding you in the sand of time"), "{t} prompt must carry the title");
            assert!(p.contains("Producer's intent: searching for love in desert"), "{t} prompt must carry the intent");
            assert!(!p.contains("(none stated — honor the title)"), "{t}: a stated intent must not show the fallback");
        }

        // empty/whitespace intent → the fallback keeps the title authoritative
        song.intent = "  \n ".into();
        let p = build_system_prompt(&mk_skill("concept"), &preset, &song);
        assert!(p.contains("Producer's intent: (none stated — honor the title)"), "got: {p}");
    }

    /// North Star: a Concept run with a seed persists it as the song's intent
    /// (once) — a later run with a different seed never overwrites it, and a
    /// seeded run of any OTHER stage never touches it.
    #[tokio::test]
    async fn concept_seed_persists_as_intent_only_when_empty() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Finding you in the sand of time").await.unwrap();
        assert_eq!(song.intent, "");
        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let concept_stage = stages.iter().find(|s| s.r#type == "concept").unwrap();
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();

        let claude_out = "```json\n".to_string() + &json!({
            "title": "Finding you in the sand of time", "alternates": ["A", "B"],
            "hook": "h", "theme": "t", "emotionalArc": "a", "mood": ["m"]
        }).to_string() + "\n```";
        std::env::set_var("SONGSMITH_MOCK_CLAUDE", &claude_out);

        // 1. concept run with a seed → the seed (trimmed) becomes the intent
        run_stage(&conn, &settings, &concept_stage.id, Some("  searching for love in desert \n".into()), |_| {}, None).await.unwrap();
        let song = db::get_song(&conn, &song.id).await.unwrap().unwrap();
        assert_eq!(song.intent, "searching for love in desert", "the concept seed is persisted as the intent");

        // 2. a concept re-run with a different seed does NOT overwrite it
        run_stage(&conn, &settings, &concept_stage.id, Some("a totally different idea".into()), |_| {}, None).await.unwrap();
        let song = db::get_song(&conn, &song.id).await.unwrap().unwrap();
        assert_eq!(song.intent, "searching for love in desert", "an existing intent is never overwritten");

        // 3. a seeded run of another stage never touches the intent
        db::update_song_intent(&conn, &song.id, "").await.unwrap();
        run_stage(&conn, &settings, &structure_stage.id, Some("four on the floor".into()), |_| {}, None).await.unwrap();
        let song = db::get_song(&conn, &song.id).await.unwrap().unwrap();
        assert_eq!(song.intent, "", "only the CONCEPT stage seeds the intent");

        std::env::remove_var("SONGSMITH_MOCK_CLAUDE");
    }

    /// (2 — pure) Progression collapse: exact repeats fold to one pass; partial
    /// repeats keep the full sequence.
    #[test]
    fn collapse_progression_folds_exact_repeats_only() {
        let seq = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(collapse_progression(&seq(&["Bb", "C", "Dm", "C", "Bb", "C", "Dm", "C"])), seq(&["Bb", "C", "Dm", "C"]));
        assert_eq!(collapse_progression(&seq(&["Bb", "C", "Dm", "C", "Bb"])), seq(&["Bb", "C", "Dm", "C", "Bb"]), "a partial repeat is NOT collapsed");
        assert_eq!(collapse_progression(&seq(&["Am", "Am", "Am"])), seq(&["Am"]));
        assert!(collapse_progression(&[]).is_empty());
    }

    /// (2/3 — pure) Tag parsing keeps chords and rejects directions; key
    /// inference picks the most frequent root and its predominant mode.
    #[test]
    fn chord_tags_and_key_inference() {
        assert_eq!(line_chord_tags("[D#m]City lights are [C#]calling [B]home"), ["D#m", "C#", "B"]);
        assert_eq!(line_chord_tags("[x2] sing it [whispered]again"), Vec::<String>::new(), "non-chord tags are not chords");
        assert_eq!(line_chord_tags("[Am(add9)]kneel at the [Bb]altar"), ["Am(add9)", "Bb"]);

        let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // [D#m]-heavy → D# minor
        assert_eq!(infer_key_from_tags(&tags(&["D#m", "C#", "B", "D#m", "D#m"])), Some(("D#".into(), "minor".into())));
        // major tonic
        assert_eq!(infer_key_from_tags(&tags(&["C", "F", "G", "C"])), Some(("C".into(), "major".into())));
        // maj7 does not count as minor
        assert_eq!(infer_key_from_tags(&tags(&["Cmaj7", "Cmaj7", "Am"])), Some(("C".into(), "major".into())));
        // frequency tie → the first tag's root
        assert_eq!(infer_key_from_tags(&tags(&["Em", "D", "Em", "D"])), Some(("E".into(), "minor".into())));
        assert_eq!(infer_key_from_tags(&[]), None);
    }

    /// (2/3) The user's real-shaped paste: [D#m]/[C#]/[B] verse tags plus a
    /// chorus whose progression repeats exactly → Chords back-filled per
    /// section (chorus collapsed to one pass, untagged Bridge empty, stage
    /// done) and the song key inferred as D# minor with the BPM untouched.
    #[tokio::test]
    async fn create_song_from_lyrics_backfills_chords_and_infers_key() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();

        let text = "[Verse 1]\n\
                    [D#m]City lights are [C#]calling me [B]home\n\
                    [D#m]Every [D#m]street I know by [D#m]heart\n\n\
                    [Chorus]\n\
                    [D#m]Run [C#]far, [B]run [C#]now\n\
                    [D#m]Run [C#]far, [B]run [C#]now\n\n\
                    [Bridge]\n\
                    No chords marked here at all";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "D# Song", text).await.unwrap();

        // key inferred from the tags: D# is the most frequent root, all its tags minor
        assert_eq!(song.key_root, "D#", "tonic = most frequent tag root");
        assert_eq!(song.key_mode, "minor", "[D#m]-heavy → minor");
        assert_eq!(song.bpm, 120, "BPM keeps the preset seeding");

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();
        let cur = db::current_artifact(&conn, &chords_stage.id).await.unwrap().unwrap();
        let cv = serde_json::from_str::<Value>(&cur.content).unwrap();
        let secs = cv["data"]["sections"].as_array().unwrap();
        let labels: Vec<&str> = secs.iter().map(|s| s["label"].as_str().unwrap()).collect();
        assert_eq!(labels, ["Verse 1", "Chorus", "Bridge"]);

        let names = |s: &Value| s["chords"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        // verse: no exact repetition → the full tag sequence, in order
        assert_eq!(names(&secs[0]), ["D#m", "C#", "B", "D#m", "D#m", "D#m"]);
        // chorus: the two identical lines collapse to ONE progression pass
        assert_eq!(names(&secs[1]), ["D#m", "C#", "B", "C#"], "repeated chorus progression collapses to one pass");
        // untagged bridge → empty chords list
        assert!(names(&secs[2]).is_empty(), "untagged section gets an empty chords list");
        // every back-filled chord defaults to 4 beats
        assert!(secs[0]["chords"].as_array().unwrap().iter().all(|c| c["beats"] == json!(4)));
        // rendered with the chords renderer + stage marked done
        assert!(cv["text"].as_str().unwrap().contains("Chorus: D#m C# B C#"), "text rendered by the chords renderer");
        assert_eq!(db::get_stage(&conn, &chords_stage.id).await.unwrap().unwrap().status, "done");

        // the inferred key lives on the SONG only (asserted above) — the
        // Structure back-fill carries no embedded copy (docs/SONG-FACTS.md)
        let structure_stage = stages.iter().find(|s| s.r#type == "structure").unwrap();
        let st = db::current_artifact(&conn, &structure_stage.id).await.unwrap().unwrap();
        let sv = serde_json::from_str::<Value>(&st.content).unwrap();
        assert!(sv["data"].get("key").is_none(), "structure data must not embed the key");
        assert!(sv["data"].get("bpm").is_none(), "structure data must not embed the bpm");
    }

    /// (2/3b) A paste with NO tags anywhere keeps today's behavior exactly:
    /// Chords untouched (no artifact, still pending) and the preset-seeded key.
    #[tokio::test]
    async fn create_song_from_lyrics_without_tags_leaves_chords_and_key_alone() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "F minor, 140 BPM".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();

        let text = "[Verse 1]\nPlain words with no chord tags\n\n[Chorus]\nStill nothing marked";
        let song = create_song_from_lyrics(&conn, &settings, &preset.id, "Plain", text).await.unwrap();
        assert_eq!(song.key_root, "F", "key keeps the preset seeding");
        assert_eq!(song.key_mode, "minor");
        assert_eq!(song.bpm, 140);

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();
        assert!(db::current_artifact(&conn, &chords_stage.id).await.unwrap().is_none(), "no tags → Chords untouched");
        assert_eq!(db::get_stage(&conn, &chords_stage.id).await.unwrap().unwrap().status, "pending");
    }

    /// (3b) `import_lyrics` into an existing song back-fills Chords from tags
    /// but leaves the song's key alone — the user may have set it on purpose.
    #[tokio::test]
    async fn import_lyrics_backfills_chords_but_keeps_song_key() {
        let (_db, conn) = mem_conn().await;
        let settings = db::get_settings(&conn).await.unwrap();
        let preset = db::create_preset(&conn, StyleInput {
            name: "Test".into(), genre: "rock".into(), mood: "".into(), influences: "".into(),
            key_tempo_feel: "".into(), vocal_range: "".into(), themes: "".into(), lyric_exemplars: "".into(),
        }).await.unwrap();
        let song = db::create_song(&conn, &preset.id, "Existing").await.unwrap();

        let text = "[Verse 1]\n[D#m]City [C#]lights [B]home";
        import_lyrics(&conn, &settings, &song.id, text).await.unwrap();

        let after = db::get_song(&conn, &song.id).await.unwrap().unwrap();
        assert_eq!(after.key_root, song.key_root, "import into an existing song never touches the key");
        assert_eq!(after.key_mode, song.key_mode);

        let stages = db::list_stages(&conn, &song.id).await.unwrap();
        let chords_stage = stages.iter().find(|s| s.r#type == "chords").unwrap();
        let cur = db::current_artifact(&conn, &chords_stage.id).await.unwrap().unwrap();
        let cv = serde_json::from_str::<Value>(&cur.content).unwrap();
        assert_eq!(cv["data"]["sections"][0]["chords"][0]["name"], "D#m");
        assert_eq!(db::get_stage(&conn, &chords_stage.id).await.unwrap().unwrap().status, "done");
    }
}
