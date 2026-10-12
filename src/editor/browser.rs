//! The sample browser: the folder tree of the chosen folder with its audio files. A click
//! auditions a file, a double click loads it into the selected pad layer, and files can be dragged
//! onto pads and layers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::collapsing_header::CollapsingState;
use egui::{self, vec2, Align, Layout, RichText, ScrollArea, Sense, TextWrapMode, Ui};

use super::dialogs::Request;
use super::{assign_sample, file_name, theme, Ctx, EditorState};
use crate::{loader, Task};

#[derive(Default)]
pub(super) struct Browser {
    /// Folder contents, read when a folder is first shown
    listings: HashMap<PathBuf, Arc<Listing>>,
    selected: Option<PathBuf>,
}

impl Browser {
    /// Forgets the folder contents so they're read again.
    pub fn refresh(&mut self) {
        self.listings.clear();
    }

    fn listing(&mut self, dir: &Path) -> Arc<Listing> {
        self.listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| Arc::new(Listing::read(dir)))
            .clone()
    }
}

/// The payload of a file dragged out of the browser, see [`super::drop`].
pub(super) struct BrowserDrag(pub PathBuf);

#[derive(Default)]
struct Listing {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
    /// Why the folder couldn't be read
    error: Option<String>,
}

impl Listing {
    /// Subfolders and supported audio files, sorted by name. Hidden entries are skipped.
    fn read(dir: &Path) -> Self {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                return Self {
                    error: Some(e.to_string()),
                    ..Self::default()
                }
            }
        };

        let mut listing = Self::default();
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if path.is_dir() {
                listing.dirs.push(path);
            } else if loader::is_supported(&path) {
                listing.files.push(path);
            }
        }
        let key = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().to_lowercase());
        listing.dirs.sort_by_key(key);
        listing.files.sort_by_key(key);
        listing
    }
}

pub(super) fn browser(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let root = state.settings.browser_dir();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("SAMPLES")
                .font(theme::label())
                .color(theme::TEXT_DIM),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .button("↻")
                .on_hover_text("Read the folders again")
                .clicked()
            {
                state.browser.refresh();
            }
            ui.add_enabled_ui(!state.dialogs.is_open(), |ui| {
                if ui.button("Folder…").clicked() {
                    state
                        .dialogs
                        .show(Request::BrowserFolder, Some(root.clone()));
                }
            });
        });
    });
    ui.add(
        egui::Label::new(
            RichText::new(root.to_string_lossy())
                .font(theme::small())
                .color(theme::TEXT_DIM),
        )
        .truncate(),
    );
    ui.add_space(4.0);

    egui::Frame::NONE
        .fill(theme::PANEL_DARK)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| {
                    ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    folder(ui, c, state, &root);
                });
        });

    drag_label(ui.ctx());
}

/// The contents of `dir`: collapsible subfolders, then files.
fn folder(ui: &mut Ui, c: &Ctx, state: &mut EditorState, dir: &Path) {
    let listing = state.browser.listing(dir);
    if let Some(error) = &listing.error {
        ui.label(RichText::new(error).font(theme::small()).color(theme::WARN));
        return;
    }
    if listing.dirs.is_empty() && listing.files.is_empty() {
        ui.label(
            RichText::new("No samples here")
                .font(theme::small())
                .color(theme::TEXT_DIM),
        );
        return;
    }

    for sub in &listing.dirs {
        let id = ui.make_persistent_id(sub);
        CollapsingState::load_with_default_open(ui.ctx(), id, false)
            .show_header(ui, |ui| {
                ui.label(RichText::new(file_name_of(sub)).color(theme::TEXT));
            })
            .body(|ui| folder(ui, c, state, sub));
    }
    for file in &listing.files {
        file_row(ui, c, state, file);
    }
}

fn file_row(ui: &mut Ui, c: &Ctx, state: &mut EditorState, file: &Path) {
    let selected = state.browser.selected.as_deref() == Some(file);
    let response = ui
        .selectable_label(selected, file_name_of(file))
        .interact(Sense::click_and_drag());

    if response.clicked() {
        state.browser.selected = Some(file.to_path_buf());
        c.executor
            .execute_background(Task::Audition(file.to_path_buf()));
    }
    if response.double_clicked() {
        let (pad, layer) = (state.selected, state.layer);
        // Pads are named after their first layer's sample
        assign_sample(
            c,
            pad,
            layer,
            Some(file.to_string_lossy().into_owned()),
            layer == 0,
        );
        state.message = Some((
            format!("Loaded {} into pad {}", file_name(file), pad + 1),
            false,
        ));
    }
    response.dnd_set_drag_payload(BrowserDrag(file.to_path_buf()));
}

/// The dragged file's name next to the pointer.
fn drag_label(ctx: &egui::Context) {
    let Some(drag) = egui::DragAndDrop::payload::<BrowserDrag>(ctx) else {
        return;
    };
    let Some(pos) = ctx.pointer_interact_pos() else {
        return;
    };
    egui::Area::new(egui::Id::new("klopfer-browser-drag"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + vec2(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(file_name_of(&drag.0)).font(theme::label()));
            });
        });
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_folders_and_audio_files_sorted() {
        let dir = std::env::temp_dir().join(format!("klopfer-browser-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Snares")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for f in ["kick.wav", "Clap.WAV", "notes.txt", ".cache.wav"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }

        let listing = Listing::read(&dir);
        assert_eq!(listing.dirs, vec![dir.join("Snares")]);
        assert_eq!(listing.files, vec![dir.join("Clap.WAV"), dir.join("kick.wav")]);
        assert!(Listing::read(&dir.join("gone")).error.is_some());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
