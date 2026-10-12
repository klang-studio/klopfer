//! Drag and drop from the file manager and the sample browser: samples onto pads or layers, kit
//! files anywhere.
//!
//! Widgets register their drop zones while the frame is laid out; at the end of the frame the
//! zone under the pointer gets highlighted while files hover and receives them when dropped.

use std::path::PathBuf;

use klopfer_dsp::{NUM_LAYERS, NUM_PADS};
use egui::{self, Align2, CornerRadius, Rect, Stroke, StrokeKind};

use super::browser::BrowserDrag;
use super::dialogs::Outcome;
use super::{apply_dialog_outcome, assign_sample, theme, Ctx, EditorState};
use crate::{kit, loader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DropTarget {
    /// Files go to layer 1 of this pad and the following pads
    Pad(usize),
    /// Files go to this layer and the following layers of the pad
    Layer { pad: usize, layer: usize },
}

#[derive(Default)]
pub(super) struct DropZones(Vec<(Rect, DropTarget)>);

impl DropZones {
    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn add(&mut self, rect: Rect, target: DropTarget) {
        self.0.push((rect, target));
    }

    /// The innermost zone at `pos`: zones registered later sit on top.
    fn at(&self, pos: egui::Pos2) -> Option<(Rect, DropTarget)> {
        self.0.iter().rev().find(|(r, _)| r.contains(pos)).copied()
    }
}

/// What a drop of these files means.
enum Plan {
    Kit(PathBuf),
    Samples(Vec<PathBuf>),
    Nothing(String),
}

fn plan(paths: &[PathBuf]) -> Plan {
    if let Some(kit) = paths.iter().find(|p| {
        p.extension().is_some_and(|e| {
            e.eq_ignore_ascii_case(kit::EXTENSION) || e.eq_ignore_ascii_case("sfz")
        })
    }) {
        return Plan::Kit(kit.clone());
    }

    let mut samples: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.is_file() && loader::is_supported(p))
        .cloned()
        .collect();
    if samples.is_empty() {
        return Plan::Nothing(String::from("No supported audio files"));
    }
    // Finder hands files over in no particular order
    samples.sort();
    Plan::Samples(samples)
}

/// Where each sample goes: `(pad, layer)` pairs, at most as many as fit.
fn placements(target: DropTarget, count: usize) -> Vec<(usize, usize)> {
    match target {
        DropTarget::Pad(pad) => (pad..NUM_PADS).take(count).map(|p| (p, 0)).collect(),
        DropTarget::Layer { pad, layer } => {
            (layer..NUM_LAYERS).take(count).map(|l| (pad, l)).collect()
        }
    }
}

fn describe(target: DropTarget, count: usize) -> String {
    let places = placements(target, count);
    let dropped = count - places.len();
    let text = match (target, places.len()) {
        (DropTarget::Pad(p), 1) => format!("Load into pad {}", p + 1),
        (DropTarget::Pad(p), n) => format!("Load {n} samples into pads {}–{}", p + 1, p + n),
        (DropTarget::Layer { layer, .. }, 1) => format!("Load into layer {}", layer + 1),
        (DropTarget::Layer { layer, .. }, n) => {
            format!("Load {n} samples into layers {}–{}", layer + 1, layer + n)
        }
    };
    if dropped > 0 {
        format!("{text} ({dropped} don't fit)")
    } else {
        text
    }
}

fn overlay(ctx: &egui::Context, rect: Rect, text: &str) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("klopfer-drop"),
    ));
    painter.rect_filled(
        rect,
        CornerRadius::same(6),
        theme::ACCENT.gamma_multiply(0.25),
    );
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(2.0f32, theme::ACCENT),
        StrokeKind::Inside,
    );
    let label_pos = rect.center_bottom() - egui::vec2(0.0, 14.0);
    let galley = painter.layout_no_wrap(text.to_owned(), theme::label(), theme::TEXT);
    let bg = Rect::from_center_size(label_pos, galley.size() + egui::vec2(12.0, 6.0));
    painter.rect_filled(bg, CornerRadius::same(4), theme::PANEL_DARK);
    painter.text(
        label_pos,
        Align2::CENTER_CENTER,
        text,
        theme::label(),
        theme::TEXT,
    );
}

