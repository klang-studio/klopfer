//! Audio file decoding. Runs on background or GUI threads, never on the audio thread.

use std::fs::File;
use std::path::Path;

use klopfer_dsp::Sample;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Refuse anything longer than this many frames (10 minutes at 48 kHz) to protect memory.
const MAX_FRAMES: usize = 48_000 * 600;

pub const SUPPORTED_EXTENSIONS: &[&str] = &["wav", "aif", "aiff", "flac", "ogg", "mp3"];

pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| SUPPORTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Decodes a file into a mono or stereo [`Sample`]. Files with more than two channels keep their
/// first two.
pub fn load_sample(path: &Path) -> Result<Sample, String> {
    let file = File::open(path).map_err(|e| format!("cannot open file: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("unsupported format: {e}"))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or("no audio track")?;
    let track_id = track.id;
    let codec_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("no audio codec parameters")?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("unsupported codec: {e}"))?;

    let mut sample_rate = 0;
    let mut stereo: Option<bool> = None;
    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("read error: {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }

        let buf = match decoder.decode(&packet) {
            Ok(buf) => buf,
            // Skip corrupt packets
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}")),
        };
        let channels = buf.spec().channels().count();
        let frames = buf.frames();
        if channels == 0 || frames == 0 {
            continue;
        }
        // Check before buffering anything, and only ever keep two channels, so neither long
        // files nor files with many channels can exhaust memory
        if left.len() + frames > MAX_FRAMES {
            return Err(String::from("file is too long"));
        }

        sample_rate = buf.spec().rate();
        let is_stereo = *stereo.get_or_insert(channels >= 2);
        scratch.resize(buf.samples_interleaved(), 0.0);
        buf.copy_to_slice_interleaved(&mut scratch);
        for frame in scratch.chunks_exact(channels) {
            left.push(frame[0]);
            if is_stereo {
                right.push(*frame.get(1).unwrap_or(&frame[0]));
            }
        }
    }

    if sample_rate == 0 || left.is_empty() {
        return Err(String::from("file contains no audio"));
    }

    Ok(if stereo == Some(true) {
        Sample::stereo(sample_rate as f32, left, right)
    } else {
        Sample::mono(sample_rate as f32, left)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, channels: u16, frames: usize) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: 44100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            for c in 0..channels {
                let v = if c == 0 { i as i16 } else { -(i as i16) };
                w.write_sample(v).unwrap();
            }
        }
        w.finalize().unwrap();
    }

    #[test]
    fn decodes_mono_and_stereo_wav() {
        let dir = std::env::temp_dir().join(format!("klopfer-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mono = dir.join("mono.wav");
        write_wav(&mono, 1, 1000);
        let s = load_sample(&mono).unwrap();
        assert_eq!(s.len(), 1000);
        assert!(!s.is_stereo());
        assert_eq!(s.sample_rate, 44100.0);

        let stereo = dir.join("stereo.wav");
        write_wav(&stereo, 2, 500);
        let s = load_sample(&stereo).unwrap();
        assert_eq!(s.len(), 500);
        assert!(s.is_stereo());
        assert!(s.left[10] > 0.0 && s.right.as_ref().unwrap()[10] < 0.0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_first_two_of_many_channels() {
        let dir = std::env::temp_dir().join(format!("klopfer-multi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("eight.wav");
        write_wav(&path, 8, 300);

        let s = load_sample(&path).unwrap();
        assert_eq!(s.len(), 300);
        assert!(s.left[10] > 0.0 && s.right.as_ref().unwrap()[10] < 0.0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_missing_files() {
        assert!(load_sample(Path::new("/definitely/not/here.wav")).is_err());
    }

    /// The test kit's loop has hits on every eighth note: slicing must find exactly those.
    #[test]
    fn slices_generated_test_loop() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testkit-out/samples/loop-120.wav");
        if !path.exists() {
            return;
        }
        let loop_sample = load_sample(&path).unwrap();
        let starts = klopfer_dsp::slicing::detect_transients(&loop_sample, 0.5, 0.05);
        assert_eq!(starts.len(), 8, "{starts:?}");
        assert!((klopfer_dsp::slicing::estimate_bpm(&loop_sample) - 120.0).abs() < 0.1);
    }
}
