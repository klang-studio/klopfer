//! The plugin GUI: a top bar, the pad view (pad grid + pad editor) and a mixer view.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use klopfer_dsp::NUM_PADS;
use nih_plug::prelude::*;
use nih_plug_egui::create_egui_editor;
use nih_plug_egui::egui::{
    self, pos2, vec2, Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, Ui,
};

use crate::bank::{PadStatus, Preview, SampleBank};
use crate::params::{note_name, KlopferParams};
use crate::{kit, Klopfer, Task};

mod dialogs;
mod theme;
mod waveform;
mod widgets;

use dialogs::{Dialogs, Outcome, Request};
use widgets::{choice, fader, section, toggle, Knob};

pub const WIDTH: u32 = 1100;
pub const HEIGHT: u32 = 560;

const PAD_SIZE: f32 = 86.0;
const PAD_GAP: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Pads,
    Mixer,
}

struct EditorState {
    selected: usize,
    view: View,
    /// The pad currently held down with the mouse, for previews
    held_pad: Option<usize>,
    /// Status line text, `true` marks errors
    message: Option<(String, bool)>,
    last_dir: Option<PathBuf>,
    dialogs: Dialogs,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected: 0,
            view: View::Pads,
            held_pad: None,
            message: None,
            last_dir: None,
            dialogs: Dialogs::default(),
        }
    }
}

/// Everything the GUI functions need, bundled to keep signatures short.
struct Ctx<'a> {
    params: &'a KlopferParams,
    bank: &'a SampleBank,
    executor: &'a AsyncExecutor<Klopfer>,
    setter: &'a ParamSetter<'a>,
}

pub(crate) fn create(
    params: Arc<KlopferParams>,
    bank: Arc<SampleBank>,
    executor: AsyncExecutor<Klopfer>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        EditorState::default(),
        |ctx, _| theme::apply(ctx),
        move |ctx, setter, state| {
            bank.collect_garbage();
            let c = Ctx {
                params: &params,
                bank: &bank,
                executor: &executor,
                setter,
            };

            if let Some(outcome) = state.dialogs.take_outcome() {
                apply_dialog_outcome(&c, state, outcome);
            }

            egui::TopBottomPanel::top("top")
                .exact_height(52.0)
                .frame(
                    egui::Frame::NONE
                        .fill(theme::PANEL_DARK)
                        .inner_margin(egui::Margin::symmetric(12, 6)),
                )
                .show(ctx, |ui| top_bar(ui, &c, state));

            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(theme::BG)
                        .inner_margin(egui::Margin::same(12)),
                )
                .show(ctx, |ui| match state.view {
                    View::Pads => pads_view(ui, &c, state),
                    View::Mixer => mixer_view(ui, &c, state),
                });

            // Release a previewed pad when the mouse button goes up anywhere
            if let Some(pad) = state.held_pad {
                if !ctx.input(|i| i.pointer.primary_down()) {
                    let _ = bank.previews.push(Preview::Off { pad });
                    state.held_pad = None;
                }
            }

            // Pad meters animate continuously
            ctx.request_repaint();
        },
    )
}

// ---------------------------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------------------------

fn file_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn assign_sample(c: &Ctx, pad: usize, path: Option<String>, rename: bool) {
    let p = &c.params.pads[pad];
    *p.sample_path.write() = path.clone();
    if rename {
        *p.name.write() = path
            .as_deref()
            .map(|s| file_name(Path::new(s)))
            .unwrap_or_default();
    }
    if c.bank.request(pad, path.clone()) {
        c.executor.execute_background(Task::Load { pad, path });
    }
}

fn pick_sample(state: &mut EditorState, pad: usize, rename: bool) {
    state
        .dialogs
        .show(Request::Sample { pad, rename }, state.last_dir.clone());
}

fn missing_samples(c: &Ctx) -> Vec<(usize, String)> {
    (0..NUM_PADS)
        .filter_map(|pad| {
            let slot = c.bank.slots[pad].read();
            match (&slot.status, &slot.requested) {
                (PadStatus::Missing, Some(path)) => Some((pad, path.clone())),
                _ => None,
            }
        })
        .collect()
}

fn relink_folder(c: &Ctx, state: &mut EditorState) {
    if missing_samples(c).is_empty() {
        state.message = Some((String::from("No missing samples"), false));
    } else {
        state
            .dialogs
            .show(Request::RelinkFolder, state.last_dir.clone());
    }
}

fn new_kit(c: &Ctx, state: &mut EditorState) {
    for (_, ptr, _) in c.params.param_map() {
        // SAFETY: the pointers come from our own parameter map
        unsafe {
            let ctx = c.setter.raw_context;
            ctx.raw_begin_set_parameter(ptr);
            ctx.raw_set_parameter_normalized(ptr, ptr.default_normalized_value());
            ctx.raw_end_set_parameter(ptr);
        }
    }
    for pad in 0..NUM_PADS {
        assign_sample(c, pad, None, true);
    }
    *c.params.kit_name.write() = String::from("Init");
    state.message = None;
}

fn save_kit(c: &Ctx, state: &mut EditorState) {
    let name = c.params.kit_name.read().clone();
    let file_name = format!("{name}.{}", kit::EXTENSION);
    state
        .dialogs
        .show(Request::SaveKit { file_name }, state.last_dir.clone());
}

