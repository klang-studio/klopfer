//! Kit files: the plugin state as JSON, with sample paths stored relative to the kit file when the
//! samples live next to it. That keeps kit folders portable between machines.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use klopfer_dsp::{slot_index, NUM_LAYERS, NUM_PADS, NUM_SAMPLE_SLOTS};
use nice_plug::prelude::PluginState;

use crate::layers::LayerConfig;

pub const EXTENSION: &str = "klopfer";

/// Fields that belong to the session, not to the kit
const EDITOR_STATE_KEY: &str = "editor-state";
const MIDI_MAP_KEY: &str = "midi-map";
const KIT_NAME_KEY: &str = "kit-name";

fn json_field<T: serde::de::DeserializeOwned>(state: &PluginState, key: &str) -> Option<T> {
    state
        .fields
        .get(key)
        .and_then(|v| serde_json::from_str(v).ok())
}

fn set_json_field(state: &mut PluginState, key: String, value: &impl serde::Serialize) {
    if let Ok(json) = serde_json::to_string(value) {
        state.fields.insert(key, json);
    }
}

/// A pad's layers. Single-sample states from before layers existed become layer 1.
pub fn pad_layers(state: &PluginState, pad: usize) -> LayerConfig {
    let mut cfg: LayerConfig =
        json_field(state, &format!("layers_{}", pad + 1)).unwrap_or_default();
    let legacy: Option<String> =
        json_field::<Option<String>>(state, &format!("path_{}", pad + 1)).flatten();
    if cfg.layers[0].path.is_none() {
        cfg.layers[0].path = legacy;
    }
    cfg
}

fn set_pad_layers(state: &mut PluginState, pad: usize, cfg: &LayerConfig) {
    set_json_field(state, format!("layers_{}", pad + 1), cfg);
    set_json_field(state, format!("path_{}", pad + 1), &None::<String>);
}

fn kit_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// UNC and other network paths. Merely checking whether such a file exists makes Windows connect
/// to the server and send the user's credentials, so paths from kit files must never point there.
fn is_network_path(path: &str) -> bool {
    // Windows accepts any mix of separators, e.g. `/\server\share` is a UNC path as well
    let norm = path.replace('/', "\\");
    let local_verbatim = norm
        .strip_prefix(r"\\?\")
        .is_some_and(|rest| rest.as_bytes().get(1) == Some(&b':'));
    norm.starts_with(r"\\") && !local_verbatim
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

/// Resolves a sample path read from a kit in `base` to a local absolute path.
fn resolve(path: &str, base: &Path) -> String {
    // Kits may come from anywhere: replace network paths by the file name next to the kit,
    // without touching the network
    if is_network_path(path) {
        let name = path.rsplit(['\\', '/']).next().unwrap_or_default();
        return base.join(name).to_string_lossy().into_owned();
    }

    let p = PathBuf::from(path);
    let resolved = if p.is_relative() {
        base.join(&p)
    } else if !p.exists() {
        // Samples that moved together with the kit are found next to it
        p.file_name()
            .map(|f| base.join(f))
            .filter(|c| c.exists())
            .unwrap_or(p)
    } else {
        p
    };
    resolved.to_string_lossy().into_owned()
}

fn set_kit_name(state: &mut PluginState, path: &Path) {
    set_json_field(state, KIT_NAME_KEY.to_owned(), &kit_name(path));
}

pub fn save(state: &PluginState, path: &Path) -> Result<(), String> {
    let mut state = state.clone();
    let base = path.parent().unwrap_or(Path::new(""));

    state.fields.remove(EDITOR_STATE_KEY);
    state.fields.remove(MIDI_MAP_KEY);
    set_kit_name(&mut state, path);
    for pad in 0..NUM_PADS {
        let mut cfg = pad_layers(&state, pad);
        for layer in &mut cfg.layers {
            if let Some(rel) = layer
                .path
                .as_deref()
                .and_then(|p| relative_to(Path::new(p), base))
            {
                layer.path = Some(rel);
            }
        }
        set_pad_layers(&mut state, pad, &cfg);
    }

    let json = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("cannot write kit: {e}"))
}

/// Loads a kit and resolves its sample paths to local absolute paths.
pub fn load(path: &Path) -> Result<PluginState, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("cannot read kit: {e}"))?;
    let mut state: PluginState =
        serde_json::from_str(&json).map_err(|e| format!("not a Klopfer kit: {e}"))?;
    let base = path.parent().unwrap_or(Path::new(""));

    state.fields.remove(EDITOR_STATE_KEY);
    state.fields.remove(MIDI_MAP_KEY);
    set_kit_name(&mut state, path);
    for pad in 0..NUM_PADS {
        // Pads the kit doesn't mention must not keep the previous kit's samples. Missing layer
        // fields deserialize to the defaults, i.e. no samples.
        state
            .fields
            .entry(format!("name_{}", pad + 1))
            .or_insert_with(|| String::from("\"\""));
        let mut cfg = pad_layers(&state, pad);
        for layer in &mut cfg.layers {
            layer.path = layer.path.as_deref().map(|p| resolve(p, base));
        }
        set_pad_layers(&mut state, pad, &cfg);
    }

    Ok(state)
}

/// The sample paths stored in a state object, indexed by sample slot.
pub fn sample_paths(state: &PluginState) -> Vec<Option<String>> {
    let mut paths = vec![None; NUM_SAMPLE_SLOTS];
    for pad in 0..NUM_PADS {
        let cfg = pad_layers(state, pad);
        for layer in 0..NUM_LAYERS {
            paths[slot_index(pad, layer)] = cfg.layers[layer].path.clone();
        }
    }
    paths
}

