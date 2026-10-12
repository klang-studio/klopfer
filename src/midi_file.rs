//! Standard MIDI files for the slice sequence ("MIDI to host").

/// Ticks per quarter note
const PPQ: u16 = 480;

/// A note at a time in seconds.
#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub key: u8,
    pub start_secs: f64,
    pub length_secs: f64,
}

fn var_len(mut v: u32, out: &mut Vec<u8>) {
    let mut bytes = vec![(v & 0x7f) as u8];
    v >>= 7;
    while v > 0 {
        bytes.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.extend(bytes.iter().rev());
}

/// A format 0 MIDI file with the notes at `bpm`.
pub fn write(notes: &[Note], bpm: f64) -> Vec<u8> {
    let to_ticks = |secs: f64| (secs * bpm / 60.0 * PPQ as f64).round().max(0.0) as u32;

    // (tick, is_note_on, key); note offs sort before note ons at the same tick
    let mut events: Vec<(u32, bool, u8)> = notes
        .iter()
        .flat_map(|n| {
            let start = to_ticks(n.start_secs);
            let end = to_ticks(n.start_secs + n.length_secs).max(start + 1);
            [(start, true, n.key), (end, false, n.key)]
        })
        .collect();
    events.sort_by_key(|(tick, on, key)| (*tick, *on, *key));

    let mut track = Vec::new();
    // Tempo meta event
    let tempo = (60_000_000.0 / bpm).round() as u32;
    track.extend([0x00, 0xff, 0x51, 0x03]);
    track.extend(&tempo.to_be_bytes()[1..]);

    let mut last = 0;
    for (tick, on, key) in events {
        var_len(tick - last, &mut track);
        last = tick;
        if on {
            track.extend([0x90, key & 0x7f, 100]);
        } else {
            track.extend([0x80, key & 0x7f, 0]);
        }
    }
    track.extend([0x00, 0xff, 0x2f, 0x00]);

    let mut file = Vec::new();
    file.extend(b"MThd");
    file.extend(6u32.to_be_bytes());
    file.extend(0u16.to_be_bytes());
    file.extend(1u16.to_be_bytes());
    file.extend(PPQ.to_be_bytes());
    file.extend(b"MTrk");
    file.extend((track.len() as u32).to_be_bytes());
    file.extend(track);
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_quantities() {
        let mut v = Vec::new();
        var_len(0x7f, &mut v);
        var_len(0x80, &mut v);
        var_len(0x3fff, &mut v);
        assert_eq!(v, vec![0x7f, 0x81, 0x00, 0xff, 0x7f]);
    }

    #[test]
    fn writes_notes_at_tempo() {
        // Two eighth notes at 120 BPM: 0.25 s each = 240 ticks
        let notes = [
            Note {
                key: 36,
                start_secs: 0.0,
                length_secs: 0.25,
            },
            Note {
                key: 37,
                start_secs: 0.25,
                length_secs: 0.25,
            },
        ];
        let f = write(&notes, 120.0);
        assert_eq!(&f[..4], b"MThd");
        assert_eq!(&f[14..18], b"MTrk");
        // Tempo 500000 µs per quarter
        assert_eq!(&f[22..29], &[0x00, 0xff, 0x51, 0x03, 0x07, 0xa1, 0x20]);
        // Note on 36, then 240 ticks later note off 36 and note on 37
        assert_eq!(&f[29..33], &[0x00, 0x90, 36, 100]);
        assert_eq!(&f[33..38], &[0x81, 0x70, 0x80, 36, 0]);
        assert_eq!(&f[38..42], &[0x00, 0x90, 37, 100]);
        assert_eq!(&f[f.len() - 4..], &[0x00, 0xff, 0x2f, 0x00]);
    }
}