/// Highlights the target while files hover and loads them when dropped. Call after the frame's
/// widgets registered their zones.
pub(super) fn handle(ctx: &egui::Context, c: &Ctx, state: &mut EditorState) {
    let (mut hovered, mut dropped, pos) = ctx.input(|i| {
        (
            i.raw
                .hovered_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect::<Vec<_>>(),
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect::<Vec<_>>(),
            i.pointer.hover_pos(),
        )
    });

    // A file dragged out of the sample browser
    let mut from_browser = false;
    if let Some(drag) = egui::DragAndDrop::payload::<BrowserDrag>(ctx) {
        from_browser = true;
        if ctx.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ctx);
            dropped.push(drag.0.clone());
        } else {
            hovered.push(drag.0.clone());
        }
    }

    let zone = pos.and_then(|p| state.drop_zones.at(p));

    if !hovered.is_empty() {
        match (plan(&hovered), zone) {
            (Plan::Kit(_), _) => overlay(ctx, ctx.content_rect().shrink(4.0), "Load kit"),
            (Plan::Samples(s), Some((rect, target))) => {
                overlay(ctx, rect, &describe(target, s.len()))
            }
            _ => {}
        }
        ctx.request_repaint();
    }

    if dropped.is_empty() {
        return;
    }
    match (plan(&dropped), zone) {
        (Plan::Kit(path), _) => apply_dialog_outcome(c, state, Outcome::LoadKit(path)),
        (Plan::Samples(samples), Some((_, target))) => {
            let places = placements(target, samples.len());
            for ((pad, layer), path) in places.iter().zip(&samples) {
                // Pads are named after their first layer's sample
                let path = Some(path.to_string_lossy().into_owned());
                assign_sample(c, *pad, *layer, path, *layer == 0);
            }
            if let Some(&(pad, layer)) = places.first() {
                state.select(pad);
                state.layer = layer;
            }
            state.last_dir = samples[0].parent().map(|p| p.to_path_buf());
            let skipped = samples.len() - places.len();
            state.message = (skipped > 0).then(|| {
                (
                    format!("{skipped} samples didn't fit and were skipped"),
                    true,
                )
            });
        }
        // Releasing a browser file anywhere else just cancels the drag
        (Plan::Samples(_), None) if from_browser => {}
        (Plan::Samples(_), None) => {
            state.message = Some((String::from("Drop samples onto a pad or a layer"), true));
        }
        (Plan::Nothing(why), _) => state.message = Some((why, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn several_files_fill_following_pads_and_layers() {
        assert_eq!(
            placements(DropTarget::Pad(4), 3),
            vec![(4, 0), (5, 0), (6, 0)]
        );
        assert_eq!(placements(DropTarget::Pad(NUM_PADS - 1), 3).len(), 1);
        assert_eq!(
            placements(DropTarget::Layer { pad: 2, layer: 2 }, 5),
            vec![(2, 2), (2, 3)]
        );
    }

    #[test]
    fn descriptions() {
        assert_eq!(describe(DropTarget::Pad(0), 1), "Load into pad 1");
        assert_eq!(
            describe(DropTarget::Layer { pad: 0, layer: 3 }, 2),
            "Load into layer 4 (1 don't fit)"
        );
    }

    #[test]
    fn kit_files_win_and_unsupported_files_are_ignored() {
        let dir = std::env::temp_dir().join(format!("klopfer-drop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (b, a, txt) = (dir.join("b.wav"), dir.join("a.WAV"), dir.join("notes.txt"));
        for f in [&a, &b, &txt] {
            std::fs::write(f, b"").unwrap();
        }

        match plan(&[b.clone(), txt.clone(), a.clone()]) {
            Plan::Samples(s) => assert_eq!(s, vec![a.clone(), b.clone()]),
            _ => panic!("expected samples"),
        }
        assert!(matches!(plan(&[txt.clone()]), Plan::Nothing(_)));
        assert!(matches!(
            plan(&[a.clone(), dir.join("My Kit.klopfer")]),
            Plan::Kit(_)
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
