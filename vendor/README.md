# Patched dependencies

Our forks of three dependencies, as git submodules on their `klopfer` branch. Each branch starts at
the commit the version on crates.io was published from and adds our fixes on top. They replace the
published crates through `[patch.crates-io]` in the workspace `Cargo.toml`.

| Submodule | Fork | Based on | License |
|---|---|---|---|
| `nice-plug` | https://github.com/klang-studio/nice-plug | 0.4.2 (`91ba75d`, upstream https://codeberg.org/RustAudio/nice-plug) | ISC |
| `baseview` | https://github.com/klang-studio/baseview | 0.3.7 (tag `v0.3.7`, upstream https://github.com/RustAudio/baseview) | MIT / Apache-2.0 |
| `egui-baseview` | https://github.com/klang-studio/egui-baseview | 0.7.2 (`877d3bb`, upstream https://codeberg.org/RustAudio/egui-baseview) | MIT / Apache-2.0 |

```bash
git submodule update --init      # after cloning Klopfer
```

## Changes

- `nice-plug`, CLAP render mode: a render mode change only stores the mode, like NIH-plug did.
  nice-plug requests a restart when the host switches to offline rendering while the plugin is
  active; REAPER handles that restart late, so the plugin is deactivated or reset in the middle of
  the render, which cuts off or silences the rendered audio (`tools/reaper-render-test.py` fails
  without this).
- `nice-plug`, crate manifests: `package.workspace = "../.."`. Without it, cargo resolves the
  inherited manifest fields against Klopfer's workspace and fails. All nice-plug crates come from
  the submodule so they share one `nice-plug-core`.
- `baseview`, macOS: drag positions are converted from window to view coordinates, like mouse
  events already are. Without this, drops land at the wrong spot whenever the plugin view is
  embedded in a host window. (Windows does this already since 0.3.)
- `egui-baseview`: forwards `DragEntered`/`DragMoved`/`DragLeft`/`DragDropped` as egui's
  `hovered_files`/`dropped_files` and accepts file drops.

## Not patched, worked around in Klopfer

- nice-plug doesn't reactivate the plugin after loading a state (NIH-plug did). Klopfer notices
  loaded states through `Tracked` fields (`src/params.rs`) and applies them in `process()`.
- nice-plug's JACK backend initializes the plugin with the `--sample-rate` option instead of the
  server's rate. The standalone (`src/main.rs`) passes the server's rate.

## Updating

Rebase the fork's `klopfer` branch onto the commit of the new release (`.cargo_vcs_info.json` in the
crate on crates.io names it), push it, and move the submodule to the new branch head. Drop patches
once upstream fixes them.
