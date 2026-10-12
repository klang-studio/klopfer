//! SFZ import. Turns a drum SFZ into a Klopfer state: every key becomes a pad, regions on the same
//! key become layers (velocity ranges, round robin via `seq_length`, random via `lorand`/`hirand`),
//! and `group`/`off_by` pairs become choke groups.
//!
//! Supported opcodes: `sample`, `default_path`, `key`, `lokey`, `hikey`, `pitch_keycenter`,
//! `lovel`, `hivel`, `seq_length`, `seq_position`, `lorand`, `hirand`, `volume`, `pan`, `tune`,
//! `transpose`, `ampeg_attack`, `ampeg_hold`, `ampeg_decay`, `ampeg_sustain`, `ampeg_release`,
//! `loop_mode`, `group`, `off_by`, `trigger`, `#define`. Everything else is ignored.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use klopfer_dsp::{NUM_LAYERS, NUM_PADS};
use nice_plug::prelude::PluginState;
use nice_plug::plugin::ParamValue;

use crate::layers::{LayerConfig, LayerModeConfig, LayerSlotConfig};

type Opcodes = HashMap<String, String>;

/// One `<region>` with all inherited opcodes.
#[derive(Debug, Clone)]
struct Region {
    opcodes: Opcodes,
}

impl Region {
    fn get(&self, key: &str) -> Option<&str> {
        self.opcodes.get(key).map(String::as_str)
    }

    fn num(&self, key: &str) -> Option<f32> {
        self.get(key).and_then(|v| v.trim().parse().ok())
    }

    fn key_opcode(&self, key: &str) -> Option<u8> {
        self.get(key).and_then(parse_key)
    }

    /// The key this region is played on.
    fn key(&self) -> Option<u8> {
        let (lo, hi) = match self.key_opcode("key") {
            Some(k) => (k, k),
            None => (
                self.key_opcode("lokey")?,
                self.key_opcode("hikey").unwrap_or(127),
            ),
        };
        // Multi-key regions land on their root key if it's in range, otherwise on the lowest key
        match self.key_opcode("pitch_keycenter") {
            Some(c) if (lo..=hi).contains(&c) => Some(c),
            _ => Some(lo),
        }
    }
}

