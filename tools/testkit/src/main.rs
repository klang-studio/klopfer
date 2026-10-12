//! Generates a synthetic drum kit (WAV files + a `.klopfer` kit file) for development, so the
//! plugin can be tried without any third-party samples.
//!
//! Usage: `cargo run -p testkit -- <output dir>`

use std::f32::consts::TAU;
use std::path::Path;

use serde_json::json;

const SR: u32 = 44100;

struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        // xorshift32
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn render(secs: f32, mut f: impl FnMut(f32) -> f32) -> Vec<f32> {
    (0..(secs * SR as f32) as usize)
        .map(|i| f(i as f32 / SR as f32))
        .collect()
}

fn env(t: f32, decay: f32) -> f32 {
    (-t / decay).exp()
}

/// A sine whose frequency glides exponentially from `f0` to `f1`.
fn sweep(f0: f32, f1: f32, glide: f32, decay: f32, secs: f32) -> Vec<f32> {
    let mut phase = 0.0;
    render(secs, |t| {
        let f = f1 + (f0 - f1) * env(t, glide);
        phase += TAU * f / SR as f32;
        phase.sin() * env(t, decay)
    })
}

/// Noise through a one-pole high-pass.
fn hiss(seed: u32, decay: f32, secs: f32, hp: f32) -> Vec<f32> {
    let mut n = Noise(seed);
    let (mut prev_in, mut prev_out) = (0.0, 0.0);
    render(secs, |t| {
        let x = n.next();
        let y = hp * (prev_out + x - prev_in);
        prev_in = x;
        prev_out = y;
        y * env(t, decay)
    })
}

fn mix(a: &[f32], b: &[f32], gb: f32) -> Vec<f32> {
    (0..a.len().max(b.len()))
        .map(|i| a.get(i).copied().unwrap_or(0.0) + gb * b.get(i).copied().unwrap_or(0.0))
        .collect()
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.0 {
        v.iter_mut().for_each(|x| *x *= 0.3 / peak);
    }
    v
}

fn write(path: &Path, data: &[f32]) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).expect("cannot create wav");
    for x in data {
        w.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
            .expect("cannot write wav");
    }
    w.finalize().expect("cannot finalize wav");
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("testkit-out"));
    let out = Path::new(&out);
    let samples_dir = out.join("samples");
    std::fs::create_dir_all(&samples_dir).expect("cannot create output dir");

    let clap = {
        let mut n = Noise(7);
        render(0.4, |t| {
            let burst = [0.0, 0.011, 0.023]
                .iter()
                .map(|s| if t >= *s { env(t - s, 0.006) } else { 0.0 })
                .sum::<f32>();
            n.next() * (burst + 0.5 * env(t, 0.12) * (t > 0.023) as u8 as f32)
        })
    };
    let rim = mix(
        &sweep(1700.0, 1700.0, 1.0, 0.012, 0.08),
        &sweep(450.0, 450.0, 1.0, 0.02, 0.08),
        0.6,
    );
    let cowbell = {
        let sq = |f: f32, t: f32| (TAU * f * t).sin().signum();
        render(0.5, |t| (sq(540.0, t) + sq(800.0, t)) * 0.5 * env(t, 0.12))
    };

    // (file, pad name, data, choke group)
    let sounds: Vec<(&str, &str, Vec<f32>, i32)> = vec![
        (
            "kick.wav",
            "Kick",
            mix(
                &sweep(160.0, 45.0, 0.03, 0.35, 1.0),
                &hiss(1, 0.004, 0.02, 0.5),
                0.3,
            ),
            0,
        ),
        (
            "snare.wav",
            "Snare",
            mix(
                &mix(
                    &sweep(190.0, 180.0, 0.05, 0.08, 0.5),
                    &sweep(330.0, 320.0, 0.05, 0.06, 0.5),
                    0.5,
                ),
                &hiss(2, 0.15, 0.5, 0.7),
                0.8,
            ),
            0,
        ),
        ("clap.wav", "Clap", clap, 0),
        ("rim.wav", "Rim", rim, 0),
        (
            "hat-closed.wav",
            "Hat Closed",
            hiss(3, 0.035, 0.25, 0.95),
            1,
        ),
        ("hat-open.wav", "Hat Open", hiss(4, 0.35, 1.2, 0.95), 1),
        (
            "shaker.wav",
            "Shaker",
            render(0.2, {
                let mut n = Noise(5);
                move |t| n.next() * (t / 0.03).min(1.0) * env(t, 0.05)
            }),
            0,
        ),
        ("crash.wav", "Crash", hiss(6, 1.2, 3.0, 0.9), 0),
        (
            "tom-low.wav",
            "Tom Low",
            sweep(130.0, 80.0, 0.08, 0.3, 1.0),
            0,
        ),
        (
            "tom-mid.wav",
            "Tom Mid",
            sweep(190.0, 120.0, 0.08, 0.28, 1.0),
            0,
        ),
        (
            "tom-high.wav",
            "Tom High",
            sweep(270.0, 170.0, 0.08, 0.25, 1.0),
            0,
        ),
        ("cowbell.wav", "Cowbell", cowbell, 0),
        ("zap.wav", "Zap", sweep(2400.0, 90.0, 0.02, 0.15, 0.6), 0),
    ];

    // Same layout as nih-plug's PluginState: fields hold JSON encoded values
    let mut fields = serde_json::Map::new();
    let mut params = serde_json::Map::new();
    for (pad, (file, name, data, choke)) in sounds.into_iter().enumerate() {
        let n = pad + 1;
        write(&samples_dir.join(file), &normalize(data));
        fields.insert(
            format!("path_{n}"),
            json!(json!(format!("samples/{file}")).to_string()),
        );
        fields.insert(format!("name_{n}"), json!(json!(name).to_string()));
        if choke > 0 {
            params.insert(format!("choke_{n}"), json!({ "i32": choke }));
        }
    }

    let kit = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "params": params,
        "fields": fields,
    });
    let kit_path = out.join("Testkit.klopfer");
    std::fs::write(
        &kit_path,
        serde_json::to_string_pretty(&kit).expect("valid json"),
    )
    .expect("cannot write kit");
    println!("Wrote {}", kit_path.display());
}
