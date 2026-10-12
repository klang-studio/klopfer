//! Mixer view: one channel strip per pad of the current bank.

use std::sync::atomic::Ordering;

use klopfer_dsp::PADS_PER_BANK;
use egui::{self, vec2, Align2, CornerRadius, RichText, Sense, Stroke, Ui};

use super::drop::DropTarget;
use super::widgets::{fader, toggle, Knob};
use super::{bank_selector, theme, Ctx, EditorState};

pub(super) fn mixer_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    bank_selector(ui, state);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for i in 0..PADS_PER_BANK {
            let pad = state.bank * PADS_PER_BANK + i;
            let p = &c.params.pads[pad];
            let s = c.setter;
            let color = theme::pad_color(pad);
            let selected = state.selected == pad;

            let strip = egui::Frame::NONE
                .fill(theme::PANEL)
                .corner_radius(CornerRadius::same(6))
                .stroke(if selected {
                    Stroke::new(1.0f32, theme::TEXT)
                } else {
                    Stroke::NONE
                })
                .inner_margin(egui::Margin::symmetric(4, 6))
                .show(ui, |ui| {
                    ui.set_width(56.0);
                    ui.vertical_centered(|ui| {
                        let (r, resp) = ui.allocate_exact_size(vec2(56.0, 30.0), Sense::click());
                        ui.painter().rect_filled(
                            r,
                            CornerRadius::same(4),
                            color.gamma_multiply(0.5),
                        );
                        let name = p.name.read();
                        let label: String = if name.is_empty() {
                            format!("{}", pad + 1)
                        } else {
                            name.chars().take(8).collect()
                        };
                        ui.painter().text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            label,
                            theme::small(),
                            theme::TEXT,
                        );
                        if resp.clicked() {
                            state.select(pad);
                        }

                        let meter = c.bank.meters[pad].load(Ordering::Relaxed);
                        fader(ui, &p.volume, s, 180.0, meter, color);
                        ui.label(
                            RichText::new(p.volume.to_string())
                                .font(theme::small())
                                .color(theme::TEXT_DIM),
                        );
                        ui.add(
                            Knob::new(&p.pan, s, "Pan")
                                .bipolar()
                                .diameter(28.0)
                                .color(color),
                        );
                        toggle(ui, &p.mute, s, "MUTE");
                        ui.add(Knob::new(&p.output, s, "Out").diameter(28.0));
                        ui.add(Knob::new(&p.choke, s, "Choke").diameter(28.0));
                    });
                });
            state
                .drop_zones
                .add(strip.response.rect, DropTarget::Pad(pad));
        }
    });
}
