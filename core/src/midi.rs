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
/// (chord events of any length). `groove` (genre-driven) swaps sustained pads for
/// rhythmic stabs/faster arps; velocities accent the chord's start and ghost the rest.
pub fn part_notes(part: &str, chords: &[(String, i64)], bars: i64, groove: bool) -> Vec<Value> {
    let mut out = Vec::new();
    let mut prev_bass = -1i64; // for bass voice-leading across the section
    for (name, start, dur) in chord_events(chords, bars) {
        let Some((pc, tones)) = chord_tones(&name) else { continue };
        let third = tones.get(1).copied().unwrap_or(4);
        let top = tones.last().copied().unwrap_or(7);
        match part {
            // walk the root to the nearest octave (no big leaps), move to the 5th halfway
            "Bass" => {
                let root = (if prev_bass < 0 { 36 + pc } else { nearest_pitch(pc, prev_bass) }).clamp(31, 47);
                prev_bass = root;
                let fifth = root + 7;
                if groove {
                    out.push(mk_note(root, start, (dur * 0.4).min(1.0), 112));
                    if dur >= 2.0 { out.push(mk_note(root, start + dur * 0.375, 0.4, 82)); }   // off-beat sub
                    out.push(mk_note(fifth, start + dur * 0.5, dur * 0.45, 94));
                } else {
                    out.push(mk_note(root, start, dur * 0.6, 106));
                    out.push(mk_note(fifth, start + dur * 0.6, dur * 0.4, 88));
                }
            }
            // wide sustained pad bed: triad octave-up held the full chord, soft, with an airy top octave
            "Pad" => {
                for t in &tones { out.push(mk_note(60 + pc + t, start, dur, 50)); }
                out.push(mk_note(72 + pc, start, dur, 38));
            }
            // sustained pad for the chord's length, or two stabs when grooving
            "Chords" => if groove {
                for t in &tones { out.push(mk_note(48 + pc + t, start, (dur * 0.25).min(0.9), 90)); }
                if dur >= 2.0 { for t in &tones { out.push(mk_note(48 + pc + t, start + dur * 0.5, (dur * 0.2).min(0.9), 74)); } }
            } else {
                for t in &tones { out.push(mk_note(48 + pc + t, start, dur, 78)); }
            },
            // contour: top tone on the chord, step to the 3rd halfway through
            "Chord melody" => {
                out.push(mk_note(60 + pc + top, start, dur * 0.45, 96));
                out.push(mk_note(60 + pc + third, start + dur * 0.5, dur * 0.45, 82));
            }
            // off-beat triad stabs within the chord
            "Filler" => for off in [0.45_f64, 0.85] { for t in &tones { out.push(mk_note(48 + pc + t, start + dur * off, (dur * 0.12).max(0.25), 68)); } },
            // arpeggio subdividing the chord's length (16ths grooving, else 8ths)
            "Arp" => {
                let step = if groove { 0.25 } else { 0.5 };
                let n = (dur / step).floor() as i64;
                for k in 0..n {
                    let t = tones[k as usize % tones.len()];
                    let oct = ((k as usize / tones.len()) % 2) as i64 * 12;
                    let vel = if k % 4 == 0 { 86 } else { 64 };
                    out.push(mk_note(60 + pc + t + oct, start + k as f64 * step, step, vel));
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
        // Bass: notes exist and stay in the walking-bass register (roots clamped
        // 31–47, fifths at most +7 above)
        let bass = part_notes("Bass", &chords, 2, false);
        assert!(!bass.is_empty());
        for n in &bass {
            let p = n["pitch"].as_i64().unwrap();
            assert!((31..=54).contains(&p), "bass pitch {p} out of register");
        }
        // Arp (no groove): 8th-note subdivision → 8 notes per 4-beat chord
        let arp = part_notes("Arp", &chords, 2, false);
        assert_eq!(arp.len(), 16);
        // an unknown part yields nothing; unparseable chords are skipped
        assert!(part_notes("Kazoo", &chords, 2, true).is_empty());
        assert!(part_notes("Bass", &[("??".into(), 4)], 2, true).is_empty());
    }

    #[test]
    fn section_parts_thins_the_arrangement_by_energy() {
        assert!(!section_parts("Intro").contains(&"Arp"));
        assert!(section_parts("Chorus 1").contains(&"Arp"));
        assert!(section_parts("Verse 2").contains(&"Chord melody"));
        assert!(!section_parts("Verse 2").contains(&"Filler"));
    }
}
