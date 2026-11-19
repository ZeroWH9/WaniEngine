//! Discover presents curated groupings from the user's local collection.
use crate::{
    ACCENT, App, BORDER, DiscoverFilter, Filter, MUTED, PANEL, Page, SECONDARY, TEXT, card_details,
    icon_link_with_metrics, icon_text_button, paint_action_icon, paint_cover_rounded,
    paint_play_outline,
};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde_json::Value;

fn filtered_wallpapers(assets: &[Value], filter: DiscoverFilter) -> Vec<Value> {
    let mut items: Vec<_> = assets
        .iter()
        .filter(|asset| {
            asset["kind"].as_str() != Some("character")
                && asset["missing"].as_bool() != Some(true)
                && asset["url"].is_string()
                && match filter {
                    DiscoverFilter::All => true,
                    DiscoverFilter::Video => asset["video"].as_bool() == Some(true),
                    DiscoverFilter::Image => asset["video"].as_bool() != Some(true),
                }
        })
        .cloned()
        .collect();
    items.sort_by_key(|asset| {
        (
            asset["discoverRank"].as_u64().unwrap_or(u64::MAX),
            std::cmp::Reverse(asset["addedAt"].as_u64().unwrap_or(0)),
            asset["url"].as_str().unwrap_or("").to_owned(),
        )
    });
    items
}

