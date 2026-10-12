//! The pad view (sample browser on the left, pad grid on the right) and the editor view (pad strip
//! and the selected pad's editor).

use std::sync::atomic::Ordering;

use klopfer_dsp::{slot_index, NUM_LAYERS, PADS_PER_BANK};
use nice_plug::prelude::*;
use egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};

use super::drop::DropTarget;
use super::widgets::{choice, section, toggle, Knob};
use super::{
    bank_selector, browser, clear_pad, edit_tab, layers_tab, mod_tab, pad_status, pick_sample, rec_tab,
    theme, waveform, Ctx, EditorState, Tab, View,
};
use crate::bank::{PadStatus, Preview};
use crate::params::note_name;

const PAD_GAP: f32 = 8.0;

pub(super) fn pads_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    const GAP: f32 = 24.0;
    // Half of what's left next to a grid of 100 px pads
    let browser_size = vec2(
        ((ui.available_width() - 4.0 * 100.0 - 3.0 * PAD_GAP - GAP) / 2.0).max(200.0),
        ui.available_height(),
    );

    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(browser_size, Layout::top_down(Align::Min), |ui| {
            ui.set_min_size(browser_size);
            browser::browser(ui, c, state);
        });
        ui.add_space(GAP);
        ui.vertical(|ui| {
            bank_selector(ui, state);
            pad_grid(ui, c, state, ui.available_size());
        });
    });
}

pub(super) fn editor_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    ui.horizontal(|ui| {
        bank_selector(ui, state);
        ui.add_space(12.0);
        pad_strip(ui, c, state);
    });
    ui.add_space(10.0);
    pad_editor(ui, c, state);
}

