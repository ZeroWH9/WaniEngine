use crate::{
    ACCENT, App, BORDER, MUTED, NAV, PANEL, SECONDARY, SURFACE, TEXT, icon_button,
    icon_text_button, paint_action_icon,
};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde_json::json;

const TABS: [&str; 5] = ["Performance", "Audio", "General", "Library", "About"];

impl App {
    pub(crate) fn settings_overlay(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape))
            && !ctx.wants_keyboard_input()
            && !ctx.memory(|m| m.any_popup_open())
        {
            self.cancel_settings();
            return;
        }
        let screen_rect = ctx.screen_rect();
        if self.settings_backdrop.is_none() {
            if self.settings_backdrop_requested.is_none() {
                self.settings_backdrop_requested = Some(std::time::Instant::now());
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                    "settings-blur",
                )));
                ctx.request_repaint();
                return;
            }
            if self
                .settings_backdrop_requested
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1))
            {
                ctx.request_repaint();
                return;
            }
        }
        egui::Area::new(egui::Id::new("settings-overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen_rect.min)
            .constrain(false)
            .show(ctx, |ui| {
                ui.set_min_size(screen_rect.size());
                ui.set_max_size(screen_rect.size());
                ui.set_clip_rect(screen_rect);
                let _backdrop = ui.interact(
                    screen_rect,
                    egui::Id::new("settings-backdrop"),
                    egui::Sense::click(),
                );
                if let Some(texture) = &self.settings_backdrop {
                    ui.painter().image(
                        texture.id(),
                        screen_rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(12, 10, 17, 153),
                );
                self.settings_ui(ui);
            });
    }

    pub(crate) fn poll_settings_backdrop(&mut self, ctx: &egui::Context) {
        if self.page != crate::Page::Settings {
            return;
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot {
                    image, user_data, ..
                } = e
                    && user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<&str>())
                        == Some(&"settings-blur")
                {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let bytes = image
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect::<Vec<_>>();
            if let Some(buffer) =
                image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, bytes)
            {
                let width = 400u32.min(buffer.width());
                let height = (buffer.height() as f32 * width as f32 / buffer.width() as f32)
                    .round()
                    .max(1.0) as u32;
                let reduced = image::imageops::resize(
                    &buffer,
                    width,
                    height,
                    image::imageops::FilterType::Triangle,
                );
                let blurred = image::imageops::blur(&reduced, 3.5);
                self.settings_backdrop = Some(ctx.load_texture(
                    "settings-blur",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [width as usize, height as usize],
                        &blurred,
                    ),
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
    }

    pub(crate) fn settings_ui(&mut self, ui: &mut egui::Ui) {
        if self.settings_draft.is_none() {
            let mut draft = self.settings.clone();
            draft["startWithSession"] = serde_json::json!(self.start_with_session);
            draft["closeToTray"] =
                serde_json::json!(self.settings["closeToTray"].as_bool().unwrap_or(true));
            draft["msaaSamples"] =
                serde_json::json!(self.settings["msaaSamples"].as_u64().unwrap_or(4));
            self.settings_draft = Some(draft);
        }
        let available = ui.available_rect_before_wrap();
        let width = (available.width() - 32.0).min(1040.0);
        let height = (available.height() - 32.0).min(720.0).max(320.0);
        let compact = width < 1000.0;
        let rect = egui::Rect::from_center_size(available.center(), Vec2::new(width, height));
        crate::preview::record_geometry(ui, "Settings Dialog", rect);
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::NONE)
                .corner_radius(22)
                .inner_margin(0)
                .show(ui, |ui| {
                    ui.set_min_size(Vec2::new(width, height));
                    ui.set_max_size(Vec2::new(width, height));
                    let origin = ui.min_rect().min;
                    let body_y = origin.y + 84.0;
                    let footer_height = if compact { 104.0 } else { 60.0 };
                    let footer_y = origin.y + height - footer_height;
                    let spectrum = [
                        Color32::from_rgb(255, 157, 92),
                        Color32::from_rgb(255, 95, 143),
                        Color32::from_rgb(176, 106, 245),
                        Color32::from_rgb(91, 141, 255),
                        Color32::from_rgb(71, 224, 194),
                    ];
                    let mut hairline = egui::Mesh::default();
                    let full = egui::Rect::from_min_size(origin, Vec2::new(width, height));
                    for row in 0..=6 {
                        let y = origin.y + row as f32 * 0.5;
                        let inset = crate::rounded_side_inset(y, full, 22.0, 22.0);
                        for col in 0..=8 {
                            let x = origin.x + inset + (width - 2.0 * inset) * col as f32 / 8.0;
                            let t = ((x - origin.x) / width * 4.0).clamp(0.0, 4.0);
                            let index = (t as usize).min(3);
                            hairline.colored_vertex(
                                egui::pos2(x, y),
                                crate::lerp_color(
                                    spectrum[index],
                                    spectrum[index + 1],
                                    t - index as f32,
                                ),
                            );
                        }
                    }
                    for row in 0..6 {
                        for col in 0..8 {
                            let a = (row * 9 + col) as u32;
                            hairline.add_triangle(a, a + 1, a + 9);
                            hairline.add_triangle(a + 9, a + 1, a + 10);
                        }
                    }
                    ui.painter().add(egui::Shape::mesh(hairline));
                    let header_rect =
                        egui::Rect::from_min_size(origin, Vec2::new(width, body_y - origin.y));
                    crate::child_ui(ui, egui::UiBuilder::new().max_rect(header_rect), |ui| {
                        self.settings_header(ui);
                    });
                    let body_rect = egui::Rect::from_min_max(
                        egui::pos2(origin.x, body_y),
                        egui::pos2(origin.x + width, footer_y),
                    );
                    let content_rect = if compact {
                        let navigation_rect = egui::Rect::from_min_size(
                            egui::pos2(origin.x + 12.0, body_y + 6.0),
                            Vec2::new((width - 24.0).max(0.0), 42.0),
                        );
                        crate::child_ui(
                            ui,
                            egui::UiBuilder::new().max_rect(navigation_rect),
                            |ui| {
                                egui::ComboBox::from_id_salt("settings-compact-navigation")
                                    .icon(crate::widgets::combo_chevron)
                                    .selected_text(TABS[self.settings_tab])
                                    .width(ui.available_width())
                                    .show_ui(ui, |ui| {
                                        for (index, name) in TABS.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut self.settings_tab,
                                                index,
                                                *name,
                                            );
                                        }
                                    });
                            },
                        );
                        egui::Rect::from_min_max(
                            egui::pos2(origin.x + 16.0, body_y + 54.0),
                            egui::pos2(origin.x + width - 16.0, footer_y),
                        )
                    } else {
                        let side_rect = egui::Rect::from_min_size(
                            body_rect.min,
                            Vec2::new(248.0, body_rect.height()),
                        );
                        crate::child_ui(
                            ui,
                            egui::UiBuilder::new()
                                .layout(egui::Layout::top_down(egui::Align::Min))
                                .max_rect(side_rect),
                            |ui| {
                                ui.set_max_size(side_rect.size());
                                ui.set_clip_rect(side_rect);
                                self.settings_sidebar(ui);
                            },
                        );
                        egui::Rect::from_min_size(
                            egui::pos2(origin.x + 284.0, body_y),
                            Vec2::new((width - 320.0).max(0.0), body_rect.height()),
                        )
                    };
                    crate::child_ui(
                        ui,
                        egui::UiBuilder::new()
                            .layout(egui::Layout::top_down(egui::Align::Min))
                            .max_rect(content_rect),
                        |ui| {
                            ui.set_max_size(content_rect.size());
                            ui.set_clip_rect(content_rect);
                            ui.spacing_mut().interact_size.y = 30.0;
                            ui.style_mut()
                                .text_styles
                                .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
                            egui::ScrollArea::vertical()
                                .id_salt("settings-content")
                                .auto_shrink([false, false])
                                .show(ui, |ui| self.settings_content(ui));
                        },
                    );
                    ui.painter().line_segment(
                        [
                            egui::pos2(origin.x, body_y - 1.0),
                            egui::pos2(origin.x + width, body_y - 1.0),
                        ],
                        Stroke::new(1.0_f32, BORDER),
                    );
                    if !compact {
                        ui.painter().line_segment(
                            [
                                egui::pos2(origin.x + 248.0, body_y),
                                egui::pos2(origin.x + 248.0, footer_y),
                            ],
                            Stroke::new(1.0_f32, BORDER),
                        );
                    }
                    ui.painter().line_segment(
                        [
                            egui::pos2(origin.x, footer_y),
                            egui::pos2(origin.x + width, footer_y),
                        ],
                        Stroke::new(1.0_f32, BORDER),
                    );
                    let footer_rect = egui::Rect::from_min_size(
                        egui::pos2(origin.x, footer_y),
                        Vec2::new(width, footer_height),
                    );
                    crate::child_ui(ui, egui::UiBuilder::new().max_rect(footer_rect), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.settings_footer(ui, compact);
                    });
                });
        });
        // Keep the outline above the opaque frame and its child surfaces.
        ui.painter().rect_stroke(
            rect,
            22.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
    }

    fn settings_header(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let p = ui.painter();
        p.text(
            egui::pos2(rect.min.x + 32.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Settings",
            egui::FontId::new(24.0, egui::FontFamily::Name("Inter SemiBold".into())),
            TEXT,
        );
        let close = egui::Rect::from_min_size(
            egui::pos2(rect.max.x - 58.0, rect.center().y - 17.0),
            Vec2::splat(34.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(close), |ui| {
            if icon_button(
                ui,
                "close",
                close.size(),
                MUTED,
                Color32::TRANSPARENT,
                Color32::TRANSPARENT,
                true,
            )
            .clicked()
            {
                self.cancel_settings();
            }
        });
    }

    fn settings_sidebar(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        for (index, name) in TABS.iter().enumerate() {
            let tab = egui::Rect::from_min_size(
                rect.min + Vec2::new(20.0, 20.0 + index as f32 * 46.0),
                Vec2::new(208.0, 38.0),
            );
            let selected = self.settings_tab == index;
            if selected {
                p.rect_filled(tab, 9.0, Color32::from_rgb(42, 36, 64));
                p.rect_filled(
                    egui::Rect::from_min_size(tab.min + Vec2::new(12.0, 9.0), Vec2::new(3.0, 20.0)),
                    2.0,
                    ACCENT,
                );
            }
            paint_settings_icon(
                &p,
                egui::Rect::from_min_size(
                    tab.min + Vec2::new(if selected { 25.0 } else { 12.0 }, 11.0),
                    Vec2::splat(16.0),
                ),
                index,
                if selected { ACCENT } else { MUTED },
            );
            p.text(
                tab.left_center() + Vec2::new(if selected { 51.0 } else { 38.0 }, 0.0),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::new(
                    12.5,
                    egui::FontFamily::Name(
                        if selected {
                            "Inter SemiBold"
                        } else {
                            "Inter Medium"
                        }
                        .into(),
                    ),
                ),
                if selected { TEXT } else { SECONDARY },
            );
            if ui
                .interact(
                    tab,
                    egui::Id::new(("settings-tab", index)),
                    egui::Sense::click(),
                )
                .clicked()
            {
                self.settings_tab = index;
            }
        }
    }

    fn settings_content(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        ui.add_space(24.0);
        match self.settings_tab {
            0 => self.performance_settings(ui),
            1 => {
                tab_title(ui, "Audio", "Volume for wallpapers and desktop characters.");
                ui.add_space(20.0);
                let mut refresh_outputs = false;
                {
                    let draft = self
                        .settings_draft
                        .get_or_insert_with(|| self.settings.clone());
                    let mut global_volume = draft["globalVolume"].as_f64().unwrap_or(100.0) as f32;
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Master volume").strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(format!("{}%", global_volume.round() as u32));
                        });
                    });
                    if crate::widgets::slider(
                        ui,
                        &mut global_volume,
                        0.0..=100.0,
                        ui.available_width(),
                    )
                    .changed()
                    {
                        draft["globalVolume"] = json!(global_volume.round());
                    }
                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        section_label(ui, "CHARACTER OUTPUT DEVICE");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            refresh_outputs = ui.small_button("Refresh").clicked()
                        });
                    });
                    let mut output_name =
                        draft["audioOutput"].as_str().unwrap_or_default().to_owned();
                    let selected_text = if output_name.is_empty() {
                        "System default".to_owned()
                    } else {
                        self.audio_outputs
                            .iter()
                            .find(|output| output.name == output_name)
                            .map(|output| output.description.clone())
                            .unwrap_or_else(|| format!("Unavailable · {output_name}"))
                    };
                    egui::ComboBox::from_id_salt("settings-audio-output")
                        .icon(crate::widgets::combo_chevron)
                        .selected_text(selected_text)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut output_name, String::new(), "System default");
                            for output in &self.audio_outputs {
                                ui.selectable_value(
                                    &mut output_name,
                                    output.name.clone(),
                                    &output.description,
                                );
                            }
                        });
                    if output_name != draft["audioOutput"].as_str().unwrap_or_default() {
                        draft["audioOutput"] = json!(output_name);
                    }
                    let output_available = output_name.is_empty()
                        || self
                            .audio_outputs
                            .iter()
                            .any(|output| output.name == output_name);
                    if self.audio_outputs_loading {
                        ui.label(
                            RichText::new("Finding audio outputs…")
                                .size(11.0)
                                .color(MUTED),
                        );
                    } else if let Some(error) = &self.audio_outputs_error {
                        ui.label(
                            RichText::new(format!("Could not list audio outputs: {error}"))
                                .size(10.0)
                                .color(MUTED),
                        );
                    } else if !output_available {
                        ui.label(
                            RichText::new("The saved output is unavailable; character audio uses the system default until you choose another output.")
                                .size(10.0)
                                .color(MUTED),
                        );
                    }
                    ui.add_space(14.0);
                    ui.separator();
                    ui.add_space(10.0);
                    section_label(ui, "WALLPAPER AUDIO");
                    ui.label(
                        RichText::new("Video wallpapers play through the system default output.")
                            .size(11.0)
                            .color(MUTED),
                    );
                }
                if refresh_outputs {
                    self.refresh_audio_outputs(ui.ctx(), true);
                }
            }
            2 => self.general_settings(ui),
            3 => self.library_settings(ui),
            _ => self.about_settings(ui),
        }
    }

    fn performance_settings(&mut self, ui: &mut egui::Ui) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 424.0), egui::Sense::hover());
        let p = ui.painter().clone();
        // Every row position derives from these pitches.
        const TOGGLE_PITCH: f32 = 48.0;
        const CONTROL_PITCH: f32 = 76.0;
        let y_fullscreen = 90.0;
        let y_maximized = y_fullscreen + TOGGLE_PITCH;
        let y_separator = y_maximized + 58.0;
        let y_fps = y_separator + 16.0;
        let y_msaa = y_fps + CONTROL_PITCH;
        let y_quality = y_msaa + CONTROL_PITCH;
        // Descriptions stop before the right-aligned controls (120px wide + gap).
        let text_width = (rect.width() - 140.0).max(60.0);
        let title_font = egui::FontId::new(13.0, egui::FontFamily::Name("Inter".into()));
        let help_font = egui::FontId::new(10.5, egui::FontFamily::Name("Inter".into()));
        let row_text = |y: f32, title: &str, help: &str| {
            p.text(
                rect.min + Vec2::new(0.0, y),
                egui::Align2::LEFT_TOP,
                crate::widgets::ellipsize(&p, title, &title_font, text_width),
                title_font.clone(),
                TEXT,
            );
            p.text(
                rect.min + Vec2::new(0.0, y + 20.0),
                egui::Align2::LEFT_TOP,
                crate::widgets::ellipsize(&p, help, &help_font, text_width),
                help_font.clone(),
                MUTED,
            );
        };
        let rule = |y: f32| {
            p.line_segment(
                [
                    rect.min + Vec2::new(0.0, y),
                    rect.min + Vec2::new(rect.width(), y),
                ],
                Stroke::new(1.0_f32, BORDER),
            );
        };
        paint_tab_title(
            &p,
            rect.min,
            "Performance",
            "Balance motion quality with desktop responsiveness.",
        );
        paint_section_label(&p, rect.min + Vec2::new(0.0, 68.0), "PLAYBACK BEHAVIOR");
        let draft = self
            .settings_draft
            .get_or_insert_with(|| self.settings.clone());
        let mut fullscreen = draft["pauseFullscreen"].as_bool().unwrap_or(true);
        let mut maximized = draft["pauseMaximized"].as_bool().unwrap_or(false);
        let mut fps = draft["frameRateLimit"]
            .as_u64()
            .unwrap_or(60)
            .clamp(15, 240);
        let mut msaa = draft["msaaSamples"].as_u64().unwrap_or(4).min(8) as u16;
        let mut quality = draft["previewQuality"]
            .as_str()
            .unwrap_or("High")
            .to_owned();
        let width = rect.width();
        for (y, label, help, value) in [
            (
                y_fullscreen,
                "Pause when an application is fullscreen",
                "Pauses wallpapers and characters while a fullscreen app is in focus.",
                &mut fullscreen,
            ),
            (
                y_maximized,
                "Pause when an application is maximized",
                "Pauses wallpapers and characters while a maximized window is in focus.",
                &mut maximized,
            ),
        ] {
            row_text(y, label, help);
            let r = egui::Rect::from_min_size(
                rect.min + Vec2::new(width - 36.0, y + 6.0),
                Vec2::new(36.0, 20.0),
            );
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(r), |ui| {
                crate::toggle_switch(ui, value, !self.busy);
            });
        }
        rule(y_maximized - 12.0);
        rule(y_separator);
        rule(y_msaa - 16.0);
        rule(y_quality - 16.0);
        row_text(
            y_fps,
            "Frame rate limit",
            "Maximum refresh rate for this window and video wallpapers.",
        );
        let stepper = egui::Rect::from_min_size(
            rect.min + Vec2::new(width - 120.0, y_fps + 2.0),
            Vec2::new(120.0, 36.0),
        );
        p.rect_stroke(
            stepper,
            8.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        p.text(
            stepper.center(),
            egui::Align2::CENTER_CENTER,
            format!("{fps} Hz"),
            egui::FontId::new(11.0, egui::FontFamily::Name("Inter SemiBold".into())),
            TEXT,
        );
        for (i, icon) in ["minus", "plus"].iter().enumerate() {
            let button = egui::Rect::from_min_size(
                stepper.min + Vec2::new(if i == 0 { 4.0 } else { 92.0 }, 6.0),
                Vec2::splat(24.0),
            );
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(button), |ui| {
                if icon_button(
                    ui,
                    icon,
                    button.size(),
                    MUTED,
                    Color32::TRANSPARENT,
                    Color32::TRANSPARENT,
                    !self.busy,
                )
                .clicked()
                {
                    fps = if i == 0 {
                        fps.saturating_sub(15).max(15)
                    } else {
                        (fps + 15).min(240)
                    };
                }
            });
        }
        row_text(
            y_msaa,
            "Edge antialiasing (MSAA)",
            "Smooths transformed edges. Applies the next time WaniEngine starts.",
        );
        let select = egui::Rect::from_min_size(
            rect.min + Vec2::new(width - 120.0, y_msaa + 2.0),
            Vec2::new(120.0, 36.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(select), |ui| {
            ui.spacing_mut().interact_size.y = 36.0;
            egui::ComboBox::from_id_salt("settings-msaa")
                .icon(crate::widgets::combo_chevron)
                .width(120.0)
                .selected_text(if msaa == 0 {
                    "Off".to_owned()
                } else {
                    format!("{msaa}×")
                })
                .show_ui(ui, |ui| {
                    for (v, label) in [(0, "Off"), (2, "2×"), (4, "4×"), (8, "8×")] {
                        ui.selectable_value(&mut msaa, v, label);
                    }
                });
        });
        row_text(
            y_quality,
            "Preview quality",
            "Resolution of thumbnails and animated previews.",
        );
        let select = select.translate(Vec2::new(0.0, y_quality - y_msaa));
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(select), |ui| {
            crate::preview::record_geometry(ui, "Preview quality select", select);
            ui.spacing_mut().interact_size.y = 36.0;
            egui::ComboBox::from_id_salt("settings-preview-quality")
                .icon(crate::widgets::combo_chevron)
                .width(120.0)
                .selected_text(&quality)
                .show_ui(ui, |ui| {
                    for level in ["Low", "Medium", "High"] {
                        ui.selectable_value(&mut quality, level.into(), level);
                    }
                });
        });
        let draft = self
            .settings_draft
            .get_or_insert_with(|| self.settings.clone());
        draft["pauseFullscreen"] = json!(fullscreen);
        draft["pauseMaximized"] = json!(maximized);
        draft["frameRateLimit"] = json!(fps);
        draft["msaaSamples"] = json!(msaa);
        draft["previewQuality"] = json!(quality);
        self.thumbnails.set_quality(match quality.as_str() {
            "Low" => 240,
            "Medium" => 360,
            _ => 480,
        });
    }

    fn general_settings(&mut self, ui: &mut egui::Ui) {
        tab_title(ui, "General", "Choose how WaniEngine starts and closes.");
        ui.add_space(20.0);
        section_label(ui, "STARTUP");
        ui.add_space(6.0);
        let mut start_with_session = self
            .settings_draft
            .as_ref()
            .and_then(|settings| settings["startWithSession"].as_bool())
            .unwrap_or(self.start_with_session);
        let mut start_changed = false;
        ui.add_enabled_ui(!self.busy, |ui| {
            start_changed = setting_toggle_row(
                ui,
                "Start in the tray when signing in",
                "Adds or removes the WaniEngine entry from your XDG session autostart. If this desktop has no tray, the window opens normally.",
                &mut start_with_session,
            );
        });
        if start_changed {
            if let Some(draft) = &mut self.settings_draft {
                draft["startWithSession"] = json!(start_with_session);
            }
        }
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(10.0);
        section_label(ui, "WINDOW LIFECYCLE");
        ui.add_space(6.0);
        let mut close_to_tray = self
            .settings_draft
            .as_ref()
            .and_then(|settings| settings["closeToTray"].as_bool())
            .unwrap_or(true);
        let tray_available = self.tray.as_ref().is_some_and(|tray| tray.is_running());
        let mut changed = false;
        ui.add_enabled_ui(!self.busy && tray_available, |ui| {
            changed = setting_toggle_row(
                ui,
                "Keep running in the tray when the window closes",
                "Use the WaniEngine tray icon to reopen the window or exit.",
                &mut close_to_tray,
            );
        });
        if changed {
            self.settings_draft
                .get_or_insert_with(|| self.settings.clone())["closeToTray"] = json!(close_to_tray);
        }
        if tray_available {
            if ui
                .add(egui::Button::new(
                    RichText::new("Exit WaniEngine").color(Color32::from_rgb(251, 113, 133)),
                ))
                .clicked()
            {
                self.force_exit = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else {
            info_panel(
                ui,
                "This desktop has no StatusNotifier tray host. Closing the window exits WaniEngine.",
            );
        }
    }

    fn library_settings(&mut self, ui: &mut egui::Ui) {
        tab_title(
            ui,
            "Library",
            "Manage where your wallpapers and characters are stored.",
        );
        ui.add_space(20.0);
        section_label(ui, "LIBRARY FOLDER");
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(NAV)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .corner_radius(8)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(self.location.root.display().to_string());
            });
        ui.add_space(8.0);
        let library_root = self.location.root.clone();
        if ui.button("Open library folder").clicked() {
            self.open_folder(&library_root);
        }
        ui.add_space(16.0);
        info_panel(
            ui,
            "Importing never moves or changes your original files, and removing an item from the library never deletes them.",
        );
    }

    fn about_settings(&mut self, ui: &mut egui::Ui) {
        tab_title(ui, "About WaniEngine", "");
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            if let Some(logo) = &self.logo {
                ui.add(egui::Image::new(logo).fit_to_exact_size(Vec2::splat(48.0)));
            }
            ui.label(
                RichText::new(format!("WaniEngine {}", env!("CARGO_PKG_VERSION")))
                    .strong()
                    .size(18.0),
            );
        });
        ui.add_space(14.0);
        ui.label(
            RichText::new("Open-source desktop customization app for animated wallpapers and interactive desktop characters. It brings a local media library, playback controls and native desktop integration together in one app.")
                .color(SECONDARY),
        );
    }

    fn settings_footer(&mut self, ui: &mut egui::Ui, compact: bool) {
        let has_changes = self.settings_dirty();
        let context = ui.ctx().clone();
        let mut reset = false;
        let mut cancel = false;
        let mut save = false;
        if compact {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                reset = footer_reset_button(ui, !self.busy);
            });
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(16.0);
                    save = footer_save_button(ui, !self.busy && has_changes);
                    cancel = footer_cancel_button(ui, !self.busy);
                });
            });
            ui.add_space(5.0);
        } else {
            let origin = ui.max_rect().min;
            let width = ui.available_width();
            let reset_rect =
                egui::Rect::from_min_size(origin + Vec2::new(20.0, 12.0), Vec2::new(148.0, 38.0));
            let cancel_rect = egui::Rect::from_min_size(
                origin + Vec2::new(width - 280.0, 12.0),
                Vec2::new(92.0, 38.0),
            );
            let save_rect = egui::Rect::from_min_size(
                origin + Vec2::new(width - 176.0, 12.0),
                Vec2::new(144.0, 38.0),
            );
            reset = crate::child_ui(ui, egui::UiBuilder::new().max_rect(reset_rect), |ui| {
                footer_reset_button(ui, !self.busy)
            })
            .inner;
            cancel = crate::child_ui(ui, egui::UiBuilder::new().max_rect(cancel_rect), |ui| {
                footer_cancel_button(ui, !self.busy)
            })
            .inner;
            save = crate::child_ui(ui, egui::UiBuilder::new().max_rect(save_rect), |ui| {
                footer_save_button(ui, !self.busy && has_changes)
            })
            .inner;
        }
        if reset {
            self.reset_settings_draft();
        }
        if cancel {
            self.cancel_settings();
        } else if save {
            self.save_settings(&context);
        }
    }

    fn reset_settings_draft(&mut self) {
        if let Some(draft) = &mut self.settings_draft {
            draft["previewQuality"] = json!("High");
            draft["globalVolume"] = json!(100);
            draft["audioOutput"] = json!("");
            draft["frameRateLimit"] = json!(60);
            draft["pauseFullscreen"] = json!(true);
            draft["pauseMaximized"] = json!(false);
            draft["startWithSession"] = json!(false);
            draft["closeToTray"] = json!(true);
            draft["msaaSamples"] = json!(4);
        }
        self.thumbnails.set_quality(480);
    }
}

