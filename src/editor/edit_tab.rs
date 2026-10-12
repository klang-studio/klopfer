//! Sample editing: slicing loops onto pads (with MIDI export) and time stretching.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use klopfer_dsp::slicing::{detect_transients, estimate_bpm, regions, slice_by_beats};
use klopfer_dsp::{slot_index, Sample, NUM_LAYERS, NUM_PADS};
use nih_plug::prelude::{Param, ParamSetter};
use nih_plug_egui::egui::{self, pos2, Align2, Rect, RichText, Stroke, Ui};

use super::dialogs::Request;
use super::widgets::{choice, plain_choice, section, toggle, Knob};
use super::{assign_sample, file_name, theme, Ctx, EditorState};
use crate::layers::LayerConfig;
use crate::midi_file;
use crate::params::StretchModeParam;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SliceMode {
    Transients,
    Grid,
}

const MODES: [(SliceMode, &str); 2] = [
    (SliceMode::Transients, "Transients"),
    (SliceMode::Grid, "Beat Grid"),
];

/// Slice lengths for the beat grid, in quarter notes.
const DIVISIONS: [(f32, &str); 6] = [
    (0.125, "1/32"),
    (0.25, "1/16"),
    (0.5, "1/8"),
    (1.0, "1/4"),
    (2.0, "1/2"),
    (4.0, "1 Bar"),
];

/// Identifies a slicing result: the sample (by address) and the settings.
type SliceKey = (usize, SliceMode, u32, u32, u32);

/// Slicing settings and the slices found with them.
pub(super) struct Slicer {
    mode: SliceMode,
    sensitivity: f32,
    bpm: f32,
    beats: f32,
    /// Slice starts for (sample, settings)
    cache: Option<(SliceKey, Arc<Vec<usize>>)>,
}

impl Default for Slicer {
    fn default() -> Self {
        Self {
            mode: SliceMode::Transients,
            sensitivity: 0.5,
            bpm: 120.0,
            beats: 1.0,
            cache: None,
        }
    }
}

impl Slicer {
    fn starts(&mut self, sample: &Arc<Sample>) -> Arc<Vec<usize>> {
        let key = (
            Arc::as_ptr(sample) as usize,
            self.mode,
            self.sensitivity.to_bits(),
            self.bpm.to_bits(),
            self.beats.to_bits(),
        );
        if let Some((k, starts)) = &self.cache {
            if *k == key {
                return starts.clone();
            }
        }
        let starts = Arc::new(match self.mode {
            SliceMode::Transients => detect_transients(sample, self.sensitivity, 0.05),
            SliceMode::Grid => slice_by_beats(sample, self.bpm, self.beats),
        });
        self.cache = Some((key, starts.clone()));
        starts
    }

