//! Per-stage `text` renderers: turn a stage artifact's structured `data` into
//! the human-readable `text` exactly the way each stage's editor serializes it,
//! so artifact `text` stays consistent no matter who wrote the data (editor,
//! agent run, freeze splice, MCP save). Split out of `agent.rs` (audit Tier-2
//! #11); the old duplicate renderer pair (`structure_text`/`chords_text` used
//! only by reference-import) was COLLAPSED into this one set — these editor
//! mirrors are supersets (string chords AND `{name,beats}` objects, key/tempo
//! notes, conditional role suffix).

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