/// The bank's pads as small buttons to switch the edited pad.
fn pad_strip(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    const GAP: f32 = 4.0;
    let n = PADS_PER_BANK as f32;
    let width = ((ui.available_width() - (n - 1.0) * GAP) / n).min(64.0);
    ui.spacing_mut().item_spacing.x = GAP;

    for i in 0..PADS_PER_BANK {
        let pad = state.bank * PADS_PER_BANK + i;
        let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
        if response.clicked() {
            state.select(pad);
        }

        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(4), theme::KNOB_BODY);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(width, 3.0)),
            CornerRadius {
                nw: 4,
                ne: 4,
                sw: 0,
                se: 0,
            },
            theme::pad_color(pad),
        );
        if state.selected == pad {
            painter.rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.5f32, theme::TEXT),
                StrokeKind::Inside,
            );
        }
        painter.text(
            rect.center() + vec2(0.0, 1.0),
            Align2::CENTER_CENTER,
            (pad + 1).to_string(),
            theme::label(),
            if state.selected == pad {
                theme::TEXT
            } else {
                theme::TEXT_DIM
            },
        );
        response.on_hover_text(c.params.pads[pad].name.read().as_str());
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// The bank's 4x4 pads, filling `size`.
fn pad_grid(ui: &mut Ui, c: &Ctx, state: &mut EditorState, size: egui::Vec2) {
    let (grid, _) = ui.allocate_exact_size(size, Sense::hover());
    let cell = (size - vec2(3.0 * PAD_GAP, 3.0 * PAD_GAP)) / 4.0;

    for i in 0..PADS_PER_BANK {
        let pad = state.bank * PADS_PER_BANK + i;
        let (row, col) = ((i / 4) as f32, (i % 4) as f32);
        let rect = Rect::from_min_size(
            grid.min + vec2(col * (cell.x + PAD_GAP), row * (cell.y + PAD_GAP)),
            cell,
        );
        let response = ui.interact(rect, ui.id().with(("pad", pad)), Sense::click());
        state.drop_zones.add(rect, DropTarget::Pad(pad));

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
        if response.double_clicked() {
            state.select(pad);
            state.view = View::Editor;
        }

        response.context_menu(|ui| {
            if ui.button("Load sample…").clicked() {
                pick_sample(state, pad, 0, true);
                ui.close();
            }
            if ui.button("Clear").clicked() {
                clear_pad(c, pad);
                ui.close();
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
            Rect::from_min_size(rect.min, vec2(cell.x, 4.0)),
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
        // Waveform of the first loaded layer, between the header and the name
        let peaks = (0..NUM_LAYERS)
            .map(|layer| c.bank.slots[slot_index(pad, layer)].read().peaks.clone())
            .find(|peaks| !peaks.is_empty());
        if let Some(peaks) = peaks {
            let area = Rect::from_min_max(
                rect.left_top() + vec2(8.0, 38.0),
                rect.right_bottom() - vec2(8.0, 30.0),
            );
            mini_wave(painter, area, &peaks, color.gamma_multiply(0.7));
        }

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

/// Plays the pad at full velocity while held down, like clicking a pad in the grid.
fn play_button(ui: &mut Ui, c: &Ctx, state: &mut EditorState, pad: usize) {
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 22.0), Sense::click());
    let pressed = response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
    if pressed && state.held_pad != Some(pad) {
        if let Some(old) = state.held_pad {
            let _ = c.bank.previews.push(Preview::Off { pad: old });
        }
        let _ = c.bank.previews.push(Preview::On { pad, velocity: 1.0 });
        state.held_pad = Some(pad);
    }

    let fill = if pressed {
        theme::ACCENT
    } else if response.hovered() {
        mix(theme::KNOB_BODY, theme::ACCENT, 0.35)
    } else {
        theme::KNOB_BODY
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), fill);
    let c0 = rect.center() + vec2(1.0, 0.0);
    painter.add(egui::Shape::convex_polygon(
        vec![
            c0 + vec2(-4.0, -6.0),
            c0 + vec2(6.0, 0.0),
            c0 + vec2(-4.0, 6.0),
        ],
        if pressed { Color32::BLACK } else { theme::TEXT },
        Stroke::NONE,
    ));
    response.on_hover_text("Play this pad");
}

/// A waveform overview: one min/max line per pixel column, normalized to the loudest peak.
fn mini_wave(painter: &egui::Painter, area: Rect, peaks: &[(f32, f32)], color: Color32) {
    let columns = area.width().floor() as usize;
    if columns == 0 || area.height() < 4.0 {
        return;
    }
    let loudest = peaks
        .iter()
        .fold(0.0f32, |m, &(lo, hi)| m.max(lo.abs()).max(hi.abs()));
    if loudest <= 0.0 {
        return;
    }
    let scale = 0.5 * area.height() / loudest;
    let mid = area.center().y;

    for col in 0..columns {
        let from = col * peaks.len() / columns;
        let to = ((col + 1) * peaks.len() / columns)
            .max(from + 1)
            .min(peaks.len());
        let (lo, hi) = peaks[from..to]
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &(lo, hi)| (l.min(lo), h.max(hi)));
        let x = area.left() + col as f32 + 0.5;
        // At least a hairline, so silence still shows where the sample is
        let (top, bottom) = (mid - hi * scale, mid - lo * scale);
        painter.line_segment(
            [pos2(x, top.min(mid - 0.5)), pos2(x, bottom.max(mid + 0.5))],
            Stroke::new(1.0f32, color),
        );
    }
}

fn pad_editor(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let pad = state.selected;
    let p = &c.params.pads[pad];
    let layer = state.layer;
    let (peaks, status, path, sample) = {
        let slot = c.bank.slots[slot_index(pad, layer)].read();
        (
            slot.peaks.clone(),
            slot.status.clone(),
            slot.requested.clone(),
            slot.sample.as_ref().map(|s| (s.len() as f64, s.sample_rate)),
        )
    };
    let playhead = c.bank.playheads[slot_index(pad, layer)].load(Ordering::Relaxed);
    let playback = sample.map(|(frames, sample_rate)| waveform::Playback {
        frames,
        sample_rate,
        layer_tune: p.layers.read().layers[layer].tune,
        playhead: (playhead >= 0.0).then_some(playhead),
    });

    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), Sense::hover());
        ui.painter()
            .rect_filled(r, CornerRadius::same(2), theme::pad_color(pad));
        ui.label(
            RichText::new(format!("PAD {}", pad + 1))
                .font(theme::heading())
                .strong(),
        );
        play_button(ui, c, state, pad);

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
            (Tab::Edit, "EDIT"),
            (Tab::Rec, "REC"),
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
    let wave = waveform::show(
        ui,
        vec2(width, 120.0),
        &peaks,
        &status,
        p,
        c.setter,
        playback,
    );
    state
        .drop_zones
        .add(wave.rect, DropTarget::Layer { pad, layer });
    if wave.clicked() && peaks.is_empty() && !state.dialogs.is_open() {
        pick_sample(state, pad, layer, layer == 0);
    }

    ui.add_space(4.0);
    match state.tab {
        Tab::Main => main_tab(ui, c, pad),
        Tab::Layers => layers_tab::show(ui, c, state),
        Tab::Edit => edit_tab::show(ui, c, state, wave.rect),
        Tab::Rec => rec_tab::show(ui, c, state),
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
