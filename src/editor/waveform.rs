//! Waveform display with draggable start and end markers.

use nice_plug::prelude::ParamSetter;
use egui::{self, pos2, vec2, Align2, CornerRadius, Rect, Sense, Stroke, Ui};

use super::theme;
use crate::bank::PadStatus;
use crate::params::PadParams;

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
