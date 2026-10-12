# Vendored dependencies

Local copies of crates.io dependencies with small fixes. They replace the published crates through
`[patch.crates-io]` in the workspace `Cargo.toml`. Every change is marked with `Klopfer patch` in
the source. The copies are the published packages as cargo unpacks them.

| Crate | Version | Upstream | License |
|---|---|---|---|
| `nice-plug` | 0.4.2 | https://codeberg.org/RustAudio/nice-plug | ISC |
| `baseview` | 0.3.7 | https://github.com/RustAudio/baseview | MIT / Apache-2.0 |
| `egui-baseview` | 0.7.2 | https://codeberg.org/RustAudio/egui-baseview | MIT / Apache-2.0 |

## Changes

- `nice-plug/src/wrapper/clap/wrapper.rs` (`ext_render_set`): a render mode change only stores the
  mode, like NIH-plug did. nice-plug requests a restart when the host switches to offline rendering
  while the plugin is active; REAPER handles that restart late, so the plugin is deactivated or
  reset in the middle of the render, which cuts off or silences the rendered audio
  (`tools/reaper-render-test.py` fails without this).
- `baseview/src/platform/macos/view.rs`: drag positions are converted from window to view
  coordinates, like mouse events already are. Without this, drops land at the wrong spot whenever
  the plugin view is embedded in a host window. (Windows does this already since 0.3.)
- `egui-baseview/src/window.rs`: forwards `DragEntered`/`DragMoved`/`DragLeft`/`DragDropped` as
  egui's `hovered_files`/`dropped_files` and accepts file drops.

## Not patched, worked around in Klopfer

- nice-plug doesn't reactivate the plugin after loading a state (NIH-plug did). Klopfer notices
  loaded states through `Tracked` fields (`src/params.rs`) and applies them in `process()`.
- nice-plug's JACK backend initializes the plugin with the `--sample-rate` option instead of the
  server's rate. The standalone (`src/main.rs`) passes the server's rate.

When updating these crates, copy the new published version (from `~/.cargo/registry/src`), re-apply
the patches, and drop each one once upstream fixes it.
