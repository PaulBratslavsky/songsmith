//! Chords → MIDI parts: the pure music/arrangement logic behind the Ableton
//! song stub (moved out of the Tauri layer — audit Tier-2 #10). No sockets,
//! no DB — everything here is deterministic and unit-testable.

use serde_json::{json, Value};

/// Parse a chord name into (root pitch-class 0-11, chord-tone semitone offsets).
pub fn chord_tones(name: &str) -> Option<(i64, Vec<i64>)> {
    let b = name.as_bytes();
    if b.is_empty() { return None; }
    let mut pc: i64 = match b[0].to_ascii_uppercase() {
        b'C' => 0, b'D' => 2, b'E' => 4, b'F' => 5, b'G' => 7, b'A' => 9, b'B' => 11, _ => return None,
    };
    let mut i = 1;
    if i < b.len() && b[i] == b'#' { pc = (pc + 1) % 12; i += 1; }
    else if i < b.len() && b[i] == b'b' { pc = (pc + 11) % 12; i += 1; }
    let rest = &name[i..];
    let tones = if rest == "5" { vec![0, 7, 12] } // power chord: 1-5-8
        else if rest.starts_with("dim") { vec![0, 3, 6] }
        else if rest.starts_with("aug") { vec![0, 4, 8] }
        else if rest.starts_with("sus2") { vec![0, 2, 7] }
        else if rest.starts_with("sus4") { vec![0, 5, 7] }
        else if rest.starts_with("maj7") { vec![0, 4, 7, 11] }
        else if rest.starts_with('m') && !rest.starts_with("maj") {
            if rest.contains('7') { vec![0, 3, 7, 10] } else { vec![0, 3, 7] }
        }
        else if rest.starts_with('7') || rest.starts_with("dom7") { vec![0, 4, 7, 10] }
        else { vec![0, 4, 7] };
    Some((pc, tones))
}

fn mk_note(pitch: i64, start: f64, dur: f64, vel: i64) -> Value {
    serde_json::json!({ "pitch": pitch.clamp(0, 127), "start_time": start, "duration": dur, "velocity": vel, "mute": false })
}

/// The pitch with pitch-class `pc` nearest to `reference` — for voice-leading
/// (the bass walks to the closest root instead of leaping a fixed octave).
pub fn nearest_pitch(pc: i64, reference: i64) -> i64 {
    let base = reference - reference.rem_euclid(12) + pc;
    [base - 12, base, base + 12].into_iter().min_by_key(|&p| (p - reference).abs()).unwrap()
}

/// Fold `pitch` into the register band by WHOLE OCTAVES, preserving its pitch
/// class. A plain `.clamp(lo, hi)` shifts by 1–11 semitones and silently
/// changes the note (audit 2026-07-28: `A → D` voice-led to 50, clamped to 47
/// = B, so the bass played B under a D chord). Bands are ≥12 semitones wide,
/// so every pitch class has a representative inside.
pub fn fold_into_band(pitch: i64, lo: i64, hi: i64) -> i64 {
    let mut p = pitch;
    while p < lo { p += 12; }
    while p > hi { p -= 12; }
    p.clamp(0, 127)
}

