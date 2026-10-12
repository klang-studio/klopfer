//! Waveform display with draggable start and end markers and the amp envelope on top.

use nice_plug::prelude::ParamSetter;
use egui::{self, pos2, vec2, Align2, CornerRadius, Rect, Sense, Stroke, Ui};

use super::theme;
use crate::bank::PadStatus;
use crate::params::PadParams;

/// `ln(1000)`: decay and release cover 60 dB in their time, see [`klopfer_dsp::envelope`].
const LN_1000: f32 = 6.907_755;
/// Below this level the envelope counts as silent and the voice ends.
const SILENCE: f32 = klopfer_dsp::envelope::SILENCE;

/// How the displayed sample plays back, to place the envelope on it.
#[derive(Clone, Copy)]
pub struct Playback {
    /// Length in frames
    pub frames: f64,
    pub sample_rate: f32,
    /// The layer's own tuning in semitones, on top of the pad's
    pub layer_tune: f32,
    /// Where the newest voice of this layer plays, relative to the sample length
    pub playhead: Option<f32>,
}

#[derive(Clone, Copy)]
enum Marker {
    Start,
    End,
}

pub fn show(
    ui: &mut Ui,
    size: egui::Vec2,
    peaks: &[(f32, f32)],
    status: &PadStatus,
    pad: &PadParams,
    setter: &ParamSetter,
    playback: Option<Playback>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), theme::PANEL_DARK);

    let x_of = |v: f32| rect.left() + v * rect.width();
    let v_of = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);

    if peaks.is_empty() {
        let (text, color) = match status {
            PadStatus::Missing => ("Sample missing: use Relink", theme::WARN),
            PadStatus::Error(_) => ("Sample could not be loaded", theme::WARN),
            _ => ("No sample: click to load one", theme::TEXT_DIM),
        };
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            text,
            theme::label(),
            color,
        );
        return response;
    }

    // Marker dragging: grab whichever marker is closer to the pointer
    let marker_id = response.id.with("marker");
    if response.drag_started() {
        if let Some(p) = response.interact_pointer_pos() {
            let to_start = (p.x - x_of(pad.start.value())).abs();
            let to_end = (p.x - x_of(pad.end.value())).abs();
            let m = if to_start <= to_end {
                Marker::Start
            } else {
                Marker::End
            };
            ui.memory_mut(|mem| mem.data.insert_temp(marker_id, m));
            let param = match m {
                Marker::Start => &pad.start,
                Marker::End => &pad.end,
            };
            setter.begin_set_parameter(param);
        }
    }
    let active: Option<Marker> = ui.memory(|mem| mem.data.get_temp(marker_id));
    if let (Some(m), Some(p)) = (active, response.interact_pointer_pos()) {
        let param = match m {
            Marker::Start => &pad.start,
            Marker::End => &pad.end,
        };
        if response.dragged() {
            setter.set_parameter(param, v_of(p.x));
        }
        if response.drag_stopped() {
            setter.end_set_parameter(param);
            ui.memory_mut(|mem| mem.data.remove::<Marker>(marker_id));
        }
    }

    let mid = rect.center().y;
    let half = rect.height() * 0.45;
    let columns = rect.width().max(1.0) as usize;
    for c in 0..columns {
        let from = c * peaks.len() / columns;
        let to = ((c + 1) * peaks.len() / columns)
            .max(from + 1)
            .min(peaks.len());
        let (lo, hi) = peaks[from..to]
            .iter()
            .fold((0.0f32, 0.0f32), |(lo, hi), (l, h)| {
                (lo.min(*l), hi.max(*h))
            });
        let x = rect.left() + c as f32 + 0.5;
        painter.line_segment(
            [pos2(x, mid - hi * half), pos2(x, mid - lo * half + 1.0)],
            Stroke::new(1.0f32, theme::WAVE),
        );
    }

    // Dim everything outside the playback region
    let (start, end) = (x_of(pad.start.value()), x_of(pad.end.value()));
    let shade = egui::Color32::from_black_alpha(150);
    painter.rect_filled(
        Rect::from_min_max(rect.min, pos2(start.min(end), rect.bottom())),
        CornerRadius::ZERO,
        shade,
    );
    painter.rect_filled(
        Rect::from_min_max(pos2(start.max(end), rect.top()), rect.max),
        CornerRadius::ZERO,
        shade,
    );

    if let Some(playback) = playback {
        envelope(&painter, rect, pad, playback);
    }

    if let Some(pos) = playback.and_then(|p| p.playhead) {
        let x = x_of(pos);
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5f32, theme::TEXT),
        );
    }

    for (x, color, label) in [
        (start, theme::MARKER_START, "S"),
        (end, theme::MARKER_END, "E"),
    ] {
        painter.line_segment(
            [pos2(x, rect.top()), pos2(x, rect.bottom())],
            Stroke::new(1.5f32, color),
        );
        // Keep the flags inside the display at the edges
        let fx = x.clamp(rect.left() + 6.0, rect.right() - 6.0);
        let flag = Rect::from_min_size(pos2(fx - 6.0, rect.top()), vec2(12.0, 12.0));
        painter.rect_filled(flag, CornerRadius::same(2), color);
        painter.text(
            flag.center(),
            Align2::CENTER_CENTER,
            label,
            theme::small(),
            egui::Color32::BLACK,
        );
    }

    if pad.reverse.value() {
        painter.text(
            rect.right_top() + vec2(-8.0, 8.0),
            Align2::RIGHT_TOP,
            "REVERSE",
            theme::small(),
            theme::ACCENT,
        );
    }

    response
}

