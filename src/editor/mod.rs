//! The plugin GUI: a top bar, the pad view (pad grid + pad editor) and a mixer view.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use klopfer_dsp::{slot_index, NUM_LAYERS, NUM_PADS, NUM_SAMPLE_SLOTS, PADS_PER_BANK};
use nih_plug::prelude::*;
use nih_plug_egui::create_egui_editor;
use nih_plug_egui::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Stroke, Ui};

use crate::bank::{PadStatus, Preview, SampleBank};
use crate::layers::LayerConfig;
use crate::midi_learn::MidiLearn;
use crate::params::KlopferParams;
use crate::{kit, Klopfer, Task};

mod dialogs;
mod drop;
mod layers_tab;
mod mixer;
mod mod_tab;
mod pads;
mod theme;
mod waveform;
mod widgets;

use dialogs::{Dialogs, Outcome, Request};
use widgets::Knob;

pub const WIDTH: u32 = 1100;
pub const HEIGHT: u32 = 600;

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Pads,
    Mixer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Main,
    Layers,
    Mod,
}

struct EditorState {
    selected: usize,
    /// The layer shown in the waveform and targeted by Load/Clear
    layer: usize,
    /// Which 16 pads the pad grid and mixer show
    bank: usize,
    view: View,
    tab: Tab,
    /// The pad currently held down with the mouse, for previews
    held_pad: Option<usize>,
    /// Status line text, `true` marks errors
    message: Option<(String, bool)>,
    last_dir: Option<PathBuf>,
    dialogs: Dialogs,
    /// Where dropped files can go, collected anew every frame
    drop_zones: drop::DropZones,
}

impl Default for EditorState {
    fn default() -> Self {
        let state = Self {
            selected: 0,
            layer: 0,
            bank: 0,
            view: View::Pads,
            tab: Tab::Main,
            held_pad: None,
            message: None,
            last_dir: None,
            dialogs: Dialogs::default(),
            drop_zones: drop::DropZones::default(),
        };
        #[cfg(feature = "standalone")]
        let state = state.with_dev_overrides();
        state
    }
}

impl EditorState {
    /// Development helper for the standalone build, e.g. for screenshots:
    /// `KLOPFER_EDITOR=pad=13,tab=mod,view=mixer` opens the editor in that state.
    #[cfg(feature = "standalone")]
    fn with_dev_overrides(mut self) -> Self {
        let Ok(spec) = std::env::var("KLOPFER_EDITOR") else {
            return self;
        };
        for (key, value) in spec.split(',').filter_map(|kv| kv.split_once('=')) {
            match (key, value) {
                ("pad", n) => {
                    if let Ok(n) = n.parse::<usize>() {
                        let pad = n.clamp(1, NUM_PADS) - 1;
                        self.selected = pad;
                        self.bank = pad / PADS_PER_BANK;
                    }
                }
                ("tab", "layers") => self.tab = Tab::Layers,
                ("tab", "mod") => self.tab = Tab::Mod,
                ("view", "mixer") => self.view = View::Mixer,
                _ => {}
            }
        }
        self
    }

