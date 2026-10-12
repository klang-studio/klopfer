//! Recording from the sidechain input into the selected pad layer.

use std::sync::atomic::Ordering;

use egui::{self, pos2, vec2, CornerRadius, Rect, RichText, Sense, Stroke, Ui};

use super::dialogs::Request;
use super::{assign_sample, theme, Ctx, EditorState};
use crate::recording::{self, State, Take, MAX_SECONDS};

pub(super) struct RecOptions {
    /// Start when the input crosses the threshold instead of right away
    wait: bool,
    threshold_db: f32,
    normalize: bool,
}

impl Default for RecOptions {
    fn default() -> Self {
        Self {
            wait: true,
            threshold_db: -30.0,
            normalize: true,
        }
    }
}

/// Saves and loads a finished take. Runs every frame, whichever tab is open.
pub(super) fn collect_take(c: &Ctx, state: &mut EditorState) {
    let Some(take) = c.recorder.take_finished() else {
        return;
    };
    if take.len == 0 {
        state.message = Some((String::from("Nothing was recorded"), true));
        return;
    }

    let path = state.settings.recordings_dir().join(recording::file_name());
    match take.save(&path) {
        Ok(()) => {
            let path_str = path.to_string_lossy().into_owned();
            assign_sample(c, take.pad, take.layer, Some(path_str), take.layer == 0);
            state.message = Some((
                format!(
                    "Recorded {:.1} s to pad {}",
                    take.len as f32 / take.sample_rate,
                    take.pad + 1
                ),
                false,
            ));
        }
        Err(e) => state.message = Some((format!("cannot save recording: {e}"), true)),
    }
}

fn db(level: f32) -> f32 {
    20.0 * level.max(1e-6).log10()
}

/// Input meter on a -60..0 dB scale with the threshold marked.
fn meter(ui: &mut Ui, level: f32, threshold_db: Option<f32>) {
    let (rect, _) = ui.allocate_exact_size(vec2(320.0, 14.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(3), theme::TRACK);
    let x = |db: f32| rect.left() + ((db + 60.0) / 60.0).clamp(0.0, 1.0) * rect.width();

    let level_db = db(level);
    let color = if level_db > -1.0 {
        theme::WARN
    } else {
        theme::WAVE
    };
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(x(level_db), rect.bottom())),
        CornerRadius::same(3),
        color,
    );
    if let Some(t) = threshold_db {
        painter.line_segment(
            [pos2(x(t), rect.top()), pos2(x(t), rect.bottom())],
            Stroke::new(2.0f32, theme::ACCENT),
        );
    }
}

pub(super) fn show(ui: &mut Ui, c: &Ctx, state: &mut EditorState) {
    let rec = c.recorder;
    let status = rec.state();
    let (pad, layer) = (state.selected, state.layer);

    egui::Frame::NONE
        .fill(theme::PANEL)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // The default font has no record/stop symbols
                let (label, color) = match status {
                    State::Idle => ("RECORD", theme::WARN),
                    _ => ("STOP", theme::TEXT),
                };
                let button = egui::Button::new(RichText::new(label).color(color).strong())
                    .min_size(vec2(110.0, 30.0));
                if ui.add(button).clicked() {
                    if status == State::Idle {
                        let threshold = state
                            .rec
                            .wait
                            .then(|| 10f32.powf(state.rec.threshold_db / 20.0));
                        let sample_rate = rec.sample_rate.load(Ordering::Relaxed);
                        let take =
                            Take::new(sample_rate, pad, layer, threshold, state.rec.normalize);
                        if !rec.start(take) {
                            state.message = Some((String::from("Recorder is busy"), true));
                        }
                    } else {
                        rec.stop();
                    }
                }

                let text = match status {
                    State::Idle => format!("Records into pad {}, layer {}", pad + 1, layer + 1),
                    State::Waiting => String::from("Waiting for the input to cross the threshold…"),
                    State::Recording => format!(
                        "Recording {:.1} s (max {MAX_SECONDS:.0} s)",
                        rec.progress.load(Ordering::Relaxed) * MAX_SECONDS
                    ),
                };
                ui.label(RichText::new(text).font(theme::label()));
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("INPUT")
                        .font(theme::small())
                        .color(theme::TEXT_DIM),
                );
                let threshold = state.rec.wait.then_some(state.rec.threshold_db);
                meter(ui, rec.level.load(Ordering::Relaxed), threshold);
            });

            ui.add_space(4.0);
            ui.add_enabled_ui(status == State::Idle, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut state.rec.wait, "Start on signal above");
                    ui.add_enabled(
                        state.rec.wait,
                        egui::DragValue::new(&mut state.rec.threshold_db)
                            .range(-60.0..=0.0)
                            .speed(0.5)
                            .suffix(" dB"),
                    );
                    ui.add_space(12.0);
                    ui.checkbox(&mut state.rec.normalize, "Normalize");
                });
            });
        });

    ui.add_space(6.0);
    egui::Frame::NONE
        .fill(theme::PANEL)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("FOLDER")
                        .font(theme::small())
                        .color(theme::TEXT_DIM),
                );
                let dir = state.settings.recordings_dir();
                ui.add(
                    egui::Label::new(RichText::new(dir.to_string_lossy()).font(theme::label()))
                        .truncate(),
                );
                if ui
                    .add_enabled(!state.dialogs.is_open(), egui::Button::new("Change…"))
                    .clicked()
                {
                    state
                        .dialogs
                        .show(Request::RecordingsFolder, Some(dir.clone()));
                }
            });
            ui.label(
                RichText::new(
                    "Route the audio you want to record to Klopfer's sidechain input in your DAW.",
                )
                .font(theme::small())
                .color(theme::TEXT_DIM),
            );
        });
}