fn setting_toggle_row(ui: &mut egui::Ui, title: &str, description: &str, value: &mut bool) -> bool {
    let before = *value;
    let width = ui.available_width();
    let text_width = (width - 56.0).max(40.0);
    let enabled = ui.is_enabled();
    let title_color = if enabled { TEXT } else { MUTED };
    let title_galley = ui.painter().layout(
        title.to_owned(),
        egui::FontId::new(13.0, egui::FontFamily::Name("Inter".into())),
        title_color,
        text_width,
    );
    let description_galley = ui.painter().layout(
        description.to_owned(),
        egui::FontId::new(10.5, egui::FontFamily::Name("Inter".into())),
        MUTED,
        text_width,
    );
    let height = (title_galley.size().y + 3.0 + description_galley.size().y).max(24.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
    let description_pos = rect.min + Vec2::new(0.0, title_galley.size().y + 3.0);
    ui.painter().galley(rect.min, title_galley, title_color);
    ui.painter()
        .galley(description_pos, description_galley, MUTED);
    let switch = egui::Rect::from_min_size(
        egui::pos2(rect.right() - 36.0, rect.center().y - 10.0),
        Vec2::new(36.0, 20.0),
    );
    crate::child_ui(ui, egui::UiBuilder::new().max_rect(switch), |ui| {
        crate::toggle_switch(ui, value, true);
    });
    *value != before
}

fn footer_reset_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    icon_text_button(
        ui,
        "refresh",
        "Reset to defaults",
        Vec2::new(148.0, 38.0),
        enabled,
        Color32::TRANSPARENT,
        MUTED,
        Color32::TRANSPARENT,
    )
    .clicked()
}