    fn tempo(&self, sample: &Sample) -> f32 {
        match self.mode {
            SliceMode::Grid => self.bpm,
            SliceMode::Transients => estimate_bpm(sample),
        }
    }
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// Puts each slice on its own pad, starting with the selected one. All of them play the same file
/// with their own start and end.
fn slices_to_pads(c: &Ctx, state: &mut EditorState, path: &str, starts: &[usize], len: usize) {
    let first = state.selected;
    let slices = regions(starts, len);
    let count = slices.len().min(NUM_PADS - first);
    let stem = file_name(Path::new(path));

    for (i, (start, end)) in slices.iter().take(count).enumerate() {
        let pad = first + i;
        let p = &c.params.pads[pad];
        *p.layers.write() = LayerConfig::default();
        for layer in 1..NUM_LAYERS {
            assign_sample(c, pad, layer, None, false);
        }
        assign_sample(c, pad, 0, Some(path.to_owned()), false);
        *p.name.write() = format!("{stem} {}", i + 1);
        set(c.setter, &p.start, *start);
        set(c.setter, &p.end, *end);
        set(c.setter, &p.reverse, false);
    }

    state.layer = 0;
    state.message = Some((
        if count < slices.len() {
            format!(
                "{count} of {} slices fit, onto pads {}–{}",
                slices.len(),
                first + 1,
                first + count
            )
        } else {
            format!("{count} slices on pads {}–{}", first + 1, first + count)
        },
        count < slices.len(),
    ));
}

/// A MIDI file playing the slices in their original rhythm, for dragging into the DAW.
fn export_midi(c: &Ctx, state: &mut EditorState, sample: &Sample, path: &str, starts: &[usize]) {
    let first = state.selected;
    let slices = regions(starts, sample.len());
    let notes: Vec<midi_file::Note> = slices
        .iter()
        .take(NUM_PADS - first)
        .enumerate()
        .map(|(i, (start, end))| midi_file::Note {
            key: c.params.pads[first + i].note.value() as u8,
            start_secs: (*start * sample.duration_secs()) as f64,
            length_secs: ((end - start) * sample.duration_secs()) as f64,
        })
        .collect();
    let bpm = state.slicer.tempo(sample) as f64;
    state.pending_midi = Some(midi_file::write(&notes, bpm));
    let file_name = format!("{} slices.mid", file_name(Path::new(path)));
    state
        .dialogs
        .show(Request::ExportMidi { file_name }, state.last_dir.clone());
}

fn paint_slices(ui: &Ui, wave: Rect, starts: &[usize], len: usize) {
    if len == 0 {
        return;
    }
    let painter = ui.painter_at(wave);
    for (i, start) in starts.iter().enumerate() {
        let x = wave.left() + *start as f32 / len as f32 * wave.width();
        painter.line_segment(
            [pos2(x, wave.top() + 14.0), pos2(x, wave.bottom())],
            Stroke::new(1.0f32, theme::ACCENT.gamma_multiply(0.7)),
        );
        painter.text(
            pos2(x + 3.0, wave.bottom() - 3.0),
            Align2::LEFT_BOTTOM,
            (i + 1).to_string(),
            theme::small(),
            theme::ACCENT,
        );
    }
}

pub(super) fn show(ui: &mut Ui, c: &Ctx, state: &mut EditorState, wave: Rect) {
    let pad = state.selected;
    let slot = slot_index(pad, state.layer);
    let (sample, path) = {
        let s = c.bank.slots[slot].read();
        (s.sample.clone(), s.requested.clone())
    };
    let starts = sample.as_ref().map(|s| state.slicer.starts(s));
    if let (Some(sample), Some(starts)) = (&sample, &starts) {
        paint_slices(ui, wave, starts, sample.len());
    }

    ui.horizontal_top(|ui| {
        section(ui, "SLICE", |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let slicer = &mut state.slicer;
                    plain_choice(ui, "slice-mode", &mut slicer.mode, &MODES, 100.0);
                    match slicer.mode {
                        SliceMode::Transients => {
                            ui.label(RichText::new("Sensitivity").font(theme::small()));
                            let mut percent = slicer.sensitivity * 100.0;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut percent)
                                        .range(0.0..=100.0)
                                        .suffix(" %"),
                                )
                                .changed()
                            {
                                slicer.sensitivity = percent / 100.0;
                            }
                        }
                        SliceMode::Grid => {
                            ui.label(RichText::new("BPM").font(theme::small()));
                            ui.add(
                                egui::DragValue::new(&mut slicer.bpm)
                                    .range(40.0..=300.0)
                                    .speed(0.1)
                                    .fixed_decimals(1),
                            );
                            if let Some(s) = &sample {
                                if ui
                                    .button("Detect")
                                    .on_hover_text("Guess the tempo from the loop length")
                                    .clicked()
                                {
                                    slicer.bpm = (estimate_bpm(s) * 10.0).round() / 10.0;
                                }
                            }
                            plain_choice(ui, "slice-div", &mut slicer.beats, &DIVISIONS, 64.0);
                        }
                    }
                });

                ui.horizontal(|ui| {
                    let count = starts.as_ref().map_or(0, |s| s.len());
                    ui.label(
                        RichText::new(format!("{count} slices"))
                            .font(theme::label())
                            .color(theme::TEXT_DIM),
                    );
                    let ready = !state.dialogs.is_open();
                    if let (Some(sample), Some(starts), Some(path)) = (&sample, &starts, &path) {
                        if ui
                            .add_enabled(ready, egui::Button::new("Slices to Pads"))
                            .on_hover_text(format!(
                                "Slice 1 replaces pad {}, the others go to the following pads",
                                pad + 1
                            ))
                            .clicked()
                        {
                            slices_to_pads(c, state, path, starts, sample.len());
                        }
                        if ui
                            .add_enabled(ready, egui::Button::new("Export MIDI…"))
                            .on_hover_text(
                                "A MIDI file that plays the slices in their original rhythm",
                            )
                            .clicked()
                        {
                            export_midi(c, state, sample, path, starts);
                        }
                    } else {
                        ui.label(
                            RichText::new("Load a loop to slice it")
                                .font(theme::label())
                                .color(theme::TEXT_DIM),
                        );
                    }
                });
            });
        });

        let p = &c.params.pads[pad];
        let s = c.setter;
        section(ui, "TIME STRETCH", |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                choice(ui, &p.stretch_mode, s, 76.0);
                toggle(ui, &p.stretch_sync, s, "SYNC");
            });
            ui.add_space(4.0);
            ui.add(Knob::new(&p.stretch_length, s, "Length"));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                choice(ui, &p.stretch_beats, s, 70.0);
                let rendering = (0..NUM_LAYERS)
                    .any(|l| c.bank.stretching[slot_index(pad, l)].load(Ordering::Acquire));
                let status = match (p.stretch_mode.value(), rendering) {
                    (StretchModeParam::Off, _) => "Off",
                    (_, true) => "Rendering…",
                    (_, false) => "Ready",
                };
                ui.label(
                    RichText::new(status)
                        .font(theme::small())
                        .color(theme::TEXT_DIM),
                );
            });
        });
    });
    ui.label(
        RichText::new(
            "Stretch keeps the pitch when changing the length; Tune then shifts the pitch without \
             changing the length. Sync fits the start–end region to the host tempo.",
        )
        .font(theme::small())
        .color(theme::TEXT_DIM),
    );
}