    fn select(&mut self, pad: usize) {
        if self.selected != pad {
            self.selected = pad;
            self.layer = 0;
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
    learn: Arc<MidiLearn>,
    executor: AsyncExecutor<Klopfer>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        params.editor_state.clone(),
        EditorState::default(),
        |ctx, _| theme::apply(ctx),
        move |ctx, setter, state| {
            bank.collect_garbage();
            widgets::provide_learn(ctx, &learn);

            // Tell the host about values set by learned MIDI CCs
            learn.take_pending(|ptr, value| {
                // SAFETY: the pointer comes from our own parameter map
                unsafe {
                    setter.raw_context.raw_begin_set_parameter(ptr);
                    setter.raw_context.raw_set_parameter_normalized(ptr, value);
                    setter.raw_context.raw_end_set_parameter(ptr);
                }
            });

            let c = Ctx {
                params: &params,
                bank: &bank,
                executor: &executor,
                setter,
            };

            if let Some(outcome) = state.dialogs.take_outcome() {
                apply_dialog_outcome(&c, state, outcome);
            }
            state.drop_zones.clear();

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
                    View::Pads => pads::pads_view(ui, &c, state),
                    View::Mixer => mixer::mixer_view(ui, &c, state),
                });

            drop::handle(ctx, &c, state);

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

/// Sets the sample of one pad layer and starts loading it. `rename` names the pad after the file.
fn assign_sample(c: &Ctx, pad: usize, layer: usize, path: Option<String>, rename: bool) {
    let p = &c.params.pads[pad];
    p.layers.write().layers[layer].path = path.clone();
    if rename {
        *p.name.write() = path
            .as_deref()
            .map(|s| file_name(Path::new(s)))
            .unwrap_or_default();
    }

    let slot = slot_index(pad, layer);
    if c.bank.request(slot, path.clone()) {
        c.executor.execute_background(Task::Load { slot, path });
    }
}

/// Removes all samples and layer settings of a pad.
fn clear_pad(c: &Ctx, pad: usize) {
    for layer in 0..NUM_LAYERS {
        assign_sample(c, pad, layer, None, false);
    }
    *c.params.pads[pad].layers.write() = LayerConfig::default();
    c.params.pads[pad].name.write().clear();
}

fn pick_sample(state: &mut EditorState, pad: usize, layer: usize, rename: bool) {
    state.dialogs.show(
        Request::Sample { pad, layer, rename },
        state.last_dir.clone(),
    );
}

/// Worst status over a pad's layers, for the pad grid.
fn pad_status(c: &Ctx, pad: usize) -> PadStatus {
    let mut worst = PadStatus::Empty;
    for layer in 0..NUM_LAYERS {
        match &c.bank.slots[slot_index(pad, layer)].read().status {
            s @ (PadStatus::Missing | PadStatus::Error(_)) => return s.clone(),
            PadStatus::Loaded => worst = PadStatus::Loaded,
            PadStatus::Empty => {}
        }
    }
    worst
}

/// `(slot, path)` of all samples that weren't found.
fn missing_samples(c: &Ctx) -> Vec<(usize, String)> {
    (0..NUM_SAMPLE_SLOTS)
        .filter_map(|slot| {
            let s = c.bank.slots[slot].read();
            match (&s.status, &s.requested) {
                (PadStatus::Missing, Some(path)) => Some((slot, path.clone())),
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
        clear_pad(c, pad);
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
        Outcome::Sample {
            pad,
            layer,
            rename,
            path,
        } => {
            state.last_dir = path.parent().map(Path::to_path_buf);
            let path = Some(path.to_string_lossy().into_owned());
            assign_sample(c, pad, layer, path, rename && layer == 0);
            state.message = None;
        }
        Outcome::RelinkFolder(dir) => {
            let missing = missing_samples(c);
            let found = kit::find_missing(&missing, &dir);
            for (slot, path) in &found {
                let (pad, layer) = (slot / NUM_LAYERS, slot % NUM_LAYERS);
                assign_sample(c, pad, layer, Some(path.clone()), false);
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
                for (slot, path) in kit::sample_paths(&kit_state).into_iter().enumerate() {
                    if c.bank.request(slot, path.clone()) {
                        c.bank.load(slot, path);
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
// Shared layout pieces
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

/// Bank switch: which 16 pads the grid and mixer show.
fn bank_selector(ui: &mut Ui, state: &mut EditorState) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("BANK")
                .font(theme::small())
                .color(theme::TEXT_DIM),
        );
        for (bank, label) in ["A", "B"].iter().enumerate() {
            let first = bank * PADS_PER_BANK + 1;
            if ui
                .selectable_label(state.bank == bank, *label)
                .on_hover_text(format!("Pads {first}–{}", first + PADS_PER_BANK - 1))
                .clicked()
            {
                state.bank = bank;
            }
        }
    });
}
