//! Layer table: up to four samples per pad with velocity ranges, gain and tuning.

use std::path::Path;

use klopfer_dsp::{slot_index, NUM_LAYERS};
use nih_plug_egui::egui::{self, RichText, Ui};

use super::drop::DropTarget;
use super::widgets::plain_choice;
use super::{assign_sample, file_name, pick_sample, theme, Ctx, EditorState};
use crate::bank::PadStatus;
use crate::layers::LayerModeConfig;

pub(super) fn show(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let pad = state.selected;
    let p = &c.params.pads[pad];
    let mut cfg = p.layers.read().clone();
    let before = cfg.clone();

    egui::Frame::NONE
        .fill(theme::PANEL)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("MODE")
                        .font(theme::small())
                        .color(theme::TEXT_DIM),
                );
                plain_choice(
                    ui,
                    ("layer-mode", pad),
                    &mut cfg.mode,
                    &LayerModeConfig::ALL,
                    110.0,
                );
                ui.label(
                    RichText::new(match cfg.mode {
                        LayerModeConfig::Velocity => {
                            "All layers whose velocity range matches play together"
                        }
                        LayerModeConfig::RoundRobin => "Matching layers take turns",
                        LayerModeConfig::Random => "A random matching layer plays",
                    })
                    .font(theme::small())
                    .color(theme::TEXT_DIM),
                );
            });
            ui.add_space(6.0);

            let mut rows = Vec::with_capacity(NUM_LAYERS);
            let grid = egui::Grid::new(("layers", pad))
                .num_columns(7)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    for h in ["", "SAMPLE", "", "VELOCITY", "", "GAIN", "TUNE"] {
                        ui.label(RichText::new(h).font(theme::small()).color(theme::TEXT_DIM));
                    }
                    ui.end_row();

                    for layer in 0..NUM_LAYERS {
                        let status = c.bank.slots[slot_index(pad, layer)].read().status.clone();
                        let l = &mut cfg.layers[layer];

                        let row = ui
                            .selectable_label(state.layer == layer, format!("L{}", layer + 1))
                            .on_hover_text("Show this layer's waveform. Drop samples here.");
                        if row.clicked() {
                            state.layer = layer;
                        }
                        rows.push((layer, row.rect.y_range()));

                        let (text, color) = match (&status, &l.path) {
                            (PadStatus::Missing, _) => (String::from("MISSING"), theme::WARN),
                            (PadStatus::Error(_), _) => (String::from("ERROR"), theme::WARN),
                            (_, Some(path)) => (file_name(Path::new(path)), theme::TEXT),
                            (_, None) => (String::from("—"), theme::TEXT_DIM),
                        };
                        ui.scope(|ui| {
                            ui.set_width(190.0);
                            ui.add(egui::Label::new(RichText::new(text).color(color)).truncate())
                                .on_hover_text(l.path.as_deref().unwrap_or("No sample"));
                        });

                        ui.horizontal(|ui| {
                            let enabled = !state.dialogs.is_open();
                            let label = if status == PadStatus::Missing {
                                "Relink…"
                            } else {
                                "Load…"
                            };
                            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                                pick_sample(state, pad, layer, false);
                            }
                            if ui
                                .add_enabled(l.path.is_some(), egui::Button::new("Clear"))
                                .clicked()
                            {
                                assign_sample(c, pad, layer, None, false);
                                l.path = None;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(&mut l.vel_lo).range(1..=127));
                            ui.label("–");
                            ui.add(egui::DragValue::new(&mut l.vel_hi).range(1..=127));
                        });
                        ui.label("");
                        ui.add(
                            egui::DragValue::new(&mut l.gain_db)
                                .range(-24.0..=12.0)
                                .speed(0.1)
                                .suffix(" dB"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut l.tune)
                                .range(-24.0..=24.0)
                                .speed(0.05)
                                .fixed_decimals(2)
                                .suffix(" st"),
                        );
                        ui.end_row();
                    }
                });
            for (layer, y) in rows {
                let rect = egui::Rect::from_x_y_ranges(grid.response.rect.x_range(), y).expand(4.0);
                state.drop_zones.add(rect, DropTarget::Layer { pad, layer });
            }
        });

    if cfg != before {
        // Paths are changed through `assign_sample()`, which also starts the loading; keep them
        let mut current = p.layers.write();
        for (dst, src) in current.layers.iter_mut().zip(&cfg.layers) {
            let path = dst.path.clone();
            *dst = src.clone();
            dst.path = path;
        }
        current.mode = cfg.mode;
    }
}
