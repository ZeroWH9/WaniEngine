//! Shared, bounded UI components. Layouts consume the same controls at every size.
use crate::{
    ACCENT, App, BORDER, MUTED, NAV, PANEL, Page, SECONDARY, TEXT, icon_text_button, icons,
    paint_character_stage,
};
use eframe::egui::{self, Color32, Stroke, Vec2};
use serde_json::Value;
use waniengine_egui::overlay::WorkerStatus;

pub(crate) fn window_radius(ctx: &egui::Context) -> u8 {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        0
    } else {
        24
    }
}

pub(crate) fn filter_popover(
    ui: &mut egui::Ui,
    assets: &[Value],
    search: &str,
    content_type: &mut crate::Filter,
    favorites_only: &mut bool,
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(340.0, 300.0), egui::Sense::hover());
    crate::preview::record_geometry(ui, "Filter Popover", rect);
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, Color32::from_rgb(33, 30, 43));
    painter.rect_stroke(
        rect,
        12.0,
        Stroke::new(1.0_f32, Color32::from_rgb(55, 51, 65)),
        egui::StrokeKind::Inside,
    );
    let clear = egui::Rect::from_min_size(rect.min + Vec2::new(244.0, 10.0), Vec2::new(64.0, 28.0));
    painter.text(
        egui::pos2(rect.min.x + 18.0, clear.center().y),
        egui::Align2::LEFT_CENTER,
        "Filters",
        font(16.0, true),
        TEXT,
    );
    painter.text(
        clear.center(),
        egui::Align2::CENTER_CENTER,
        "Clear all",
        font(9.5, false),
        ACCENT,
    );
    let clear_all = ui
        .interact(clear, ui.id().with("clear-filters"), egui::Sense::click())
        .clicked();
    for y in [48.0, 164.0] {
        painter.line_segment(
            [rect.min + Vec2::new(0.0, y), rect.min + Vec2::new(340.0, y)],
            Stroke::new(1.0_f32, BORDER),
        );
    }
    tracked_text(
        painter,
        rect.min + Vec2::new(18.0, 64.0),
        egui::Align2::LEFT_TOP,
        "CONTENT TYPE",
        font(8.5, true),
        MUTED,
        12.0,
    );
    for (i, (filter, label)) in [
        (crate::Filter::All, "All"),
        (crate::Filter::Wallpapers, "Wallpapers"),
        (crate::Filter::Animations, "Animations"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = egui::Rect::from_min_size(
            rect.min + Vec2::new(18.0, 82.0 + i as f32 * 26.0),
            Vec2::new(300.0, 24.0),
        );
        let selected = *content_type == filter;
        let center = row.min + Vec2::new(7.0, 12.0);
        painter.circle_stroke(
            center,
            6.2,
            Stroke::new(
                1.6_f32,
                if selected {
                    ACCENT
                } else {
                    Color32::from_rgb(79, 73, 93)
                },
            ),
        );
        if selected {
            painter.circle_filled(center, 3.0, ACCENT);
        }
        painter.text(
            egui::pos2(row.min.x + 24.0, center.y),
            egui::Align2::LEFT_CENTER,
            label,
            font(10.5, selected),
            if selected { TEXT } else { SECONDARY },
        );
        if ui
            .interact(row, ui.id().with(("filter-type", i)), egui::Sense::click())
            .clicked()
        {
            *content_type = filter;
        }
    }
    painter.text(
        rect.min + Vec2::new(18.0, 180.0),
        egui::Align2::LEFT_TOP,
        "Favorites only",
        font(10.5, false),
        SECONDARY,
    );
    let toggle =
        egui::Rect::from_min_size(rect.min + Vec2::new(294.0, 178.0), Vec2::new(28.0, 16.0));
    crate::child_ui(ui, egui::UiBuilder::new().max_rect(toggle), |ui| {
        crate::toggle_switch_sized(ui, favorites_only, true, toggle.size());
    });
    let count = assets
        .iter()
        .filter(|asset| crate::library_asset_matches(asset, *content_type, *favorites_only, search))
        .count();
    ui.painter().text(
        rect.min + Vec2::new(18.0, 214.0),
        egui::Align2::LEFT_TOP,
        format!(
            "{count} {} match",
            if count == 1 { "item" } else { "items" }
        ),
        egui::FontId::proportional(9.0),
        MUTED,
    );
    let results =
        egui::Rect::from_min_size(rect.min + Vec2::new(18.0, 244.0), Vec2::new(304.0, 34.0));
    let response = ui.interact(
        results,
        ui.id().with("show-filter-results"),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(results, 8.0, ACCENT);
    ui.painter().text(
        results.center(),
        egui::Align2::CENTER_CENTER,
        format!(
            "Show {count} {}",
            if count == 1 { "result" } else { "results" }
        ),
        font(10.5, true),
        crate::SURFACE,
    );
    (clear_all, response.clicked())
}

fn font(size: f32, bold: bool) -> egui::FontId {
    egui::FontId::new(
        size,
        egui::FontFamily::Name(
            if bold {
                "Inter SemiBold"
            } else {
                "Inter Medium"
            }
            .into(),
        ),
    )
}

/// Shortens `text` with an ellipsis so it fits `max_width` when drawn with `font`.
pub(crate) fn ellipsize(
    painter: &egui::Painter,
    text: &str,
    font: &egui::FontId,
    max_width: f32,
) -> String {
    let width = |value: &str| {
        painter
            .layout_no_wrap(value.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if width(&candidate) <= max_width {
            return candidate;
        }
    }
    "…".to_owned()
}

pub(crate) fn tracked_text(
    painter: &egui::Painter,
    position: egui::Pos2,
    anchor: egui::Align2,
    text: impl Into<String>,
    font: egui::FontId,
    color: Color32,
    tracking_percent: f32,
) {
    let spacing = font.size * tracking_percent / 100.0;
    let mut job = egui::text::LayoutJob::simple(text.into(), font, color, f32::INFINITY);
    job.sections[0].format.extra_letter_spacing = spacing;
    let galley = painter.layout_job(job);
    let bounds = anchor.anchor_rect(egui::Rect::from_min_size(position, galley.size()));
    painter.galley(bounds.min, galley, color);
}

pub(crate) fn combo_chevron(
    ui: &egui::Ui,
    rect: egui::Rect,
    _: &egui::style::WidgetVisuals,
    _: bool,
    _: egui::AboveOrBelow,
) {
    icons::paint_node(
        ui.painter(),
        rect.center(),
        72,
        if ui.is_enabled() { MUTED } else { BORDER },
        Some(Vec2::new(10.0, 5.0)),
    );
}

pub(crate) fn slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    width: f32,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, 20.0), egui::Sense::drag());
    let min = *range.start();
    let max = *range.end();
    let old = *value;
    let rail = rect.shrink2(Vec2::new(5.0, 0.0));
    if response.is_pointer_button_down_on() || response.dragged() {
        response.request_focus();
        if let Some(pos) = response.interact_pointer_pos() {
            *value = (min + (max - min) * ((pos.x - rail.left()) / rail.width()).clamp(0.0, 1.0))
                .clamp(min, max);
        }
    }
    if response.has_focus() && ui.is_enabled() {
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) || i.key_pressed(egui::Key::ArrowUp) {
                *value = (*value + 1.0).min(max);
            }
            if i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowDown) {
                *value = (*value - 1.0).max(min);
            }
            if i.key_pressed(egui::Key::Home) {
                *value = min;
            }
            if i.key_pressed(egui::Key::End) {
                *value = max;
            }
        });
    }
    if *value != old {
        response.mark_changed();
    }
    let x = rail.left() + rail.width() * ((*value - min) / (max - min)).clamp(0.0, 1.0);
    let y = rect.center().y;
    ui.painter().line_segment(
        [egui::pos2(rail.left(), y), egui::pos2(rail.right(), y)],
        Stroke::new(3.0_f32, BORDER),
    );
    ui.painter().line_segment(
        [egui::pos2(rail.left(), y), egui::pos2(x, y)],
        Stroke::new(3.0_f32, if ui.is_enabled() { ACCENT } else { MUTED }),
    );
    ui.painter().circle_filled(
        egui::pos2(x, y),
        5.0,
        if ui.is_enabled() { TEXT } else { MUTED },
    );
    response
}

