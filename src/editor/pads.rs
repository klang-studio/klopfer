//! The pad view: pad grid on the left, the selected pad's editor on the right.

use std::sync::atomic::Ordering;

use klopfer_dsp::{slot_index, NUM_LAYERS, PADS_PER_BANK};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};

use super::widgets::{choice, section, toggle, Knob};
use super::{
    bank_selector, clear_pad, layers_tab, mod_tab, pad_status, pick_sample, theme, waveform, Ctx,
    EditorState, Tab,
};
use crate::bank::{PadStatus, Preview};
use crate::params::note_name;

const PAD_SIZE: f32 = 86.0;
const PAD_GAP: f32 = 8.0;

pub(super) fn pads_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            bank_selector(ui, state);
            pad_grid(ui, c, state);
            ui.add_space(8.0);
            ui.label(
                RichText::new(
                    "Click a pad to play it (higher = louder).\nRight-click for more options.",
                )
                .font(theme::label())
                .color(theme::TEXT_DIM),
            );
        });
        ui.add_space(16.0);
        ui.vertical(|ui| pad_editor(ui, c, state));
    });
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

fn pad_grid(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let side = 4.0 * PAD_SIZE + 3.0 * PAD_GAP;
    let (grid, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());

    for i in 0..PADS_PER_BANK {
        let pad = state.bank * PADS_PER_BANK + i;
        let (row, col) = ((i / 4) as f32, (i % 4) as f32);
        let rect = Rect::from_min_size(
            grid.min + vec2(col * (PAD_SIZE + PAD_GAP), row * (PAD_SIZE + PAD_GAP)),
            vec2(PAD_SIZE, PAD_SIZE),
        );
        let response = ui.interact(rect, ui.id().with(("pad", pad)), Sense::click());

        let pressed =
            response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
        if pressed && state.held_pad != Some(pad) {
            if let Some(old) = state.held_pad {
                let _ = c.bank.previews.push(Preview::Off { pad: old });
            }
            let y = response
                .interact_pointer_pos()
                .map(|p| (p.y - rect.top()) / rect.height())
                .unwrap_or(0.0);
            let velocity = 1.0 - 0.7 * y.clamp(0.0, 1.0);
            let _ = c.bank.previews.push(Preview::On { pad, velocity });
            state.held_pad = Some(pad);
            state.select(pad);
        }

        response.context_menu(|ui| {
            if ui.button("Load sample…").clicked() {
                pick_sample(state, pad, 0, true);
                ui.close_menu();
            }
            if ui.button("Clear").clicked() {
                clear_pad(c, pad);
                ui.close_menu();
            }
        });

        let p = &c.params.pads[pad];
        let color = theme::pad_color(pad);
        let meter = c.bank.meters[pad].load(Ordering::Relaxed);
        let status = pad_status(c, pad);

        let glow = (meter * 1.5).min(1.0);
        let base = mix(theme::KNOB_BODY, color, 0.12 + 0.6 * glow);
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(6), base);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(PAD_SIZE, 4.0)),
            CornerRadius {
                nw: 6,
                ne: 6,
                sw: 0,
                se: 0,
            },
            color,
        );

        let outline = if matches!(status, PadStatus::Missing | PadStatus::Error(_)) {
            Some(Stroke::new(2.0f32, theme::WARN))
        } else if state.selected == pad {
            Some(Stroke::new(2.0f32, theme::TEXT))
        } else if response.hovered() {
            Some(Stroke::new(1.0f32, theme::TEXT_DIM))
        } else {
            None
        };
        if let Some(stroke) = outline {
            painter.rect_stroke(rect, CornerRadius::same(6), stroke, StrokeKind::Inside);
        }

        painter.text(
            rect.left_top() + vec2(7.0, 9.0),
            Align2::LEFT_TOP,
            (pad + 1).to_string(),
            theme::label(),
            theme::TEXT_DIM,
        );
        painter.text(
            rect.right_top() + vec2(-7.0, 9.0),
            Align2::RIGHT_TOP,
            note_name(p.note.value()),
            theme::small(),
            theme::TEXT_DIM,
        );

        // One dot per loaded layer
        let layers = p.layers.read();
        let loaded = layers.layers.iter().filter(|l| l.path.is_some()).count();
        drop(layers);
        if loaded > 1 {
            for d in 0..loaded {
                painter.circle_filled(
                    rect.left_top() + vec2(10.0 + d as f32 * 7.0, 30.0),
                    2.0,
                    theme::TEXT_DIM,
                );
            }
        }

        let name = p.name.read();
        let label = match status {
            PadStatus::Missing => "MISSING",
            PadStatus::Error(_) => "ERROR",
            _ if name.is_empty() => "—",
            _ => name.as_str(),
        };
        let label: String = label.chars().take(13).collect();
        painter.text(
            pos2(rect.center().x, rect.bottom() - 16.0),
            Align2::CENTER_CENTER,
            label,
            theme::label(),
            theme::TEXT,
        );
        if p.mute.value() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "MUTE",
                theme::small(),
                theme::WARN,
            );
        }
    }
}

