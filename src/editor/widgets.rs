//! Parameter widgets: knobs, toggles, choice boxes and faders.

use std::f32::consts::PI;

use nih_plug::prelude::{BoolParam, Enum, EnumParam, Param, ParamSetter};
use nih_plug_egui::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, Response, Sense, Shape, Stroke, StrokeKind,
    Ui, Widget,
};

use super::theme;

pub const KNOB_CELL_WIDTH: f32 = 52.0;

/// Shared drag handling: vertical drags change the normalized value, shift drags finely,
/// double-click resets, the scroll wheel nudges.
fn drag_param<P: Param>(
    ui: &Ui,
    response: &Response,
    param: &P,
    setter: &ParamSetter,
    vertical_px: f32,
) {
    let id = response.id;

    if response.double_clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, param.default_normalized_value());
        setter.end_set_parameter(param);
        return;
    }

    if response.drag_started() {
        setter.begin_set_parameter(param);
        ui.memory_mut(|m| m.data.insert_temp(id, param.unmodulated_normalized_value()));
    }
    if response.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let speed = if fine { 0.1 } else { 1.0 } / vertical_px;
        let start: f32 = ui
            .memory(|m| m.data.get_temp(id))
            .unwrap_or_else(|| param.unmodulated_normalized_value());
        // Track the unsnapped value so discrete parameters move smoothly as well
        let value = (start - response.drag_delta().y * speed).clamp(0.0, 1.0);
        ui.memory_mut(|m| m.data.insert_temp(id, value));
        setter.set_parameter_normalized(param, value);
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }

    if response.hovered() && !response.dragged() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            let step = match param.step_count() {
                Some(n) if n > 0 => 1.0 / n as f32,
                _ => 0.01,
            };
            let value =
                (param.unmodulated_normalized_value() + step * scroll.signum()).clamp(0.0, 1.0);
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(param, value);
            setter.end_set_parameter(param);
        }
    }
}

fn arc_points(center: egui::Pos2, radius: f32, from: f32, to: f32) -> Vec<egui::Pos2> {
    let steps = ((to - from).abs() * 24.0).ceil().max(2.0) as usize;
    (0..=steps)
        .map(|i| {
            let a = from + (to - from) * i as f32 / steps as f32;
            center + radius * vec2(a.cos(), a.sin())
        })
        .collect()
}

/// A rotary knob with a value arc, the label below and the value on hover.
pub struct Knob<'a, P: Param> {
    param: &'a P,
    setter: &'a ParamSetter<'a>,
    label: &'a str,
    diameter: f32,
    bipolar: bool,
    compact: bool,
    color: Color32,
}

impl<'a, P: Param> Knob<'a, P> {
    pub fn new(param: &'a P, setter: &'a ParamSetter<'a>, label: &'a str) -> Self {
        Self {
            param,
            setter,
            label,
            diameter: 36.0,
            bipolar: false,
            compact: false,
            color: theme::ACCENT,
        }
    }

    /// Draw the value arc from the center instead of from the minimum.
    pub fn bipolar(mut self) -> Self {
        self.bipolar = true;
        self
    }

    /// Without the label row; the value is shown in the tooltip only.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn diameter(mut self, diameter: f32) -> Self {
        self.diameter = diameter;
        self
    }

    pub fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }
}

impl<P: Param> Widget for Knob<'_, P> {
    fn ui(self, ui: &mut Ui) -> Response {
        let (width, height) = if self.compact {
            (self.diameter + 4.0, self.diameter + 4.0)
        } else {
            (
                KNOB_CELL_WIDTH.max(self.diameter + 8.0),
                self.diameter + 30.0,
            )
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
        drag_param(ui, &response, self.param, self.setter, 200.0);

        let value = self.param.unmodulated_normalized_value();
        let active = response.dragged() || response.hovered();
        let painter = ui.painter_at(rect.expand(2.0));
        let r = self.diameter / 2.0;
        let center = pos2(rect.center().x, rect.top() + r + 2.0);

        const START: f32 = 0.75 * PI;
        const SWEEP: f32 = 1.5 * PI;
        let angle = START + SWEEP * value;

        painter.add(Shape::line(
            arc_points(center, r, START, START + SWEEP),
            Stroke::new(3.0f32, theme::TRACK),
        ));
        let from = if self.bipolar {
            START + SWEEP * 0.5
        } else {
            START
        };
        if (angle - from).abs() > 0.001 {
            painter.add(Shape::line(
                arc_points(center, r, from.min(angle), from.max(angle)),
                Stroke::new(3.0f32, self.color),
            ));
        }

        painter.circle_filled(center, r - 5.0, theme::KNOB_BODY);
        painter.circle_stroke(
            center,
            r - 5.0,
            Stroke::new(1.0f32, Color32::from_black_alpha(120)),
        );
        let dir = vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [center + dir * (r * 0.25), center + dir * (r - 7.0)],
            Stroke::new(2.0f32, theme::TEXT),
        );

        if !self.compact {
            let text = if active {
                self.param.normalized_value_to_string(value, true)
            } else {
                self.label.to_owned()
            };
            painter.text(
                pos2(rect.center().x, rect.bottom() - 13.0),
                Align2::CENTER_CENTER,
                text,
                theme::label(),
                if active { theme::TEXT } else { theme::TEXT_DIM },
            );
        }

        response.on_hover_text(format!(
            "{}: {}\nDrag to change, Shift for fine, double-click to reset",
            self.param.name(),
            self.param
        ))
    }
}

