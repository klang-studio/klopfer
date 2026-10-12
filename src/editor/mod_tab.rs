//! Modulation: LFO, two modulation envelopes and the eight slot matrix.

use nih_plug_egui::egui::{self, RichText, Ui};

use super::widgets::{amount_bar, choice, section, toggle, Knob};
use super::{theme, Ctx};
use crate::params::EnvelopeParams;

fn envelope(ui: &mut Ui, title: &str, env: EnvelopeParams, c: &Ctx) {
    let s = c.setter;
    section(ui, title, |ui| {
        ui.add(Knob::new(env.attack, s, "Attack"));
        ui.add(Knob::new(env.hold, s, "Hold"));
        ui.add(Knob::new(env.decay, s, "Decay"));
        ui.add(Knob::new(env.sustain, s, "Sustain"));
        ui.add(Knob::new(env.release, s, "Release"));
    });
}

pub(super) fn show(ui: &mut Ui, c: &Ctx, pad: usize) {
    let p = &c.params.pads[pad];
    let s = c.setter;

    ui.horizontal(|ui| {
        section(ui, "LFO", |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                choice(ui, &p.lfo_shape, s, 76.0);
                toggle(ui, &p.lfo_retrigger, s, "RETRIG");
            });
            ui.add_space(4.0);
            ui.add(Knob::new(&p.lfo_rate, s, "Rate"));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                toggle(ui, &p.lfo_sync, s, "SYNC");
                choice(ui, &p.lfo_division, s, 64.0);
            });
        });
        envelope(ui, "MOD ENV 1", p.env1(), c);
    });
    ui.horizontal(|ui| {
        envelope(ui, "MOD ENV 2", p.env2(), c);
        ui.label(
            RichText::new(
                "Sources: LFO and noise are bipolar,\nenvelopes, velocity and random unipolar.\n\
                 Start and amp times apply per hit.",
            )
            .font(theme::small())
            .color(theme::TEXT_DIM),
        );
    });

    egui::Frame::NONE
        .fill(theme::PANEL)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.label(
                RichText::new("MATRIX")
                    .font(theme::small())
                    .color(theme::TEXT_DIM)
                    .strong(),
            );
            ui.horizontal_top(|ui| {
                for column in 0..2 {
                    ui.vertical(|ui| {
                        for row in 0..4 {
                            let i = column * 4 + row;
                            let slot = &p.slots[i];
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("{}", i + 1))
                                        .font(theme::small())
                                        .color(theme::TEXT_DIM),
                                );
                                choice(ui, &slot.source, s, 90.0);
                                ui.label(
                                    RichText::new("to")
                                        .font(theme::small())
                                        .color(theme::TEXT_DIM),
                                );
                                choice(ui, &slot.target, s, 90.0);
                                amount_bar(ui, &slot.amount, s, 90.0);
                            });
                        }
                    });
                    if column == 0 {
                        ui.add_space(16.0);
                    }
                }
            });
        });
}