impl App {
    pub(crate) fn wallpaper_controls(&mut self, ui: &mut egui::Ui, asset: &Value) {
        ui.add_space(12.0);
        let width = ui.available_width();
        let compact = width < 1300.0;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(width, if compact { 108.0 } else { 64.0 }),
            egui::Sense::hover(),
        );
        crate::preview::record_geometry(ui, "Wallpaper Controls", rect);
        ui.painter().rect_filled(rect, 14.0, PANEL);
        ui.painter().rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let origin = rect.min + Vec2::new(14.0, 12.0);
        let field = egui::Rect::from_min_size(origin, Vec2::new(248.0, 40.0));
        let monitors = self
            .overlay
            .as_ref()
            .map(|o| o.monitors.clone())
            .unwrap_or_default();
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(field), |ui| {
            if self.busy || self.wallpaper.is_gnome() || monitors.is_empty() {
                ui.disable();
            }
            ui.spacing_mut().interact_size.y = 40.0;
            let response = egui::ComboBox::from_id_salt("wallpaper-display")
                .icon(crate::widgets::combo_chevron)
                .selected_text("")
                .width(248.0)
                .show_ui(ui, |ui| {
                    ui.set_min_width(248.0);
                    for m in &monitors {
                        if let Some(name) = &m.name {
                            ui.selectable_value(
                                &mut self.selected_output,
                                Some(name.clone()),
                                m.description.as_deref().unwrap_or(name),
                            );
                        }
                    }
                });
            icons::paint_node(
                ui.painter(),
                field.left_center() + Vec2::new(18.0, 0.0),
                69,
                SECONDARY,
                None,
            );
            let title = if self.wallpaper.is_gnome() {
                "All displays · GNOME".to_owned()
            } else {
                self.selected_output
                    .as_deref()
                    .unwrap_or("Display")
                    .to_owned()
            };
            let detail = self
                .selected_output
                .as_ref()
                .and_then(|n| monitors.iter().find(|m| m.name.as_ref() == Some(n)))
                .and_then(|m| m.logical_size)
                .map(|(w, h)| format!("{w} × {h}  ·  60 Hz"))
                .unwrap_or_else(|| "No display connected".into());
            let text_max = (field.width() - 38.0 - 30.0).max(20.0);
            let title_font = font(11.5, false);
            let detail_font = egui::FontId::proportional(9.0);
            let title = ellipsize(ui.painter(), &title, &title_font, text_max);
            let detail = ellipsize(ui.painter(), &detail, &detail_font, text_max);
            let title_galley = ui.painter().layout_no_wrap(title, title_font, SECONDARY);
            let detail_galley = ui.painter().layout_no_wrap(detail, detail_font, MUTED);
            let total_h = title_galley.size().y + 2.0 + detail_galley.size().y;
            let top = field.center().y - total_h / 2.0;
            let x = field.min.x + 38.0;
            let detail_y = top + title_galley.size().y + 2.0;
            ui.painter()
                .galley(egui::pos2(x, top), title_galley, SECONDARY);
            ui.painter()
                .galley(egui::pos2(x, detail_y), detail_galley, MUTED);
            response
                .response
                .on_hover_text("Choose the wallpaper display");
        });
        let fit = egui::Rect::from_min_size(origin + Vec2::new(266.0, 0.0), Vec2::new(110.0, 40.0));
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(fit), |ui| {
            if self.busy || asset["kind"].as_str() == Some("character") {
                ui.disable();
            }
            // Same height as the Display field beside it.
            ui.spacing_mut().interact_size.y = 40.0;
            let current = asset["fit"].as_str().unwrap_or("cover");
            let mut next = current;
            egui::ComboBox::from_id_salt("wallpaper-fit")
                .icon(crate::widgets::combo_chevron)
                .selected_text(
                    egui::RichText::new(match current {
                        "contain" => "Contain",
                        "stretch" => "Stretch",
                        "center" => "Center",
                        _ => "Cover",
                    })
                    .font(font(11.5, false)),
                )
                .width(110.0)
                .show_ui(ui, |ui| {
                    for (mode, label) in [
                        ("cover", "Cover"),
                        ("contain", "Contain"),
                        ("stretch", "Stretch"),
                        ("center", "Center"),
                    ] {
                        let response = ui.selectable_value(&mut next, mode, label);
                        crate::preview::record_geometry(
                            ui,
                            format!("Fit option {mode}"),
                            response.rect,
                        );
                    }
                });
            if next != current {
                self.update_asset_property(
                    asset["url"].as_str().unwrap_or_default().into(),
                    "fit",
                    serde_json::json!(next),
                    ui.ctx(),
                );
            }
        });
        let capabilities = self.ui_capabilities();
        let has_audio = capabilities.audio
            && asset["video"].as_bool() == Some(true)
            && asset["hasAudio"].as_bool().unwrap_or(false);
        let audio =
            egui::Rect::from_min_size(origin + Vec2::new(420.0, 10.0), Vec2::new(82.0, 20.0));
        let mut enabled = asset["audioEnabled"].as_bool().unwrap_or(has_audio);
        crate::child_ui(
            ui,
            egui::UiBuilder::new()
                .max_rect(audio)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if crate::toggle_switch(ui, &mut enabled, has_audio && !self.busy).changed() {
                    self.update_asset_property(
                        asset["url"].as_str().unwrap_or_default().into(),
                        "audioEnabled",
                        serde_json::json!(enabled),
                        ui.ctx(),
                    );
                }
                ui.label(egui::RichText::new("Audio").size(11.5).color(SECONDARY));
            },
        );
        let row_y = if compact {
            rect.top() + 64.0
        } else {
            rect.top() + 12.0
        };
        let apply = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 186.0, row_y),
            Vec2::new(172.0, 40.0),
        );
        let adjustments = egui::Rect::from_min_size(
            egui::pos2(apply.left() - 144.0, row_y),
            Vec2::new(132.0, 40.0),
        );
        let loop_rect = egui::Rect::from_min_size(
            egui::pos2(
                if compact {
                    origin.x
                } else {
                    adjustments.left() - 128.0
                },
                row_y + 10.0,
            ),
            Vec2::new(90.0, 20.0),
        );
        let slider_left = audio.right() + 16.0;
        let slider_right = if compact {
            rect.right() - 56.0
        } else {
            loop_rect.left() - 66.0
        };
        let slider_rect = egui::Rect::from_min_size(
            egui::pos2(slider_left, origin.y + 10.0),
            Vec2::new((slider_right - slider_left).min(468.0).max(70.0), 20.0),
        );
        let url = asset["url"].as_str().unwrap_or_default();
        let mut volume = crate::volume_draft_value(
            &mut self.wallpaper_volume_draft,
            url,
            asset["volume"].as_f64().unwrap_or(50.0) as f32,
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(slider_rect), |ui| {
            let response = ui
                .add_enabled_ui(has_audio && enabled && !self.busy, |ui| {
                    slider(ui, &mut volume, 0.0..=100.0, slider_rect.width())
                })
                .inner;
            if crate::record_volume_response(
                &mut self.wallpaper_volume_draft,
                url,
                volume,
                &response,
            ) {
                self.update_asset_property(
                    url.into(),
                    "volume",
                    serde_json::json!(volume.round()),
                    ui.ctx(),
                );
            }
        });
        ui.painter().text(
            slider_rect.right_center() + Vec2::new(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{volume:.0}%"),
            egui::FontId::proportional(10.0),
            MUTED,
        );
        let mut looping = asset["loop"].as_bool().unwrap_or(true);
        crate::child_ui(
            ui,
            egui::UiBuilder::new()
                .max_rect(loop_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if crate::toggle_switch(
                    ui,
                    &mut looping,
                    asset["video"].as_bool() == Some(true)
                        && !self.busy
                        && self.wallpaper.supports_video(),
                )
                .changed()
                {
                    self.update_asset_property(
                        url.into(),
                        "loop",
                        serde_json::json!(looping),
                        ui.ctx(),
                    );
                }
                ui.label(egui::RichText::new("Loop").size(11.5).color(SECONDARY));
            },
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(adjustments), |ui| {
            icon_text_button(
                ui,
                "sliders",
                "Adjustments",
                adjustments.size(),
                false,
                PANEL,
                SECONDARY,
                BORDER,
            )
            .on_hover_text(
                "Brightness and color adjustments are not exposed by the current playback service.",
            );
        });
        let missing = asset["missing"].as_bool() == Some(true);
        let supported = capabilities.wallpaper
            && self.wallpaper_format_supported(asset)
            && (self.wallpaper.is_gnome() || self.selected_output.is_some());
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(apply), |ui| {
            let response = icon_text_button(
                ui,
                "check",
                "Apply wallpaper",
                apply.size(),
                !self.busy && !missing && supported && asset["kind"].as_str() != Some("character"),
                ACCENT,
                crate::SURFACE,
                Color32::TRANSPARENT,
            );
            if self.preview.is_none()
                && asset["video"].as_bool() == Some(true)
                && waniengine_egui::desktop::is_plasma_session()
            {
                response.clone().on_hover_text("Video playback uses the Rust renderer behind Plasma desktop icons and widgets. A small Plasma adapter preserves desktop input and restores a static fallback if playback stops.");
            }
            if response.clicked() {
                self.apply_wallpaper(asset, ui.ctx(), false);
            }
        });
    }

    pub(crate) fn library_toolbar(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let compact = ui.available_width() < 1400.0;
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), if compact { 96.0 } else { 40.0 }),
            egui::Sense::hover(),
        );
        let row1_y = rect.top();
        let row1_center = row1_y + 20.0;
        ui.painter().text(
            egui::pos2(rect.left(), row1_center),
            egui::Align2::LEFT_CENTER,
            "Library",
            font(20.0, true),
            TEXT,
        );
        crate::preview::record_geometry(ui, "Library Toolbar", rect);
        let count = self.preview.as_ref().map_or_else(
            || {
                self.assets
                    .iter()
                    .filter(|asset| {
                        crate::library_asset_matches(
                            asset,
                            self.filter,
                            self.favorites_only,
                            &self.search,
                        )
                    })
                    .count()
            },
            |p| p.snapshot.library_count,
        );
        let badge_galley = ui.painter().layout_no_wrap(
            format!("{count} {}", if count == 1 { "item" } else { "items" }),
            egui::FontId::proportional(9.0),
            MUTED,
        );
        let badge = egui::Rect::from_center_size(
            egui::pos2(
                rect.left() + 82.0 + (badge_galley.size().x + 20.0) / 2.0,
                row1_center,
            ),
            Vec2::new(badge_galley.size().x + 20.0, 18.0),
        );
        ui.painter().rect_filled(badge, 9.0, PANEL);
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            badge_galley.text(),
            egui::FontId::proportional(9.0),
            MUTED,
        );

        // Right-hand controls of the first row, laid out from the right edge.
        let refresh = egui::Rect::from_min_size(
            egui::pos2(rect.right() - if compact { 40.0 } else { 48.0 }, row1_y),
            Vec2::splat(40.0),
        );
        let import = egui::Rect::from_min_size(
            egui::pos2(refresh.left() - 116.0, row1_y),
            Vec2::new(108.0, 40.0),
        );
        // Row 2 (compact) or the same row (wide).
        let row2_y = if compact { row1_y + 56.0 } else { row1_y };
        let views = egui::Rect::from_min_size(
            egui::pos2(
                if compact {
                    rect.right() - 68.0
                } else {
                    import.left() - 76.0
                },
                row2_y,
            ),
            Vec2::new(68.0, 40.0),
        );

        let search_x =
            (badge.right() - rect.left() + 16.0).max(if compact { 160.0 } else { 196.0 });
        let search_right = if compact {
            import.left() - 16.0
        } else {
            rect.left() + 196.0 + 336.0
        };
        let search_w = (search_right - rect.left() - search_x).clamp(120.0, 336.0);
        let search = egui::Rect::from_min_size(
            egui::pos2(rect.left() + search_x, row1_y),
            Vec2::new(search_w, 40.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(search), |ui| {
            let response = crate::library_search_field(ui, &mut self.search, search.width());
            if self.focus_library_search {
                response.request_focus();
                self.focus_library_search = false;
            }
        });

        let filters = egui::Rect::from_min_size(
            if compact {
                egui::pos2(rect.left(), row2_y)
            } else {
                egui::pos2(search.right() + 14.0, row2_y)
            },
            Vec2::new(348.0, 40.0),
        );
        crate::child_ui(
            ui,
            egui::UiBuilder::new()
                .max_rect(filters)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                crate::library_filter_segments(ui, &mut self.filter, &mut self.favorites_only);
            },
        );
        let options = egui::Rect::from_min_size(
            filters.right_top() + Vec2::new(12.0, 0.0),
            Vec2::splat(40.0),
        );
        let response = crate::child_ui(
            ui,
            egui::UiBuilder::new().max_rect(options),
            crate::library_filter_options_button,
        )
        .inner;
        if response.clicked() {
            self.filter_menu_open = !self.filter_menu_open;
        }
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(refresh), |ui| {
            if crate::icon_button(
                ui,
                "refresh",
                refresh.size(),
                SECONDARY,
                Color32::TRANSPARENT,
                BORDER,
                !self.busy,
            )
            .on_hover_text("Refresh library")
            .clicked()
            {
                self.reload();
            }
        });
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(import), |ui| {
            if icon_text_button(
                ui,
                "upload",
                "Import",
                import.size(),
                !self.busy,
                PANEL,
                SECONDARY,
                BORDER,
            )
            .clicked()
            {
                self.choose_files(ui.ctx());
            }
        });
        ui.painter().rect_filled(views, 10.0, PANEL);
        ui.painter().rect_stroke(
            views,
            10.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        for (i, grid) in [true, false].iter().enumerate() {
            let r = egui::Rect::from_min_size(
                views.min + Vec2::new(4.0 + i as f32 * 30.0, 4.0),
                Vec2::new(28.0, 32.0),
            );
            let selected = self.grid_view == *grid;
            if selected {
                ui.painter()
                    .rect_filled(r, 7.0, Color32::from_rgb(42, 36, 64));
            }
            icons::paint_node(
                ui.painter(),
                r.center(),
                if *grid { 121 } else { 123 },
                if selected { ACCENT } else { MUTED },
                None,
            );
            if ui
                .interact(r, egui::Id::new(("view-mode", i)), egui::Sense::click())
                .clicked()
            {
                self.grid_view = *grid;
            }
        }
        // Sort combo: shrink to the room left of the view toggle, hide when too tight.
        let sort_left_limit = options.right() + 16.0;
        let sort_w = (views.left() - 8.0 - sort_left_limit).min(140.0);
        if sort_w >= 96.0 {
            let sort = egui::Rect::from_min_size(
                egui::pos2(views.left() - 8.0 - sort_w, views.top()),
                Vec2::new(sort_w, 40.0),
            );
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(sort), |ui| {
                ui.spacing_mut().interact_size.y = 40.0;
                egui::ComboBox::from_id_salt("library-sort")
                    .icon(crate::widgets::combo_chevron)
                    .selected_text("")
                    .width(sort_w)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sort_by_name, false, "Most recent");
                        ui.selectable_value(&mut self.sort_by_name, true, "Name A–Z");
                    });
                icons::paint_node(
                    ui.painter(),
                    egui::pos2(sort.left() + 20.0, sort.center().y),
                    116,
                    MUTED,
                    None,
                );
                let label_font = font(11.5, false);
                let label_max = (sort.width() - 36.0 - 30.0).max(10.0);
                let label = ellipsize(
                    ui.painter(),
                    if self.sort_by_name {
                        "Name A–Z"
                    } else {
                        "Most recent"
                    },
                    &label_font,
                    label_max,
                );
                let clip = egui::Rect::from_min_max(
                    egui::pos2(sort.left() + 36.0, sort.top()),
                    egui::pos2(sort.right() - 30.0, sort.bottom()),
                );
                ui.painter().with_clip_rect(clip).text(
                    egui::pos2(sort.left() + 36.0, sort.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    label_font,
                    SECONDARY,
                );
            });
        }
        response
    }

    pub(crate) fn navigation(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("nav")
            .exact_height(72.0)
            .frame(
                egui::Frame::new()
                    .fill(NAV)
                    .inner_margin(0)
                    // The top corners are only lightly rounded (a third of the bottom ones).
                    .corner_radius(egui::CornerRadius {
                        nw: window_radius(ctx) / 3,
                        ne: window_radius(ctx) / 3,
                        sw: 0,
                        se: 0,
                    }),
            )
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                crate::preview::record_geometry(
                    ui,
                    "Navigation",
                    egui::Rect::from_min_size(screen.min, Vec2::new(screen.width(), 72.0)),
                );
                let width = screen.width();
                let origin = screen.min;
                let p = |x, y| origin + Vec2::new(x, y);
                let painter = ui.painter();
                painter.line_segment(
                    [p(0.0, 71.0), p(width, 71.0)],
                    Stroke::new(1.0_f32, Color32::from_rgb(34, 30, 43)),
                );
                if let Some(logo) = &self.logo {
                    painter.image(
                        logo.id(),
                        egui::Rect::from_min_size(p(28.0, 19.0), Vec2::splat(34.0)),
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                tracked_text(
                    painter,
                    p(74.0, 25.0),
                    egui::Align2::LEFT_TOP,
                    "WaniEngine",
                    font(17.0, true),
                    TEXT,
                    -1.0,
                );
                let tab_width = if width < 1100.0 { 100.0 } else { 124.0 };
                let tabs_end = 250.0 + 3.0 * tab_width;
                for (i, (page, label)) in [
                    (Page::Library, "Library"),
                    (Page::Discover, "Discover"),
                    (Page::Animations, "Animations"),
                ]
                .iter()
                .enumerate()
                {
                    let rect = egui::Rect::from_min_size(
                        p(250.0 + i as f32 * tab_width, 0.0),
                        Vec2::new(tab_width, 72.0),
                    );
                    let response =
                        ui.interact(rect, egui::Id::new(("navigation", i)), egui::Sense::click());
                    let selected = self.page == *page;
                    ui.painter().text(
                        egui::pos2(rect.center().x, rect.center().y),
                        egui::Align2::CENTER_CENTER,
                        *label,
                        font(14.0, selected),
                        if selected {
                            Color32::from_rgb(216, 211, 255)
                        } else {
                            MUTED
                        },
                    );
                    if selected {
                        ui.painter().rect_filled(
                            egui::Rect::from_center_size(
                                egui::pos2(rect.center().x, rect.bottom() - 2.0),
                                Vec2::new(40.0, 2.0),
                            ),
                            1.0,
                            ACCENT,
                        );
                    }
                    let clicked = response.clicked();
                    if clicked {
                        if self.page == Page::Settings {
                            self.cancel_settings();
                        }
                        self.page = *page;
                    }
                }
                let settings = egui::Rect::from_min_size(
                    p((width - 250.0).max(tabs_end + 16.0), 16.0),
                    Vec2::new(110.0, 40.0),
                );
                crate::child_ui(ui, egui::UiBuilder::new().max_rect(settings), |ui| {
                    if icon_text_button(
                        ui,
                        "gear",
                        "Settings",
                        settings.size(),
                        true,
                        NAV,
                        SECONDARY,
                        BORDER,
                    )
                    .clicked()
                    {
                        self.open_settings(ctx);
                    }
                });
                for i in 0..3 {
                    let rect = egui::Rect::from_min_size(
                        p(width - 114.0 + i as f32 * 31.0, 18.0),
                        Vec2::new(30.0, 36.0),
                    );
                    let response = ui.interact(
                        rect,
                        egui::Id::new(("window-control", i)),
                        egui::Sense::click(),
                    );
                    if response.hovered() {
                        ui.painter().rect_filled(
                            rect.shrink2(Vec2::new(1.0, 4.0)),
                            8.0,
                            if i == 2 {
                                Color32::from_rgba_unmultiplied(251, 113, 133, 36)
                            } else {
                                Color32::from_white_alpha(12)
                            },
                        );
                    }
                    paint_window_glyph(
                        ui.painter(),
                        rect.center(),
                        i,
                        ctx.input(|input| input.viewport().maximized.unwrap_or(false)),
                        if response.hovered() {
                            if i == 2 {
                                Color32::from_rgb(251, 113, 133)
                            } else {
                                TEXT
                            }
                        } else {
                            MUTED
                        },
                    );
                    if response.clicked() {
                        match i {
                            0 => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                            1 => {
                                let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                            }
                            _ => self.request_window_close(ctx),
                        }
                    }
                }
                let drag = egui::Rect::from_min_max(
                    p(tabs_end, 0.0),
                    p((width - 262.0).max(tabs_end), 72.0),
                );
                let response = ui.interact(
                    drag,
                    egui::Id::new("title-drag"),
                    egui::Sense::click_and_drag(),
                );
                if response.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if response.double_clicked() {
                    let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
                }
            });
    }

    pub(crate) fn character_tile(
        &mut self,
        ui: &mut egui::Ui,
        asset: &Value,
        width: f32,
        discover: bool,
    ) {
        let height = if discover { 150.0 } else { 126.0 };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
        crate::preview::record_geometry(
            ui,
            format!(
                "{} {}",
                asset["name"].as_str().unwrap_or("Character"),
                if discover {
                    "Character Card"
                } else {
                    "Character Row"
                }
            ),
            rect,
        );
        let url = asset["url"].as_str().unwrap_or_default();
        let state = self.actor_ui_state(url);
        let active = state.is_some();
        let running = state
            .as_ref()
            .is_some_and(|(s, _)| *s == WorkerStatus::Running);
        ui.painter().rect_filled(rect, 12.0, PANEL);
        ui.painter().rect_stroke(
            rect,
            12.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let art_width = if discover { 156.0 } else { 108.0 };
        let art = egui::Rect::from_min_size(rect.min, Vec2::new(art_width, height));
        if let Some(texture) = asset["thumbnailUrl"]
            .as_str()
            .or_else(|| asset["url"].as_str())
            .and_then(|u| self.thumbnails.get(u))
        {
            paint_character_stage(ui, art, texture, asset);
        }
        let x = art.right() + 20.0;
        let title = asset["name"].as_str().unwrap_or("Character");
        let button = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 108.0, rect.bottom() - 44.0),
            Vec2::new(92.0, 30.0),
        );
        let narrow = rect.width() < 260.0;
        let text_max = (rect.right() - 16.0 - x - if narrow { 100.0 } else { 0.0 }).max(24.0);
        let side_max = (button.left() - 8.0 - x).max(24.0);
        let text_clip = |max: f32| {
            ui.painter().with_clip_rect(egui::Rect::from_min_max(
                egui::pos2(x, rect.top()),
                egui::pos2(x + max, rect.bottom()),
            ))
        };
        let title_font = font(if discover { 16.0 } else { 14.0 }, true);
        let title = ellipsize(ui.painter(), title, &title_font, text_max);
        text_clip(text_max).text(
            egui::pos2(x, rect.top() + if discover { 20.0 } else { 18.0 }),
            egui::Align2::LEFT_TOP,
            title,
            title_font,
            TEXT,
        );
        let details = [
            asset["characterLoop"].as_str().map(str::to_owned),
            if discover {
                asset["width"]
                    .as_u64()
                    .zip(asset["height"].as_u64())
                    .filter(|(w, h)| *w > 0 && *h > 0)
                    .map(|(w, h)| format!("{w} × {h}"))
            } else {
                asset["nativeFps"]
                    .as_u64()
                    .filter(|fps| *fps > 0)
                    .map(|fps| format!("{fps} FPS"))
            },
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ·  ");
        let details_font = egui::FontId::proportional(10.5);
        let details = ellipsize(ui.painter(), &details, &details_font, text_max);
        text_clip(text_max).text(
            egui::pos2(x, rect.top() + if discover { 44.0 } else { 40.0 }),
            egui::Align2::LEFT_TOP,
            details,
            details_font,
            MUTED,
        );
        if discover && running {
            let active_galley =
                ui.painter()
                    .layout_no_wrap("ACTIVE".to_owned(), font(9.0, true), Color32::WHITE);
            let badge = egui::Rect::from_min_size(
                egui::pos2(x, rect.top() + 72.0),
                Vec2::new(active_galley.size().x + 38.0, 22.0),
            );
            ui.painter()
                .rect_filled(badge, 11.0, Color32::from_rgb(30, 58, 50));
            ui.painter().circle_filled(
                badge.left_center() + Vec2::new(14.0, 0.0),
                2.5,
                Color32::from_rgb(52, 211, 153),
            );
            ui.painter().text(
                badge.center() + Vec2::new(5.0, 0.0),
                egui::Align2::CENTER_CENTER,
                "ACTIVE",
                font(9.0, true),
                Color32::from_rgb(110, 231, 183),
            );
        }
        let info = if discover {
            "Overlay · click-through".to_owned()
        } else if running {
            format!(
                "Active on {}",
                state.as_ref().map(|(_, m)| m.as_str()).unwrap_or("display")
            )
        } else {
            String::new()
        };
        let info_font = egui::FontId::proportional(9.5);
        let info_max = if discover { side_max } else { text_max };
        let info = ellipsize(ui.painter(), &info, &info_font, info_max);
        text_clip(info_max).text(
            egui::pos2(x, rect.top() + if discover { 106.0 } else { 64.0 }),
            egui::Align2::LEFT_TOP,
            info,
            info_font,
            if running && !discover {
                Color32::from_rgb(52, 211, 153)
            } else {
                MUTED
            },
        );
        let enabled = !self.busy
            && !active
            && self.ui_capabilities().overlay
            && asset["missing"].as_bool() != Some(true)
            && asset["video"].as_bool() != Some(true);
        if active {
            if discover {
                ui.painter().rect_stroke(
                    button,
                    16.0,
                    Stroke::new(1.0_f32, Color32::from_rgb(40, 100, 78)),
                    egui::StrokeKind::Inside,
                );
            }
            ui.painter().text(
                button.center(),
                egui::Align2::CENTER_CENTER,
                if running { "Active" } else { "Starting…" },
                font(10.5, false),
                Color32::from_rgb(110, 231, 183),
            );
        } else {
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(button), |ui| {
                if icon_text_button(
                    ui,
                    "power",
                    "Enable",
                    button.size(),
                    enabled,
                    Color32::from_rgb(30, 26, 39),
                    SECONDARY,
                    BORDER,
                )
                .clicked()
                {
                    self.toggle_character(asset, ui.ctx());
                }
            });
        }
        if response.clicked() && !button.contains(ui.ctx().pointer_latest_pos().unwrap_or_default())
        {
            self.selected_actor = Some(url.into());
            if discover {
                self.page = Page::Animations;
            }
        }
    }
}

