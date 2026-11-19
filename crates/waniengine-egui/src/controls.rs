//! Shared button and draft behavior for all layouts.
use crate::{ACCENT, MUTED, PANEL, SURFACE, paint_action_icon};
use eframe::egui::{self, Color32, Stroke, Vec2};

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    size: Vec2,
    color: Color32,
    fill: Color32,
    border: Color32,
    enabled: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let radius = if fill == ACCENT && (size.x - size.y).abs() < 1.0 {
        size.x * 0.5
    } else {
        8.0_f32.min(size.y * 0.5)
    };
    let fill = if !enabled && fill == ACCENT {
        PANEL
    } else {
        fill
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, radius, fill);
    }
    if response.hovered() && enabled {
        ui.painter().rect_filled(
            rect,
            radius,
            Color32::from_rgba_premultiplied(20, 20, 20, 20),
        );
    }
    if enabled && response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, radius, Color32::from_rgba_unmultiplied(0, 0, 0, 64));
    }
    if border != Color32::TRANSPARENT {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0_f32, border),
            egui::StrokeKind::Inside,
        );
    }
    if enabled && response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            radius + 1.0,
            Stroke::new(1.5_f32, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    paint_action_icon(
        ui.painter(),
        rect.center(),
        icon,
        if enabled { color } else { MUTED },
    );
    response
}

fn label_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    size: Vec2,
    enabled: bool,
    fill: Color32,
    text_color: Color32,
    border: Color32,
    trailing: bool,
    small: bool,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let (rect, response) = ui.allocate_exact_size(
        size,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let radius = 9.0_f32.min(rect.height() * 0.5);
    let button_fill = if enabled || fill == Color32::TRANSPARENT {
        fill
    } else {
        PANEL
    };
    let color = if !enabled {
        MUTED
    } else if fill == ACCENT {
        SURFACE
    } else {
        text_color
    };
    if button_fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, radius, button_fill);
    }
    if response.hovered() && enabled {
        ui.painter().rect_filled(
            rect,
            radius,
            Color32::from_rgba_premultiplied(20, 20, 20, 20),
        );
    }
    if enabled && response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, radius, Color32::from_rgba_unmultiplied(0, 0, 0, 64));
    }
    if border != Color32::TRANSPARENT {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0_f32, border),
            egui::StrokeKind::Inside,
        );
    }
    if enabled && response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(1.0),
            radius + 1.0,
            Stroke::new(1.5_f32, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    let font = egui::FontId::new(
        if small { 10.5 } else { 11.5 },
        egui::FontFamily::Name(
            if fill == ACCENT {
                "Inter SemiBold"
            } else {
                "Inter Medium"
            }
            .into(),
        ),
    );
    let width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), color)
        .size()
        .x;
    let icon_width = if small { 13.0 } else { 14.0 };
    let gap = if small { 6.0 } else { 8.0 };
    let start = rect.center().x - (width + icon_width + gap) * 0.5;
    let (text_x, icon_x) = if trailing {
        (start, start + width + gap + icon_width * 0.5)
    } else {
        (start + icon_width + gap, start + icon_width * 0.5)
    };
    paint_action_icon(
        ui.painter(),
        egui::pos2(icon_x, rect.center().y),
        icon,
        color,
    );
    ui.painter().text(
        egui::pos2(text_x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    response
}
pub(crate) fn icon_text_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    size: Vec2,
    enabled: bool,
    fill: Color32,
    text_color: Color32,
    border: Color32,
) -> egui::Response {
    label_button(
        ui, icon, label, size, enabled, fill, text_color, border, false, false,
    )
}
pub(crate) fn dialog_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    size: Vec2,
    enabled: bool,
    fill: Color32,
    text_color: Color32,
    border: Color32,
) -> egui::Response {
    label_button(
        ui, icon, label, size, enabled, fill, text_color, border, false, true,
    )
}

pub(crate) fn volume_draft_value(
    draft: &mut Option<(String, f32)>,
    url: &str,
    committed: f32,
) -> f32 {
    if draft.as_ref().is_some_and(|(key, _)| key != url) {
        *draft = None;
    }
    draft
        .as_ref()
        .map_or(committed.clamp(0.0, 100.0), |(_, v)| *v)
}
pub(crate) fn record_volume_response(
    draft: &mut Option<(String, f32)>,
    url: &str,
    value: f32,
    response: &egui::Response,
) -> bool {
    if response.changed() {
        *draft = Some((url.to_owned(), value));
    }
    (response.drag_stopped() && draft.as_ref().is_some_and(|(key, _)| key == url))
        || (response.changed() && !response.dragged() && !response.is_pointer_button_down_on())
}