/// The pad names stored in a state object, indexed by pad.
pub fn pad_names(state: &PluginState) -> Vec<String> {
    (0..NUM_PADS)
        .map(|pad| json_field(state, &format!("name_{}", pad + 1)).unwrap_or_default())
        .collect()
}

/// Upper bound for directory entries visited by [`find_missing()`], so picking a huge folder
/// (or the file system root) can't hang the editor.
const MAX_SEARCH_ENTRIES: usize = 200_000;

/// Searches `dir` recursively for files named like the missing samples. Returns `(pad, new path)`.
/// Each directory is visited once, so symlink cycles terminate.
pub fn find_missing(missing: &[(usize, String)], dir: &Path) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut visited = HashSet::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut budget = MAX_SEARCH_ENTRIES;

    while let Some(d) = stack.pop() {
        let Ok(real) = d.canonicalize() else {
            continue;
        };
        if !visited.insert(real) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            if budget == 0 || found.len() == missing.len() {
                return found;
            }
            budget -= 1;

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

    fn empty_state() -> PluginState {
        PluginState {
            version: String::from("0.1.0"),
            params: BTreeMap::new(),
            fields: BTreeMap::new(),
        }
    }

    /// Paths for layer 1 of the given pads.
    fn state_with_paths(paths: &[(usize, &str)]) -> PluginState {
        let mut state = empty_state();
        for (pad, p) in paths {
            let mut cfg = LayerConfig::default();
            cfg.layers[0].path = Some(p.to_string());
            set_pad_layers(&mut state, *pad, &cfg);
        }
        state
    }

    fn layer_path(state: &PluginState, pad: usize, layer: usize) -> Option<String> {
        sample_paths(state)[slot_index(pad, layer)].clone()
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
        assert_eq!(
            layer_path(&loaded, 0, 0).as_deref(),
            Some(kick.to_str().unwrap())
        );
        assert_eq!(
            layer_path(&loaded, 5, 0).as_deref(),
            Some("/elsewhere/snare.wav")
        );
        assert_eq!(loaded.fields[KIT_NAME_KEY], "\"My Kit\"");

        assert_eq!(
            find_missing(&[(5, String::from("/gone/kick.wav"))], &dir),
            vec![(5, kick.to_string_lossy().into_owned())]
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_network_paths() {
        assert!(is_network_path(r"\\server\share\kick.wav"));
        assert!(is_network_path(r"\\?\UNC\server\share\kick.wav"));
        assert!(is_network_path("//server/share/kick.wav"));
        assert!(is_network_path(r"/\server\share\kick.wav"));
        assert!(is_network_path(r"\/server/share/kick.wav"));
        assert!(is_network_path("//?/UNC/server/share/kick.wav"));
        assert!(!is_network_path(r"\\?\C:\samples\kick.wav"));
        assert!(!is_network_path("/Users/me/kick.wav"));
        assert!(!is_network_path(r"C:\samples\kick.wav"));
    }

    #[test]
    fn network_paths_in_kits_are_replaced() {
        let dir = std::env::temp_dir().join(format!("klopfer-unc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kit = dir.join("evil.klopfer");
        let state = state_with_paths(&[(0, r"\\attacker\share\kick.wav")]);
        std::fs::write(&kit, serde_json::to_string(&state).unwrap()).unwrap();

        let loaded = load(&kit).unwrap();
        let path = layer_path(&loaded, 0, 0).unwrap();
        assert_eq!(Path::new(&path), dir.join("kick.wav"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_search_survives_symlink_cycles() {
        let dir = std::env::temp_dir().join(format!("klopfer-cycle-{}", std::process::id()));
        let sub = dir.join("a");
        std::fs::create_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(&dir, sub.join("loop")).unwrap();
        std::fs::write(sub.join("snare.wav"), b"").unwrap();

        let found = find_missing(
            &[
                (1, String::from("/old/snare.wav")),
                (2, String::from("/old/gone.wav")),
            ],
            &dir,
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn single_sample_states_migrate_to_layer_one() {
        let mut state = empty_state();
        state
            .fields
            .insert(String::from("path_3"), String::from("\"/old/clap.wav\""));
        assert_eq!(layer_path(&state, 2, 0).as_deref(), Some("/old/clap.wav"));
        assert_eq!(layer_path(&state, 2, 1), None);
    }

    #[test]
    fn all_layers_are_resolved_and_session_fields_dropped() {
        let dir = std::env::temp_dir().join(format!("klopfer-layers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = empty_state();
        let mut cfg = LayerConfig::default();
        cfg.layers[2].path = Some(String::from("soft.wav"));
        set_pad_layers(&mut state, 7, &cfg);
        state
            .fields
            .insert(String::from(MIDI_MAP_KEY), String::from("[[1,\"vol_1\"]]"));
        let kit = dir.join("layers.klopfer");
        std::fs::write(&kit, serde_json::to_string(&state).unwrap()).unwrap();

        let loaded = load(&kit).unwrap();
        assert_eq!(
            layer_path(&loaded, 7, 2),
            Some(dir.join("soft.wav").to_string_lossy().into_owned())
        );
        assert!(!loaded.fields.contains_key(MIDI_MAP_KEY));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