/// The amp envelope level `t` seconds after the note starts, ignoring the release: in one shot
/// mode there is none, in gate mode it depends on when the note ends.
fn level_at(pad: &PadParams, t: f32) -> f32 {
    let attack = pad.attack.value() / 1000.0;
    let hold = pad.hold.value() / 1000.0;
    let decay = pad.decay.value() / 1000.0;
    let sustain = pad.sustain.value().clamp(0.0, 1.0);

    if t < attack {
        return t / attack;
    }
    let t = t - attack - hold;
    if t < 0.0 {
        return 1.0;
    }
    if decay <= 0.0 {
        return sustain;
    }
    let level = sustain + (1.0 - sustain) * (-LN_1000 * t / decay).exp();
    // A decay to silence ends the voice
    if sustain <= SILENCE && level < SILENCE {
        0.0
    } else {
        level
    }
}

/// Draws the amp envelope from the playback start to the end marker, following the playback speed
/// (tuning) and direction.
fn envelope(painter: &egui::Painter, rect: Rect, pad: &PadParams, playback: Playback) {
    let pitch = pad.tune.value() as f32 + pad.fine.value() / 100.0 + playback.layer_tune;
    // Sample frames played per second
    let speed = playback.sample_rate as f64 * (pitch as f64 / 12.0).exp2();
    let (start, end) = (pad.start.value(), pad.end.value());
    if speed <= 0.0 || end <= start {
        return;
    }

    let x_of = |v: f32| rect.left() + v * rect.width();
    let (x0, x1) = (x_of(start), x_of(end));
    let top = rect.top() + 14.0;
    let bottom = rect.bottom() - 2.0;
    let reverse = pad.reverse.value();

    let mut points = Vec::with_capacity((x1 - x0) as usize + 2);
    let mut x = x0;
    loop {
        // Frames from the playback start to this column
        let distance = if reverse { x1 - x } else { x - x0 };
        let frames = distance / rect.width() * playback.frames as f32;
        let level = level_at(pad, (frames as f64 / speed) as f32);
        points.push(pos2(x, bottom - level * (bottom - top)));
        if x >= x1 {
            break;
        }
        x = (x + 1.0).min(x1);
    }

    painter.add(egui::Shape::line(points, Stroke::new(1.5f32, theme::ACCENT)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use nice_plug::prelude::{FloatParam, FloatRange};
    use std::sync::atomic::AtomicU64;
    use std::sync::Arc;

    fn param(value: f32, max: f32) -> FloatParam {
        FloatParam::new("p", value, FloatRange::Linear { min: 0.0, max })
    }

    #[test]
    fn envelope_follows_attack_hold_decay_sustain() {
        let mut pad = PadParams::new(0, &Arc::new(AtomicU64::new(0)));
        pad.attack = param(10.0, 2000.0);
        pad.hold = param(20.0, 2000.0);
        pad.decay = param(100.0, 10000.0);
        pad.sustain = param(0.5, 1.0);

        assert_eq!(level_at(&pad, 0.0), 0.0);
        assert!((level_at(&pad, 0.005) - 0.5).abs() < 1e-6);
        assert_eq!(level_at(&pad, 0.02), 1.0);
        // After the decay time the level is 60 dB closer to the sustain level
        assert!((level_at(&pad, 0.13) - (0.5 + 0.5e-3)).abs() < 1e-4);
        assert!((level_at(&pad, 5.0) - 0.5).abs() < 1e-6);

        // Decaying to silence ends the voice
        pad.sustain = param(0.0, 1.0);
        assert_eq!(level_at(&pad, 0.5), 0.0);
    }
}