// ---- Arrangement profiles (style-aware builds, Phase 1) ---------------------
//
// The style knowledge behind the Ableton stub. Replaces the old single
// `groove: bool` (user-hit: "ethereal witch house … heavy sub-bass 808s,
// 85 BPM half-time" matched the `house`/`wave` keywords and got the BOUNCY
// variant). A profile names the arrangement character per track; Phase 2 will
// let the style skill store one per preset — this keyword mapping stays as the
// fallback.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BassPattern {
    /// long root held the chord's length (folk, ambient)
    Sustain,
    /// sparse low 808 hits that ring — half-time weight (witch house, trap)
    HalfTime808,
    /// driving repeated eighth-note roots (synthwave, rock)
    EighthDrive,
    /// root then fifth (the old "sustained" default — pop)
    Walking,
    /// punchy root + off-beat ghost + fifth (the old "groove" — house, funk)
    OffbeatSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordStyle {
    /// triad held the chord's length
    Held,
    /// short stabs on the chord start and midpoint
    Stabs,
    /// triad pulsing on eighth notes
    Pulse8ths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpRate {
    Off,
    Eighths,
    Sixteenths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumPattern {
    Off,
    /// kick every beat, hats on the off-8ths, snare on 2 & 4
    FourFloor,
    /// kick on 1, snare on 3 — the weight sits half-time (trap, witch house)
    HalfTime,
    /// kick 1 & the and-of-3, snare 2 & 4, straight 8th hats (pop, rock)
    Backbeat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArrangementProfile {
    pub name: &'static str,
    pub bass: BassPattern,
    /// drop the bass an octave into sub territory (808s)
    pub sub_bass: bool,
    pub chords: ChordStyle,
    /// include the atmospheric Pad bed
    pub pad: bool,
    pub arp: ArpRate,
    /// chord-melody plays only the top tone (no mid-chord step)
    pub sparse_melody: bool,
    /// velocity multiplier — soft washes vs punchy mixes
    pub vel_scale: f64,
    /// Phase 3 — swing 0..1: how far off-beat 8ths lag (1.0 ≈ triplet feel)
    pub swing: f64,
    /// Phase 3 — humanize 0..1: deterministic timing/velocity jitter depth
    pub humanize: f64,
    /// Phase 3 — the GM drum stub pattern (kick 36 / snare 38 / closed hat 42)
    pub drums: DrumPattern,
}

pub static DARK_HALFTIME: ArrangementProfile = ArrangementProfile { name: "dark half-time", bass: BassPattern::HalfTime808, sub_bass: true, chords: ChordStyle::Held, pad: true, arp: ArpRate::Off, sparse_melody: true, vel_scale: 0.85, swing: 0.0, humanize: 0.35, drums: DrumPattern::HalfTime };
pub static TRAP_808: ArrangementProfile = ArrangementProfile { name: "trap 808", bass: BassPattern::HalfTime808, sub_bass: true, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Off, sparse_melody: true, vel_scale: 1.0, swing: 0.12, humanize: 0.25, drums: DrumPattern::HalfTime };
pub static SYNTHWAVE: ArrangementProfile = ArrangementProfile { name: "synthwave drive", bass: BassPattern::EighthDrive, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Sixteenths, sparse_melody: false, vel_scale: 1.0, swing: 0.0, humanize: 0.12, drums: DrumPattern::FourFloor };
pub static FOUR_FLOOR: ArrangementProfile = ArrangementProfile { name: "four-on-the-floor", bass: BassPattern::OffbeatSync, sub_bass: false, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 1.0, swing: 0.08, humanize: 0.15, drums: DrumPattern::FourFloor };
pub static ROCK_DRIVE: ArrangementProfile = ArrangementProfile { name: "rock drive", bass: BassPattern::EighthDrive, sub_bass: false, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Off, sparse_melody: false, vel_scale: 1.0, swing: 0.0, humanize: 0.3, drums: DrumPattern::Backbeat };
pub static FOLK_ACOUSTIC: ArrangementProfile = ArrangementProfile { name: "folk acoustic", bass: BassPattern::Sustain, sub_bass: false, chords: ChordStyle::Held, pad: false, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 0.9, swing: 0.15, humanize: 0.45, drums: DrumPattern::Off };
pub static AMBIENT_WASH: ArrangementProfile = ArrangementProfile { name: "ambient wash", bass: BassPattern::Sustain, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Off, sparse_melody: true, vel_scale: 0.7, swing: 0.0, humanize: 0.4, drums: DrumPattern::Off };
pub static POP_DEFAULT: ArrangementProfile = ArrangementProfile { name: "pop default", bass: BassPattern::Walking, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 1.0, swing: 0.0, humanize: 0.2, drums: DrumPattern::Backbeat };

/// Parse a preset's stored arrangement JSON into a profile (Phase 2 — the
/// style-skill-generated, user-editable per-preset profile). Shape:
/// `{"bass":"half_time_808","sub_bass":true,"chords":"held","pad":true,
///   "arp":"off","sparse_melody":true,"vel_scale":0.85}`
/// Unknown/missing fields fall back to the pop default's value; a string that
/// isn't a JSON object (or is empty) returns None → keyword fallback.
pub fn profile_from_json(s: &str) -> Option<ArrangementProfile> {
    let v: serde_json::Value = serde_json::from_str(s.trim()).ok()?;
    let o = v.as_object()?;
    let d = &POP_DEFAULT;
    let bass = match o.get("bass").and_then(|x| x.as_str()).unwrap_or("") {
        "sustain" => BassPattern::Sustain,
        "half_time_808" => BassPattern::HalfTime808,
        "eighth_drive" => BassPattern::EighthDrive,
        "walking" => BassPattern::Walking,
        "offbeat_sync" => BassPattern::OffbeatSync,
        _ => d.bass,
    };
    let chords = match o.get("chords").and_then(|x| x.as_str()).unwrap_or("") {
        "held" => ChordStyle::Held,
        "stabs" => ChordStyle::Stabs,
        "pulse_8ths" => ChordStyle::Pulse8ths,
        _ => d.chords,
    };
    let arp = match o.get("arp").and_then(|x| x.as_str()).unwrap_or("") {
        "off" => ArpRate::Off,
        "eighths" => ArpRate::Eighths,
        "sixteenths" => ArpRate::Sixteenths,
        _ => d.arp,
    };
    let drums = match o.get("drums").and_then(|x| x.as_str()).unwrap_or("") {
        "off" => DrumPattern::Off,
        "four_floor" => DrumPattern::FourFloor,
        "half_time" => DrumPattern::HalfTime,
        "backbeat" => DrumPattern::Backbeat,
        _ => d.drums,
    };
    Some(ArrangementProfile {
        name: "preset arrangement",
        bass,
        sub_bass: o.get("sub_bass").and_then(|x| x.as_bool()).unwrap_or(d.sub_bass),
        chords,
        pad: o.get("pad").and_then(|x| x.as_bool()).unwrap_or(d.pad),
        arp,
        sparse_melody: o.get("sparse_melody").and_then(|x| x.as_bool()).unwrap_or(d.sparse_melody),
        vel_scale: o.get("vel_scale").and_then(|x| x.as_f64()).unwrap_or(d.vel_scale).clamp(0.4, 1.2),
        swing: o.get("swing").and_then(|x| x.as_f64()).unwrap_or(d.swing).clamp(0.0, 1.0),
        humanize: o.get("humanize").and_then(|x| x.as_f64()).unwrap_or(d.humanize).clamp(0.0, 1.0),
        drums,
    })
}

/// Fallback style mapping: preset genre text → builtin profile. Ordered by
/// specificity — "synthwave" must beat the bare "wave" family, and "witch
/// house" / "darkwave" must land on the dark profile before the "house"
/// keyword can claim them (the exact bug the old bool mapping had).
pub fn profile_for_genre(genre: &str) -> &'static ArrangementProfile {
    let g = genre.to_lowercase();
    let any = |ks: &[&str]| ks.iter().any(|k| g.contains(k));
    if any(&["synthwave", "retrowave", "chillwave", "vaporwave", "outrun", "synth-pop", "synth pop", "synthpop", "new wave"]) { return &SYNTHWAVE; }
    if any(&["witch", "darkwave", "dark wave", "goth", "ethereal", "shoegaze", "dream pop", "dreampop", "doom"]) { return &DARK_HALFTIME; }
    if any(&["trap", "drill", "phonk", "hip hop", "hip-hop", "rap", "808", "grime"]) { return &TRAP_808; }
    if any(&["house", "techno", "edm", "dance", "club", "electro", "dnb", "drum and bass", "garage"]) { return &FOUR_FLOOR; }
    if any(&["metal", "punk", "rock", "grunge", "hardcore"]) { return &ROCK_DRIVE; }
    if any(&["folk", "acoustic", "country", "singer-songwriter", "singer songwriter", "americana", "bluegrass", "ballad"]) { return &FOLK_ACOUSTIC; }
    if any(&["ambient", "drone", "cinematic", "soundtrack", "score", "new age", "meditat"]) { return &AMBIENT_WASH; }
    &POP_DEFAULT
}

/// Which parts play in a section — thins arrangement so it BUILDS with the energy
/// arc (intro = sparse → chorus/drop = everything) instead of all parts everywhere.
pub fn section_parts(label: &str) -> &'static [&'static str] {
    let l = label.to_lowercase();
    // Pad is the atmospheric bed — it plays under everything
    if l.contains("intro") || l.contains("outro") { &["Sections", "Bass", "Chords", "Pad"] }
    else if l.contains("pre") || l.contains("build") { &["Sections", "Bass", "Chords", "Pad", "Chord melody", "Filler", "Drums"] }
    else if l.contains("break") || l.contains("bridge") { &["Sections", "Bass", "Chords", "Pad", "Filler"] }
    else if l.contains("verse") { &["Sections", "Bass", "Chords", "Pad", "Chord melody", "Drums"] }
    else { &["Sections", "Bass", "Chords", "Pad", "Chord melody", "Filler", "Arp", "Drums"] } // chorus / drop / hook / default
}

/// Lay the looped progression along the section timeline using each chord's beats,
/// returning (source_index, chord, start_beat, duration_beats) events filling
/// `bars` × 4 beats — the source index keys per-chord options (inversions).
pub fn chord_events(chords: &[(String, i64)], bars: i64) -> Vec<(usize, String, f64, f64)> {
    let total = (bars * 4) as f64;
    let mut out = Vec::new();
    if chords.is_empty() { return out; }
    let (mut t, mut i) = (0.0_f64, 0usize);
    while t < total - 0.01 {
        let idx = i % chords.len();
        let (name, beats) = &chords[idx];
        let len = (*beats as f64).max(0.5);
        out.push((idx, name.clone(), t, len.min(total - t)));
        t += len;
        i += 1;
    }
    out
}

/// Rotate chord tones into closed-voicing inversion `k` — wrapped tones jump
/// an octave (same convention as the piano/pad views).
pub fn invert_tones(tones: &[i64], k: i64) -> Vec<i64> {
    let n = tones.len() as i64;
    if n == 0 { return Vec::new(); }
    let k = ((k % n) + n) % n;
    (0..n).map(|i| tones[((k + i) % n) as usize] + if k + i >= n { 12 } else { 0 }).collect()
}

/// Generate the MIDI notes for one part over a section, honoring each chord's beats
/// (chord events of any length). The profile picks each track's pattern —
/// bass figure, chord treatment, arp rate, pad presence, melody density —
/// and scales velocities (soft washes vs punchy mixes).
/// `invs` aligns with `chords` by index (empty = all root position): the
/// picked closed-voicing inversion rotates the tone stack for the harmonic
/// tracks; the BASS stays on the root (a picked inversion is a voicing
/// choice, not a slash-bass instruction).
/// Degree (1–7) + octave band (0|1) → absolute MIDI in the lead register
/// (base C4=60 + key root), same convention as the Composer's melody lane.
pub fn degree_to_midi(degree: i64, octave: i64, root_pc: i64, minor: bool) -> i64 {
    let steps = if minor { [0, 2, 3, 5, 7, 8, 10] } else { [0, 2, 4, 5, 7, 9, 11] };
    60 + root_pc + steps[((degree - 1).clamp(0, 6)) as usize] + 12 * octave.clamp(0, 1)
}

/// Validate + clamp a Melodist output against the song's REAL sections:
/// sections match by label (unknown output labels drop; missing sections get
/// empty notes), degrees clamp to 1–7, octaves to 0–1, every note fits inside
/// its section's bars (16ths), sorted by onset, MONOPHONIC (a later onset
/// truncates the ringing note; zero-length leftovers drop). The model's
/// output is a proposal — this is the contract.
pub fn clamp_melody(sections_out: &[Value], parts: &[(String, i64, Vec<(String, i64)>)]) -> Vec<Value> {
    clamp_part_take(sections_out, parts, true)
}

/// The generalized take contract (Melodist + Arranger): `mono` enforces the
/// monophonic truncation (lead/bass/arp); polyphonic parts (pad/chords) keep
/// overlaps — their stacks ARE simultaneous notes — but still clamp/clip/sort.
pub fn clamp_part_take(sections_out: &[Value], parts: &[(String, i64, Vec<(String, i64)>)], mono: bool) -> Vec<Value> {
    // Match each song section to a DISTINCT output section (first unused):
    // `.find()` gave every same-labeled section the FIRST one's notes, so a
    // song with two "Chorus" rows played identical material clipped to the
    // wrong bar counts (audit 2026-07-28).
    let mut used: Vec<bool> = vec![false; sections_out.len()];
    parts.iter().map(|(label, bars, _)| {
        let cap = (*bars).max(1) * 16;
        let pick = sections_out.iter().position(|s| {
            s.get("label").and_then(|v| v.as_str())
                .map(|l| l.trim().eq_ignore_ascii_case(label.trim())).unwrap_or(false)
        }).map(|first| {
            // prefer an unused entry with this label; fall back to the first
            sections_out.iter().enumerate().position(|(j, s)| {
                !used[j] && s.get("label").and_then(|v| v.as_str())
                    .map(|l| l.trim().eq_ignore_ascii_case(label.trim())).unwrap_or(false)
            }).unwrap_or(first)
        });
        if let Some(j) = pick { if j < used.len() { used[j] = true; } }
        let mut notes: Vec<(i64, i64, i64, i64)> = pick
            .and_then(|j| sections_out[j].get("notes").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|n| {
                let d = n.get("degree")?.as_i64()?.clamp(1, 7);
                let o = n.get("octave").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 1);
                let st = n.get("start")?.as_i64()?;
                let len = n.get("length").and_then(|v| v.as_i64()).unwrap_or(4).max(1);
                if st < 0 || st >= cap { return None; }
                Some((d, o, st, len.min(cap - st)))
            })
            .collect();
        notes.sort_by_key(|n| n.2);
        let mut out: Vec<Value> = vec![];
        for i in 0..notes.len() {
            let (d, o, st, mut len) = notes[i];
            if mono {
                if let Some(&(_, _, nst, _)) = notes.get(i + 1) {
                    if nst < st + len { len = nst - st; }
                }
            }
            if len >= 1 {
                out.push(json!({ "degree": d, "octave": o, "start": st, "length": len }));
            }
        }
        json!({ "label": label, "notes": out })
    }).collect()
}