fn apply_dialog_outcome(c: &Ctx, state: &mut EditorState, outcome: Outcome) {
    match outcome {
        Outcome::Sample { pad, rename, path } => {
            state.last_dir = path.parent().map(Path::to_path_buf);
            assign_sample(c, pad, Some(path.to_string_lossy().into_owned()), rename);
            state.message = None;
        }
        Outcome::RelinkFolder(dir) => {
            let missing = missing_samples(c);
            let found = kit::find_missing(&missing, &dir);
            for (pad, path) in &found {
                assign_sample(c, *pad, Some(path.clone()), false);
            }
            state.message = Some((
                format!(
                    "Relinked {} of {} missing samples",
                    found.len(),
                    missing.len()
                ),
                found.len() < missing.len(),
            ));
        }
        Outcome::LoadKit(file) => match kit::load(&file) {
            Ok(kit_state) => {
                // Decode on this thread so restoring the state doesn't have to
                for (pad, path) in kit::sample_paths(&kit_state).into_iter().enumerate() {
                    if c.bank.request(pad, path.clone()) {
                        c.bank.load(pad, path);
                    }
                }
                c.setter.raw_context.set_state(kit_state);
                state.last_dir = file.parent().map(Path::to_path_buf);
                state.message = None;
            }
            Err(e) => state.message = Some((e, true)),
        },
        Outcome::SaveKit(file) => match kit::save(&c.setter.raw_context.get_state(), &file) {
            Ok(()) => {
                *c.params.kit_name.write() = file_name(&file);
                state.last_dir = file.parent().map(Path::to_path_buf);
                state.message = Some((String::from("Kit saved"), false));
            }
            Err(e) => state.message = Some((e, true)),
        },
    }
}

// ---------------------------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------------------------

fn top_bar(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    ui.horizontal_centered(|ui| {
        ui.label(
            RichText::new("KLOPFER")
                .font(egui::FontId::proportional(22.0))
                .color(theme::ACCENT)
                .strong(),
        );
        ui.add_space(12.0);

        // Kit name in an LCD style display
        egui::Frame::NONE
            .fill(Color32::BLACK)
            .corner_radius(CornerRadius::same(4))
            .stroke(Stroke::new(1.0f32, theme::KNOB_BODY))
            .inner_margin(egui::Margin::symmetric(10, 4))
            .show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.label(
                    RichText::new(c.params.kit_name.read().as_str())
                        .monospace()
                        .color(theme::WAVE),
                );
            });

        // One dialog at a time
        ui.add_enabled_ui(!state.dialogs.is_open(), |ui| {
            if ui.button("New").on_hover_text("Reset all pads").clicked() {
                new_kit(c, state);
            }
            if ui.button("Load Kit…").clicked() {
                state.dialogs.show(Request::LoadKit, state.last_dir.clone());
            }
            if ui.button("Save Kit…").clicked() {
                save_kit(c, state);
            }
            if ui
                .button("Relink…")
                .on_hover_text("Search a folder for missing samples")
                .clicked()
            {
                relink_folder(c, state);
            }
        });

        if let Some((msg, is_error)) = &state.message {
            ui.label(RichText::new(msg).font(theme::label()).color(if *is_error {
                theme::WARN
            } else {
                theme::TEXT_DIM
            }));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add(
                Knob::new(&c.params.master, c.setter, "Master")
                    .diameter(30.0)
                    .compact(),
            );
            ui.add_space(12.0);
            for (view, label) in [(View::Mixer, "MIXER"), (View::Pads, "PADS")] {
                if ui.selectable_label(state.view == view, label).clicked() {
                    state.view = view;
                }
            }
        });
    });
}

fn pads_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
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

    for pad in 0..NUM_PADS {
        let (row, col) = ((pad / 4) as f32, (pad % 4) as f32);
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
            state.selected = pad;
        }

        response.context_menu(|ui| {
            if ui.button("Load sample…").clicked() {
                pick_sample(state, pad, true);
                ui.close_menu();
            }
            if ui.button("Clear").clicked() {
                assign_sample(c, pad, None, true);
                ui.close_menu();
            }
        });

        let p = &c.params.pads[pad];
        let color = theme::pad_color(pad);
        let meter = c.bank.meters[pad].load(Ordering::Relaxed);
        let status = c.bank.slots[pad].read().status.clone();

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

        let outline = if status == PadStatus::Missing || matches!(status, PadStatus::Error(_)) {
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
    let s = c.setter;
    let (peaks, status, path) = {
        let slot = c.bank.slots[pad].read();
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
            .add(egui::TextEdit::singleline(&mut name).desired_width(170.0))
            .changed()
        {
            *p.name.write() = name;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Clear").clicked() {
                assign_sample(c, pad, None, true);
            }
            if matches!(status, PadStatus::Missing) {
                if ui.button("Relink…").clicked() {
                    pick_sample(state, pad, false);
                }
            } else if ui.button("Load…").clicked() {
                pick_sample(state, pad, true);
            }
        });
    });

    let detail = match (&status, &path) {
        (PadStatus::Error(e), Some(p)) => RichText::new(format!("{p}: {e}")).color(theme::WARN),
        (PadStatus::Missing, Some(p)) => {
            RichText::new(format!("Not found: {p}")).color(theme::WARN)
        }
        (_, Some(p)) => RichText::new(p.as_str()).color(theme::TEXT_DIM),
        _ => RichText::new("No sample loaded").color(theme::TEXT_DIM),
    };
    ui.label(detail.font(theme::small()));

    let width = ui.available_width();
    let wave = waveform::show(ui, vec2(width, 130.0), &peaks, &status, p, s);
    if wave.clicked() && peaks.is_empty() {
        pick_sample(state, pad, true);
    }

    ui.add_space(4.0);
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

fn mixer_view(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for pad in 0..NUM_PADS {
            let p = &c.params.pads[pad];
            let s = c.setter;
            let color = theme::pad_color(pad);
            let selected = state.selected == pad;

            egui::Frame::NONE
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
                            state.selected = pad;
                        }

                        let meter = c.bank.meters[pad].load(Ordering::Relaxed);
                        fader(ui, &p.volume, s, 190.0, meter, color);
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
        }
    });
}