/// Minimize, maximize/restore and close drawn as one matched set: same 12px box, same
/// 1.5px round-capped stroke, pixel-snapped so they stay crisp at any scale.
fn paint_window_glyph(
    painter: &egui::Painter,
    center: egui::Pos2,
    kind: usize,
    maximized: bool,
    color: Color32,
) {
    let stroke = Stroke::new(1.5_f32, color);
    let snap = |value: f32| value.floor() + 0.5;
    let c = egui::pos2(snap(center.x), snap(center.y));
    let half = 6.0;
    match kind {
        0 => {
            painter.line_segment(
                [c + Vec2::new(-half, 0.0), c + Vec2::new(half, 0.0)],
                Stroke::new(1.0_f32, color),
            );
        }
        1 if maximized => {
            let back = egui::Rect::from_min_size(c + Vec2::new(-4.0, -half), Vec2::splat(9.0));
            let front = egui::Rect::from_min_size(c + Vec2::new(-half, -3.0), Vec2::splat(9.0));
            painter.rect_stroke(back, 2.0, stroke, egui::StrokeKind::Inside);
            painter.rect_filled(front.expand(1.0), 2.0, Color32::from_rgb(11, 10, 15));
            painter.rect_stroke(front, 2.0, stroke, egui::StrokeKind::Inside);
        }
        1 => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::splat(half * 2.0 - 1.0)),
                2.5,
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        _ => {
            let d = half - 0.5;
            painter.line_segment([c + Vec2::new(-d, -d), c + Vec2::new(d, d)], stroke);
            painter.line_segment([c + Vec2::new(-d, d), c + Vec2::new(d, -d)], stroke);
        }
    }
}
