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

/// Scales to the given peak level.
fn normalize(mut v: Vec<f32>, level: f32) -> Vec<f32> {
    let peak = v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.0 {
        v.iter_mut().for_each(|x| *x *= level / peak);
    }
    v
}

/// (file, audio, peak level, velocity range)
type Layer = (&'static str, Vec<f32>, f32, (u8, u8));

/// One pad of the test kit: its layers and any parameters that differ from the defaults.
struct Pad {
    name: &'static str,
    mode: &'static str,
    layers: Vec<Layer>,
    /// (parameter ID without the pad suffix, serialized value)
    params: Vec<(&'static str, serde_json::Value)>,
}

impl Pad {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            mode: "Velocity",
            layers: Vec::new(),
            params: Vec::new(),
        }
    }

    fn mode(mut self, mode: &'static str) -> Self {
        self.mode = mode;
        self
    }

    fn layer(mut self, file: &'static str, data: Vec<f32>, level: f32, vel: (u8, u8)) -> Self {
        self.layers.push((file, data, level, vel));
        self
    }

    fn param(mut self, id: &'static str, value: serde_json::Value) -> Self {
        self.params.push((id, value));
        self
    }
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

    let snare = |seed: u32, detune: f32| {
        mix(
            &mix(
                &sweep(190.0 * detune, 180.0 * detune, 0.05, 0.08, 0.5),
                &sweep(330.0 * detune, 320.0 * detune, 0.05, 0.06, 0.5),
                0.5,
            ),
            &hiss(seed, 0.15, 0.5, 0.7),
            0.8,
        )
    };

    let pads = vec![
        Pad::new("Kick")
            // Velocity layers: a softer, rounder kick for quiet hits
            .layer(
                "kick-soft.wav",
                sweep(140.0, 48.0, 0.05, 0.3, 1.0),
                0.15,
                (1, 90),
            )
            .layer(
                "kick.wav",
                mix(
                    &sweep(160.0, 45.0, 0.03, 0.35, 1.0),
                    &hiss(1, 0.004, 0.02, 0.5),
                    0.3,
                ),
                0.3,
                (91, 127),
            ),
        Pad::new("Snare")
            // Round robin: three slightly different hits
            .mode("RoundRobin")
            .layer("snare-1.wav", snare(2, 1.0), 0.3, (1, 127))
            .layer("snare-2.wav", snare(12, 1.02), 0.3, (1, 127))
            .layer("snare-3.wav", snare(22, 0.98), 0.3, (1, 127)),
        Pad::new("Clap").layer("clap.wav", clap, 0.3, (1, 127)),
        Pad::new("Rim").layer("rim.wav", rim, 0.3, (1, 127)),
        Pad::new("Hat Closed")
            .layer("hat-closed.wav", hiss(3, 0.035, 0.25, 0.95), 0.3, (1, 127))
            .param("choke", json!({ "i32": 1 })),
        Pad::new("Hat Open")
            .layer("hat-open.wav", hiss(4, 0.35, 1.2, 0.95), 0.3, (1, 127))
            .param("choke", json!({ "i32": 1 })),
        Pad::new("Shaker").layer(
            "shaker.wav",
            render(0.2, {
                let mut n = Noise(5);
                move |t| n.next() * (t / 0.03).min(1.0) * env(t, 0.05)
            }),
            0.3,
            (1, 127),
        ),
        Pad::new("Crash").layer("crash.wav", hiss(6, 1.2, 3.0, 0.9), 0.3, (1, 127)),
        Pad::new("Tom Low").layer(
            "tom-low.wav",
            sweep(130.0, 80.0, 0.08, 0.3, 1.0),
            0.3,
            (1, 127),
        ),
        Pad::new("Tom Mid").layer(
            "tom-mid.wav",
            sweep(190.0, 120.0, 0.08, 0.28, 1.0),
            0.3,
            (1, 127),
        ),
        Pad::new("Tom High").layer(
            "tom-high.wav",
            sweep(270.0, 170.0, 0.08, 0.25, 1.0),
            0.3,
            (1, 127),
        ),
        Pad::new("Cowbell").layer("cowbell.wav", cowbell, 0.3, (1, 127)),
        Pad::new("Zap")
            .layer(
                "zap.wav",
                sweep(2400.0, 90.0, 0.02, 0.15, 0.6),
                0.3,
                (1, 127),
            )
            // Modulation: a fast triangle LFO wobbles the pitch
            .param("lfoshape", json!({ "string": "triangle" }))
            .param("lforate", json!({ "f32": 12.0 }))
            .param("src_1", json!({ "string": "lfo" }))
            .param("dst_1", json!({ "string": "pitch" }))
            .param("amt_1", json!({ "f32": 0.08 })),
    ];

    // Same layout as nih-plug's PluginState: fields hold JSON encoded values, parameter IDs of
    // pad parameters end in the pad number
    let mut fields = serde_json::Map::new();
    let mut params = serde_json::Map::new();
    for (pad, def) in pads.into_iter().enumerate() {
        let n = pad + 1;
        let mut layers = Vec::new();
        for (file, data, level, (lo, hi)) in def.layers {
            write(&samples_dir.join(file), &normalize(data, level));
            layers.push(json!({ "path": format!("samples/{file}"), "vel_lo": lo, "vel_hi": hi }));
        }
        let layer_config = json!({ "mode": def.mode, "layers": layers });
        fields.insert(format!("layers_{n}"), json!(layer_config.to_string()));
        fields.insert(format!("name_{n}"), json!(json!(def.name).to_string()));
        for (id, value) in def.params {
            params.insert(format!("{id}_{n}"), value);
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