fn footer_cancel_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(92.0, 38.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    ui.painter().rect_stroke(
        rect,
        9.0,
        Stroke::new(1.0_f32, if hovered { MUTED } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Cancel",
        tab_font(false, 12.0),
        if enabled { SECONDARY } else { MUTED },
    );
    enabled && response.clicked()
}

fn footer_save_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    icon_text_button(
        ui,
        "check",
        "Save changes",
        Vec2::new(144.0, 38.0),
        enabled,
        ACCENT,
        SURFACE,
        Color32::TRANSPARENT,
    )
    .clicked()
}

fn tab_font(bold: bool, size: f32) -> egui::FontId {
    egui::FontId::new(
        size,
        egui::FontFamily::Name(if bold { "Inter SemiBold" } else { "Inter" }.into()),
    )
}

/// Tab heading: Inter SemiBold 20 over an Inter 11.5 subtitle (48px tall).
fn paint_tab_title(p: &egui::Painter, pos: egui::Pos2, title: &str, subtitle: &str) {
    p.text(
        pos,
        egui::Align2::LEFT_TOP,
        title,
        tab_font(true, 20.0),
        TEXT,
    );
    p.text(
        pos + Vec2::new(0.0, 28.0),
        egui::Align2::LEFT_TOP,
        subtitle,
        tab_font(false, 11.5),
        MUTED,
    );
}