/// MIDI note number or SFZ note name (`c4` = 60, `f#2`, `eb3`).
fn parse_key(v: &str) -> Option<u8> {
    let v = v.trim();
    if let Ok(n) = v.parse::<i32>() {
        return u8::try_from(n).ok().filter(|n| *n < 128);
    }
    let mut chars = v.chars();
    let base = match chars.next()?.to_ascii_lowercase() {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (accidental, octave) = match rest.chars().next() {
        Some('#') => (1, &rest[1..]),
        Some('b') if rest.len() > 1 => (-1, &rest[1..]),
        _ => (0, rest.as_str()),
    };
    let octave: i32 = octave.parse().ok()?;
    u8::try_from((octave + 1) * 12 + base + accidental)
        .ok()
        .filter(|n| *n < 128)
}

/// Strips `//` and `/* */` comments.
fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map(|i| &r[i..]).unwrap_or("");
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map(|i| &r[i + 2..]).unwrap_or("");
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// Splits a header's body into opcodes. Values may contain spaces (sample paths): a value runs
/// until the next `name=` or the end of the line.
fn parse_opcodes(body: &str, into: &mut Opcodes) {
    for line in body.lines() {
        let mut rest = line.trim();
        while let Some(eq) = rest.find('=') {
            let name = rest[..eq].trim().to_owned();
            let value_part = &rest[eq + 1..];
            // The value ends where the next " name=" starts
            let mut end = value_part.len();
            let mut search = 0;
            while let Some(next_eq) = value_part[search..].find('=') {
                let candidate = &value_part[..search + next_eq];
                match candidate.rfind(char::is_whitespace) {
                    Some(ws) if !candidate[ws + 1..].is_empty() => {
                        end = ws;
                        break;
                    }
                    _ => search += next_eq + 1,
                }
            }
            into.insert(name, value_part[..end].trim().to_owned());
            rest = value_part[end..].trim_start();
        }
    }
}

/// Parses SFZ text into regions with inherited opcodes and the `<control>` default path.
fn parse(text: &str) -> (Vec<Region>, String) {
    // #define $NAME value
    let mut defines: Vec<(String, String)> = Vec::new();
    let mut lines = Vec::new();
    for line in strip_comments(text).lines() {
        let trimmed = line.trim_start();
        if let Some(def) = trimmed.strip_prefix("#define") {
            let mut parts = def.split_whitespace();
            if let (Some(name), Some(value)) = (parts.next(), parts.next()) {
                defines.push((name.to_owned(), value.to_owned()));
            }
            continue;
        }
        let mut line = line.to_owned();
        // Longest names first so $A doesn't clobber $AB
        defines.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
        for (name, value) in &defines {
            line = line.replace(name.as_str(), value);
        }
        lines.push(line);
    }
    let text = lines.join("\n");

    let mut regions = Vec::new();
    let (mut global, mut master, mut group, mut control) = (
        Opcodes::new(),
        Opcodes::new(),
        Opcodes::new(),
        Opcodes::new(),
    );
    let mut current: Option<(String, Opcodes)> = None;

    let flush = |current: &mut Option<(String, Opcodes)>,
                 global: &mut Opcodes,
                 master: &mut Opcodes,
                 group: &mut Opcodes,
                 control: &mut Opcodes,
                 regions: &mut Vec<Region>| {
        let Some((header, ops)) = current.take() else {
            return;
        };
        match header.as_str() {
            "control" => control.extend(ops),
            "global" => *global = ops,
            "master" => *master = ops,
            "group" => *group = ops,
            "region" => {
                let mut opcodes = global.clone();
                opcodes.extend(master.clone());
                opcodes.extend(group.clone());
                opcodes.extend(ops);
                regions.push(Region { opcodes });
            }
            _ => {}
        }
    };

    let mut rest = text.as_str();
    loop {
        let next = rest.find('<');
        let body = &rest[..next.unwrap_or(rest.len())];
        if let Some((_, ops)) = current.as_mut() {
            parse_opcodes(body, ops);
        }
        let Some(start) = next else {
            break;
        };
        let Some(len) = rest[start..].find('>') else {
            break;
        };
        flush(
            &mut current,
            &mut global,
            &mut master,
            &mut group,
            &mut control,
            &mut regions,
        );
        let header = rest[start + 1..start + len].trim().to_ascii_lowercase();
        // A new master resets the group, a new global resets both
        match header.as_str() {
            "global" => {
                master.clear();
                group.clear();
            }
            "master" => group.clear(),
            _ => {}
        }
        current = Some((header, Opcodes::new()));
        rest = &rest[start + len + 1..];
    }
    flush(
        &mut current,
        &mut global,
        &mut master,
        &mut group,
        &mut control,
        &mut regions,
    );

    let default_path = control.get("default_path").cloned().unwrap_or_default();
    (regions, default_path)
}

pub struct Import {
    /// Plugin state with the kit's pads; parameters not mentioned keep their values
    pub state: PluginState,
    pub pads: usize,
    pub warnings: Vec<String>,
}

fn json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// Turns the SFZ text, located in `dir`, into a plugin state.
pub fn import(text: &str, dir: &Path, name: &str) -> Import {
    let (regions, default_path) = parse(text);
    let mut warnings = Vec::new();

    // Group regions by key, ignoring release triggers
    let mut by_key: BTreeMap<u8, Vec<Region>> = BTreeMap::new();
    let mut skipped = 0;
    for region in regions {
        let playable = region.get("sample").is_some()
            && !matches!(region.get("trigger"), Some("release" | "release_key"));
        match (playable, region.key()) {
            (true, Some(key)) => by_key.entry(key).or_default().push(region),
            _ => skipped += 1,
        }
    }
    if skipped > 0 {
        warnings.push(format!(
            "{skipped} regions without sample, key or with release trigger skipped"
        ));
    }
    if by_key.len() > NUM_PADS {
        warnings.push(format!(
            "{} keys, only the lowest {NUM_PADS} were imported",
            by_key.len()
        ));
    }

    // Choke groups: SFZ groups that cut each other off share a Klopfer choke group
    let mut union: HashMap<i64, i64> = HashMap::new();
    fn find(union: &mut HashMap<i64, i64>, g: i64) -> i64 {
        let parent = *union.entry(g).or_insert(g);
        if parent == g {
            g
        } else {
            let root = find(union, parent);
            union.insert(g, root);
            root
        }
    }
    for region in by_key.values().flatten() {
        if let (Some(g), Some(o)) = (region.num("group"), region.num("off_by")) {
            let (a, b) = (find(&mut union, g as i64), find(&mut union, o as i64));
            union.insert(a, b);
        }
    }
    let mut choke_numbers: HashMap<i64, i32> = HashMap::new();

    let mut state = PluginState {
        version: String::new(),
        params: BTreeMap::new(),
        fields: BTreeMap::new(),
    };
    state
        .fields
        .insert(String::from("kit-name"), json(&name.to_owned()));

    let mut too_many_layers = 0;
    let pads = by_key.len().min(NUM_PADS);
    for (pad, (key, mut regions)) in by_key.into_iter().take(NUM_PADS).enumerate() {
        let n = pad + 1;
        regions.sort_by_key(|r| {
            (
                r.num("seq_position").unwrap_or(1.0) as i32,
                r.num("lovel").unwrap_or(1.0) as i32,
            )
        });
        if regions.len() > NUM_LAYERS {
            too_many_layers += regions.len() - NUM_LAYERS;
            regions.truncate(NUM_LAYERS);
        }

        let mode = if regions
            .iter()
            .any(|r| r.num("seq_length").unwrap_or(1.0) > 1.0)
        {
            LayerModeConfig::RoundRobin
        } else if regions
            .iter()
            .any(|r| r.get("lorand").is_some() || r.get("hirand").is_some())
        {
            LayerModeConfig::Random
        } else {
            LayerModeConfig::Velocity
        };

        let mut layers = LayerConfig {
            mode,
            ..LayerConfig::default()
        };
        for (slot, region) in layers.layers.iter_mut().zip(&regions) {
            let sample = region.get("sample").unwrap_or_default().replace('\\', "/");
            let path: PathBuf = dir.join(default_path.replace('\\', "/")).join(sample);
            *slot = LayerSlotConfig {
                path: Some(path.to_string_lossy().into_owned()),
                vel_lo: region.num("lovel").unwrap_or(1.0).clamp(1.0, 127.0) as u8,
                vel_hi: region.num("hivel").unwrap_or(127.0).clamp(1.0, 127.0) as u8,
                gain_db: region.num("volume").unwrap_or(0.0).clamp(-24.0, 12.0),
                tune: region.num("transpose").unwrap_or(0.0)
                    + region.num("tune").unwrap_or(0.0) / 100.0,
            };
        }

        let first = &regions[0];
        let pad_name = first
            .get("sample")
            .map(|s| {
                Path::new(&s.replace('\\', "/"))
                    .file_stem()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        state.fields.insert(format!("layers_{n}"), json(&layers));
        state.fields.insert(format!("name_{n}"), json(&pad_name));
        state
            .fields
            .insert(format!("path_{n}"), json(&None::<String>));

        let mut set = |id: &str, value: ParamValue| {
            state.params.insert(format!("{id}_{n}"), value);
        };
        set("note", ParamValue::I32(key as i32));
        if let Some(pan) = first.num("pan") {
            set("pan", ParamValue::F32((pan / 100.0).clamp(-1.0, 1.0)));
        }
        let ms = |secs: f32| ParamValue::F32((secs * 1000.0).max(0.0));
        for (opcode, id) in [
            ("ampeg_attack", "att"),
            ("ampeg_hold", "hold"),
            ("ampeg_decay", "dec"),
            ("ampeg_release", "rel"),
        ] {
            if let Some(v) = first.num(opcode) {
                set(id, ms(v));
            }
        }
        if let Some(s) = first.num("ampeg_sustain") {
            set("sus", ParamValue::F32((s / 100.0).clamp(0.0, 1.0)));
        }
        // Without one_shot, SFZ regions stop at note off
        let one_shot = first.get("loop_mode") == Some("one_shot");
        set(
            "mode",
            ParamValue::String(String::from(if one_shot { "one_shot" } else { "gate" })),
        );
        if let Some(g) = first.num("group") {
            let root = find(&mut union, g as i64);
            let members = union.keys().filter(|k| **k != root).count();
            if members > 0 || first.get("off_by").is_some() {
                let next = choke_numbers.len() as i32 + 1;
                let choke = *choke_numbers.entry(root).or_insert(next);
                if choke <= 16 {
                    set("choke", ParamValue::I32(choke));
                }
            }
        }
    }
    if too_many_layers > 0 {
        warnings.push(format!(
            "{too_many_layers} regions beyond {NUM_LAYERS} layers per key were skipped"
        ));
    }

    // Pads beyond the kit are cleared
    for pad in pads..NUM_PADS {
        let n = pad + 1;
        state
            .fields
            .insert(format!("layers_{n}"), json(&LayerConfig::default()));
        state
            .fields
            .insert(format!("name_{n}"), json(&String::new()));
        state
            .fields
            .insert(format!("path_{n}"), json(&None::<String>));
    }

    Import {
        state,
        pads,
        warnings,
    }
}

/// Loads an SFZ file.
pub fn load(path: &Path) -> Result<Import, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read SFZ: {e}"))?;
    let text = String::from_utf8_lossy(&bytes);
    let dir = path.parent().unwrap_or(Path::new(""));
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let import = import(&text, dir, &name);
    if import.pads == 0 {
        return Err(String::from("SFZ contains no playable regions"));
    }
    Ok(import)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIT: &str = r#"
// A small drum kit
<control> default_path=samples/
#define $HAT 42
<global> ampeg_release=0.3
<group> loop_mode=one_shot
<region> sample=kick soft.wav key=36 lovel=1 hivel=90 volume=-3
<region> sample=kick hard.wav key=36 lovel=91 hivel=127
<group> seq_length=2 loop_mode=one_shot
<region> sample=Snare\snare 1.wav key=d2 seq_position=1 /* first */
<region> sample=Snare\snare 2.wav key=d2 seq_position=2 tune=-25
<group> group=1 off_by=2
<region> sample=hat open.wav key=46 pan=-50
<group> group=2
<region> sample=hat closed.wav key=$HAT ampeg_sustain=50
<region> sample=release.wav key=36 trigger=release
"#;

    #[test]
    fn note_names() {
        assert_eq!(parse_key("c4"), Some(60));
        assert_eq!(parse_key("C#2"), Some(37));
        assert_eq!(parse_key("eb3"), Some(51));
        assert_eq!(parse_key("36"), Some(36));
        assert_eq!(parse_key("x1"), None);
    }

    #[test]
    fn values_with_spaces() {
        let mut ops = Opcodes::new();
        parse_opcodes("sample=my kick 01.wav key=36 volume=-3", &mut ops);
        assert_eq!(ops["sample"], "my kick 01.wav");
        assert_eq!(ops["key"], "36");
        assert_eq!(ops["volume"], "-3");
    }

    #[test]
    fn imports_a_drum_kit() {
        let dir = Path::new("/kits/demo");
        let import = import(KIT, dir, "Demo");
        assert_eq!(import.pads, 4);

        // Keys sorted: 36 kick, 38 snare, 42 closed hat, 46 open hat
        let layers = |n: usize| -> LayerConfig {
            serde_json::from_str(&import.state.fields[&format!("layers_{n}")]).unwrap()
        };
        let kick = layers(1);
        assert_eq!(kick.mode, LayerModeConfig::Velocity);
        assert_eq!(
            kick.layers[0].path.as_deref(),
            Some("/kits/demo/samples/kick soft.wav")
        );
        assert_eq!((kick.layers[0].vel_lo, kick.layers[0].vel_hi), (1, 90));
        assert_eq!(kick.layers[0].gain_db, -3.0);
        assert_eq!(kick.layers[1].vel_lo, 91);
        assert!(
            kick.layers[2].path.is_none(),
            "release trigger must be skipped"
        );

        let snare = layers(2);
        assert_eq!(snare.mode, LayerModeConfig::RoundRobin);
        assert_eq!(
            snare.layers[1].path.as_deref(),
            Some("/kits/demo/samples/Snare/snare 2.wav")
        );
        assert_eq!(snare.layers[1].tune, -0.25);

        let params = &import.state.params;
        assert!(matches!(params["note_2"], ParamValue::I32(38)));
        assert!(matches!(params["note_3"], ParamValue::I32(42)));
        assert!(matches!(params["rel_1"], ParamValue::F32(v) if (v - 300.0).abs() < 1e-3));
        assert!(matches!(params["sus_3"], ParamValue::F32(v) if (v - 0.5).abs() < 1e-6));
        assert!(matches!(params["pan_4"], ParamValue::F32(v) if (v + 0.5).abs() < 1e-6));
        assert!(matches!(&params["mode_1"], ParamValue::String(m) if m == "one_shot"));

        // Open and closed hat share a choke group, kick and snare don't choke
        let choke = |n: usize| match params.get(&format!("choke_{n}")) {
            Some(ParamValue::I32(c)) => *c,
            _ => 0,
        };
        assert_eq!(choke(1), 0);
        assert_ne!(choke(3), 0);
        assert_eq!(choke(3), choke(4));

        // Pads after the kit are cleared
        assert_eq!(layers(5), LayerConfig::default());
    }
}

#[cfg(test)]
mod testkit {
    use super::*;

    /// Imports the SFZ written by `cargo run -p testkit -- testkit-out`, if it's there.
    #[test]
    fn imports_generated_test_kit() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("testkit-out/Testkit.sfz");
        if !path.exists() {
            return;
        }
        let import = load(&path).unwrap();
        assert_eq!(import.pads, 14);
        assert!(import.warnings.is_empty(), "{:?}", import.warnings);
        let snare: LayerConfig = serde_json::from_str(&import.state.fields["layers_2"]).unwrap();
        assert_eq!(snare.mode, LayerModeConfig::RoundRobin);
        assert!(Path::new(snare.layers[2].path.as_deref().unwrap()).exists());
        let choke = |n: usize| match import.state.params.get(&format!("choke_{n}")) {
            Some(ParamValue::I32(c)) => *c,
            _ => 0,
        };
        assert_eq!((choke(5), choke(6), choke(1)), (1, 1, 0));
    }
}