/// An illuminated push button for a boolean parameter.
pub fn toggle(ui: &mut Ui, param: &BoolParam, setter: &ParamSetter, label: &str) -> Response {
    let on = param.value();
    let (rect, response) =
        ui.allocate_exact_size(vec2(KNOB_CELL_WIDTH - 4.0, 22.0), Sense::click());
    if response.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !on);
        setter.end_set_parameter(param);
    }

    let fill = if on { theme::ACCENT } else { theme::KNOB_BODY };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), fill);
    if response.hovered() {
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0f32, theme::TEXT_DIM),
            StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::label(),
        if on { Color32::BLACK } else { theme::TEXT },
    );
    response.on_hover_text(param.name())
}

/// A drop-down for an enum parameter.
pub fn choice<T: Enum + PartialEq + 'static>(
    ui: &mut Ui,
    param: &EnumParam<T>,
    setter: &ParamSetter,
    width: f32,
) -> Response {
    let current = param.value().to_index();
    let names = T::variants();
    egui::ComboBox::from_id_salt(param.name())
        .width(width)
        .selected_text(names[current])
        .show_ui(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                if ui.selectable_label(current == i, *name).clicked() {
                    setter.begin_set_parameter(param);
                    setter.set_parameter(param, T::from_index(i));
                    setter.end_set_parameter(param);
                }
            }
        })
        .response
}

/// A vertical fader with a level meter next to it, for the mixer.
pub fn fader<P: Param>(
    ui: &mut Ui,
    param: &P,
    setter: &ParamSetter,
    height: f32,
    meter: f32,
    color: Color32,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(36.0, height), Sense::click_and_drag());
    drag_param(ui, &response, param, setter, height);

    let value = param.unmodulated_normalized_value();
    let painter = ui.painter_at(rect);

    let track = egui::Rect::from_center_size(
        pos2(rect.left() + 12.0, rect.center().y),
        vec2(4.0, height - 12.0),
    );
    painter.rect_filled(track, CornerRadius::same(2), theme::TRACK);
    let y = track.bottom() - track.height() * value;
    painter.rect_filled(
        egui::Rect::from_min_max(pos2(track.left(), y), track.right_bottom()),
        CornerRadius::same(2),
        color.gamma_multiply(0.6),
    );
    let cap = egui::Rect::from_center_size(pos2(track.center().x, y), vec2(20.0, 10.0));
    painter.rect_filled(cap, CornerRadius::same(2), theme::TEXT);
    painter.line_segment(
        [pos2(cap.left() + 3.0, y), pos2(cap.right() - 3.0, y)],
        Stroke::new(1.0f32, Color32::BLACK),
    );

    // Meter on a -60..+6 dB scale
    let meter_rect = egui::Rect::from_min_max(
        pos2(rect.right() - 10.0, track.top()),
        pos2(rect.right() - 4.0, track.bottom()),
    );
    painter.rect_filled(meter_rect, CornerRadius::ZERO, theme::TRACK);
    let db = 20.0 * meter.max(1e-6).log10();
    let level = ((db + 60.0) / 66.0).clamp(0.0, 1.0);
    if level > 0.0 {
        let top = meter_rect.bottom() - meter_rect.height() * level;
        let c = if db > 0.0 { theme::WARN } else { theme::WAVE };
        painter.rect_filled(
            egui::Rect::from_min_max(pos2(meter_rect.left(), top), meter_rect.right_bottom()),
            CornerRadius::ZERO,
            c,
        );
    }

    response.on_hover_text(format!("{}: {}", param.name(), param))
}

/// A titled panel section, like the labelled blocks on a hardware front panel.
pub fn section<R>(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(theme::PANEL)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::symmetric(6, 5))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .font(theme::small())
                        .color(theme::TEXT_DIM)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    add_contents(ui)
                })
                .inner
            })
            .inner
        })
        .inner
}