/// Per-part rendering constants: (base MIDI for degree 1 at octave 0,
/// accent velocity, off-accent velocity). Registers match the formulaic
/// parts so a generated take sits where the old one did.
pub fn part_take_render(part: &str) -> (i64, f64, f64) {
    match part {
        "bass" => (36, 104.0, 92.0),
        "pad" => (60, 52.0, 44.0),
        "chords" => (48, 82.0, 70.0),
        "arp" => (60, 88.0, 64.0),
        _ => (60, 98.0, 84.0), // lead
    }
}

/// Stored take notes (degree/octave/start/length in 16ths) → the
/// absolute-MIDI note Values the Ableton clip API takes (beats), in the
/// part's own register/velocity band.
pub fn take_notes_abs(notes: &[Value], root_pc: i64, minor: bool, vel_scale: f64, part: &str) -> Vec<Value> {
    let (base, hi, lo) = part_take_render(part);
    notes.iter().filter_map(|n| {
        let d = n.get("degree")?.as_i64()?;
        let o = n.get("octave").and_then(|v| v.as_i64()).unwrap_or(0);
        let st = n.get("start")?.as_i64()?;
        let len = n.get("length").and_then(|v| v.as_i64()).unwrap_or(4);
        let vel = ((if st % 4 == 0 { hi } else { lo }) * vel_scale).clamp(1.0, 127.0) as i64;
        let steps = if minor { [0, 2, 3, 5, 7, 8, 10] } else { [0, 2, 4, 5, 7, 9, 11] };
        let pitch = base + root_pc + steps[((d - 1).clamp(0, 6)) as usize] + 12 * o.clamp(0, 1);
        Some(json!({
            "pitch": pitch,
            "start_time": st as f64 / 4.0,
            "duration": (len as f64 / 4.0) * 0.95,
            "velocity": vel,
        }))
    }).collect()
}

