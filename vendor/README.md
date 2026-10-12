# Vendored dependencies

Local copies of two GUI dependencies with small fixes for file drag and drop. They replace the git
dependencies through `[patch]` in the workspace `Cargo.toml`. Every change is marked with
`Klopfer patch` in the source.

| Crate | Upstream | Revision | License |
|---|---|---|---|
| `baseview` | https://github.com/RustAudio/baseview | `9a0b42c09d712777b2edb4c5e0cb6baf21e988f0` | MIT / Apache-2.0 |
| `egui-baseview` | https://github.com/BillyDM/egui-baseview | `ec70c3fe6b2f070dcacbc22924431edbe24bd1c0` | MIT |

## Changes

- `baseview/src/macos/view.rs`: drag positions are converted from window to view coordinates, like
  mouse events already were. Without this, drops land at the wrong spot whenever the plugin view is
  embedded in a host window.
- `baseview/src/win/drop_target.rs`: drag positions are converted from screen to client
  coordinates (`ScreenToClient`) for the same reason.
- `baseview/Cargo.toml`: allows the `unexpected_cfgs` lint the `objc` macros trigger; cargo capped
  it while this was a git dependency.
- `egui-baseview/src/window.rs`: forwards `DragEntered`/`DragMoved`/`DragLeft`/`DragDropped` as
  egui's `hovered_files`/`dropped_files` and accepts file drops.
- `egui-baseview/Cargo.toml`: depends on the local `baseview`.

`baseview` doesn't support drag and drop on Linux (X11), so dropping files has no effect there.

nih-plug's standalone wrapper pins a different `baseview` revision (`579130ec`); the patch replaces
it with this copy as well, which builds and runs fine.

When updating nih-plug, check whether these revisions changed and re-apply the patches, or drop
them once upstream handles drag and drop.
