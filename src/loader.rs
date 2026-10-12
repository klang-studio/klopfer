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
    let mut channels = 0;
    let mut interleaved: Vec<f32> = Vec::new();
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

        match decoder.decode(&packet) {
            Ok(buf) => {
                sample_rate = buf.spec().rate();
                channels = buf.spec().channels().count();
                scratch.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut scratch);
                interleaved.extend_from_slice(&scratch);
            }
            // Skip corrupt packets
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("decode error: {e}")),
        }

        if channels > 0 && interleaved.len() / channels > MAX_FRAMES {
            return Err(String::from("file is too long"));
        }
    }

    if channels == 0 || sample_rate == 0 || interleaved.is_empty() {
        return Err(String::from("file contains no audio"));
    }

    let frames = interleaved.len() / channels;
    let channel =
        |c: usize| -> Vec<f32> { (0..frames).map(|i| interleaved[i * channels + c]).collect() };

    Ok(if channels == 1 {
        Sample::mono(sample_rate as f32, interleaved)
    } else {
        Sample::stereo(sample_rate as f32, channel(0), channel(1))
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
    fn reports_missing_files() {
        assert!(load_sample(Path::new("/definitely/not/here.wav")).is_err());
    }
}