/// Stored Melodist notes → absolute MIDI in the lead register (compat shim).
pub fn melody_notes_abs(notes: &[Value], root_pc: i64, minor: bool, vel_scale: f64) -> Vec<Value> {
    take_notes_abs(notes, root_pc, minor, vel_scale, "lead")
}

pub fn part_notes(part: &str, chords: &[(String, i64)], bars: i64, p: &ArrangementProfile, invs: &[i64]) -> Vec<Value> {
    let mut out = Vec::new();
    let v = |base: i64| ((base as f64 * p.vel_scale) as i64).clamp(20, 127);
    let mut prev_bass = -1i64; // for bass voice-leading across the section
    for (idx, name, start, dur) in chord_events(chords, bars) {
        let Some((pc, root_tones)) = chord_tones(&name) else { continue };
        let tones = invert_tones(&root_tones, invs.get(idx).copied().unwrap_or(0));
        let third = tones.get(1).copied().unwrap_or(4);
        let top = tones.last().copied().unwrap_or(7);
        match part {
            "Bass" => {
                // voice-led root register; sub_bass drops an octave into 808 land
                let (base, lo, hi) = if p.sub_bass { (24 + pc, 24, 40) } else { (36 + pc, 31, 47) };
                let root = fold_into_band(if prev_bass < 0 { base } else { nearest_pitch(pc, prev_bass) }, lo, hi);
                prev_bass = root;
                let fifth = root + 7;
                match p.bass {
                    // one long root that rings the chord out
                    BassPattern::Sustain => out.push(mk_note(root, start, dur * 0.95, v(100))),
                    // sparse half-time weight: a ringing hit, plus a late ghost pickup on long chords
                    BassPattern::HalfTime808 => {
                        out.push(mk_note(root, start, dur * 0.8, v(112)));
                        if dur >= 4.0 { out.push(mk_note(root, start + dur * 0.875, (dur * 0.12).max(0.5), v(72))); }
                    }
                    // relentless eighth-note roots, accent on the beat
                    BassPattern::EighthDrive => {
                        let n = (dur / 0.5).floor() as i64;
                        for k in 0..n {
                            out.push(mk_note(root, start + k as f64 * 0.5, 0.45, v(if k % 2 == 0 { 104 } else { 82 })));
                        }
                    }
                    // root then fifth (the old sustained default)
                    BassPattern::Walking => {
                        out.push(mk_note(root, start, dur * 0.6, v(106)));
                        out.push(mk_note(fifth, start + dur * 0.6, dur * 0.4, v(88)));
                    }
                    // punchy root + off-beat ghost + fifth (the old groove)
                    BassPattern::OffbeatSync => {
                        out.push(mk_note(root, start, (dur * 0.4).min(1.0), v(112)));
                        if dur >= 2.0 { out.push(mk_note(root, start + dur * 0.375, 0.4, v(82))); }
                        out.push(mk_note(fifth, start + dur * 0.5, dur * 0.45, v(94)));
                    }
                }
            }
            // wide sustained pad bed: triad octave-up held the full chord, soft, with an airy top octave
            "Pad" => {
                if !p.pad { return out; }
                for t in &tones { out.push(mk_note(60 + pc + t, start, dur, v(50))); }
                out.push(mk_note(72 + pc, start, dur, v(38)));
            }
            "Chords" => match p.chords {
                ChordStyle::Held => for t in &tones { out.push(mk_note(48 + pc + t, start, dur, v(78))); },
                ChordStyle::Stabs => {
                    for t in &tones { out.push(mk_note(48 + pc + t, start, (dur * 0.25).min(0.9), v(90))); }
                    if dur >= 2.0 { for t in &tones { out.push(mk_note(48 + pc + t, start + dur * 0.5, (dur * 0.2).min(0.9), v(74))); } }
                }
                ChordStyle::Pulse8ths => {
                    let n = (dur / 0.5).floor() as i64;
                    for k in 0..n {
                        for t in &tones { out.push(mk_note(48 + pc + t, start + k as f64 * 0.5, 0.4, v(if k % 2 == 0 { 84 } else { 66 }))); }
                    }
                }
            },
            // contour: top tone on the chord; the mid-chord step to the 3rd only
            // when the profile wants an active melody
            "Chord melody" => {
                out.push(mk_note(60 + pc + top, start, dur * 0.45, v(96)));
                if !p.sparse_melody { out.push(mk_note(60 + pc + third, start + dur * 0.5, dur * 0.45, v(82))); }
            }
            // off-beat triad stabs within the chord
            "Filler" => for off in [0.45_f64, 0.85] { for t in &tones { out.push(mk_note(48 + pc + t, start + dur * off, (dur * 0.12).max(0.25), v(68))); } },
            // arpeggio subdividing the chord's length at the profile's rate
            "Arp" => {
                let step = match p.arp { ArpRate::Off => return out, ArpRate::Eighths => 0.5, ArpRate::Sixteenths => 0.25 };
                let n = (dur / step).floor() as i64;
                for k in 0..n {
                    let t = tones[k as usize % tones.len()];
                    let oct = ((k as usize / tones.len()) % 2) as i64 * 12;
                    let vel = if k % 4 == 0 { 86 } else { 64 };
                    out.push(mk_note(60 + pc + t + oct, start + k as f64 * step, step, v(vel)));
                }
            }
            _ => {}
        }
    }
    // Drums are bar-driven, not chord-driven — generate once over the section
    // (GM: kick 36, snare 38, closed hat 42; you drop a drum rack on the track)
    if part == "Drums" && p.drums != DrumPattern::Off {
        let total = bars * 4;
        let v = |base: i64| ((base as f64 * p.vel_scale) as i64).clamp(20, 127);
        for beat in 0..total {
            let b = beat as f64;
            let pos = beat % 4; // beat within the bar
            match p.drums {
                DrumPattern::FourFloor => {
                    out.push(mk_note(36, b, 0.4, v(112)));
                    if pos == 1 || pos == 3 { out.push(mk_note(38, b, 0.3, v(96))); }
                    out.push(mk_note(42, b + 0.5, 0.2, v(64)));
                }
                DrumPattern::HalfTime => {
                    if pos == 0 { out.push(mk_note(36, b, 0.4, v(114))); }
                    if pos == 2 { out.push(mk_note(38, b, 0.35, v(100))); }
                    out.push(mk_note(42, b, 0.2, v(if pos == 0 { 70 } else { 52 })));
                }
                DrumPattern::Backbeat => {
                    if pos == 0 { out.push(mk_note(36, b, 0.4, v(110))); }
                    if pos == 2 { out.push(mk_note(36, b + 0.5, 0.35, v(88))); }
                    if pos == 1 || pos == 3 { out.push(mk_note(38, b, 0.3, v(102))); }
                    out.push(mk_note(42, b, 0.18, v(66)));
                    out.push(mk_note(42, b + 0.5, 0.18, v(52)));
                }
                DrumPattern::Off => {}
            }
        }
    }
    apply_feel(&mut out, p);
    out
}