fn pad_editor(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let pad = state.selected;
    let p = &c.params.pads[pad];
    let layer = state.layer;
    let (peaks, status, path) = {
        let slot = c.bank.slots[slot_index(pad, layer)].read();
        (
            slot.peaks.clone(),
            slot.status.clone(),
            slot.requested.clone(),
        )
    };

    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
        ui.painter()
            .rect_filled(r, CornerRadius::same(2), theme::pad_color(pad));
        ui.label(
            RichText::new(format!("PAD {}", pad + 1))
                .font(theme::heading())
                .strong(),
        );

        let mut name = p.name.read().clone();
        if ui
            .add(egui::TextEdit::singleline(&mut name).desired_width(150.0))
            .changed()
        {
            *p.name.write() = name;
        }

        ui.add_space(8.0);
        for (tab, label) in [
            (Tab::Main, "MAIN"),
            (Tab::Layers, "LAYERS"),
            (Tab::Mod, "MOD"),
        ] {
            if ui.selectable_label(state.tab == tab, label).clicked() {
                state.tab = tab;
            }
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let enabled = !state.dialogs.is_open();
            if ui.button("Clear").clicked() {
                super::assign_sample(c, pad, layer, None, layer == 0);
            }
            let label = if matches!(status, PadStatus::Missing) {
                "Relink…"
            } else {
                "Load…"
            };
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                let rename = layer == 0 && !matches!(status, PadStatus::Missing);
                pick_sample(state, pad, layer, rename);
            }
        });
    });

    if state.tab == Tab::Mod {
        ui.add_space(4.0);
        mod_tab::show(ui, c, pad);
        return;
    }

    ui.horizontal(|ui| {
        let detail = match (&status, &path) {
            (PadStatus::Error(e), Some(p)) => RichText::new(format!("{p}: {e}")).color(theme::WARN),
            (PadStatus::Missing, Some(p)) => {
                RichText::new(format!("Not found: {p}")).color(theme::WARN)
            }
            (_, Some(p)) => RichText::new(p.as_str()).color(theme::TEXT_DIM),
            _ => RichText::new("No sample loaded").color(theme::TEXT_DIM),
        };
        ui.add(egui::Label::new(detail.font(theme::small())).truncate());

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for l in (0..NUM_LAYERS).rev() {
                let loaded = c.bank.slots[slot_index(pad, l)].read().sample.is_some();
                let text = RichText::new(format!("L{}", l + 1)).font(theme::small());
                let text = if loaded {
                    text
                } else {
                    text.color(theme::TEXT_DIM)
                };
                if ui.selectable_label(state.layer == l, text).clicked() {
                    state.layer = l;
                }
            }
            ui.label(
                RichText::new("LAYER")
                    .font(theme::small())
                    .color(theme::TEXT_DIM),
            );
        });
    });

    let width = ui.available_width();
    let wave = waveform::show(ui, vec2(width, 120.0), &peaks, &status, p, c.setter);
    if wave.clicked() && peaks.is_empty() && !state.dialogs.is_open() {
        pick_sample(state, pad, layer, layer == 0);
    }

    ui.add_space(4.0);
    match state.tab {
        Tab::Main => main_tab(ui, c, pad),
        Tab::Layers => layers_tab::show(ui, c, state),
        Tab::Mod => {}
    }
}

fn main_tab(ui: &mut Ui, c: &Ctx, pad: usize) {
    let p = &c.params.pads[pad];
    let s: &ParamSetter = c.setter;

    ui.horizontal(|ui| {
        section(ui, "SAMPLE", |ui| {
            ui.add(Knob::new(&p.tune, s, "Tune").bipolar());
            ui.add(Knob::new(&p.fine, s, "Fine").bipolar());
            ui.add(Knob::new(&p.start, s, "Start"));
            ui.add(Knob::new(&p.end, s, "End"));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                toggle(ui, &p.reverse, s, "REV");
                choice(ui, &p.mode, s, 64.0);
            });
        });
        section(ui, "AMP ENVELOPE", |ui| {
            ui.add(Knob::new(&p.attack, s, "Attack"));
            ui.add(Knob::new(&p.hold, s, "Hold"));
            ui.add(Knob::new(&p.decay, s, "Decay"));
            ui.add(Knob::new(&p.sustain, s, "Sustain"));
            ui.add(Knob::new(&p.release, s, "Release"));
            ui.add(Knob::new(&p.velocity_sens, s, "Velocity"));
        });
    });
    ui.horizontal(|ui| {
        section(ui, "FILTER", |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);
                choice(ui, &p.filter_type, s, 76.0);
            });
            ui.add_space(4.0);
            ui.add(Knob::new(&p.cutoff, s, "Cutoff"));
            ui.add(Knob::new(&p.resonance, s, "Reso"));
            ui.add(Knob::new(&p.filter_env, s, "Env").bipolar());
            ui.add(Knob::new(&p.filter_decay, s, "Decay"));
            ui.add(Knob::new(&p.drive, s, "Drive"));
        });
        section(ui, "PITCH ENV", |ui| {
            ui.add(Knob::new(&p.pitch_env, s, "Amount").bipolar());
            ui.add(Knob::new(&p.pitch_decay, s, "Decay"));
        });
        section(ui, "LO-FI", |ui| {
            ui.add(Knob::new(&p.bits, s, "Bits"));
            ui.add(Knob::new(&p.downsample, s, "Rate"));
        });
    });
    ui.horizontal(|ui| {
        section(ui, "EQ", |ui| {
            ui.add(Knob::new(&p.eq_freq, s, "Freq"));
            ui.add(Knob::new(&p.eq_gain, s, "Gain").bipolar());
            ui.add(Knob::new(&p.eq_q, s, "Q"));
        });
        section(ui, "OUTPUT", |ui| {
            ui.add(Knob::new(&p.volume, s, "Volume"));
            ui.add(Knob::new(&p.pan, s, "Pan").bipolar());
            ui.add(Knob::new(&p.output, s, "Output"));
            ui.add(Knob::new(&p.choke, s, "Choke"));
            ui.add(Knob::new(&p.note, s, "Note"));
            ui.vertical(|ui| {
                ui.add_space(8.0);
                toggle(ui, &p.mute, s, "MUTE");
            });
        });
    });
}