fn tab_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), egui::Sense::hover());
    paint_tab_title(ui.painter(), rect.min, title, subtitle);
}

/// Small tracked uppercase section label (9px, 8% tracking).
fn paint_section_label(p: &egui::Painter, pos: egui::Pos2, text: &str) {
    crate::widgets::tracked_text(
        p,
        pos,
        egui::Align2::LEFT_TOP,
        text,
        tab_font(false, 9.0),
        MUTED,
        8.0,
    );
}

fn section_label(ui: &mut egui::Ui, text: &str) {
    let natural = ui
        .painter()
        .layout_no_wrap(text.to_owned(), tab_font(false, 9.0), MUTED)
        .size()
        .x;
    let width = natural + text.chars().count() as f32 * 0.72;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 14.0), egui::Sense::hover());
    paint_section_label(ui.painter(), rect.min, text);
}

impl App {
    fn settings_dirty(&self) -> bool {
        let Some(draft) = &self.settings_draft else {
            return false;
        };
        draft["previewQuality"].as_str().unwrap_or("High")
            != self.settings["previewQuality"].as_str().unwrap_or("High")
            || draft["globalVolume"].as_f64().unwrap_or(100.0)
                != self.settings["globalVolume"].as_f64().unwrap_or(100.0)
            || draft["audioOutput"].as_str().unwrap_or_default()
                != self.settings["audioOutput"].as_str().unwrap_or_default()
            || draft["frameRateLimit"].as_u64().unwrap_or(60)
                != self.settings["frameRateLimit"].as_u64().unwrap_or(60)
            || draft["pauseFullscreen"].as_bool().unwrap_or(true)
                != self.settings["pauseFullscreen"].as_bool().unwrap_or(true)
            || draft["pauseMaximized"].as_bool().unwrap_or(false)
                != self.settings["pauseMaximized"].as_bool().unwrap_or(false)
            || draft["startWithSession"]
                .as_bool()
                .unwrap_or(self.start_with_session)
                != self.start_with_session
            || draft["closeToTray"].as_bool().unwrap_or(true)
                != self.settings["closeToTray"].as_bool().unwrap_or(true)
            || draft["msaaSamples"].as_u64().unwrap_or(4)
                != self.settings["msaaSamples"].as_u64().unwrap_or(4)
    }
}

fn info_panel(ui: &mut egui::Ui, message: &str) {
    egui::Frame::new()
        .fill(NAV)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(10)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
                paint_action_icon(ui.painter(), icon_rect.center(), "info", ACCENT);
                ui.add(egui::Label::new(RichText::new(message).size(11.0).color(MUTED)).wrap());
            });
        });
}

fn paint_settings_icon(painter: &egui::Painter, rect: egui::Rect, index: usize, color: Color32) {
    crate::icons::paint_node(
        painter,
        rect.center(),
        match index {
            0 => 774,
            1 => 777,
            2 => 780,
            3 => 783,
            _ => 786,
        },
        color,
        None,
    );
}
