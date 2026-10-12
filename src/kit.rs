//! Kit files: the plugin state as JSON, with sample paths stored relative to the kit file when the
//! samples live next to it. That keeps kit folders portable between machines.

use std::path::{Component, Path, PathBuf};

use klopfer_dsp::NUM_PADS;
use nih_plug::prelude::PluginState;

pub const EXTENSION: &str = "klopfer";

const EDITOR_STATE_KEY: &str = "editor-state";
const KIT_NAME_KEY: &str = "kit-name";

fn path_key(pad: usize) -> String {
    format!("path_{}", pad + 1)
}

fn get_path(state: &PluginState, pad: usize) -> Option<String> {
    state
        .fields
        .get(&path_key(pad))
        .and_then(|v| serde_json::from_str::<Option<String>>(v).ok())
        .flatten()
}

fn set_path(state: &mut PluginState, pad: usize, path: Option<String>) {
    // Serializing an Option<String> can't fail
    let json = serde_json::to_string(&path).unwrap_or_else(|_| String::from("null"));
    state.fields.insert(path_key(pad), json);
}

fn kit_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Returns `path` relative to `base` using forward slashes, if it's inside `base`.
fn relative_to(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let parts: Option<Vec<_>> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}

pub fn save(state: &PluginState, path: &Path) -> Result<(), String> {
    let mut state = state.clone();
    let base = path.parent().unwrap_or(Path::new(""));

    state.fields.remove(EDITOR_STATE_KEY);
    state.fields.insert(
        KIT_NAME_KEY.to_owned(),
        serde_json::to_string(&kit_name(path)).map_err(|e| e.to_string())?,
    );
    for pad in 0..NUM_PADS {
        if let Some(p) = get_path(&state, pad) {
            if let Some(rel) = relative_to(Path::new(&p), base) {
                set_path(&mut state, pad, Some(rel));
            }
        }
    }

    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("cannot write kit: {e}"))
}

/// Loads a kit and resolves its sample paths to absolute paths. Samples that moved together with
/// the kit file are found next to it.
pub fn load(path: &Path) -> Result<PluginState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("cannot read kit: {e}"))?;
    let mut state: PluginState =
        serde_json::from_str(&json).map_err(|e| format!("not a Klopfer kit: {e}"))?;
    let base = path.parent().unwrap_or(Path::new(""));

    state.fields.remove(EDITOR_STATE_KEY);
    state.fields.insert(
        KIT_NAME_KEY.to_owned(),
        serde_json::to_string(&kit_name(path)).map_err(|e| e.to_string())?,
    );
    for pad in 0..NUM_PADS {
        // Pads the kit doesn't mention must not keep the previous kit's sample
        state
            .fields
            .entry(format!("name_{}", pad + 1))
            .or_insert_with(|| String::from("\"\""));
        let Some(p) = get_path(&state, pad) else {
            set_path(&mut state, pad, None);
            continue;
        };
        let p = PathBuf::from(p);
        let resolved = if p.is_relative() {
            base.join(&p)
        } else if !p.exists() {
            p.file_name()
                .map(|f| base.join(f))
                .filter(|c| c.exists())
                .unwrap_or(p)
        } else {
            p
        };
        set_path(
            &mut state,
            pad,
            Some(resolved.to_string_lossy().into_owned()),
        );
    }

    Ok(state)
}

/// The sample paths stored in a state object, indexed by pad.
pub fn sample_paths(state: &PluginState) -> [Option<String>; NUM_PADS] {
    std::array::from_fn(|pad| get_path(state, pad))
}

/// The pad names stored in a state object, indexed by pad.
pub fn pad_names(state: &PluginState) -> [String; NUM_PADS] {
    std::array::from_fn(|pad| {
        state
            .fields
            .get(&format!("name_{}", pad + 1))
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default()
    })
}

/// Searches `dir` recursively for files named like the missing samples. Returns `(pad, new path)`.
pub fn find_missing(missing: &[(usize, String)], dir: &Path) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            for (pad, old) in missing {
                let same_name = Path::new(old).file_name() == p.file_name();
                if same_name && !found.iter().any(|(f, _)| f == pad) {
                    found.push((*pad, p.to_string_lossy().into_owned()));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn state_with_paths(paths: &[(usize, &str)]) -> PluginState {
        let mut state = PluginState {
            version: String::from("0.1.0"),
            params: BTreeMap::new(),
            fields: BTreeMap::new(),
        };
        for (pad, p) in paths {
            set_path(&mut state, *pad, Some(p.to_string()));
        }
        state
    }

    #[test]
    fn round_trip_keeps_samples_next_to_kit_portable() {
        let dir = std::env::temp_dir().join(format!("klopfer-kit-{}", std::process::id()));
        let samples = dir.join("samples");
        std::fs::create_dir_all(&samples).unwrap();
        let kick = samples.join("kick.wav");
        std::fs::write(&kick, b"").unwrap();

        let state = state_with_paths(&[(0, kick.to_str().unwrap()), (5, "/elsewhere/snare.wav")]);
        let kit = dir.join("My Kit.klopfer");
        save(&state, &kit).unwrap();

        let raw = std::fs::read_to_string(&kit).unwrap();
        assert!(raw.contains("samples/kick.wav"));
        assert!(!raw.contains(dir.to_str().unwrap()));

        let loaded = load(&kit).unwrap();
        let paths = sample_paths(&loaded);
        assert_eq!(paths[0].as_deref(), Some(kick.to_str().unwrap()));
        assert_eq!(paths[5].as_deref(), Some("/elsewhere/snare.wav"));
        assert_eq!(loaded.fields[KIT_NAME_KEY], "\"My Kit\"");

        assert_eq!(
            find_missing(&[(5, String::from("/gone/kick.wav"))], &dir),
            vec![(5, kick.to_string_lossy().into_owned())]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