impl App {
    pub(crate) fn discover_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.spacing_mut().interact_size.y = 20.0;
            let wallpapers = filtered_wallpapers(&self.assets, self.discover_filter);
            let library_empty = filtered_wallpapers(&self.assets, DiscoverFilter::All).is_empty();
            let count = wallpapers.len();
            let avail = ui.available_width();
            let pill_beside = avail >= 580.0;
            let show_badge = !library_empty && avail >= 730.0;
            let header_height = if pill_beside { 86.0 } else { 140.0 };
            let (header, _) =
                ui.allocate_exact_size(Vec2::new(avail, header_height), egui::Sense::hover());
            crate::widgets::tracked_text(
                ui.painter(),
                header.min + Vec2::new(0.0, 20.0),
                egui::Align2::LEFT_TOP,
                "Discover",
                egui::FontId::new(34.0, egui::FontFamily::Name("Inter SemiBold".into())),
                TEXT,
                -1.5,
            );
            ui.painter().text(
                header.min + Vec2::new(0.0, 68.0),
                egui::Align2::LEFT_TOP,
                "Hand-picked moods from your local library.",
                egui::FontId::proportional(13.0),
                MUTED,
            );
            let filters = if pill_beside {
                egui::Rect::from_min_size(
                    egui::pos2(header.right() - 264.0, header.top() + 24.0),
                    Vec2::new(264.0, 40.0),
                )
            } else {
                egui::Rect::from_min_size(
                    egui::pos2(header.left(), header.top() + 96.0),
                    Vec2::new(264.0_f32.min(avail), 40.0),
                )
            };
            ui.painter().rect_filled(filters, 10.0, PANEL);
            ui.painter().rect_stroke(
                filters,
                10.0,
                Stroke::new(1.0_f32, BORDER),
                egui::StrokeKind::Inside,
            );
            for (i, (filter, label)) in [
                (DiscoverFilter::All, "All"),
                (DiscoverFilter::Video, "Video"),
                (DiscoverFilter::Image, "Image"),
            ]
            .iter()
            .enumerate()
            {
                let rect = egui::Rect::from_min_size(
                    filters.min + Vec2::new(4.0 + i as f32 * 86.0, 4.0),
                    Vec2::new(82.0, 32.0),
                );
                let selected = self.discover_filter == *filter;
                if selected {
                    ui.painter()
                        .rect_filled(rect, 8.0, Color32::from_rgb(42, 36, 64));
                }
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    *label,
                    egui::FontId::new(11.0, egui::FontFamily::Name("Inter Medium".into())),
                    if selected { TEXT } else { MUTED },
                );
                if ui
                    .interact(
                        rect,
                        egui::Id::new(("discover-filter", i)),
                        egui::Sense::click(),
                    )
                    .clicked()
                {
                    self.discover_filter = *filter;
                }
            }
            if show_badge {
                let badge = egui::Rect::from_min_size(
                    egui::pos2(filters.left() - 146.0, header.top() + 30.0),
                    Vec2::new(132.0, 30.0),
                );
                ui.painter().rect_filled(badge, 15.0, PANEL);
                ui.painter().text(
                    badge.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("• {count} items"),
                    egui::FontId::proportional(10.0),
                    SECONDARY,
                );
            }
            ui.add_space(16.0);
            if let Some(error) = &self.error {
                ui.colored_label(
                    Color32::from_rgb(251, 113, 133),
                    format!("Could not load library: {error}"),
                );
                return;
            }
            // `wallpapers` is already sorted by discoverRank, then newest addedAt, then url.
            if let Some(featured) = wallpapers.first().cloned() {
                let has_4k = wallpapers.iter().any(|asset| {
                    asset["width"].as_u64().unwrap_or_default() >= 3840
                        || asset["height"].as_u64().unwrap_or_default() >= 2160
                });
                self.featured_collection(ui, &featured, wallpapers.len(), has_4k);
            } else if library_empty {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(14)
                    .inner_margin(24)
                    .show(ui, |ui| {
                        ui.set_min_height(140.0);
                        ui.vertical_centered(|ui| {
                            ui.heading("Your collection starts here");
                            ui.label(
                                RichText::new("Import wallpapers in Library to see them here.")
                                    .color(MUTED),
                            );
                            if icon_text_button(
                                ui,
                                "arrow-right",
                                "Open Library",
                                Vec2::new(152.0, 36.0),
                                true,
                                PANEL,
                                SECONDARY,
                                BORDER,
                            )
                            .clicked()
                            {
                                self.page = Page::Library;
                            }
                        });
                    });
            }

            if !library_empty {
                ui.add_space(24.0);
                ui.horizontal_wrapped(|ui| {
                    ui.heading(RichText::new("Recent in your library").size(18.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_link_with_metrics(ui, "arrow-right", "View all", 11.5, 21.0)
                            .clicked()
                        {
                            self.page = Page::Library;
                            self.filter = Filter::Wallpapers;
                        }
                    });
                });
                if wallpapers.is_empty() {
                    ui.label(RichText::new("No wallpapers match this filter.").color(MUTED));
                } else {
                    ui.add_space(4.0);
                    egui::ScrollArea::horizontal()
                        .id_salt("discover-trending")
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = 16.0;
                                for item in wallpapers.iter().take(6) {
                                    if self.discover_card(ui, item, 244.0) {
                                        self.open_wallpaper(item);
                                    }
                                }
                            });
                        });
                }
            }

            ui.add_space(20.0);
            ui.horizontal_wrapped(|ui| {
                ui.vertical(|ui| {
                    ui.heading(RichText::new("Characters in motion").size(18.0));
                    ui.label(
                        RichText::new("They live above your windows and follow the cursor.")
                            .size(11.5)
                            .color(MUTED),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_link_with_metrics(ui, "arrow-right", "Open animations", 11.5, 3.0)
                        .clicked()
                    {
                        self.page = Page::Animations;
                    }
                });
            });
            let characters: Vec<_> = self
                .assets
                .iter()
                .filter(|asset| {
                    asset["kind"].as_str() == Some("character")
                        && asset["missing"].as_bool() != Some(true)
                        && asset["url"].is_string()
                })
                .take(3)
                .cloned()
                .collect();
            if characters.is_empty() {
                ui.label(RichText::new("No characters in your library yet.").color(MUTED));
            } else {
                ui.add_space(0.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    let gaps = 16.0_f32;
                    let avail = ui.available_width();
                    let cols = (((avail + gaps) / (260.0 + gaps)).floor()).clamp(1.0, 3.0);
                    let width =
                        ((avail - gaps * (cols - 1.0)) / cols - 0.5).max(260.0_f32.min(avail));
                    for character in &characters {
                        self.character_card(ui, character, width);
                    }
                });
            }
            ui.add_space(24.0);
        });
    }

    fn featured_collection(
        &mut self,
        ui: &mut egui::Ui,
        featured: &Value,
        count: usize,
        has_4k: bool,
    ) {
        let width = ui.available_width();
        let compact = width < 920.0;
        let featured_name = featured["name"].as_str().unwrap_or("Wallpaper");
        let height = if compact { 224.0 } else { 244.0 };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        crate::preview::record_geometry(ui, "Featured Collection", rect);
        ui.painter().rect_filled(rect, 14.0, PANEL);
        let source = featured["thumbnailUrl"]
            .as_str()
            .or_else(|| featured["url"].as_str());
        if let Some(texture) = self.discover_background.as_ref() {
            paint_rounded_cover(ui, rect, texture, 14.0);
        } else if let Some(texture) = source.and_then(|url| self.thumbnails.get(url)) {
            paint_cover_rounded(ui, rect, texture, [14.0; 4]);
        }
        let shade = egui::Rect::from_min_size(
            rect.min,
            Vec2::new(900.0_f32.min(rect.width()), rect.height()),
        );
        ui.painter().add(egui::Shape::mesh(shade_mesh(
            shade,
            true,
            &[(0.0, 242), (0.5, 179), (1.0, 0)],
        )));
        let bottom =
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 150.0), rect.max);
        ui.painter().add(egui::Shape::mesh(shade_mesh(
            bottom,
            false,
            &[(0.0, 0), (1.0, 179)],
        )));
        if compact {
            let info = egui::Rect::from_min_max(
                rect.min + egui::vec2(18.0, 14.0),
                rect.max - egui::vec2(18.0, 14.0),
            );
            let mut open = false;
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(info), |ui| {
                ui.set_width(info.width());
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new("FEATURED").size(9.0).color(TEXT));
                ui.heading(RichText::new(featured_name).size(24.0));
                ui.label(
                    RichText::new(if has_4k {
                        format!("{count} wallpapers · Includes 4K · Local library")
                    } else {
                        format!("{count} wallpapers · Local library")
                    })
                    .size(10.0)
                    .color(MUTED),
                );
                ui.add_space(4.0);
                open = icon_text_button(
                    ui,
                    "arrow-right",
                    "View wallpaper",
                    Vec2::new(176.0, 40.0),
                    true,
                    Color32::from_black_alpha(30),
                    TEXT,
                    Color32::from_rgb(58, 52, 72),
                )
                .clicked();
            });
            if open {
                self.open_wallpaper(featured);
            }
            return;
        }
        let info = egui::Rect::from_min_max(
            rect.min + egui::vec2(32.0, 30.0),
            egui::pos2(rect.left() + 532.0, rect.max.y - 20.0),
        );
        let painter = ui.painter().with_clip_rect(info);
        crate::widgets::tracked_text(
            &painter,
            info.left_top(),
            egui::Align2::LEFT_TOP,
            "FEATURED",
            egui::FontId::new(9.5, egui::FontFamily::Name("Inter SemiBold".into())),
            Color32::from_rgb(216, 211, 255),
            12.0,
        );
        crate::widgets::tracked_text(
            &painter,
            info.left_top() + egui::vec2(0.0, 18.0),
            egui::Align2::LEFT_TOP,
            featured_name,
            egui::FontId::new(32.0, egui::FontFamily::Name("Inter SemiBold".into())),
            TEXT,
            -1.2,
        );
        painter.text(
            info.left_top() + egui::vec2(0.0, 90.0),
            egui::Align2::LEFT_TOP,
            if has_4k {
                format!("{count} wallpapers · Includes 4K · Local library")
            } else {
                format!("{count} wallpapers · Local library")
            },
            egui::FontId::proportional(10.5),
            MUTED,
        );
        let mut open = false;
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(info.left(), rect.top() + 160.0),
            Vec2::new(176.0, 40.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(button_rect), |ui| {
            open = icon_text_button(
                ui,
                "arrow-right",
                "View wallpaper",
                Vec2::new(176.0, 40.0),
                true,
                Color32::from_black_alpha(30),
                TEXT,
                Color32::from_rgb(58, 52, 72),
            )
            .clicked();
        });
        let clip = ui.painter().with_clip_rect(rect);
        let poster = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 363.5, rect.top() + 23.75),
            Vec2::new(300.0, 176.0),
        );
        // FigCraft's -4 degree rotation uses an upward canvas axis; egui's y axis points down.
        let angle = 4.0_f32.to_radians();
        let rotation = egui::emath::Rot2::from_angle(angle);
        let rotate = |point: egui::Pos2| poster.min + rotation * (point - poster.min);
        let mut mesh = source
            .and_then(|url| self.thumbnails.get(url))
            .map(|texture| crate::rounded_cover_mesh(poster, texture, [14.0; 4]));
        if let Some(mesh) = mesh.as_mut() {
            for v in &mut mesh.vertices {
                v.pos = rotate(v.pos);
            }
            clip.add(egui::Shape::mesh(mesh.clone()));
        }
        let mut shade = shade_mesh(
            egui::Rect::from_min_max(poster.min + Vec2::new(0.0, 96.0), poster.max),
            false,
            &[(0.0, 0), (1.0, 217)],
        );
        for v in &mut shade.vertices {
            v.pos = rotate(v.pos);
        }
        clip.add(egui::Shape::mesh(shade));
        let draw_text = |pos: egui::Pos2, label: &str, size: f32, color: Color32, bold: bool| {
            let font = egui::FontId::new(
                size,
                egui::FontFamily::Name(if bold { "Inter SemiBold" } else { "Inter" }.into()),
            );
            let galley = clip.layout_no_wrap(label.to_owned(), font, color);
            clip.add(egui::epaint::TextShape::new(rotate(pos), galley, color).with_angle(angle));
        };
        draw_text(
            poster.min + Vec2::new(16.0, 134.0),
            featured["name"].as_str().unwrap_or("Wallpaper"),
            13.0,
            TEXT,
            true,
        );
        draw_text(
            poster.min + Vec2::new(16.0, 152.0),
            &poster_details(featured),
            9.0,
            MUTED,
            false,
        );
        draw_text(
            poster.min + Vec2::new(268.0, 14.0),
            &crate::asset_badge(featured),
            9.0,
            TEXT,
            true,
        );
        let play_rect = egui::Rect::from_center_size(
            rotate(poster.min + Vec2::new(269.0, 147.0)),
            Vec2::splat(30.0),
        );
        clip.circle_filled(play_rect.center(), 15.0, Color32::from_rgb(217, 215, 225));
        paint_play_outline(&clip, play_rect.center(), PANEL, Vec2::new(12.0, 13.5), 1.6);
        let play = ui
            .interact(
                play_rect,
                egui::Id::new(("discover-featured-play", featured["url"].as_str())),
                if self.busy {
                    egui::Sense::hover()
                } else {
                    egui::Sense::click()
                },
            )
            .on_hover_text(if self.busy {
                "Wait for the current operation to finish."
            } else if featured["video"].as_bool() == Some(true) {
                "Preview this video in Library."
            } else {
                "Open this wallpaper in Library."
            });
        if play.clicked() {
            self.open_wallpaper(featured);
            if featured["video"].as_bool() == Some(true) {
                self.autoplay_video_preview = true;
                ui.ctx().request_repaint();
            }
        }
        if open {
            self.open_wallpaper(featured);
        }
    }

    fn open_wallpaper(&mut self, asset: &Value) {
        self.selected = asset["url"].as_str().map(str::to_owned);
        self.filter = Filter::Wallpapers;
        self.search.clear();
        self.page = Page::Library;
    }

    fn discover_card(&mut self, ui: &mut egui::Ui, asset: &Value, width: f32) -> bool {
        let height = 176.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
        crate::preview::record_geometry(
            ui,
            format!(
                "{} Discover Card",
                asset["name"].as_str().unwrap_or("Content")
            ),
            rect,
        );
        ui.painter().rect_filled(rect, 12.0, PANEL);
        let art = rect;
        let url = asset["thumbnailUrl"]
            .as_str()
            .or_else(|| asset["url"].as_str());
        if ui.is_rect_visible(art) {
            if let Some(texture) = url.and_then(|url| self.thumbnails.get(url)) {
                paint_rounded_cover(ui, art, texture, 12.0);
            }
        }
        ui.painter().add(egui::Shape::mesh(shade_mesh(
            egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 86.0), rect.max),
            false,
            &[(0.0, 0), (1.0, 230)],
        )));
        let favorite = asset["favorite"].as_bool().unwrap_or(false);
        let asset_url = asset["url"].as_str().unwrap_or_default();
        let favorite_enabled = self.managed_urls.contains(asset_url) && !self.busy;
        let heart_rect =
            egui::Rect::from_min_size(art.left_top() + egui::vec2(8.0, 8.0), Vec2::splat(26.0));
        let heart = ui
            .interact(
                heart_rect,
                egui::Id::new(("discover-favorite", asset_url)),
                if favorite_enabled {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            )
            .on_hover_text(if self.busy {
                "Wait for the current operation to finish."
            } else if favorite_enabled {
                if favorite {
                    "Remove favorite"
                } else {
                    "Add favorite"
                }
            } else {
                "Import this item before favoriting it."
            });
        ui.painter()
            .circle_filled(heart_rect.center(), 13.0, Color32::from_black_alpha(190));
        paint_action_icon(
            ui.painter(),
            heart_rect.center(),
            if favorite { "heart-filled" } else { "♡" },
            if favorite {
                Color32::from_rgb(251, 113, 133)
            } else if favorite_enabled {
                TEXT
            } else {
                MUTED
            },
        );
        if heart.clicked() {
            self.toggle_favorite(asset_url.to_owned(), !favorite, ui.ctx());
            return false;
        }
        let badge = asset["resolution"]
            .as_str()
            .or_else(|| asset["resolutionLabel"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let width = asset["width"].as_u64().unwrap_or(0);
                let height = asset["height"].as_u64().unwrap_or(0);
                match (width.max(height), width.min(height)) {
                    (w, h) if w >= 7680 || h >= 4320 => "8K".to_owned(),
                    (w, h) if w >= 5120 || h >= 2880 => "5K".to_owned(),
                    (w, h) if w >= 3840 || h >= 2160 => "4K".to_owned(),
                    (w, h) if w >= 2560 || h >= 1440 => "2K".to_owned(),
                    _ => {
                        if asset["video"].as_bool() == Some(true) {
                            "VIDEO".to_owned()
                        } else {
                            "IMAGE".to_owned()
                        }
                    }
                }
            });
        let badge_width = (badge.chars().count() as f32 * 7.0 + 14.0).max(36.0);
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(art.right() - badge_width - 8.0, art.top() + 8.0),
            Vec2::new(badge_width, 20.0),
        );
        ui.painter()
            .rect_filled(badge_rect, 10.0, Color32::from_black_alpha(180));
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            &badge,
            egui::FontId::proportional(9.0),
            TEXT,
        );
        let painter = ui.painter().with_clip_rect(rect);
        painter.text(
            egui::pos2(rect.left() + 14.0, rect.top() + 128.0),
            egui::Align2::LEFT_TOP,
            asset["name"].as_str().unwrap_or("Content"),
            egui::FontId::proportional(13.0),
            TEXT,
        );
        painter.text(
            egui::pos2(rect.left() + 14.0, rect.top() + 147.0),
            egui::Align2::LEFT_TOP,
            card_details(asset),
            egui::FontId::proportional(9.5),
            MUTED,
        );
        ui.painter().rect_stroke(
            rect,
            12.0,
            Stroke::new(1.0_f32, if response.hovered() { ACCENT } else { BORDER }),
            egui::StrokeKind::Inside,
        );
        response.clicked() && !heart.hovered()
    }

    fn character_card(&mut self, ui: &mut egui::Ui, asset: &Value, width: f32) {
        self.character_tile(ui, asset, width, true);
    }
}

