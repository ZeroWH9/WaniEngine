//! One bounded playlist dock at every window size; source frame nodes 249–293.
use crate::{
    ACCENT, App, BORDER, MUTED, NAV, PANEL, Page, SECONDARY, SURFACE, TEXT, child_ui, icon_button,
    paint_cover_rounded,
};
use eframe::egui::{self, Color32, Stroke, Vec2};
use std::time::{Duration, Instant};

impl App {
    pub(crate) fn playlist_dock(&mut self, ctx: &egui::Context) {
        let radius = crate::widgets::window_radius(ctx);
        let mut changed = None;
        let mut manage_clicked = false;
        let mut load_clicked = false;
        let mut save_named_clicked = false;
        let mut shuffle_clicked = false;
        let mut previous_clicked = false;
        let mut next_clicked = false;
        let can_rotate = !self.rotation_assets().is_empty();
        let can_save_named = !self.playlist.is_empty();
        let can_add = self.selected.as_ref().is_some_and(|u| {
            self.managed_urls.contains(u)
                && !self.playlist.contains(u)
                && self.assets.iter().any(|a| {
                    a["url"].as_str() == Some(u)
                        && a["kind"].as_str() != Some("character")
                        && a["missing"].as_bool() != Some(true)
                })
        });
        let shuffle_enabled = self.settings["playlistRandom"].as_bool().unwrap_or(false);
        egui::TopBottomPanel::bottom("playlist-dock")
            .exact_height(88.0)
            .frame(
                egui::Frame::new()
                    .fill(NAV)
                    .inner_margin(0)
                    .corner_radius(egui::CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: radius,
                        se: radius,
                    }),
            )
            .show(ctx, |ui| {
                let screen = ctx.screen_rect();
                let width = screen.width();
                let y = screen.bottom() - 88.0;
                let origin = egui::pos2(screen.left(), y);
                crate::preview::record_geometry(
                    ui,
                    "Playlist Dock",
                    egui::Rect::from_min_size(origin, Vec2::new(width, 88.0)),
                );
                let p = ui.painter().clone();
                p.line_segment(
                    [origin, egui::pos2(screen.right(), y)],
                    Stroke::new(1.0_f32, BORDER),
                );
                // Room left of the control cluster (6 buttons + 5 gaps, 28px margin).
                let cluster_width = 36.0 * 5.0 + 52.0 + 5.0 * 8.0;
                let avail = width - cluster_width - 28.0 - 24.0;
                let compact = avail < 1040.0;
                let tiny = avail < 698.0;
                let queue_count = if !compact {
                    5
                } else if avail >= 572.0 {
                    3
                } else if avail >= 488.0 {
                    1
                } else {
                    0
                };
                let cover =
                    egui::Rect::from_min_size(origin + Vec2::new(28.0, 22.0), Vec2::splat(44.0));
                p.rect_filled(cover, 9.0, PANEL);
                if let Some(texture) = self
                    .playlist
                    .first()
                    .and_then(|u| self.assets.iter().find(|a| a["url"].as_str() == Some(u)))
                    .and_then(|a| a["thumbnailUrl"].as_str().or_else(|| a["url"].as_str()))
                    .and_then(|u| self.thumbnails.get(u))
                {
                    paint_cover_rounded(ui, cover, texture, [9.0; 4]);
                }
                let music = cover.right_bottom() - Vec2::splat(8.0);
                p.circle_filled(music, 10.0, ACCENT);
                crate::icons::paint_node(&p, music, 1356, SURFACE, Some(Vec2::new(11.0, 12.0)));
                let info = egui::Rect::from_min_size(
                    origin + Vec2::new(84.0, 20.0),
                    Vec2::new(if compact { 184.0 } else { 216.0 }, 44.0),
                );
                let name_font =
                    egui::FontId::new(14.0, egui::FontFamily::Name("Inter SemiBold".into()));
                let name_text = crate::widgets::ellipsize(
                    &p,
                    &self.playlist_name,
                    &name_font,
                    info.width() - 12.0,
                );
                let name = p.layout_no_wrap(name_text, name_font, TEXT);
                p.with_clip_rect(info)
                    .galley(origin + Vec2::new(84.0, 23.0), name, TEXT);
                if self.playlist_dirty {
                    p.circle_filled(
                        info.right_top() + Vec2::new(-4.0, 9.0),
                        2.5,
                        Color32::from_rgb(251, 191, 36),
                    );
                }
                let state = if self.rotation_waiting {
                    "Applying"
                } else if self.rotation_running {
                    "Running"
                } else {
                    "Paused"
                };
                let interval = self.settings["playlistInterval"]
                    .as_u64()
                    .unwrap_or(30)
                    .clamp(1, 1440);
                let status_font = egui::FontId::proportional(10.0);
                let status_text = crate::widgets::ellipsize(
                    &p,
                    &format!(
                        "{state}  ·  {} wallpapers  ·  every {interval} min",
                        self.playlist.len()
                    ),
                    &status_font,
                    info.width() - 10.0,
                );
                p.with_clip_rect(info).text(
                    origin + Vec2::new(84.0, 42.0),
                    egui::Align2::LEFT_TOP,
                    status_text,
                    status_font,
                    MUTED,
                );
                let queue_x = if compact { 304.0 } else { 344.0 };
                if queue_count > 0 {
                    p.line_segment(
                        [
                            origin + Vec2::new(queue_x - 22.0, 20.0),
                            origin + Vec2::new(queue_x - 22.0, 66.0),
                        ],
                        Stroke::new(1.0_f32, BORDER),
                    );
                    p.text(
                        origin + Vec2::new(queue_x, 19.0),
                        egui::Align2::LEFT_TOP,
                        "UP NEXT",
                        egui::FontId::proportional(9.0),
                        MUTED,
                    );
                }
                let count = queue_count;
                let size = if compact {
                    Vec2::new(34.0, 28.0)
                } else {
                    Vec2::new(52.0, 40.0)
                };
                for (i, url) in self.playlist.iter().take(count).enumerate() {
                    let r = egui::Rect::from_min_size(
                        origin + Vec2::new(queue_x + i as f32 * (size.x + 8.0), 34.0),
                        size,
                    );
                    p.rect_filled(r, 7.0, PANEL);
                    let asset = self.assets.iter().find(|a| a["url"].as_str() == Some(url));
                    if let Some(texture) = asset
                        .and_then(|a| a["thumbnailUrl"].as_str().or_else(|| a["url"].as_str()))
                        .and_then(|u| self.thumbnails.get(u))
                    {
                        paint_cover_rounded(ui, r, texture, [7.0; 4]);
                    }
                    p.rect_stroke(
                        r,
                        7.0,
                        Stroke::new(
                            1.0_f32,
                            if self.rotation_current_url.as_ref() == Some(url) {
                                ACCENT
                            } else {
                                BORDER
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    let hit = egui::Rect::from_center_size(
                        r.center(),
                        Vec2::new(r.width().max(28.0), r.height().max(28.0)),
                    );
                    let response =
                        ui.interact(hit, ui.id().with(("queue", i)), egui::Sense::click());
                    let click = response.clicked();
                    response.on_hover_text(
                        asset
                            .and_then(|a| a["name"].as_str())
                            .unwrap_or("Unavailable"),
                    );
                    if click {
                        self.selected = Some(url.clone());
                        self.page = Page::Library;
                    }
                }
                if count > 0 && self.playlist.len() > count {
                    let r = egui::Rect::from_min_size(
                        origin + Vec2::new(queue_x + count as f32 * (size.x + 8.0), 34.0),
                        if compact { size } else { Vec2::new(44.0, 40.0) },
                    );
                    p.rect_filled(r, 7.0, PANEL);
                    p.rect_stroke(
                        r,
                        7.0,
                        Stroke::new(1.0_f32, BORDER),
                        egui::StrokeKind::Inside,
                    );
                    p.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("+{}", self.playlist.len() - count),
                        egui::FontId::proportional(10.0),
                        MUTED,
                    );
                }
                if !tiny {
                    let x = if compact { 588.0 } else { 820.0 };
                    let length = if compact { 110.0 } else { 220.0 };
                    p.text(
                        origin + Vec2::new(x, 19.0),
                        egui::Align2::LEFT_TOP,
                        "NEXT ROTATION",
                        egui::FontId::proportional(9.0),
                        MUTED,
                    );
                    let remaining = if self.preview.is_some() {
                        Duration::from_secs(1104)
                    } else {
                        self.rotation_deadline.map_or(Duration::ZERO, |d| {
                            d.saturating_duration_since(Instant::now())
                        })
                    };
                    let caption = if self.rotation_waiting {
                        "Applying…".into()
                    } else if self.rotation_running {
                        format!(
                            "{}:{:02} remaining",
                            remaining.as_secs() / 60,
                            remaining.as_secs() % 60
                        )
                    } else {
                        "Paused".into()
                    };
                    p.text(
                        origin + Vec2::new(x, 36.0),
                        egui::Align2::LEFT_TOP,
                        caption,
                        egui::FontId::proportional(11.0),
                        SECONDARY,
                    );
                    let rail = egui::Rect::from_min_size(
                        origin + Vec2::new(x, 58.0),
                        Vec2::new(length, 3.0),
                    );
                    p.rect_filled(rail, 1.5, BORDER);
                    let value = if self.rotation_waiting {
                        0.35
                    } else if self.rotation_running {
                        (1.0 - remaining.as_secs_f32()
                            / self.rotation_interval().as_secs_f32().max(1.0))
                        .clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    p.rect_filled(
                        egui::Rect::from_min_size(rail.min, Vec2::new(length * value, 3.0)),
                        1.5,
                        ACCENT,
                    );
                }
                let cluster = [
                    (0, "shuffle", 36.0_f32),
                    (1, "skip-previous", 36.0),
                    (
                        2,
                        if self.rotation_running {
                            "pause"
                        } else {
                            "play-large"
                        },
                        52.0,
                    ),
                    (3, "skip-next", 36.0),
                    (5, "plus", 36.0),
                    (6, "more", 36.0),
                ];
                let gap = 8.0_f32;
                let total: f32 =
                    cluster.iter().map(|c| c.2).sum::<f32>() + gap * (cluster.len() as f32 - 1.0);
                let mut cursor = width - 28.0 - total;
                for (index, key, size) in cluster {
                    let r = egui::Rect::from_center_size(
                        origin + Vec2::new(cursor + size / 2.0, 44.0),
                        Vec2::splat(size),
                    );
                    cursor += size + gap;
                    child_ui(ui, egui::UiBuilder::new().max_rect(r), |ui| {
                        let enabled = !self.busy
                            && match index {
                                1 | 3 => can_rotate,
                                2 => self.rotation_running || can_rotate,
                                5 => can_add,
                                _ => true,
                            };
                        let response = icon_button(
                            ui,
                            key,
                            r.size(),
                            if index == 2 {
                                SURFACE
                            } else if index == 0 && shuffle_enabled {
                                ACCENT
                            } else if index == 5 {
                                ACCENT
                            } else {
                                SECONDARY
                            },
                            if index == 2 {
                                ACCENT
                            } else if index == 5 {
                                PANEL
                            } else if index == 0 && shuffle_enabled {
                                Color32::from_rgba_unmultiplied(
                                    ACCENT.r(),
                                    ACCENT.g(),
                                    ACCENT.b(),
                                    40,
                                )
                            } else {
                                Color32::TRANSPARENT
                            },
                            if index == 5 {
                                BORDER
                            } else {
                                Color32::TRANSPARENT
                            },
                            enabled,
                        );
                        if response.clicked() {
                            match index {
                                0 => shuffle_clicked = true,
                                1 => previous_clicked = true,
                                2 => self.toggle_rotation(ctx),
                                3 => next_clicked = true,
                                5 => {
                                    if let Some(url) = self.selected.clone() {
                                        let mut next = self.playlist.clone();
                                        next.push(url);
                                        changed = Some(next);
                                    }
                                }
                                6 => {
                                    let id = ui.make_persistent_id("playlist-dock-more-menu");
                                    if !ui.memory(|m| m.is_popup_open(id)) {
                                        response.clone().on_hover_text("More playlist actions");
                                    }
                                    ui.memory_mut(|m| m.toggle_popup(id));
                                }
                                _ => {}
                            }
                        }
                        if index == 6 {
                            let id = ui.make_persistent_id("playlist-dock-more-menu");
                            egui::popup::popup_above_or_below_widget(
                                ui,
                                id,
                                &response,
                                egui::AboveOrBelow::Above,
                                egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                                |ui| {
                                    // The popup is as wide as its 36px anchor by default.
                                    ui.set_min_width(200.0);
                                    if ui
                                        .add_enabled(
                                            !self.busy,
                                            egui::Button::new("Manage playlist…"),
                                        )
                                        .clicked()
                                    {
                                        manage_clicked = true;
                                        ui.memory_mut(|m| m.close_popup());
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.busy,
                                            egui::Button::new("Load playlist…"),
                                        )
                                        .clicked()
                                    {
                                        load_clicked = true;
                                        ui.memory_mut(|m| m.close_popup());
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.busy && can_save_named,
                                            egui::Button::new("Save playlist as…"),
                                        )
                                        .clicked()
                                    {
                                        save_named_clicked = true;
                                        ui.memory_mut(|m| m.close_popup());
                                    }
                                },
                            );
                        } else {
                            if enabled {
                                response.on_hover_text(match index {
                                    0 => "Toggle shuffle order",
                                    1 => "Previous wallpaper",
                                    2 => "Play or pause rotation (Space)",
                                    3 => "Next wallpaper",
                                    5 => "Add selected wallpaper to playlist",
                                    _ => "",
                                });
                            } else {
                                response.on_hover_text(if self.busy {
                                    "Busy — wait for the current task to finish"
                                } else if index == 5 {
                                    "Select a wallpaper that is not already in the playlist"
                                } else {
                                    "The playlist is empty — add wallpapers first"
                                });
                            }
                        }
                    });
                }
            });
        if manage_clicked {
            self.playlist_draft = self.playlist.clone();
            self.playlist_interval_draft = self.settings["playlistInterval"]
                .as_u64()
                .unwrap_or(30)
                .clamp(1, 1440);
            self.playlist_random_draft = self.settings["playlistRandom"].as_bool().unwrap_or(false);
            self.playlist_dialog_open = true;
        }
        if load_clicked {
            self.load_named_playlist(ctx);
        }
        if save_named_clicked {
            self.save_named_playlist(ctx);
        }
        if shuffle_clicked {
            self.playlist_interval_draft = self.settings["playlistInterval"]
                .as_u64()
                .unwrap_or(30)
                .clamp(1, 1440);
            self.playlist_random_draft = !shuffle_enabled;
            self.settings["playlistRandom"] = serde_json::json!(self.playlist_random_draft);
            self.save_playlist_configuration(ctx);
        }
        if previous_clicked {
            self.step_rotation(-1, ctx);
        }
        if next_clicked {
            self.step_rotation(1, ctx);
        }
        if let Some(next) = changed {
            self.playlist = next.clone();
            self.playlist_dirty = true;
            self.save_playlist(next, ctx);
        }
    }
}
