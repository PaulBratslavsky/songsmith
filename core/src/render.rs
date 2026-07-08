//! Per-stage `text` renderers: turn a stage artifact's structured `data` into
//! the human-readable `text` exactly the way each stage's editor serializes it,
//! so artifact `text` stays consistent no matter who wrote the data (editor,
//! agent run, freeze splice, MCP save). Split out of `agent.rs` (audit Tier-2
//! #11); the old duplicate renderer pair (`structure_text`/`chords_text` used
//! only by reference-import) was COLLAPSED into this one set — these editor
//! mirrors are supersets (string chords AND `{name,beats}` objects, key/tempo
//! notes, conditional role suffix).

use crate::models::Song;
use serde_json::Value;

/// Render the human-readable `text` for a section-based stage from its merged
/// `data`, mirroring exactly how each stage's editor serializes text → so the
/// artifact `text` (which `gather_prior_context` reads) stays consistent after a
/// splice. Returns `None` for non-section stages (keep Claude's text).
pub(crate) fn render_stage_text(stage_type: &str, data: &Value) -> Option<String> {
    match stage_type {
        "structure" => Some(structure_editor_text(data)),
        "chords" => Some(chords_editor_text(data)),
        "lyrics" => Some(lyrics_text(data)),
        "lyric_spec" => Some(lyric_spec_text(data)),
        _ => None,
    }
}