fn paint_rounded_cover(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    texture: &egui::TextureHandle,
    radius: f32,
) {
    paint_cover_rounded(ui, rect, texture, [radius; 4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn filters_real_available_wallpapers_and_orders_recent_first() {
        let items = vec![
            json!({"url":"file:///a", "addedAt":1}),
            json!({"url":"file:///v", "video":true, "addedAt":3}),
            json!({"url":"file:///character", "kind":"character", "addedAt":4}),
            json!({"url":"file:///missing", "missing":true, "addedAt":5}),
        ];
        let all = filtered_wallpapers(&items, DiscoverFilter::All);
        assert_eq!(
            all.iter()
                .map(|a| a["url"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["file:///v", "file:///a"]
        );
        assert_eq!(filtered_wallpapers(&items, DiscoverFilter::Video).len(), 1);
        assert_eq!(filtered_wallpapers(&items, DiscoverFilter::Image).len(), 1);
    }
}

fn poster_details(asset: &Value) -> String {
    if asset["video"].as_bool() == Some(true) {
        format!(
            "Video · {}",
            crate::format_duration(asset["duration"].as_u64().unwrap_or(0))
        )
    } else {
        let w = asset["width"].as_u64().unwrap_or(0);
        let h = asset["height"].as_u64().unwrap_or(0);
        if w > 0 && h > 0 {
            format!("Image · {w} × {h}")
        } else {
            "Image".into()
        }
    }
}

fn shade_mesh(rect: egui::Rect, horizontal: bool, stops: &[(f32, u8)]) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    for (t, a) in stops {
        // Preserve the source's sRGB compositing for the dark #0C0A11 shade.
        let color = Color32::from_rgba_premultiplied(
            (12u16 * (*a as u16) / 255) as u8,
            (10u16 * (*a as u16) / 255) as u8,
            (17u16 * (*a as u16) / 255) as u8,
            *a,
        );
        for point in if horizontal {
            let x = rect.left() + t * rect.width();
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())]
        } else {
            let y = rect.top() + t * rect.height();
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)]
        } {
            mesh.colored_vertex(point, color);
        }
    }
    for i in 0..stops.len().saturating_sub(1) {
        let a = (i * 2) as u32;
        mesh.add_triangle(a, a + 1, a + 2);
        mesh.add_triangle(a + 1, a + 3, a + 2);
    }
    mesh
}