/// Phase 3 FEEL pass, applied to every generated part: swing (off-beat 8ths
/// lag toward a triplet feel) then deterministic humanization (timing ±12ms-ish
/// and velocity jitter scaled by `humanize`). Deterministic on purpose — a
/// tiny xorshift seeded from each note's pitch/position, so tests and repeat
/// builds are stable (no clock, no RNG state).
fn apply_feel(notes: &mut [Value], p: &ArrangementProfile) {
    if p.swing <= 0.0 && p.humanize <= 0.0 {
        return;
    }
    let max_swing = 0.17; // beats — full triplet-ish lag at swing = 1.0
    for (i, n) in notes.iter_mut().enumerate() {
        let Some(start) = n.get("start_time").and_then(|v| v.as_f64()) else { continue };
        let Some(vel) = n.get("velocity").and_then(|v| v.as_i64()) else { continue };
        let pitch = n.get("pitch").and_then(|v| v.as_i64()).unwrap_or(60);
        let mut t = start;
        // swing: notes sitting on the off-8th (x.5 within the beat) lag
        let frac = t - t.floor();
        if p.swing > 0.0 && (frac - 0.5).abs() < 0.05 {
            t += p.swing * max_swing;
        }
        if p.humanize > 0.0 {
            // xorshift seeded from stable note identity → same build, same feel
            let mut s = (pitch as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add((start * 1000.0) as u64)
                .wrapping_add(i as u64) | 1;
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let r1 = ((s % 1000) as f64 / 1000.0) - 0.5; // -0.5..0.5
            s ^= s << 13; s ^= s >> 7; s ^= s << 17;
            let r2 = ((s % 1000) as f64 / 1000.0) - 0.5;
            t += r1 * p.humanize * 0.06; // up to ±30ms-ish at 100 BPM
            let dv = (r2 * p.humanize * 24.0) as i64;
            n["velocity"] = serde_json::json!((vel + dv).clamp(15, 127));
        }
        n["start_time"] = serde_json::json!((t.max(0.0) * 1000.0).round() / 1000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_tones_parses_roots_accidentals_and_qualities() {
        assert_eq!(chord_tones("C"), Some((0, vec![0, 4, 7])));
        assert_eq!(chord_tones("Am"), Some((9, vec![0, 3, 7])));
        assert_eq!(chord_tones("F#m7"), Some((6, vec![0, 3, 7, 10])));
        assert_eq!(chord_tones("Bb"), Some((10, vec![0, 4, 7])));
        assert_eq!(chord_tones("Gmaj7"), Some((7, vec![0, 4, 7, 11])));
        assert_eq!(chord_tones("Dsus4"), Some((2, vec![0, 5, 7])));
        assert_eq!(chord_tones("C5"), Some((0, vec![0, 7, 12])), "power chord = 1-5-8");
        assert_eq!(chord_tones("F#5"), Some((6, vec![0, 7, 12])));
        assert_eq!(chord_tones(""), None);
        assert_eq!(chord_tones("H"), None);
    }

    #[test]
    fn chord_events_loops_progression_honoring_beats_and_clips_the_tail() {
        // 2 bars = 8 beats; Am(4) F(2) loops → Am@0(4), F@4(2), Am@6 clipped to 2
        let evs = chord_events(&[("Am".into(), 4), ("F".into(), 2)], 2);
        assert_eq!(evs, vec![
            (0, "Am".into(), 0.0, 4.0),
            (1, "F".into(), 4.0, 2.0),
            (0, "Am".into(), 6.0, 2.0),
        ]);
        assert!(chord_events(&[], 4).is_empty());
    }

    /// REGRESSION (audit 2026-07-28): the bass must always play the chord's
    /// ROOT. Voice-leading picks the nearest octave, which can land outside
    /// the register band — folding keeps the pitch class, the old linear
    /// clamp silently changed the note (A→D played B under the D chord).
    #[test]
    fn bass_root_pitch_class_survives_the_register_band() {
        const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        for (profile, lo, hi) in [(&POP_DEFAULT, 31, 47), (&TRAP_808, 24, 40)] {
            // every ordered pair of roots — the voice-leading walks between them
            for a in 0..12i64 {
                for b in 0..12i64 {
                    let names = [NAMES[a as usize].to_string(), NAMES[b as usize].to_string()];
                    let chords: Vec<(String, i64)> = names.iter().map(|n: &String| (n.clone(), 4)).collect();
                    let notes = part_notes("Bass", &chords, 2, profile, &[]);
                    assert!(!notes.is_empty(), "bass wrote nothing for {names:?}");
                    // the FIRST note of each chord is its root (humanize jitters
                    // onsets by a few ms, so the window starts slightly early)
                    for (i, want_pc) in [(0.0_f64, a), (4.0, b)] {
                        let first = notes.iter()
                            .filter(|n| { let t = n["start_time"].as_f64().unwrap(); t >= i - 0.25 && t < i + 1.0 })
                            .min_by(|x, y| x["start_time"].as_f64().unwrap().partial_cmp(&y["start_time"].as_f64().unwrap()).unwrap());
                        if let Some(n) = first {
                            let pitch = n["pitch"].as_i64().unwrap();
                            assert_eq!(pitch.rem_euclid(12), want_pc,
                                "{} {names:?}: bass played pc {} instead of the root {want_pc}", profile.name, pitch.rem_euclid(12));
                            assert!((lo..=hi).contains(&pitch), "{} bass {pitch} left the band {lo}..{hi}", profile.name);
                        }
                    }
                }
            }
        }
    }

    /// REGRESSION (audit 2026-07-28): two sections sharing a label must get
    /// their OWN take entries — `.find()` handed both the first one's notes,
    /// clipped to the wrong bar counts.
    #[test]
    fn duplicate_labels_get_distinct_take_sections() {
        let parts = vec![
            ("Chorus".to_string(), 1i64, vec![]),  // cap 16
            ("Chorus".to_string(), 2i64, vec![]),  // cap 32
        ];
        let out = vec![
            serde_json::json!({ "label": "Chorus", "notes": [{ "degree": 1, "start": 0, "length": 4 }] }),
            serde_json::json!({ "label": "Chorus", "notes": [
                { "degree": 5, "start": 0, "length": 4 }, { "degree": 6, "start": 20, "length": 4 }] }),
        ];
        let c = clamp_part_take(&out, &parts, true);
        let deg = |i: usize| c[i]["notes"].as_array().unwrap().iter().map(|n| n["degree"].as_i64().unwrap()).collect::<Vec<_>>();
        assert_eq!(deg(0), vec![1], "first Chorus keeps the first entry");
        assert_eq!(deg(1), vec![5, 6], "second Chorus gets the SECOND entry, not a copy");
    }

    /// fold_into_band keeps the pitch class (a linear clamp does not).
    #[test]
    fn fold_into_band_preserves_pitch_class() {
        for (lo, hi) in [(31, 47), (24, 40)] {
            for p in 0..96i64 {
                let f = fold_into_band(p, lo, hi);
                assert_eq!(f.rem_euclid(12), p.rem_euclid(12), "fold changed the pitch class of {p}");
                assert!((lo..=hi).contains(&f), "fold left {p} outside {lo}..{hi}");
            }
        }
    }

    /// clamp_melody enforces the Melodist contract: label matching (case/space
    /// tolerant), degree/octave clamps, section-bounds clipping, monophonic
    /// truncation, and unknown labels dropping; degree_to_midi lands the
    /// melody register (A minor: 1/0 → A4=69).
    #[test]
    fn melodist_clamp_and_degree_mapping() {
        let parts = vec![
            ("Verse 1".to_string(), 2i64, vec![]),   // cap 32 sixteenths
            ("Chorus".to_string(), 1i64, vec![]),
        ];
        let out = vec![
            serde_json::json!({ "label": "verse 1 ", "notes": [
                { "degree": 9, "octave": 5, "start": 0, "length": 8 },   // clamps to 7 / 1
                { "degree": 3, "start": 4, "length": 40 },               // overlaps prev → prev truncates to 4; clips to cap
                { "degree": 2, "start": 31, "length": 4 },               // clips to len 1
                { "degree": 1, "start": 32, "length": 4 },               // outside → drops
                { "degree": 1, "start": -1, "length": 4 },               // negative → drops
            ]}),
            serde_json::json!({ "label": "Bridge", "notes": [ { "degree": 1, "start": 0, "length": 4 } ] }),
        ];
        let c = clamp_melody(&out, &parts);
        assert_eq!(c.len(), 2, "one entry per REAL section");
        let v: Vec<(i64, i64, i64, i64)> = c[0]["notes"].as_array().unwrap().iter()
            .map(|n| (n["degree"].as_i64().unwrap(), n["octave"].as_i64().unwrap(), n["start"].as_i64().unwrap(), n["length"].as_i64().unwrap()))
            .collect();
        assert_eq!(v, vec![(7, 1, 0, 4), (3, 0, 4, 27), (2, 0, 31, 1)]);
        assert!(c[1]["notes"].as_array().unwrap().is_empty(), "unknown Bridge label dropped; Chorus empty");
        assert_eq!(degree_to_midi(1, 0, 9, true), 69, "A minor tonic → A4");
        assert_eq!(degree_to_midi(3, 1, 9, true), 84, "A minor 3rd, octave up → C6");
    }

    #[test]
    fn part_notes_smoke_bass_register_and_arp_subdivision() {
        let chords = vec![("Am".to_string(), 4), ("F".to_string(), 4)];
        // Bass (pop default = Walking): notes exist and stay in the walking-bass
        // register (roots clamped 31–47, fifths at most +7 above)
        let bass = part_notes("Bass", &chords, 2, &POP_DEFAULT, &[]);
        assert!(!bass.is_empty());
        for n in &bass {
            let p = n["pitch"].as_i64().unwrap();
            assert!((31..=54).contains(&p), "bass pitch {p} out of register");
        }
        // Arp (pop default = Eighths): 8th-note subdivision → 8 notes per 4-beat chord
        let arp = part_notes("Arp", &chords, 2, &POP_DEFAULT, &[]);
        assert_eq!(arp.len(), 16);
        // an unknown part yields nothing; unparseable chords are skipped
        assert!(part_notes("Kazoo", &chords, 2, &FOUR_FLOOR, &[]).is_empty());
        assert!(part_notes("Bass", &[("??".into(), 4)], 2, &FOUR_FLOOR, &[]).is_empty());
    }

    /// The style-aware profiles change the actual notes: dark half-time gets
    /// SPARSE SUB bass and no arp; synthwave gets driving eighths and 16th arps.
    #[test]
    fn profiles_shape_bass_density_register_and_arp() {
        let chords = vec![("F#m".to_string(), 4), ("D".to_string(), 4)];
        // dark half-time: one ringing hit per 4-beat chord (no >=4.0-only ghost
        // fires at exactly 4.0 → 2 notes), all in the sub register
        let dark = part_notes("Bass", &chords, 2, &DARK_HALFTIME, &[]);
        let drive = part_notes("Bass", &chords, 2, &SYNTHWAVE, &[]);
        assert!(dark.len() < drive.len(), "half-time 808 must be sparser than eighth drive ({} vs {})", dark.len(), drive.len());
        for n in &dark {
            let p = n["pitch"].as_i64().unwrap();
            assert!((24..=40).contains(&p), "808 bass pitch {p} must sit in the sub register");
        }
        // eighth drive: 8 root hits per 4-beat chord
        assert_eq!(drive.len(), 16);
        // arp rates: dark = off, synthwave = 16ths (16 notes per 4-beat chord)
        assert!(part_notes("Arp", &chords, 2, &DARK_HALFTIME, &[]).is_empty());
        assert_eq!(part_notes("Arp", &chords, 2, &SYNTHWAVE, &[]).len(), 32);
        // pad presence follows the profile
        assert!(part_notes("Pad", &chords, 2, &TRAP_808, &[]).is_empty());
        assert!(!part_notes("Pad", &chords, 2, &DARK_HALFTIME, &[]).is_empty());
        // ambient wash scales velocities down
        let wash = part_notes("Chords", &chords, 2, &AMBIENT_WASH, &[]);
        assert!(wash.iter().all(|n| n["velocity"].as_i64().unwrap() <= 60));
    }

    /// Closed-voicing inversion rotation + its effect on the Chords track:
    /// inversion picks change the voiced pitches, bass root stays put.
    #[test]
    fn inversions_rotate_voicings_but_not_the_bass() {
        assert_eq!(invert_tones(&[0, 4, 7], 0), vec![0, 4, 7]);
        assert_eq!(invert_tones(&[0, 4, 7], 1), vec![4, 7, 12]);
        assert_eq!(invert_tones(&[0, 4, 7], 2), vec![7, 12, 16]);
        assert_eq!(invert_tones(&[0, 3, 7, 10], 3), vec![10, 12, 15, 19]);
        let chords = vec![("C".to_string(), 4)];
        let root_pos: Vec<i64> = part_notes("Chords", &chords, 1, &POP_DEFAULT, &[])
            .iter().map(|n| n["pitch"].as_i64().unwrap()).collect();
        let inv2: Vec<i64> = part_notes("Chords", &chords, 1, &POP_DEFAULT, &[2])
            .iter().map(|n| n["pitch"].as_i64().unwrap()).collect();
        assert_eq!(root_pos, vec![48, 52, 55]);
        assert_eq!(inv2, vec![55, 60, 64], "2nd inversion voices G C E");
        let bass_a = part_notes("Bass", &chords, 1, &POP_DEFAULT, &[]);
        let bass_b = part_notes("Bass", &chords, 1, &POP_DEFAULT, &[2]);
        assert_eq!(bass_a, bass_b, "the bass stays on the root regardless of inversion");
    }

    /// Preset-stored arrangement JSON: valid JSON parses (with defaults for
    /// missing fields), garbage/empty falls back to None (keyword mapping).
    #[test]
    fn profile_from_json_parses_and_falls_back() {
        let p = profile_from_json(r#"{"bass":"half_time_808","sub_bass":true,"chords":"held","pad":true,"arp":"off","sparse_melody":true,"vel_scale":0.85}"#).unwrap();
        assert_eq!(p.bass, BassPattern::HalfTime808);
        assert!(p.sub_bass);
        assert_eq!(p.arp, ArpRate::Off);
        assert!((p.vel_scale - 0.85).abs() < 1e-9);
        // partial JSON: missing fields take the pop default's values
        let q = profile_from_json(r#"{"bass":"eighth_drive"}"#).unwrap();
        assert_eq!(q.bass, BassPattern::EighthDrive);
        assert_eq!(q.chords, POP_DEFAULT.chords);
        // vel_scale clamps into the sane range
        assert!((profile_from_json(r#"{"vel_scale": 9.0}"#).unwrap().vel_scale - 1.2).abs() < 1e-9);
        // empty / non-JSON → None (keyword fallback)
        assert!(profile_from_json("").is_none());
        assert!(profile_from_json("not json").is_none());
    }

    /// Phase 3 feel: drums follow the pattern, swing lags off-8ths, humanize
    /// jitters deterministically (same input → byte-identical output).
    #[test]
    fn phase3_drums_swing_and_deterministic_humanize() {
        let chords = vec![("Am".to_string(), 4)];
        // four-floor: per bar → 4 kicks + 2 snares + 4 hats = 10 notes
        let ff = part_notes("Drums", &chords, 1, &SYNTHWAVE, &[]);
        assert_eq!(ff.len(), 10, "four-floor bar = 4 kick + 2 snare + 4 hat");
        assert!(ff.iter().any(|n| n["pitch"] == 36) && ff.iter().any(|n| n["pitch"] == 38) && ff.iter().any(|n| n["pitch"] == 42));
        // half-time: 1 kick + 1 snare + 4 hats
        assert_eq!(part_notes("Drums", &chords, 1, &DARK_HALFTIME, &[]).len(), 6);
        // drums Off → nothing
        assert!(part_notes("Drums", &chords, 1, &AMBIENT_WASH, &[]).is_empty());
        // swing: an off-8th hat lands LATE vs the straight profile
        let swung = ArrangementProfile { swing: 1.0, humanize: 0.0, ..SYNTHWAVE };
        let straight = ArrangementProfile { swing: 0.0, humanize: 0.0, ..SYNTHWAVE };
        let hat = |notes: &Vec<Value>| notes.iter().find(|n| n["pitch"] == 42 && n["start_time"].as_f64().unwrap() > 0.4 && n["start_time"].as_f64().unwrap() < 0.8).unwrap()["start_time"].as_f64().unwrap();
        assert!(hat(&part_notes("Drums", &chords, 1, &swung, &[])) > hat(&part_notes("Drums", &chords, 1, &straight, &[])) + 0.1);
        // humanize is deterministic: two identical builds are byte-identical
        let a = part_notes("Chords", &chords, 1, &POP_DEFAULT, &[]);
        let b = part_notes("Chords", &chords, 1, &POP_DEFAULT, &[]);
        assert_eq!(a, b, "humanize must be deterministic");
        // …and actually changes velocities vs humanize 0
        let dry = ArrangementProfile { humanize: 0.0, swing: 0.0, ..POP_DEFAULT };
        let c = part_notes("Chords", &chords, 1, &dry, &[]);
        assert_ne!(a, c, "humanize must do SOMETHING");
    }

    /// The keyword fallback mapping — incl. the exact bug the old bool had:
    /// "witch house" / "darkwave" must NOT land on the four-on-the-floor or
    /// synthwave buckets via their `house`/`wave` substrings.
    #[test]
    fn profile_for_genre_maps_families_with_precedence() {
        assert_eq!(profile_for_genre("ethereal witch house darkwave").name, DARK_HALFTIME.name);
        assert_eq!(profile_for_genre("synthwave").name, SYNTHWAVE.name);
        assert_eq!(profile_for_genre("Memphis Phonk").name, TRAP_808.name);
        assert_eq!(profile_for_genre("deep house").name, FOUR_FLOOR.name);
        assert_eq!(profile_for_genre("indie rock").name, ROCK_DRIVE.name);
        assert_eq!(profile_for_genre("folk ballad").name, FOLK_ACOUSTIC.name);
        assert_eq!(profile_for_genre("cinematic ambient").name, AMBIENT_WASH.name);
        assert_eq!(profile_for_genre("k-pop").name, POP_DEFAULT.name);
    }

    #[test]
    fn section_parts_thins_the_arrangement_by_energy() {
        assert!(!section_parts("Intro").contains(&"Arp"));
        assert!(section_parts("Chorus 1").contains(&"Arp"));
        assert!(section_parts("Verse 2").contains(&"Chord melody"));
        assert!(!section_parts("Verse 2").contains(&"Filler"));
    }
}