/// Mirror of `Composer`'s save: `label: name name …` per section. The Composer
/// stores each chord as `{name,beats}`, so read `name` (falling back to a bare
/// string — reference-import saves string chords).
pub(crate) fn chords_editor_text(c: &Value) -> String {
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
pub(crate) fn structure_editor_text(d: &Value) -> String {
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
pub(crate) fn lyrics_text(d: &Value) -> String {
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
pub(crate) fn lyric_spec_text(d: &Value) -> String {
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

// ---- Lyrics TECHNICAL BRIEF -------------------------------------------------
//
// Quantitative context for the Lyricist, computed from REAL song data (the
// Structure stage's sections, the Chords stage's per-section changes, and the
// song's BPM) — so line counts and syllable weights stop wandering.
//
// Documented heuristics (deliberately simple):
// - Syllables per sung line, by tempo: <=90 BPM → 8-12 · 91-130 → 6-10 ·
//   >130 → 4-8. The structure's `tempoNote` (e.g. "half-time feel") is quoted
//   on the tempo line for context but NOT modeled into the budget.
// - Suggested lines per sung section: between the section's chord-change count
//   and its bar count, each clamped to 2..=10 (a degenerate x-x range is
//   widened down by 2 so there is always room to breathe). Without chords data
//   the range falls back to bars/2 .. bars, same clamp.
// - A section is INSTRUMENTAL when its structure `role` says so ("instrumental",
//   "no vocals", "no lyrics") or its label is a classic instrumental slot
//   (intro/outro/interlude/solo/break/instrumental) → "bare chord tags only".
// - Missing data degrades to an EMPTY string, leaving the prompt unchanged.

/// Tempo → a comfortable per-line syllable budget (see module heuristics).
fn syllable_budget(bpm: i64) -> (i64, i64) {
    if bpm <= 90 { (8, 12) } else if bpm <= 130 { (6, 10) } else { (4, 8) }
}

/// Case/whitespace-insensitive label key (mirror of `freeze::norm_label`).
fn brief_norm(s: &str) -> String {
    s.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_instrumental(label: &str, role: &str) -> bool {
    let r = role.to_lowercase();
    if r.contains("instrumental") || r.contains("no vocals") || r.contains("no lyrics") {
        return true;
    }
    label
        .to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .any(|w| matches!(w, "intro" | "outro" | "interlude" | "instrumental" | "solo" | "break"))
}

/// Compute the TECHNICAL BRIEF injected into the lyrics-stage prompt (run +
/// self-check). Pure: real Structure/Chords `data` + the song's BPM in, prompt
/// text out; any missing input degrades to `""` (prompt unchanged).
pub(crate) fn lyrics_technical_brief(song: &Song, structure: Option<&Value>, chords: Option<&Value>) -> String {
    let sections = match structure.and_then(|d| d.get("sections")).and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return String::new(),
    };
    let bpm = if song.bpm > 0 { song.bpm } else { 120 };
    let (syl_lo, syl_hi) = syllable_budget(bpm);
    let tempo_note = structure
        .and_then(|d| d.get("tempoNote"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| format!(" ({})", s.trim()))
        .unwrap_or_default();

    // chord sections by normalized label → (chord changes, total beats)
    let chord_stats: Vec<(String, i64, i64)> = chords
        .and_then(|d| d.get("sections"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|sec| {
                    let label = sec.get("label").and_then(|v| v.as_str())
                        .or_else(|| sec.get("type").and_then(|v| v.as_str())).unwrap_or("");
                    let (mut changes, mut beats) = (0i64, 0i64);
                    if let Some(ch) = sec.get("chords").and_then(|v| v.as_array()) {
                        for c in ch {
                            changes += 1;
                            beats += c.get("beats").and_then(|b| b.as_i64()).unwrap_or(4);
                        }
                    }
                    (brief_norm(label), changes, beats)
                })
                .collect()
        })
        .unwrap_or_default();

    let mut out = vec![
        "----- TECHNICAL BRIEF (computed from this song's structure/chords/tempo — honor it) -----".to_string(),
        format!("Tempo: {bpm} BPM{tempo_note} → a comfortable sung line is roughly {syl_lo}-{syl_hi} syllables."),
    ];
    for sec in sections {
        let label = sec.get("label").and_then(|v| v.as_str())
            .or_else(|| sec.get("type").and_then(|v| v.as_str())).unwrap_or("Section");
        let bars = sec.get("bars").and_then(|v| v.as_i64()).unwrap_or(8).max(1);
        let role = sec.get("role").and_then(|v| v.as_str()).unwrap_or("");

        if is_instrumental(label, role) {
            out.push(format!("- {label}: {bars} bars, instrumental (no sung lines — bare chord tags only)."));
            continue;
        }

        let stats = chord_stats.iter().find(|(l, _, _)| *l == brief_norm(label));
        let changes = stats.map(|(_, c, _)| *c).unwrap_or(0);
        let beats = stats.map(|(_, _, b)| *b).unwrap_or(0);

        // suggested line count: between chord changes and bars, clamped 2..=10
        let (mut lo, mut hi) = if changes > 0 {
            (changes.min(bars), changes.max(bars))
        } else {
            ((bars / 2).max(1), bars)
        };
        lo = lo.clamp(2, 10);
        hi = hi.clamp(2, 10);
        if lo == hi {
            lo = (lo - 2).max(2); // widen a degenerate range
        }
        let lines_txt = if lo == hi { format!("aim for about {lo} lines") } else { format!("aim for {lo}-{hi} lines") };

        let ll = label.to_lowercase();
        let hook = if ll.contains("chorus") && !ll.contains("pre") { "; land the hook on line 1" } else { "" };
        let per_line = if changes >= lo && changes <= hi { ", roughly one chord change per line" } else { "" };

        if changes > 0 {
            let ch_word = if changes == 1 { "chord change" } else { "chord changes" };
            out.push(format!("- {label}: {bars} bars, {changes} {ch_word} ({beats} beats) → {lines_txt}{per_line}{hook}."));
        } else {
            out.push(format!("- {label}: {bars} bars → {lines_txt}{hook}."));
        }
    }
    out.push("------------------------------------------------------------------------------------------".to_string());
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn song(bpm: i64) -> Song {
        Song {
            id: "s1".into(), style_preset_id: "p1".into(), title: "T".into(), intent: String::new(),
            status: "in_progress".into(), current_stage: "lyrics".into(),
            key_root: "A".into(), key_mode: "minor".into(), bpm,
            voicings: "{}".into(), created_at: String::new(), updated_at: String::new(),
        }
    }

    /// Missing/empty structure data degrades to "" — the prompt stays unchanged.
    #[test]
    fn brief_empty_without_structure() {
        let s = song(120);
        assert_eq!(lyrics_technical_brief(&s, None, None), "");
        assert_eq!(lyrics_technical_brief(&s, Some(&json!({})), None), "");
        assert_eq!(lyrics_technical_brief(&s, Some(&json!({"sections": []})), None), "");
        // chords alone (no structure) is not enough either
        let ch = json!({"sections": [{"label": "Verse 1", "chords": [{"name":"Am","beats":4}]}]});
        assert_eq!(lyrics_technical_brief(&s, None, Some(&ch)), "");
    }

    /// The tempo buckets: <=90 → 8-12, 91-130 → 6-10, >130 → 4-8 syllables.
    #[test]
    fn brief_syllable_budget_tracks_tempo() {
        let st = json!({"sections": [{"label": "Verse 1", "bars": 8}]});
        for (bpm, want) in [(72, "roughly 8-12 syllables"), (90, "roughly 8-12 syllables"),
                            (91, "roughly 6-10 syllables"), (130, "roughly 6-10 syllables"),
                            (140, "roughly 4-8 syllables")] {
            let b = lyrics_technical_brief(&song(bpm), Some(&st), None);
            assert!(b.contains(want), "{bpm} BPM → {want}, got: {b}");
            assert!(b.contains(&format!("Tempo: {bpm} BPM")), "got: {b}");
        }
    }

    /// A realistic song: bars + per-section chord changes/beats, the line-count
    /// range between changes and bars, the chorus hook nudge, and instrumental
    /// intro detection — matched case/space-insensitively against the chords.
    #[test]
    fn brief_realistic_song() {
        let structure = json!({"tempoNote": "half-time feel", "sections": [
            {"label": "Intro", "bars": 4},
            {"label": "Verse 1", "bars": 8, "role": "set the scene"},
            {"label": "Chorus 1", "bars": 8, "role": "the release"}
        ]});
        let chords = json!({"sections": [
            {"label": "Intro", "chords": [{"name":"Am","beats":8},{"name":"F","beats":8}]},
            {"label": "verse  1", "chords": [
                {"name":"Am","beats":4},{"name":"F","beats":4},{"name":"C","beats":4},{"name":"G","beats":4},
                {"name":"Am","beats":4},{"name":"F","beats":4},{"name":"C","beats":4},{"name":"G","beats":4}]},
            {"label": "Chorus 1", "chords": [{"name":"F","beats":4},{"name":"C","beats":4},{"name":"G","beats":4},{"name":"Am","beats":4}]}
        ]});
        let b = lyrics_technical_brief(&song(120), Some(&structure), Some(&chords));
        assert!(b.contains("Tempo: 120 BPM (half-time feel) → a comfortable sung line is roughly 6-10 syllables."), "got: {b}");
        assert!(b.contains("- Intro: 4 bars, instrumental (no sung lines — bare chord tags only)."), "got: {b}");
        // 8 changes over 8 bars → degenerate 8-8 range widened down to 6-8
        assert!(b.contains("- Verse 1: 8 bars, 8 chord changes (32 beats) → aim for 6-8 lines, roughly one chord change per line."), "got: {b}");
        // 4 changes over 8 bars → 4-8 lines, hook nudge on the chorus
        assert!(b.contains("- Chorus 1: 8 bars, 4 chord changes (16 beats) → aim for 4-8 lines, roughly one chord change per line; land the hook on line 1."), "got: {b}");
        assert!(b.starts_with("----- TECHNICAL BRIEF"), "got: {b}");
    }

    /// Instrumental sections are flagged by role text or by classic labels;
    /// "Pre-Chorus" is sung and gets no hook nudge.
    #[test]
    fn brief_instrumental_detection() {
        let st = json!({"sections": [
            {"label": "Guitar Solo", "bars": 8},
            {"label": "Verse 2", "bars": 8, "role": "instrumental breakdown, no vocals"},
            {"label": "Pre-Chorus", "bars": 4}
        ]});
        let b = lyrics_technical_brief(&song(100), Some(&st), None);
        assert!(b.contains("- Guitar Solo: 8 bars, instrumental"), "got: {b}");
        assert!(b.contains("- Verse 2: 8 bars, instrumental"), "got: {b}");
        assert!(b.contains("- Pre-Chorus: 4 bars → aim for 2-4 lines."), "got: {b}");
        assert!(!b.contains("Pre-Chorus: 4 bars → aim for 2-4 lines; land the hook"), "pre-chorus must not get the hook nudge, got: {b}");
    }

    /// No chords data for a sung section → the bars-only fallback range
    /// (bars/2 .. bars, clamped 2..=10); huge sections clamp at 10.
    #[test]
    fn brief_no_chords_falls_back_to_bars() {
        let st = json!({"sections": [
            {"label": "Verse 1", "bars": 8},
            {"label": "Chorus", "bars": 24}
        ]});
        let b = lyrics_technical_brief(&song(120), Some(&st), None);
        assert!(b.contains("- Verse 1: 8 bars → aim for 4-8 lines."), "got: {b}");
        // 24 bars: 12..24 clamps to 10..10, widened to 8-10
        assert!(b.contains("- Chorus: 24 bars → aim for 8-10 lines; land the hook on line 1."), "got: {b}");
    }
}
