//! Dialog geometry from the cached FigCraft nodes 1183, 1236 and 1314.
use crate::{
    ACCENT, App, BORDER, ImportKind, MUTED, PANEL, SECONDARY, SURFACE, TEXT, child_ui, icon_button,
    icons, paint_cover_rounded, toggle_switch_sized,
};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};

fn label(
    p: &egui::Painter,
    o: egui::Pos2,
    x: f32,
    y: f32,
    text: impl Into<String>,
    size: f32,
    color: Color32,
    bold: bool,
) {
    p.text(
        o + Vec2::new(x, y),
        egui::Align2::LEFT_TOP,
        text.into(),
        egui::FontId::new(
            size,
            egui::FontFamily::Name(if bold { "Inter SemiBold" } else { "Inter" }.into()),
        ),
        color,
    );
}
/// Ellipsizes `text` so it fits `max_width` when drawn by `label` at `size`.
fn fit(p: &egui::Painter, text: &str, size: f32, bold: bool, max_width: f32) -> String {
    let font = egui::FontId::new(
        size,
        egui::FontFamily::Name(if bold { "Inter SemiBold" } else { "Inter" }.into()),
    );
    crate::widgets::ellipsize(p, text, &font, max_width)
}
fn line(p: &egui::Painter, r: egui::Rect, y: f32) {
    p.line_segment(
        [
            r.min + Vec2::new(0.0, y),
            egui::pos2(r.right(), r.top() + y),
        ],
        Stroke::new(1.0_f32, BORDER),
    );
}
fn shell(ui: &mut egui::Ui, size: Vec2, name: &str) -> egui::Rect {
    let (r, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    crate::preview::record_geometry(ui, name, r);
    ui.painter().rect_filled(r, 22.0, PANEL);
    ui.painter().rect_stroke(
        r,
        22.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    r
}
fn header(ui: &mut egui::Ui, r: egui::Rect, title: &str, subtitle: &str) -> bool {
    label(ui.painter(), r.min, 24.0, 18.0, title, 22.0, TEXT, true);
    label(
        ui.painter(),
        r.min,
        24.0,
        46.0,
        subtitle,
        10.0,
        MUTED,
        false,
    );
    line(ui.painter(), r, 70.0);
    let close =
        egui::Rect::from_min_size(r.min + Vec2::new(r.width() - 60.0, 18.0), Vec2::splat(34.0));
    child_ui(ui, egui::UiBuilder::new().max_rect(close), |ui| {
        icon_button(
            ui,
            "dialog-close",
            close.size(),
            MUTED,
            Color32::TRANSPARENT,
            Color32::TRANSPARENT,
            true,
        )
        .clicked()
    })
    .inner
}
fn plain_button(ui: &mut egui::Ui, r: egui::Rect, text: &str, enabled: bool) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let response = ui.interact(
        r,
        ui.id().with(text),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    ui.painter().rect_filled(
        r,
        8.0,
        if response.hovered() {
            Color32::from_rgb(34, 29, 46)
        } else {
            PANEL
        },
    );
    ui.painter().rect_stroke(
        r,
        8.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(10.5, egui::FontFamily::Name("Inter Medium".into())),
        if enabled { SECONDARY } else { MUTED },
    );
    response
}
fn footer(
    ui: &mut egui::Ui,
    r: egui::Rect,
    y: f32,
    title: &str,
    icon: &str,
    enabled: bool,
) -> (bool, bool) {
    let cancel = egui::Rect::from_min_size(
        r.min + Vec2::new(r.width() - 246.0, y),
        Vec2::new(80.0, 34.0),
    );
    let save = egui::Rect::from_min_size(
        r.min + Vec2::new(r.width() - 154.0, y),
        Vec2::new(130.0, 34.0),
    );
    let c = plain_button(ui, cancel, "Cancel", true).clicked();
    let s = child_ui(ui, egui::UiBuilder::new().max_rect(save), |ui| {
        crate::controls::dialog_button(
            ui,
            if icon == "upload" {
                "import-upload"
            } else if icon == "check" {
                "playlist-check"
            } else {
                icon
            },
            title,
            save.size(),
            enabled,
            ACCENT,
            SURFACE,
            Color32::TRANSPARENT,
        )
        .clicked()
    })
    .inner;
    (c, s)
}

impl App {
    fn import_file_view(&self, path: &std::path::Path) -> crate::preview::UiImportFile {
        self.preview
            .as_ref()
            .and_then(|p| p.snapshot.import_selection.iter().find(|f| f.path == path))
            .cloned()
            .unwrap_or_else(|| {
                let extension = path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_uppercase();
                crate::preview::UiImportFile {
                    path: path.to_path_buf(),
                    name: path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    video: matches!(extension.as_str(), "MP4" | "WEBM" | "MKV" | "MOV"),
                    extension,
                    bytes: path.metadata().map_or(0, |m| m.len()),
                }
            })
    }
    pub(crate) fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(files) = self.import_files.clone() else {
            return;
        };
        let selection = files
            .iter()
            .map(|f| self.import_file_view(f))
            .collect::<Vec<_>>();
        let total = selection.iter().map(|f| f.bytes).sum();
        let mut cancel = false;
        let mut import = false;
        let modal = egui::Modal::new(egui::Id::new("import-content"))
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let r = shell(ui, Vec2::new(720.0, 480.0), "Import Dialog");
                let p = ui.painter().clone();
                cancel = header(
                    ui,
                    r,
                    "Import content",
                    "Review your selection before adding it to the library.",
                );
                let art =
                    egui::Rect::from_min_size(r.min + Vec2::new(24.0, 92.0), Vec2::splat(220.0));
                p.rect_filled(art, 12.0, SURFACE);
                if let Some(file) = selection.first() {
                    let url = crate::file_url(&file.path);
                    if let Some(texture) = self.thumbnails.get(&url) {
                        paint_cover_rounded(ui, art, texture, [12.0; 4]);
                    }
                    crate::paint_rounded_vertical_gradient(
                        &p,
                        egui::Rect::from_min_max(art.min + Vec2::new(0.0, 150.0), art.max),
                        [0.0, 0.0, 12.0, 12.0],
                        Color32::TRANSPARENT,
                        Color32::from_rgba_premultiplied(10, 8, 16, 230),
                    );
                    let name = &file.name;
                    label(
                        &p,
                        art.min,
                        14.0,
                        180.0,
                        fit(&p, name, 10.5, true, art.width() - 28.0),
                        10.5,
                        TEXT,
                        true,
                    );
                    let ext = &file.extension;
                    label(
                        &p,
                        art.min,
                        14.0,
                        196.0,
                        format!("{ext} · {}", crate::format_file_size(file.bytes)),
                        9.0,
                        MUTED,
                        false,
                    );
                    if file.video {
                        let center = art.min + Vec2::splat(20.0);
                        p.circle_filled(center, 16.0, Color32::from_rgb(211, 208, 219));
                        icons::paint_node(&p, center, 1352, SURFACE, Some(Vec2::new(12.0, 13.5)));
                    }
                }
                p.rect_stroke(
                    art,
                    12.0,
                    Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
                label(&p, r.min, 264.0, 92.0, "SELECTED FILES", 8.5, MUTED, true);
                label(
                    &p,
                    r.min,
                    264.0,
                    110.0,
                    format!("{} items · {}", files.len(), crate::format_file_size(total)),
                    9.0,
                    MUTED,
                    false,
                );
                let list = egui::Rect::from_min_size(
                    r.min + Vec2::new(264.0, 128.0),
                    Vec2::new(432.0, 74.0),
                );
                child_ui(ui, egui::UiBuilder::new().max_rect(list), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("import-files")
                        .max_height(74.0)
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            for file in &selection {
                                let (row, _) = ui.allocate_exact_size(
                                    Vec2::new(432.0, 22.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_stroke(
                                    row.min + Vec2::new(6.0, 11.0),
                                    5.0,
                                    Stroke::new(1.6_f32, Color32::from_rgb(52, 211, 153)),
                                );
                                icons::paint_node(
                                    ui.painter(),
                                    row.min + Vec2::new(6.0, 11.0),
                                    962,
                                    Color32::from_rgb(52, 211, 153),
                                    Some(Vec2::new(6.0, 4.0)),
                                );
                                let name_font = egui::FontId::new(
                                    10.0,
                                    egui::FontFamily::Name("Inter Medium".into()),
                                );
                                ui.painter().text(
                                    row.min + Vec2::new(19.0, 3.0),
                                    egui::Align2::LEFT_TOP,
                                    crate::widgets::ellipsize(
                                        ui.painter(),
                                        &file.name,
                                        &name_font,
                                        354.0 - 19.0,
                                    ),
                                    name_font,
                                    SECONDARY,
                                );
                                let ext = &file.extension;
                                let badge = egui::Rect::from_min_size(
                                    row.min + Vec2::new(384.0, 2.0),
                                    Vec2::new(40.0, 18.0),
                                );
                                ui.painter()
                                    .rect_filled(badge, 5.0, Color32::from_rgb(28, 24, 38));
                                ui.painter().text(
                                    badge.center(),
                                    egui::Align2::CENTER_CENTER,
                                    ext,
                                    egui::FontId::proportional(8.0),
                                    MUTED,
                                );
                            }
                        });
                });
                p.line_segment(
                    [
                        r.min + Vec2::new(264.0, 216.0),
                        r.min + Vec2::new(696.0, 216.0),
                    ],
                    Stroke::new(1.0_f32, BORDER),
                );
                label(&p, r.min, 264.0, 232.0, "IMPORT AS", 8.5, MUTED, true);
                for (i, kind, title, detail, node) in [
                    (
                        0,
                        ImportKind::Wallpaper,
                        "Wallpaper",
                        "Fills the desktop behind your windows.",
                        1216,
                    ),
                    (
                        1,
                        ImportKind::Character,
                        "Desktop character",
                        "Lives above windows, follows your cursor.",
                        1221,
                    ),
                ] {
                    let card = egui::Rect::from_min_size(
                        r.min + Vec2::new(264.0 + i as f32 * 220.0, 250.0),
                        Vec2::new(208.0, 78.0),
                    );
                    let selected = self.import_kind == kind;
                    p.rect_filled(
                        card,
                        10.0,
                        if selected {
                            Color32::from_rgb(42, 36, 64)
                        } else {
                            Color32::from_rgb(20, 17, 27)
                        },
                    );
                    p.rect_stroke(
                        card,
                        10.0,
                        Stroke::new(1.0_f32, if selected { ACCENT } else { BORDER }),
                        egui::StrokeKind::Inside,
                    );
                    icons::paint_node(
                        &p,
                        card.min + Vec2::new(25.0, 25.0),
                        node,
                        if selected { ACCENT } else { MUTED },
                        Some(Vec2::splat(18.0)),
                    );
                    label(
                        &p,
                        card.min,
                        44.0,
                        12.0,
                        title,
                        11.0,
                        if selected { TEXT } else { SECONDARY },
                        true,
                    );
                    label(&p, card.min, 14.0, 42.0, detail, 9.0, MUTED, false);
                    if selected {
                        p.circle_filled(card.right_top() + Vec2::new(-24.0, 16.0), 4.0, ACCENT);
                    }
                    if ui
                        .interact(card, ui.id().with(("import-kind", i)), egui::Sense::click())
                        .clicked()
                    {
                        self.import_kind = kind;
                    }
                }
                if self.import_kind == ImportKind::Wallpaper {
                    label(&p, r.min, 264.0, 336.0, "AFTER IMPORT", 8.5, MUTED, true);
                    let toggle = egui::Rect::from_min_size(
                        r.min + Vec2::new(264.0, 352.0),
                        Vec2::new(28.0, 16.0),
                    );
                    child_ui(ui, egui::UiBuilder::new().max_rect(toggle), |ui| {
                        toggle_switch_sized(
                            ui,
                            &mut self.import_start_playing,
                            true,
                            toggle.size(),
                        );
                    });
                    label(
                        &p,
                        r.min,
                        300.0,
                        350.0,
                        "Start playing after import",
                        10.0,
                        SECONDARY,
                        false,
                    );
                    label(
                        &p,
                        r.min,
                        264.0,
                        374.0,
                        "Your original files are never moved or changed.",
                        9.0,
                        MUTED,
                        false,
                    );
                }
                line(&p, r, 402.0);
                label(
                    &p,
                    r.min,
                    24.0,
                    422.0,
                    format!("{} total", crate::format_file_size(total)),
                    9.5,
                    MUTED,
                    false,
                );
                let (c, s) = footer(
                    ui,
                    r,
                    420.0,
                    &if files.len() == 1 {
                        "Import 1 file".to_owned()
                    } else {
                        format!("Import {} files", files.len())
                    },
                    "upload",
                    !self.busy && !files.is_empty(),
                );
                cancel |= c;
                import = s;
            });
        if !import
            && !self.busy
            && !files.is_empty()
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
        {
            import = true;
        }
        if !import && (cancel || modal.should_close()) {
            self.import_files = None;
        }
        if import {
            self.import_files = None;
            self.import_selected(files, self.import_start_playing, ctx);
        }
    }

    pub(crate) fn playlist_dialog(&mut self, ctx: &egui::Context) {
        if !self.playlist_dialog_open {
            return;
        }
        let mut cancel = false;
        let mut save = false;
        let mut movement = None;
        let mut removal = None;
        let total = self
            .playlist_draft
            .iter()
            .filter_map(|u| {
                self.assets
                    .iter()
                    .find(|a| a["url"].as_str() == Some(u))
                    .and_then(|a| a["duration"].as_f64())
            })
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| s.round() as u64)
            .sum();
        let modal = egui::Modal::new(egui::Id::new("playlist-manager"))
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                let r = shell(ui, Vec2::new(720.0, 560.0), "Manage Playlist Dialog");
                let p = ui.painter().clone();
                cancel = header(
                    ui,
                    r,
                    "Manage playlist",
                    "Reorder, remove and schedule your rotation.",
                );
                let list = egui::Rect::from_min_size(
                    r.min + Vec2::new(24.0, 86.0),
                    Vec2::new(672.0, 300.0),
                );
                p.rect_filled(list, 12.0, Color32::from_rgb(20, 17, 27));
                p.rect_stroke(
                    list,
                    12.0,
                    Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
                label(
                    &p,
                    list.min,
                    16.0,
                    12.0,
                    format!("{} WALLPAPERS", self.playlist_draft.len()),
                    9.0,
                    MUTED,
                    true,
                );
                p.text(
                    list.right_top() + Vec2::new(-16.0, 12.0),
                    egui::Align2::RIGHT_TOP,
                    format!("{} total", crate::format_duration(total)),
                    egui::FontId::proportional(9.0),
                    MUTED,
                );
                let rows = egui::Rect::from_min_max(
                    list.min + Vec2::new(16.0, 36.0),
                    list.max - Vec2::new(16.0, 4.0),
                );
                child_ui(ui, egui::UiBuilder::new().max_rect(rows), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("playlist-dialog-rows")
                        .max_height(rows.height())
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing = Vec2::ZERO;
                            let urls = self.playlist_draft.clone();
                            for (index, url) in urls.iter().enumerate() {
                                let (row, _) = ui.allocate_exact_size(
                                    Vec2::new(rows.width(), 52.0),
                                    egui::Sense::hover(),
                                );
                                if index > 0 {
                                    ui.painter().line_segment(
                                        [row.min, row.right_top()],
                                        Stroke::new(1.0_f32, BORDER),
                                    );
                                }
                                let asset = self
                                    .assets
                                    .iter()
                                    .find(|a| a["url"].as_str() == Some(url))
                                    .cloned();
                                let grip =
                                    egui::Rect::from_min_size(row.min, Vec2::new(16.0, 52.0));
                                crate::paint_action_icon(
                                    ui.painter(),
                                    grip.min + Vec2::new(10.0, 32.75),
                                    "playlist-grip",
                                    MUTED,
                                );
                                let drag = ui
                                    .interact(
                                        egui::Rect::from_min_max(
                                            row.min,
                                            egui::pos2(row.right() - 100.0, row.bottom()),
                                        ),
                                        ui.id().with(("playlist-drag", index)),
                                        if self.busy {
                                            egui::Sense::hover()
                                        } else {
                                            egui::Sense::drag()
                                        },
                                    )
                                    .on_hover_cursor(egui::CursorIcon::Grab);
                                if drag.drag_started() {
                                    self.playlist_drag_index = Some(index);
                                }
                                if self.playlist_drag_index.is_some() && drag.hovered() {
                                    ui.painter().rect_stroke(
                                        row,
                                        4.0,
                                        Stroke::new(1.0_f32, ACCENT),
                                        egui::StrokeKind::Inside,
                                    );
                                }
                                if ui.input(|i| i.pointer.any_released())
                                    && row
                                        .contains(ui.ctx().pointer_latest_pos().unwrap_or_default())
                                    && let Some(from) = self.playlist_drag_index.take()
                                {
                                    movement = Some((from, index));
                                }
                                let thumb = egui::Rect::from_min_size(
                                    row.min + Vec2::new(22.0, 10.0),
                                    Vec2::new(40.0, 32.0),
                                );
                                if let Some(texture) = asset
                                    .as_ref()
                                    .and_then(|a| {
                                        a["thumbnailUrl"].as_str().or_else(|| a["url"].as_str())
                                    })
                                    .and_then(|u| self.thumbnails.get(u))
                                {
                                    paint_cover_rounded(ui, thumb, texture, [6.0; 4]);
                                }
                                let row_name = fit(
                                    ui.painter(),
                                    asset
                                        .as_ref()
                                        .and_then(|a| a["name"].as_str())
                                        .unwrap_or("Unavailable wallpaper"),
                                    11.0,
                                    false,
                                    row.width() - 72.0 - 108.0,
                                );
                                label(
                                    ui.painter(),
                                    row.min,
                                    72.0,
                                    12.0,
                                    row_name,
                                    11.0,
                                    SECONDARY,
                                    false,
                                );
                                label(
                                    ui.painter(),
                                    row.min,
                                    72.0,
                                    28.0,
                                    asset
                                        .as_ref()
                                        .and_then(|a| a["duration"].as_f64())
                                        .filter(|d| *d > 0.0)
                                        .map_or_else(
                                            || "Still image".into(),
                                            |d| crate::format_duration(d.round() as u64),
                                        ),
                                    9.0,
                                    MUTED,
                                    false,
                                );
                                for (i, key) in ["playlist-down", "playlist-up", "playlist-remove"]
                                    .iter()
                                    .enumerate()
                                {
                                    let center = row.right_top()
                                        + match i {
                                            0 => Vec2::new(-71.0, 28.0),
                                            1 => Vec2::new(-45.0, 28.0),
                                            _ => Vec2::new(-15.0, 28.0),
                                        };
                                    let button =
                                        egui::Rect::from_center_size(center, Vec2::new(28.0, 32.0));
                                    let enabled = !self.busy
                                        && match i {
                                            0 => index + 1 < urls.len(),
                                            1 => index > 0,
                                            _ => true,
                                        };
                                    child_ui(ui, egui::UiBuilder::new().max_rect(button), |ui| {
                                        if icon_button(
                                            ui,
                                            key,
                                            button.size(),
                                            MUTED,
                                            Color32::TRANSPARENT,
                                            Color32::TRANSPARENT,
                                            enabled,
                                        )
                                        .clicked()
                                        {
                                            match i {
                                                0 => movement = Some((index, index + 1)),
                                                1 => movement = Some((index, index - 1)),
                                                _ => removal = Some(index),
                                            }
                                        }
                                    });
                                }
                            }
                            if urls.is_empty() {
                                ui.label(RichText::new("This playlist is empty.").color(MUTED));
                            }
                        });
                });
                label(
                    &p,
                    r.min,
                    24.0,
                    406.0,
                    "Rotate every",
                    10.0,
                    SECONDARY,
                    false,
                );
                let interval = egui::Rect::from_min_size(
                    r.min + Vec2::new(104.0, 400.0),
                    Vec2::new(92.0, 32.0),
                );
                p.rect_stroke(
                    interval,
                    8.0,
                    Stroke::new(1.0_f32, BORDER),
                    egui::StrokeKind::Inside,
                );
                p.text(
                    interval.center(),
                    egui::Align2::CENTER_CENTER,
                    self.playlist_interval_draft.to_string(),
                    egui::FontId::proportional(11.0),
                    TEXT,
                );
                for (i, key) in ["interval-minus", "interval-plus"].iter().enumerate() {
                    let button = egui::Rect::from_min_size(
                        interval.min + Vec2::new(if i == 0 { 0.0 } else { 64.0 }, 2.0),
                        Vec2::new(28.0, 28.0),
                    );
                    child_ui(ui, egui::UiBuilder::new().max_rect(button), |ui| {
                        if icon_button(
                            ui,
                            key,
                            button.size(),
                            MUTED,
                            Color32::TRANSPARENT,
                            Color32::TRANSPARENT,
                            !self.busy,
                        )
                        .clicked()
                        {
                            self.playlist_interval_draft = if i == 0 {
                                self.playlist_interval_draft.saturating_sub(1).max(1)
                            } else {
                                (self.playlist_interval_draft + 1).min(1440)
                            };
                        }
                    });
                }
                label(&p, r.min, 204.0, 406.0, "minutes", 9.0, MUTED, false);
                let toggle = egui::Rect::from_min_size(
                    r.min + Vec2::new(360.0, 408.0),
                    Vec2::new(28.0, 16.0),
                );
                child_ui(ui, egui::UiBuilder::new().max_rect(toggle), |ui| {
                    toggle_switch_sized(
                        ui,
                        &mut self.playlist_random_draft,
                        !self.busy,
                        toggle.size(),
                    );
                });
                label(
                    &p,
                    r.min,
                    396.0,
                    406.0,
                    "Shuffle order",
                    10.0,
                    SECONDARY,
                    false,
                );
                label(&p, r.min, 24.0, 448.0, "Playlist folder", 9.0, MUTED, false);
                let folder = self.location.root.join("Playlists");
                label(
                    &p,
                    r.min,
                    24.0,
                    464.0,
                    fit(
                        &p,
                        &folder.display().to_string(),
                        9.0,
                        false,
                        r.width() - 48.0,
                    ),
                    9.0,
                    MUTED,
                    false,
                );
                line(&p, r, 500.0);
                icons::paint_node(&p, r.min + Vec2::new(32.0, 524.0), 786, MUTED, None);
                label(
                    &p,
                    r.min,
                    46.0,
                    518.0,
                    "Changes apply to the active rotation.",
                    9.0,
                    MUTED,
                    false,
                );
                let (c, s) = footer(ui, r, 512.0, "Save changes", "check", !self.busy);
                cancel |= c;
                save = s;
            });
        if let Some((from, to)) = movement {
            if from < self.playlist_draft.len() && to < self.playlist_draft.len() {
                let item = self.playlist_draft.remove(from);
                self.playlist_draft.insert(to, item);
            }
        }
        if let Some(index) = removal {
            self.playlist_draft.remove(index);
        }
        if ctx.input(|i| i.pointer.any_released()) {
            self.playlist_drag_index = None;
        }
        if cancel || modal.should_close() {
            self.playlist_dialog_open = false;
            self.playlist_drag_index = None;
        }
        if save {
            self.playlist = self.playlist_draft.clone();
            self.save_playlist_configuration(ctx);
        }
    }

    pub(crate) fn removal_dialog(&mut self, ctx: &egui::Context) {
        let Some((url, name)) = self.remove_confirmation.clone() else {
            return;
        };
        let mut cancel = false;
        let mut remove = false;
        let modal=egui::Modal::new(egui::Id::new("remove-dialog")).frame(egui::Frame::NONE).show(ctx,|ui|{
            let r=shell(ui,Vec2::new(360.0,220.0),"Remove Dialog");let p=ui.painter().clone();
            label(&p,r.min,20.0,18.0,"Remove from library",17.0,TEXT,true);
            let close=egui::Rect::from_min_size(r.min+Vec2::new(316.0,16.0),Vec2::splat(28.0));
            cancel=child_ui(ui,egui::UiBuilder::new().max_rect(close),|ui|icon_button(ui,"remove-close",close.size(),MUTED,Color32::TRANSPARENT,Color32::TRANSPARENT,true).clicked()).inner;
            line(&p, r, 56.0);
            let short_name = if name.chars().count() > 48 {
                format!("{}…", name.chars().take(47).collect::<String>())
            } else {
                name.clone()
            };
            let sentence = egui::Rect::from_min_size(r.min + Vec2::new(20.0, 70.0), Vec2::new(320.0, 34.0));
            child_ui(ui, egui::UiBuilder::new().max_rect(sentence), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Remove “{short_name}” from your library?"))
                            .font(egui::FontId::new(12.0, egui::FontFamily::Name("Inter".into())))
                            .color(TEXT),
                    )
                    .wrap(),
                );
            });
            let info = egui::Rect::from_min_size(r.min + Vec2::new(20.0, 110.0), Vec2::new(320.0, 42.0));
            child_ui(ui, egui::UiBuilder::new().max_rect(info), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new("Your original file is never deleted. You can import it again at any time.")
                            .size(9.5)
                            .color(MUTED),
                    )
                    .wrap(),
                );
            });
            line(&p, r, 160.0);
            cancel |= plain_button(ui, egui::Rect::from_min_size(r.min + Vec2::new(164.0, 172.0), Vec2::new(78.0, 34.0)), "Cancel", true).clicked();
            let button = egui::Rect::from_min_size(r.min + Vec2::new(250.0, 172.0), Vec2::new(90.0, 34.0));
            remove = child_ui(ui, egui::UiBuilder::new().max_rect(button), |ui| crate::controls::dialog_button(ui, "remove-trash", "Remove", button.size(), !self.busy, Color32::from_rgb(251, 113, 133), Color32::from_rgb(26, 10, 16), Color32::TRANSPARENT).clicked()).inner;
        });
        if cancel || modal.should_close() {
            self.remove_confirmation = None;
        }
        if remove {
            self.remove_confirmation = None;
            self.remove_selected(url, ctx);
        }
    }
}
