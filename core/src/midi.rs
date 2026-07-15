//! Chords → MIDI parts: the pure music/arrangement logic behind the Ableton
//! song stub (moved out of the Tauri layer — audit Tier-2 #10). No sockets,
//! no DB — everything here is deterministic and unit-testable.

use serde_json::Value;

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
    let tones = if rest.starts_with("dim") { vec![0, 3, 6] }
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
}

pub static DARK_HALFTIME: ArrangementProfile = ArrangementProfile { name: "dark half-time", bass: BassPattern::HalfTime808, sub_bass: true, chords: ChordStyle::Held, pad: true, arp: ArpRate::Off, sparse_melody: true, vel_scale: 0.85 };
pub static TRAP_808: ArrangementProfile = ArrangementProfile { name: "trap 808", bass: BassPattern::HalfTime808, sub_bass: true, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Off, sparse_melody: true, vel_scale: 1.0 };
pub static SYNTHWAVE: ArrangementProfile = ArrangementProfile { name: "synthwave drive", bass: BassPattern::EighthDrive, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Sixteenths, sparse_melody: false, vel_scale: 1.0 };
pub static FOUR_FLOOR: ArrangementProfile = ArrangementProfile { name: "four-on-the-floor", bass: BassPattern::OffbeatSync, sub_bass: false, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 1.0 };
pub static ROCK_DRIVE: ArrangementProfile = ArrangementProfile { name: "rock drive", bass: BassPattern::EighthDrive, sub_bass: false, chords: ChordStyle::Stabs, pad: false, arp: ArpRate::Off, sparse_melody: false, vel_scale: 1.0 };
pub static FOLK_ACOUSTIC: ArrangementProfile = ArrangementProfile { name: "folk acoustic", bass: BassPattern::Sustain, sub_bass: false, chords: ChordStyle::Held, pad: false, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 0.9 };
pub static AMBIENT_WASH: ArrangementProfile = ArrangementProfile { name: "ambient wash", bass: BassPattern::Sustain, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Off, sparse_melody: true, vel_scale: 0.7 };
pub static POP_DEFAULT: ArrangementProfile = ArrangementProfile { name: "pop default", bass: BassPattern::Walking, sub_bass: false, chords: ChordStyle::Held, pad: true, arp: ArpRate::Eighths, sparse_melody: false, vel_scale: 1.0 };

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
    Some(ArrangementProfile {
        name: "preset arrangement",
        bass,
        sub_bass: o.get("sub_bass").and_then(|x| x.as_bool()).unwrap_or(d.sub_bass),
        chords,
        pad: o.get("pad").and_then(|x| x.as_bool()).unwrap_or(d.pad),
        arp,
        sparse_melody: o.get("sparse_melody").and_then(|x| x.as_bool()).unwrap_or(d.sparse_melody),
        vel_scale: o.get("vel_scale").and_then(|x| x.as_f64()).unwrap_or(d.vel_scale).clamp(0.4, 1.2),
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
    else if l.contains("pre") || l.contains("build") { &["Sections", "Bass", "Chords", "Pad", "Chord melody", "Filler"] }
    else if l.contains("break") || l.contains("bridge") { &["Sections", "Bass", "Chords", "Pad", "Filler"] }
    else if l.contains("verse") { &["Sections", "Bass", "Chords", "Pad", "Chord melody"] }
    else { &["Sections", "Bass", "Chords", "Pad", "Chord melody", "Filler", "Arp"] } // chorus / drop / hook / default
}

/// Lay the looped progression along the section timeline using each chord's beats,
/// returning (chord, start_beat, duration_beats) events filling `bars` × 4 beats.
pub fn chord_events(chords: &[(String, i64)], bars: i64) -> Vec<(String, f64, f64)> {
    let total = (bars * 4) as f64;
    let mut out = Vec::new();
    if chords.is_empty() { return out; }
    let (mut t, mut i) = (0.0_f64, 0usize);
    while t < total - 0.01 {
        let (name, beats) = &chords[i % chords.len()];
        let len = (*beats as f64).max(0.5);
        out.push((name.clone(), t, len.min(total - t)));
        t += len;
        i += 1;
    }
    out
}

/// Generate the MIDI notes for one part over a section, honoring each chord's beats
/// (chord events of any length). The profile picks each track's pattern —
/// bass figure, chord treatment, arp rate, pad presence, melody density —
/// and scales velocities (soft washes vs punchy mixes).
pub fn part_notes(part: &str, chords: &[(String, i64)], bars: i64, p: &ArrangementProfile) -> Vec<Value> {
    let mut out = Vec::new();
    let v = |base: i64| ((base as f64 * p.vel_scale) as i64).clamp(20, 127);
    let mut prev_bass = -1i64; // for bass voice-leading across the section
    for (name, start, dur) in chord_events(chords, bars) {
        let Some((pc, tones)) = chord_tones(&name) else { continue };
        let third = tones.get(1).copied().unwrap_or(4);
        let top = tones.last().copied().unwrap_or(7);
        match part {
            "Bass" => {
                // voice-led root register; sub_bass drops an octave into 808 land
                let (base, lo, hi) = if p.sub_bass { (24 + pc, 24, 40) } else { (36 + pc, 31, 47) };
                let root = (if prev_bass < 0 { base } else { nearest_pitch(pc, prev_bass) }).clamp(lo, hi);
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
    out
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
        assert_eq!(chord_tones(""), None);
        assert_eq!(chord_tones("H"), None);
    }

    #[test]
    fn chord_events_loops_progression_honoring_beats_and_clips_the_tail() {
        // 2 bars = 8 beats; Am(4) F(2) loops → Am@0(4), F@4(2), Am@6 clipped to 2
        let evs = chord_events(&[("Am".into(), 4), ("F".into(), 2)], 2);
        assert_eq!(evs, vec![
            ("Am".into(), 0.0, 4.0),
            ("F".into(), 4.0, 2.0),
            ("Am".into(), 6.0, 2.0),
        ]);
        assert!(chord_events(&[], 4).is_empty());
    }

    #[test]
    fn part_notes_smoke_bass_register_and_arp_subdivision() {
        let chords = vec![("Am".to_string(), 4), ("F".to_string(), 4)];
        // Bass (pop default = Walking): notes exist and stay in the walking-bass
        // register (roots clamped 31–47, fifths at most +7 above)
        let bass = part_notes("Bass", &chords, 2, &POP_DEFAULT);
        assert!(!bass.is_empty());
        for n in &bass {
            let p = n["pitch"].as_i64().unwrap();
            assert!((31..=54).contains(&p), "bass pitch {p} out of register");
        }
        // Arp (pop default = Eighths): 8th-note subdivision → 8 notes per 4-beat chord
        let arp = part_notes("Arp", &chords, 2, &POP_DEFAULT);
        assert_eq!(arp.len(), 16);
        // an unknown part yields nothing; unparseable chords are skipped
        assert!(part_notes("Kazoo", &chords, 2, &FOUR_FLOOR).is_empty());
        assert!(part_notes("Bass", &[("??".into(), 4)], 2, &FOUR_FLOOR).is_empty());
    }

    /// The style-aware profiles change the actual notes: dark half-time gets
    /// SPARSE SUB bass and no arp; synthwave gets driving eighths and 16th arps.
    #[test]
    fn profiles_shape_bass_density_register_and_arp() {
        let chords = vec![("F#m".to_string(), 4), ("D".to_string(), 4)];
        // dark half-time: one ringing hit per 4-beat chord (no >=4.0-only ghost
        // fires at exactly 4.0 → 2 notes), all in the sub register
        let dark = part_notes("Bass", &chords, 2, &DARK_HALFTIME);
        let drive = part_notes("Bass", &chords, 2, &SYNTHWAVE);
        assert!(dark.len() < drive.len(), "half-time 808 must be sparser than eighth drive ({} vs {})", dark.len(), drive.len());
        for n in &dark {
            let p = n["pitch"].as_i64().unwrap();
            assert!((24..=40).contains(&p), "808 bass pitch {p} must sit in the sub register");
        }
        // eighth drive: 8 root hits per 4-beat chord
        assert_eq!(drive.len(), 16);
        // arp rates: dark = off, synthwave = 16ths (16 notes per 4-beat chord)
        assert!(part_notes("Arp", &chords, 2, &DARK_HALFTIME).is_empty());
        assert_eq!(part_notes("Arp", &chords, 2, &SYNTHWAVE).len(), 32);
        // pad presence follows the profile
        assert!(part_notes("Pad", &chords, 2, &TRAP_808).is_empty());
        assert!(!part_notes("Pad", &chords, 2, &DARK_HALFTIME).is_empty());
        // ambient wash scales velocities down
        let wash = part_notes("Chords", &chords, 2, &AMBIENT_WASH);
        assert!(wash.iter().all(|n| n["velocity"].as_i64().unwrap() <= 60));
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
