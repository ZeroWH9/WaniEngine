//! Character selection and live control for the experimental Wayland overlay.
use crate::{
    ACCENT, App, BORDER, MUTED, PANEL, SECONDARY, TEXT, icon_button, icon_link_with_metrics,
    icon_text_button, paint_action_icon, paint_cover_rounded, paint_speaker_icon,
    record_volume_response, search_field, toggle_switch, volume_draft_value,
};
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde_json::{Value, json};
use waniengine_core::library::file_path;
use waniengine_egui::overlay::{ActorSettings, WorkerStatus};

fn asset_number(asset: &Value, key: &str, default: f32, min: f32, max: f32) -> f32 {
    asset[key]
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| (value as f32).clamp(min, max))
        .unwrap_or(default)
}

fn is_animated_character(asset: &Value) -> bool {
    asset["video"].as_bool() != Some(true)
        && (asset["animated"].as_bool() == Some(true)
            || asset["extension"].as_str().is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "gif" | "apng" | "webp"
                )
            }))
}

pub(crate) fn initial_settings(asset: &Value) -> ActorSettings {
    ActorSettings {
        x: asset_number(asset, "posX", 50.0, 0.0, 100.0) / 100.0,
        y: asset_number(asset, "posY", 96.0, 0.0, 100.0) / 100.0,
        scale: asset_number(asset, "scale", 100.0, 10.0, 300.0),
        opacity: asset_number(asset, "opacity", 100.0, 0.0, 100.0),
        rotation: asset_number(asset, "rotation", 0.0, -180.0, 180.0),
        flip_x: asset["flipX"].as_bool().unwrap_or(false),
        smoothing: asset["smoothing"].as_bool().unwrap_or(true),
        speed: asset_number(asset, "speed", 100.0, 10.0, 300.0),
        playing: asset["playing"].as_bool().unwrap_or(true),
        looping: asset["loop"].as_bool().unwrap_or(true),
    }
}
fn panel() -> egui::Frame {
    egui::Frame::default()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(14)
        .inner_margin(16)
}

#[derive(Clone, Copy)]
enum ResetGroup {
    Transform,
    Appearance,
    Playback,
}

fn reset_actor_group(settings: &mut ActorSettings, group: ResetGroup) {
    let defaults = ActorSettings::default();
    match group {
        ResetGroup::Transform => {
            settings.scale = defaults.scale;
            settings.x = defaults.x;
            settings.y = defaults.y;
        }
        ResetGroup::Appearance => {
            settings.opacity = defaults.opacity;
            settings.rotation = defaults.rotation;
            settings.flip_x = defaults.flip_x;
            settings.smoothing = defaults.smoothing;
        }
        ResetGroup::Playback => {
            settings.playing = defaults.playing;
            settings.looping = defaults.looping;
            settings.speed = defaults.speed;
        }
    }
}

fn reset_audio_controls(audio: &mut Value, volume: &mut f32) -> bool {
    let changed = audio["enabled"].as_bool().unwrap_or(false) || *volume != 50.0;
    audio["enabled"] = json!(false);
    *volume = 50.0;
    changed
}

fn control_panel(
    ui: &mut egui::Ui,
    title: &str,
    enabled: bool,
    note: Option<&str>,
) -> (egui::Rect, bool) {
    const NOTE_Y: f32 = 174.0;
    let note_galley = note.map(|note| {
        let wrap = (ui.available_width() - 32.0).max(40.0);
        ui.painter().layout(
            note.to_owned(),
            egui::FontId::proportional(10.0),
            MUTED,
            wrap,
        )
    });
    let height = note_galley
        .as_ref()
        .map_or(216.0, |galley| (NOTE_Y + galley.size().y + 14.0).max(216.0));
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), height),
        egui::Sense::hover(),
    );
    crate::preview::record_geometry(
        ui,
        if title == "Playback and sounds" {
            "Playback Panel".to_owned()
        } else {
            format!("{title} Panel")
        },
        rect,
    );
    ui.painter().rect_filled(rect, 14.0, PANEL);
    ui.painter().rect_stroke(
        rect,
        14.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.min + Vec2::new(16.0, 13.0),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::new(13.5, egui::FontFamily::Name("Inter SemiBold".into())),
        TEXT,
    );
    if let Some(galley) = note_galley {
        ui.painter().line_segment(
            [
                rect.min + Vec2::new(16.0, NOTE_Y - 12.0),
                rect.min + Vec2::new(rect.width() - 16.0, NOTE_Y - 12.0),
            ],
            Stroke::new(1.0_f32, BORDER),
        );
        ui.painter()
            .galley(rect.min + Vec2::new(16.0, NOTE_Y), galley, MUTED);
    }
    let reset = egui::Rect::from_min_size(
        rect.right_top() + Vec2::new(-56.0, 6.0),
        Vec2::new(44.0, 28.0),
    );
    ui.painter().text(
        reset.center(),
        egui::Align2::CENTER_CENTER,
        "Reset",
        egui::FontId::proportional(10.0),
        if enabled { ACCENT } else { MUTED },
    );
    let response = ui.interact(
        reset,
        ui.id().with(("panel-reset", title)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let response = response.on_hover_text(if enabled {
        format!("Reset {title} to defaults")
    } else {
        "Import this character to change its settings.".to_owned()
    });
    (rect.shrink2(Vec2::new(16.0, 0.0)), response.clicked())
}

fn panel_slider(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    y: f32,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    suffix: &str,
    enabled: bool,
) -> egui::Response {
    let r = egui::Rect::from_min_size(rect.min + Vec2::new(0.0, y), Vec2::new(rect.width(), 20.0));
    crate::child_ui(ui, egui::UiBuilder::new().max_rect(r), |ui| {
        labeled_slider(ui, value, range, label, suffix, enabled)
    })
    .inner
}

fn panel_note(ui: &egui::Ui, rect: egui::Rect, y: f32, note: &str) {
    ui.painter().line_segment(
        [
            rect.min + Vec2::new(0.0, y - 12.0),
            rect.min + Vec2::new(rect.width(), y - 12.0),
        ],
        Stroke::new(1.0_f32, BORDER),
    );
    ui.painter().text(
        rect.min + Vec2::new(0.0, y),
        egui::Align2::LEFT_TOP,
        note,
        egui::FontId::proportional(9.0),
        MUTED,
    );
}

#[derive(Clone, Copy)]
enum AudioCardAction {
    Add,
    Configure,
    Save,
}

pub(crate) fn audio_defaults(asset: &Value) -> Value {
    let mut audio = asset["audio"].clone();
    if !audio.is_object() {
        audio = json!({"enabled":false,"files":[]});
    }
    if audio["enabled"].as_bool().is_none() {
        audio["enabled"] = json!(false);
    }
    if !audio["files"].is_array() {
        audio["files"] = json!([]);
    }
    for track in audio["files"].as_array_mut().into_iter().flatten() {
        if !track["playback"].is_object() {
            track["playback"] = json!({});
        }
        let playback = &mut track["playback"];
        for (key, value) in [
            ("enabled", json!(true)),
            ("mode", json!("once")),
            ("volume", json!(100)),
            ("intervalSeconds", json!(30)),
            ("radiusPixels", json!(40)),
            ("cooldownSeconds", json!(2)),
            ("startImmediately", json!(true)),
        ] {
            if playback[key].is_null() {
                playback[key] = value;
            }
        }
    }
    audio
}

fn labeled_switch(ui: &mut egui::Ui, value: &mut bool, label: &str, enabled: bool) -> bool {
    let enabled = enabled && ui.is_enabled();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let response = toggle_switch(ui, value, enabled);
        let label_response = ui.add(
            egui::Label::new(RichText::new(label).size(11.0).color(if enabled {
                SECONDARY
            } else {
                MUTED
            }))
            .sense(if enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            }),
        );
        if enabled && label_response.clicked() {
            *value = !*value;
        }
        response.changed() || (enabled && label_response.clicked())
    })
    .inner
}

fn labeled_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    label: &str,
    suffix: &str,
    enabled: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.add_sized(
            [72.0, 20.0],
            egui::Label::new(RichText::new(label).size(11.0).color(MUTED)),
        );
        let slider_width = (ui.available_width() - 44.0).max(48.0);
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                let r = crate::widgets::slider(ui, value, range.clone(), slider_width);
                if r.changed() {
                    *value = value.clamp(*range.start(), *range.end());
                }
                r
            })
            .inner;
        let number = ui.allocate_ui_with_layout(
            Vec2::new(36.0, 20.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().interact_size = Vec2::new(0.0, 20.0);
                ui.spacing_mut().button_padding = Vec2::ZERO;
                ui.style_mut().override_font_id = Some(egui::FontId::proportional(10.5));
                ui.style_mut()
                    .text_styles
                    .insert(egui::TextStyle::Body, egui::FontId::proportional(10.5));
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
                ui.add_enabled(
                    enabled,
                    egui::DragValue::new(value)
                        .range(range)
                        .speed(1.0)
                        .suffix(suffix)
                        .max_decimals(0),
                )
            },
        );
        response.union(number.inner)
    })
    .inner
}

fn draw_audio_controls(
    ui: &mut egui::Ui,
    key: &str,
    audio: &mut Value,
    selected_track: &mut usize,
    settings: &mut ActorSettings,
    actor_volume: &mut f32,
    volume_draft: &mut Option<(String, f32)>,
    animated: bool,
    action: &mut Option<AudioCardAction>,
    interactive: bool,
    busy: bool,
) {
    let origin = ui.max_rect().min;
    let width = ui.available_width();
    let track_count = audio["files"].as_array().map_or(0, Vec::len);
    let mut sounds_enabled = audio["enabled"].as_bool().unwrap_or(false);
    let row = egui::Rect::from_min_size(origin, Vec2::new(width - 128.0, 20.0));
    crate::child_ui(ui, egui::UiBuilder::new().max_rect(row), |ui| {
        if labeled_switch(
            ui,
            &mut sounds_enabled,
            "Enable sounds",
            interactive && !busy && track_count > 0,
        ) {
            audio["enabled"] = json!(sounds_enabled);
            *action = Some(AudioCardAction::Save);
        }
    });
    let add = egui::Rect::from_min_size(
        origin + Vec2::new(width - 120.0, -4.0),
        Vec2::new(120.0, 26.0),
    );
    crate::child_ui(ui, egui::UiBuilder::new().max_rect(add), |ui| {
        if ui
            .add_enabled(
                interactive && !busy,
                egui::Button::new(RichText::new("Add sound").size(9.5).color(SECONDARY))
                    .min_size(add.size()),
            )
            .clicked()
        {
            *action = Some(AudioCardAction::Add);
        }
    });
    if track_count == 0 {
        ui.painter().text(
            origin + Vec2::new(0.0, 34.0),
            egui::Align2::LEFT_TOP,
            "Add an audio track to configure playback.",
            egui::FontId::proportional(10.0),
            MUTED,
        );
    } else {
        *selected_track = (*selected_track).min(track_count - 1);
        let tracks = audio["files"].as_array().cloned().unwrap_or_default();
        let track = &tracks[*selected_track];
        let mode_label = |track: &Value| match track["playback"]["mode"].as_str().unwrap_or("once")
        {
            "loop" => "soft loop",
            "interval" => "at intervals",
            "click" => "on character click",
            _ => "when activated",
        };
        let select = egui::Rect::from_min_size(
            origin + Vec2::new(0.0, 30.0),
            Vec2::new((width - 120.0).max(100.0), 28.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(select), |ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            egui::ComboBox::from_id_salt(("character-audio-track", key))
                .icon(crate::widgets::combo_chevron)
                .width(select.width())
                .selected_text("")
                .show_ui(ui, |ui| {
                    ui.set_min_width(select.width().max(220.0));
                    for (index, track) in tracks.iter().enumerate() {
                        ui.selectable_value(
                            selected_track,
                            index,
                            format!(
                                "{} · {}",
                                track["name"].as_str().unwrap_or("Audio track"),
                                mode_label(track)
                            ),
                        );
                    }
                });
            crate::icons::paint_node(
                ui.painter(),
                select.left_center() + Vec2::new(12.0, 0.0),
                1356,
                MUTED,
                Some(Vec2::splat(12.0)),
            );
            ui.painter().text(
                select.left_center() + Vec2::new(24.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!(
                    "{} · {}",
                    track["name"].as_str().unwrap_or("Audio track"),
                    mode_label(track)
                ),
                egui::FontId::proportional(9.5),
                SECONDARY,
            );
        });
        let configure = egui::Rect::from_min_size(
            origin + Vec2::new(width - 112.0, 30.0),
            Vec2::new(112.0, 28.0),
        );
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(configure), |ui| {
            if ui
                .add_enabled(
                    interactive && !busy,
                    egui::Button::new(RichText::new("Configure").size(10.0).color(SECONDARY))
                        .min_size(configure.size()),
                )
                .clicked()
            {
                *action = Some(AudioCardAction::Configure);
            }
        });
    }
    let rect = egui::Rect::from_min_size(origin, Vec2::new(width, 142.0));
    if animated {
        panel_slider(
            ui,
            rect,
            60.0,
            &mut settings.speed,
            10.0..=300.0,
            "Speed",
            "%",
            interactive && !busy,
        );
    }
    let response = panel_slider(
        ui,
        rect,
        90.0,
        actor_volume,
        0.0..=100.0,
        "Volume",
        "%",
        interactive && !busy && sounds_enabled,
    );
    if record_volume_response(volume_draft, key, *actor_volume, &response) {
        *action = Some(AudioCardAction::Save);
    }
    panel_note(
        ui,
        rect,
        128.0,
        "Character audio uses the system default output.",
    );
}

impl App {
    pub(crate) fn toggle_character(&mut self, asset: &Value, ctx: &egui::Context) {
        if self.preview_action(crate::preview::UiAction::ToggleActor {
            url: asset["url"].as_str().unwrap_or_default().into(),
        }) {
            return;
        }
        if self.busy {
            return;
        }
        let Some(url) = asset["url"].as_str().map(str::to_owned) else {
            self.status = Some("This character has no local file.".into());
            return;
        };
        let active = self
            .overlay
            .as_ref()
            .and_then(|overlay| overlay.worker(&url))
            .is_some();
        if active {
            let result = self
                .overlay
                .as_mut()
                .ok_or_else(|| "Character overlay is unavailable".to_owned())
                .and_then(|overlay| overlay.stop(&url));
            match result {
                Ok(()) => {
                    self.pause_saved_playing.remove(&url);
                    let mut urls = self.saved_actor_urls.clone();
                    urls.retain(|saved| saved != &url);
                    self.persist_active_actors(urls, ctx);
                }
                Err(error) => self.status = Some(error),
            }
            return;
        }
        let is_video = asset["video"].as_bool() == Some(true);
        let is_svg = asset["extension"]
            .as_str()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"));
        let supported = !is_video && (!is_svg || waniengine_wayland::svg_renderer_available());
        if !supported {
            self.status = Some(if is_video {
                "Video characters are not supported yet.".into()
            } else {
                "SVG characters need rsvg-convert (librsvg).".into()
            });
            return;
        }
        if asset["missing"].as_bool() == Some(true) {
            self.status = Some("This character file is unavailable.".into());
            return;
        }
        let Some(monitor) = self
            .selected_output
            .as_deref()
            .filter(|selected| {
                self.overlay.as_ref().is_some_and(|overlay| {
                    overlay
                        .monitors
                        .iter()
                        .any(|monitor| monitor.name.as_deref() == Some(*selected))
                })
            })
            .map(str::to_owned)
            .or_else(|| {
                self.overlay.as_ref().and_then(|overlay| {
                    overlay
                        .monitors
                        .iter()
                        .find_map(|monitor| monitor.name.clone())
                })
            })
        else {
            if let Some(overlay) = &mut self.overlay {
                overlay.refresh_monitors();
            }
            self.status = Some("Connect or refresh a display before enabling a character.".into());
            return;
        };
        self.selected_output = Some(monitor.clone());
        let settings = self
            .actor_drafts
            .get(&url)
            .copied()
            .unwrap_or_else(|| initial_settings(asset));
        let result = file_path(&url)
            .map_err(|error| error.to_string())
            .and_then(|path| self.start_character_overlay(&url, &path, &monitor, settings));
        match result {
            Ok(()) => {
                let mut urls = self.saved_actor_urls.clone();
                if !urls.iter().any(|saved| saved == &url) {
                    urls.push(url);
                }
                self.persist_active_actors(urls, ctx);
            }
            Err(error) => self.status = Some(error),
        }
    }

    pub(crate) fn animations_ui(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let header_width = ui.available_width();
            let add_width = 164.0_f32.min(header_width.max(0.0));
            let two_rows = header_width < 280.0 + 24.0 + add_width + 8.0 + 180.0;
            let (header, _) = ui.allocate_exact_size(
                Vec2::new(header_width, if two_rows { 140.0 } else { 84.0 }),
                egui::Sense::hover(),
            );
            crate::widgets::tracked_text(
                ui.painter(),
                header.min + Vec2::new(0.0, 20.0),
                egui::Align2::LEFT_TOP,
                "Animations",
                egui::FontId::new(34.0, egui::FontFamily::Name("Inter SemiBold".into())),
                TEXT,
                -1.5,
            );
            ui.painter().text(
                header.min + Vec2::new(0.0, 68.0),
                egui::Align2::LEFT_TOP,
                "Characters that live above your windows.",
                egui::FontId::proportional(13.0),
                MUTED,
            );
            let (add_top, search_top) = if two_rows {
                (header.top() + 90.0, header.top() + 92.0)
            } else {
                (header.top() + 26.0, header.top() + 28.0)
            };
            let add = egui::Rect::from_min_size(
                egui::pos2(header.right() - add_width, add_top),
                Vec2::new(add_width, 44.0),
            );
            let search_width = if two_rows {
                (header_width - add_width - 8.0).clamp(0.0, 320.0)
            } else {
                (header_width - 280.0 - 24.0 - add_width - 8.0).clamp(180.0, 320.0)
            };
            let search = egui::Rect::from_min_size(
                egui::pos2((add.left() - 8.0 - search_width).max(header.left()), search_top),
                Vec2::new(search_width, 40.0),
            );
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(search), |ui| {
                search_field(
                    ui,
                    &mut self.actor_search,
                    search_width,
                    "Search characters…",
                    None,
                );
            });
            crate::child_ui(ui, egui::UiBuilder::new().max_rect(add), |ui| {
                if icon_text_button(
                    ui,
                    "plus",
                    "Add animation",
                    add.size(),
                    !self.busy,
                    ACCENT,
                    Color32::from_rgb(12, 10, 17),
                    Color32::TRANSPARENT,
                )
                .clicked()
                {
                    self.import_kind = crate::ImportKind::Character;
                    self.choose_files(ui.ctx());
                }
            });
            ui.add_space(16.0);

            let characters: Vec<Value> = self
                .assets
                .iter()
                .filter(|asset| {
                    asset["kind"].as_str() == Some("character") && asset["url"].is_string()
                })
                .cloned()
                .collect();
            if !characters
                .iter()
                .any(|asset| asset["url"].as_str() == self.selected_actor.as_deref())
            {
                self.selected_actor = characters
                    .iter()
                    .find(|asset| asset["missing"].as_bool() != Some(true))
                    .and_then(|asset| asset["url"].as_str())
                    .map(str::to_owned);
            }
            if let Some(error) = &self.error {
                ui.colored_label(Color32::from_rgb(251, 113, 133), error);
            }
            if let Some(notice) = self
                .overlay
                .as_ref()
                .and_then(|overlay| overlay.notice.as_ref())
            {
                ui.label(RichText::new(notice).color(Color32::from_rgb(251, 113, 133)));
            }
            if characters.is_empty() {
                panel().show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 32.0).max(0.0));
                    ui.heading("Your characters start here");
                    ui.label(
                        RichText::new("Add an image or animation to your library.").color(MUTED),
                    );
                });
                return;
            }
            let selected = characters
                .iter()
                .find(|asset| asset["url"].as_str() == self.selected_actor.as_deref())
                .cloned();
            if let Some(asset) = selected {
                let key = asset["url"].as_str().unwrap().to_owned();
                let mut settings = *self
                    .actor_drafts
                    .entry(key.clone())
                    .or_insert_with(|| initial_settings(&asset));
                let active = self.actor_ui_state(&key);
                if ui.available_width() >= 820.0 {
                    ui.horizontal_top(|ui| {
                        let total_width = ui.available_width();
                        let gap = (total_width * (40.0 / 1544.0)).clamp(20.0, 40.0);
                        let inspector_width = (total_width * (504.0 / 1544.0)).clamp(280.0, 504.0);
                        let preview_width = (total_width - gap - inspector_width).max(1.0);
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.allocate_ui_with_layout(
                            Vec2::new(preview_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| self.actor_preview(ui, &asset, &mut settings),
                        );
                        ui.allocate_ui_with_layout(
                            Vec2::new(inspector_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                if total_width >= 1500.0 {
                                    let extra = egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        Vec2::new(inspector_width, 288.0),
                                    );
                                    crate::child_ui(
                                        ui,
                                        egui::UiBuilder::new().max_rect(extra),
                                        |ui| {
                                            let clip = ui.clip_rect().intersect(extra);
                                            ui.set_clip_rect(clip);
                                            self.actor_summary(ui, &asset, active.clone());
                                        },
                                    );
                                    ui.allocate_exact_size(
                                        Vec2::new(inspector_width, 288.0),
                                        egui::Sense::hover(),
                                    );
                                } else {
                                    self.actor_summary(ui, &asset, active.clone());
                                }
                            },
                        );
                    });
                } else {
                    self.actor_preview(ui, &asset, &mut settings);
                    self.actor_summary(ui, &asset, active);
                }
                ui.add_space(6.0);
                self.actor_controls(ui, &asset, &key, settings);
            } else {
                panel().show(ui, |ui| {
                    ui.set_min_width((ui.available_width() - 32.0).max(0.0));
                    ui.heading("No character available");
                    ui.label(
                        RichText::new(
                            "Every character in your library is missing its file. Add a new animation or restore the files to edit it here.",
                        )
                        .color(MUTED),
                    );
                });
            }
            ui.add_space(10.0);
            let (heading, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), egui::Sense::hover());
            crate::child_ui(
                ui,
                egui::UiBuilder::new()
                    .max_rect(heading)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.heading(RichText::new("Your characters").size(18.0));
                    egui::Frame::default()
                        .fill(PANEL)
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(8, 3))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(characters.len().to_string())
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Overlay · pointer and keyboard input passes through")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_link_with_metrics(ui, "arrow-right", "Manage library", 10.5, 14.0)
                            .clicked()
                        {
                            self.page = crate::Page::Library;
                            self.filter = crate::Filter::Animations;
                            self.favorites_only = false;
                            self.search.clear();
                        }
                    });
                },
            );
            ui.add_space(6.0);
            let query = self.actor_search.trim().to_lowercase();
            let visible: Vec<_> = characters
                .iter()
                .filter(|asset| {
                    asset["name"]
                        .as_str()
                        .unwrap_or("Content")
                        .to_lowercase()
                        .contains(&query)
                })
                .collect();
            if visible.is_empty() {
                ui.label(RichText::new("No characters match your search.").color(MUTED));
            }
            let available_width = ui.available_width();
            let columns = if available_width >= 1120.0 {
                3
            } else if available_width >= 720.0 {
                2
            } else {
                1
            };
            let card_width = (available_width - (columns - 1) as f32 * 16.0) / columns as f32;
            for row in visible.chunks(columns) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;
                    for asset in row {
                        self.character_library_card(ui, asset, card_width);
                    }
                });
                ui.add_space(8.0);
            }
        });
    }

    fn character_library_card(&mut self, ui: &mut egui::Ui, asset: &Value, width: f32) {
        self.character_tile(ui, asset, width, false);
    }

    fn actor_preview(&mut self, ui: &mut egui::Ui, asset: &Value, settings: &mut ActorSettings) {
        let original = *settings;
        let mut preview_settings = original;
        let width = ui.available_width();
        let height = 288.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        crate::preview::record_geometry(ui, "Character Preview", rect);
        ui.painter().rect_filled(rect, 14.0, PANEL);
        let background = self
            .assets
            .iter()
            .find(|item| {
                item["url"].as_str() == self.selected.as_deref()
                    && item["kind"].as_str() != Some("character")
            })
            .or_else(|| {
                self.assets
                    .iter()
                    .find(|item| item["kind"].as_str() == Some("wallpaper"))
            });
        let bg_url = background
            .and_then(|item| {
                item["thumbnailUrl"]
                    .as_str()
                    .or_else(|| item["url"].as_str())
            })
            .map(str::to_owned);
        if let Some(texture) = bg_url.as_deref().and_then(|url| self.thumbnails.get(url)) {
            paint_cover_rounded(ui, rect, texture, [14.0; 4]);
        }
        ui.painter()
            .rect_filled(rect, 14.0, Color32::from_rgba_premultiplied(2, 2, 3, 51));
        crate::paint_rounded_vertical_gradient(
            ui.painter(),
            egui::Rect::from_min_max(rect.min + Vec2::new(0.0, 150.0), rect.max),
            [0.0, 0.0, 14.0, 14.0],
            Color32::TRANSPARENT,
            Color32::from_rgba_premultiplied(9, 7, 12, 187),
        );
        let actor_url = asset["thumbnailUrl"]
            .as_str()
            .or_else(|| asset["url"].as_str());
        let still_texture = actor_url.and_then(|url| self.thumbnails.get(url)).cloned();
        let animated_asset = is_animated_character(asset);
        let static_actor_texture = (!animated_asset)
            .then(|| asset["url"].as_str())
            .flatten()
            .and_then(|url| {
                self.thumbnails
                    .static_actor_preview(url, preview_settings.smoothing)
            })
            .cloned();
        let animated_texture = animated_asset
            .then(|| asset["url"].as_str())
            .flatten()
            .and_then(|url| {
                self.thumbnails.animation_frame(
                    url,
                    preview_settings.speed,
                    preview_settings.playing,
                    preview_settings.looping,
                    preview_settings.smoothing,
                    ui.ctx(),
                )
            })
            .cloned();
        if let Some(texture) = animated_texture.or(static_actor_texture).or(still_texture) {
            let source_width = asset["width"]
                .as_u64()
                .filter(|&width| width > 0)
                .unwrap_or(texture.size()[0] as u64) as f32;
            let source_height = asset["height"]
                .as_u64()
                .filter(|&height| height > 0)
                .unwrap_or(texture.size()[1] as u64) as f32;
            // The editor uses the design's 1000-point canvas; positions stay normalized.
            let actor_width =
                source_width * preview_settings.scale / 100.0 * (rect.width() / 1000.0);
            let actor_height = actor_width * source_height / source_width;
            let mut center = egui::pos2(
                rect.left() + rect.width() * settings.x,
                rect.top() + rect.height() * settings.y - actor_height / 2.0,
            );
            let actor_size = Vec2::new(actor_width, actor_height);
            let hit_bounds =
                rotated_bounds(center, actor_size, preview_settings.rotation).intersect(rect);
            let drag_id = egui::Id::new(("actor-preview-drag", asset["url"].as_str()));
            let editable = !self.busy
                && self
                    .managed_urls
                    .contains(asset["url"].as_str().unwrap_or_default());
            let drag = ui.interact(
                hit_bounds,
                drag_id,
                if editable {
                    egui::Sense::drag()
                } else {
                    egui::Sense::hover()
                },
            );
            if drag.drag_started() {
                if let Some(pointer) = drag.interact_pointer_pos() {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(drag_id, pointer - center));
                }
            }
            if drag.dragged()
                && let Some(pointer) = drag.interact_pointer_pos()
            {
                let offset = ui
                    .ctx()
                    .data(|data| data.get_temp::<Vec2>(drag_id))
                    .unwrap_or_default();
                center = pointer - offset;
                preview_settings.x = ((center.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                preview_settings.y =
                    ((center.y - rect.top() + actor_height / 2.0) / rect.height()).clamp(0.0, 1.0);
                if ui.input(|i| i.modifiers.shift) {
                    preview_settings.x = ((preview_settings.x * rect.width() / 4.0).round() * 4.0
                        / rect.width())
                    .clamp(0.0, 1.0);
                    preview_settings.y = ((preview_settings.y * rect.height() / 4.0).round() * 4.0
                        / rect.height())
                    .clamp(0.0, 1.0);
                }
            }
            if drag.drag_stopped() {
                ui.ctx().data_mut(|data| data.remove::<Vec2>(drag_id));
            }
            center = egui::pos2(
                rect.left() + rect.width() * preview_settings.x,
                rect.top() + rect.height() * preview_settings.y - actor_height / 2.0,
            );
            paint_actor(ui, rect, &texture, center, actor_size, preview_settings);
            let bounds =
                rotated_bounds(center, actor_size, preview_settings.rotation).intersect(rect);
            paint_dashed_rect(
                ui.painter().with_clip_rect(rect),
                bounds,
                ACCENT,
                1.5,
                7.0,
                5.0,
            );
            let name = asset["name"].as_str().unwrap_or("Character");
            let selection_label = format!("{name} · {:.0}%", preview_settings.scale);
            let label_font = egui::FontId::proportional(10.0);
            let selection_label =
                crate::widgets::ellipsize(ui.painter(), &selection_label, &label_font, 138.0);
            let label_text_width = ui
                .painter()
                .layout_no_wrap(selection_label.clone(), label_font.clone(), TEXT)
                .size()
                .x;
            let label_rect = egui::Rect::from_center_size(
                egui::pos2(
                    bounds.center().x,
                    (bounds.top() - 12.0).max(rect.top() + 26.0),
                ),
                egui::vec2((label_text_width + 18.0).min(156.0), 22.0),
            );
            ui.painter()
                .rect_filled(label_rect, 6.0, Color32::from_rgb(18, 16, 23));
            ui.painter().text(
                label_rect.center(),
                egui::Align2::CENTER_CENTER,
                selection_label,
                label_font,
                Color32::from_rgb(201, 195, 214),
            );
            ui.painter().text(
                egui::pos2(
                    (bounds.right() + 8.0).min(rect.right() - 6.0),
                    bounds.center().y,
                ),
                egui::Align2::LEFT_CENTER,
                "click-through",
                egui::FontId::proportional(9.0),
                MUTED,
            );
        }
        let painter = ui.painter().with_clip_rect(rect);
        let animated = is_animated_character(asset);
        let badge = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(16.0, 16.0),
            egui::vec2(if animated { 124.0 } else { 94.0 }, 28.0),
        );
        painter.rect_filled(badge, 14.0, Color32::from_black_alpha(150));
        painter.circle_filled(
            badge.left_center() + egui::vec2(12.0, 0.0),
            2.2,
            Color32::from_rgb(52, 211, 153),
        );
        painter.text(
            badge.left_center() + egui::vec2(20.0, 0.0),
            egui::Align2::LEFT_CENTER,
            if animated { "LIVE PREVIEW" } else { "PREVIEW" },
            egui::FontId::proportional(8.5),
            SECONDARY,
        );
        let output_label = self.selected_output.as_deref().unwrap_or("Display 1");
        let display_chip = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 188.0, rect.top() + 16.0),
            egui::vec2(172.0, 32.0),
        );
        painter.rect_filled(display_chip, 8.0, Color32::from_black_alpha(150));
        paint_action_icon(
            &painter,
            display_chip.left_center() + egui::vec2(16.0, 0.0),
            "monitor",
            SECONDARY,
        );
        painter.text(
            display_chip.left_center() + egui::vec2(29.0, 0.0),
            egui::Align2::LEFT_CENTER,
            output_label,
            egui::FontId::proportional(10.0),
            SECONDARY,
        );
        paint_action_icon(
            &painter,
            display_chip.right_center() - egui::vec2(12.0, 0.0),
            "chevron-down",
            MUTED,
        );
        let monitors = self
            .overlay
            .as_ref()
            .map(|o| o.monitors.clone())
            .unwrap_or_default();
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(display_chip), |ui| {
            if self.busy || !self.ui_capabilities().overlay {
                ui.disable();
            }
            ui.spacing_mut().interact_size.y = 32.0;
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::NONE;
            egui::ComboBox::from_id_salt("preview-display").width(display_chip.width()).selected_text("").icon(|_,_,_,_,_|{})
                .show_ui(ui,|ui|{ui.set_min_width(180.0);for monitor in &monitors {if let Some(name)=&monitor.name{ui.selectable_value(&mut self.selected_output,Some(name.clone()),name);}}})
                .response.on_hover_text("Display used when enabling a character. Disable an active character before moving it.");
        });
        let info = egui::Rect::from_min_size(
            rect.left_bottom() + Vec2::new(16.0, -56.0),
            Vec2::new(268.0_f32.min(rect.width() * 0.6), 40.0),
        );
        painter.rect_filled(
            info,
            10.0,
            Color32::from_rgba_premultiplied(12, 11, 16, 179),
        );
        let mini = egui::Rect::from_min_size(info.min + Vec2::new(6.0, 6.0), Vec2::splat(26.0));
        painter.rect_filled(mini, 6.0, Color32::from_rgb(63, 47, 88));
        if let Some(texture) = asset["thumbnailUrl"]
            .as_str()
            .and_then(|u| self.thumbnails.get(u))
        {
            let size = texture.size_vec2() * (24.0 / texture.size_vec2().y);
            painter.image(
                texture.id(),
                egui::Rect::from_center_size(mini.center(), size),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let name = asset["name"].as_str().unwrap_or("Character");
        let font = egui::FontId::new(9.0, egui::FontFamily::Name("Inter SemiBold".into()));
        let name_width = painter
            .layout_no_wrap(name.into(), font.clone(), TEXT)
            .size()
            .x;
        painter.text(
            info.min + Vec2::new(34.0, 13.0),
            egui::Align2::LEFT_TOP,
            name,
            font,
            TEXT,
        );
        painter.with_clip_rect(info).text(
            info.min + Vec2::new(42.0 + name_width, 13.0),
            egui::Align2::LEFT_TOP,
            {
                let mut parts: Vec<String> = Vec::new();
                if let Some(loop_name) = asset["characterLoop"].as_str().filter(|v| !v.is_empty()) {
                    parts.push(loop_name.to_owned());
                }
                if let Some(fps) = asset["nativeFps"].as_u64().filter(|fps| *fps > 0) {
                    parts.push(format!("{fps} FPS"));
                }
                parts.join("  ·  ")
            },
            egui::FontId::proportional(9.0),
            MUTED,
        );
        let character_url = asset["url"].as_str().unwrap_or_default().to_owned();
        let managed = self.managed_urls.contains(&character_url);
        if animated {
            let pause_rect = egui::Rect::from_center_size(
                rect.right_bottom() - egui::vec2(84.0, 36.0),
                egui::vec2(40.0, 40.0),
            );
            let pause = ui.interact(
                pause_rect,
                egui::Id::new("character-preview-playback"),
                if managed && !self.busy {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            ui.painter()
                .circle_filled(pause_rect.center(), 20.0, ACCENT);
            paint_action_icon(
                ui.painter(),
                pause_rect.center(),
                if preview_settings.playing {
                    "pause"
                } else {
                    "play-large"
                },
                Color32::BLACK,
            );
            if pause.clicked() {
                preview_settings.playing = !preview_settings.playing;
            }
        }
        let audio = audio_defaults(asset);
        let track_count = audio["files"].as_array().map_or(0, Vec::len);
        if track_count > 0 {
            let speaker_rect = egui::Rect::from_center_size(
                rect.right_bottom() - egui::vec2(36.0, 36.0),
                egui::vec2(40.0, 40.0),
            );
            let enabled = audio["enabled"].as_bool() == Some(true);
            let can_toggle_audio = managed && !self.busy;
            let speaker = ui
                .interact(
                    speaker_rect,
                    egui::Id::new("character-preview-audio"),
                    if can_toggle_audio {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                )
                .on_hover_text(if can_toggle_audio {
                    if enabled {
                        "Mute character audio"
                    } else {
                        "Enable character audio"
                    }
                } else if !managed {
                    "Import this character before changing its audio."
                } else {
                    "Wait for the current operation to finish."
                });
            ui.painter()
                .circle_filled(speaker_rect.center(), 20.0, PANEL);
            ui.painter()
                .circle_stroke(speaker_rect.center(), 20.0, Stroke::new(1.0_f32, BORDER));
            paint_speaker_icon(ui.painter(), speaker_rect, enabled);
            if speaker.clicked() {
                let mut audio = audio;
                audio["enabled"] = json!(!enabled);
                let volume = asset["volume"].as_f64().unwrap_or(50.0);
                self.save_character_audio(character_url.clone(), audio, volume, ui.ctx());
            }
        }
        if preview_settings != original {
            *settings = preview_settings;
            let has_worker = self
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.worker(&character_url).is_some());
            match has_worker
                .then(|| self.update_character_overlay(&character_url, preview_settings))
            {
                Some(Err(error)) => self.status = Some(error),
                _ => {
                    self.actor_drafts.insert(character_url, preview_settings);
                }
            }
        }
    }

    fn actor_summary(
        &mut self,
        ui: &mut egui::Ui,
        asset: &Value,
        active: Option<(WorkerStatus, String)>,
    ) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 288.0), egui::Sense::hover());
        crate::preview::record_geometry(ui, "Character Inspector", rect);
        let painter = ui.painter();
        painter.rect_filled(rect, 14.0, PANEL);
        painter.rect_stroke(
            rect,
            14.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let thumb = egui::Rect::from_min_size(rect.min + Vec2::splat(16.0), Vec2::splat(64.0));
        painter.rect_filled(thumb, 10.0, Color32::from_rgb(63, 47, 88));
        if let Some(texture) = asset["thumbnailUrl"]
            .as_str()
            .and_then(|u| self.thumbnails.get(u))
        {
            let scale = (56.0 / texture.size_vec2().x).min(60.0 / texture.size_vec2().y);
            ui.painter().image(
                texture.id(),
                egui::Rect::from_center_size(thumb.center(), texture.size_vec2() * scale),
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        let text = rect.min + Vec2::new(96.0, 20.0);
        ui.painter().text(
            text,
            egui::Align2::LEFT_TOP,
            asset["name"].as_str().unwrap_or("Character"),
            egui::FontId::new(20.0, egui::FontFamily::Name("Inter SemiBold".into())),
            TEXT,
        );
        ui.painter().text(
            text + Vec2::new(0.0, 28.0),
            egui::Align2::LEFT_TOP,
            {
                let mut parts = vec![
                    "Character".to_owned(),
                    format!(
                        "{} × {}",
                        asset["width"].as_u64().unwrap_or(0),
                        asset["height"].as_u64().unwrap_or(0)
                    ),
                ];
                if let Some(loop_name) = asset["characterLoop"].as_str().filter(|v| !v.is_empty()) {
                    parts.push(loop_name.to_owned());
                }
                parts.join("  ·  ")
            },
            egui::FontId::proportional(10.5),
            MUTED,
        );
        if active.is_some() {
            let badge = egui::Rect::from_min_size(
                egui::pos2(rect.right() - 108.0, rect.top() + 20.0),
                Vec2::new(90.0, 22.0),
            );
            ui.painter()
                .rect_filled(badge, 11.0, Color32::from_rgb(30, 58, 50));
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                "• ACTIVE",
                egui::FontId::proportional(9.0),
                Color32::from_rgb(110, 231, 183),
            );
        }
        ui.painter().line_segment(
            [
                rect.min + Vec2::new(16.0, 96.0),
                egui::pos2(rect.right() - 16.0, rect.top() + 96.0),
            ],
            Stroke::new(1.0_f32, BORDER),
        );
        let output_name = active
            .as_ref()
            .map(|(_, name)| name.as_str())
            .or(self.selected_output.as_deref());
        let output_description = output_name
            .and_then(|name| {
                self.overlay
                    .as_ref()?
                    .monitors
                    .iter()
                    .find(|monitor| monitor.name.as_deref() == Some(name))?
                    .logical_size
                    .map(|(width, height)| format!("{name} · {width} × {height}"))
            })
            .unwrap_or_else(|| output_name.unwrap_or("No display available").to_owned());
        let tracks = asset["audio"]["files"].as_array();
        let track_count = tracks.map_or(0, Vec::len);
        let sound_description = match tracks
            .and_then(|tracks| tracks.first())
            .and_then(|track| track["name"].as_str())
        {
            Some(name) => format!(
                "{track_count} audio {} · {name}",
                if track_count == 1 { "track" } else { "tracks" }
            ),
            None => format!("{track_count} audio tracks"),
        };
        for (i, (icon, label, value)) in [
            ("monitor", "DISPLAY", output_description),
            (
                "layers",
                "OVERLAY",
                "Above normal windows · Input passes through".into(),
            ),
            ("music", "SOUND", sound_description),
        ]
        .iter()
        .enumerate()
        {
            let y = rect.top() + 110.0 + i as f32 * 38.0;
            paint_action_icon(
                ui.painter(),
                egui::pos2(rect.left() + 26.0, y + 14.0),
                icon,
                MUTED,
            );
            ui.painter().text(
                egui::pos2(rect.left() + 44.0, y),
                egui::Align2::LEFT_TOP,
                *label,
                egui::FontId::proportional(8.5),
                MUTED,
            );
            let value_font = egui::FontId::proportional(10.5);
            ui.painter().text(
                egui::pos2(rect.left() + 44.0, y + 14.0),
                egui::Align2::LEFT_TOP,
                crate::widgets::ellipsize(
                    ui.painter(),
                    &value,
                    &value_font,
                    rect.width() - 44.0 - 40.0,
                ),
                value_font,
                SECONDARY,
            );
        }
        for index in 0..3 {
            let row = egui::Rect::from_min_size(
                rect.min + Vec2::new(16.0, 106.0 + index as f32 * 38.0),
                Vec2::new(rect.width() - 32.0, 38.0),
            );
            let editable = index == 2
                && asset["audio"]["files"]
                    .as_array()
                    .is_some_and(|f| !f.is_empty())
                && !self.busy;
            crate::icons::paint_node(
                ui.painter(),
                egui::pos2(rect.right() - 18.0, row.center().y),
                633,
                if editable { MUTED } else { BORDER },
                None,
            );
            let response = ui.interact(
                row,
                ui.id().with(("inspector-row", index)),
                if editable {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            if response.clicked() {
                self.audio_dialog = Some(crate::AudioDialog {
                    actor_url: asset["url"].as_str().unwrap_or_default().into(),
                    track_index: 0,
                    audio: audio_defaults(asset),
                    volume: asset["volume"].as_f64().unwrap_or(50.0),
                });
            }
            response.on_hover_text(match index {
                0 => "Disable an active character before changing its display.",
                1 => "The existing overlay uses a fixed layer and passes input through.",
                _ => "Configure character audio",
            });
        }
        if active.is_none() {
            let combo = egui::Rect::from_min_size(
                rect.min + Vec2::new(44.0, 120.0),
                Vec2::new((rect.width() - 100.0).min(220.0), 24.0),
            );
            let monitors = self
                .overlay
                .as_ref()
                .map(|o| o.monitors.clone())
                .unwrap_or_default();
            crate::child_ui(
                ui,
                egui::UiBuilder::new()
                    .max_rect(combo)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().interact_size.y = 20.0;
                    egui::ComboBox::from_id_salt("character-display")
                        .icon(crate::widgets::combo_chevron)
                        .selected_text(self.selected_output.as_deref().unwrap_or("Display"))
                        .show_ui(ui, |ui| {
                            ui.set_min_width(160.0);
                            for m in &monitors {
                                if let Some(name) = &m.name {
                                    ui.selectable_value(
                                        &mut self.selected_output,
                                        Some(name.clone()),
                                        name,
                                    );
                                }
                            }
                        });
                },
            );
        }
        let action =
            egui::Rect::from_min_size(rect.min + Vec2::new(18.0, 232.0), Vec2::new(116.0, 38.0));
        let enabled = !self.busy
            && self.overlay.is_some()
            && self.selected_output.is_some()
            && asset["missing"].as_bool() != Some(true)
            && asset["video"].as_bool() != Some(true);
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(action), |ui| {
            if icon_text_button(
                ui,
                "power",
                if active.is_some() {
                    "Disable"
                } else {
                    "Enable"
                },
                action.size(),
                enabled,
                Color32::TRANSPARENT,
                if active.is_some() {
                    Color32::from_rgb(251, 113, 133)
                } else {
                    SECONDARY
                },
                BORDER,
            )
            .clicked()
            {
                self.toggle_character(asset, ui.ctx());
            }
        });
        let library =
            egui::Rect::from_min_size(action.min + Vec2::new(124.0, 0.0), Vec2::new(132.0, 38.0));
        crate::child_ui(ui, egui::UiBuilder::new().max_rect(library), |ui| {
            if icon_text_button(
                ui,
                "folder",
                "Open library",
                library.size(),
                true,
                PANEL,
                SECONDARY,
                BORDER,
            )
            .clicked()
            {
                self.page = crate::Page::Library;
                self.filter = crate::Filter::Animations;
                self.favorites_only = false;
                self.selected = asset["url"].as_str().map(str::to_owned);
                self.search.clear();
            }
        });
    }

    fn actor_controls(
        &mut self,
        ui: &mut egui::Ui,
        asset: &Value,
        key: &str,
        original: ActorSettings,
    ) {
        let mut settings = original;
        let transition = self
            .overlay
            .as_ref()
            .and_then(|overlay| overlay.worker(key))
            .is_some_and(|worker| worker.status != WorkerStatus::Running);
        let animated = is_animated_character(asset);
        let mut volume_draft = self.character_volume_draft.take();
        let mut actor_volume = volume_draft_value(
            &mut volume_draft,
            key,
            asset["volume"].as_f64().unwrap_or(50.0) as f32,
        );
        let mut audio = audio_defaults(asset);
        let mut selected_audio_track = self.audio_track_indices.get(key).copied().unwrap_or(0);
        let mut audio_action = None;
        let busy = self.busy;
        let managed_char = self.managed_urls.contains(key);
        let ctrl = !busy && managed_char;
        let audio_interactive = !transition && self.managed_urls.contains(key);
        let mut draw_panel = |ui: &mut egui::Ui, index: usize| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
            ui.spacing_mut().interact_size.y = 20.0;
            ui.add_enabled_ui(!transition,|ui| {
                let title=["Transform","Appearance","Playback and sounds"][index];
                let (rect,reset)=control_panel(ui,title,ctrl,match index {0=>Some("Drag the character in the preview to reposition it. Hold Shift to snap position."),1=>Some("Mirroring flips the artwork without changing its anchor point."),_=>None});
                if reset {
                    reset_actor_group(&mut settings,match index {0=>ResetGroup::Transform,1=>ResetGroup::Appearance,_=>ResetGroup::Playback});
                    if index==2 && audio_interactive && reset_audio_controls(&mut audio,&mut actor_volume) {
                        volume_draft=Some((key.to_owned(),actor_volume));audio_action=Some(AudioCardAction::Save);
                    }
                }
                match index {
                    0=>{
                        let mut x=settings.x*100.0;let mut y=settings.y*100.0;
                        panel_slider(ui,rect,47.0,&mut settings.scale,10.0..=300.0,"Size","%",ctrl);
                        panel_slider(ui,rect,83.0,&mut x,0.0..=100.0,"Position X","%",ctrl);
                        panel_slider(ui,rect,119.0,&mut y,0.0..=100.0,"Position Y","%",ctrl);
                        settings.x=x/100.0;settings.y=y/100.0;
                            }
                    1=>{
                        panel_slider(ui,rect,47.0,&mut settings.opacity,0.0..=100.0,"Opacity","%",ctrl);
                        panel_slider(ui,rect,83.0,&mut settings.rotation,-180.0..=180.0,"Rotation","°",ctrl);
                        let row=egui::Rect::from_min_size(rect.min+Vec2::new(0.0,134.0),Vec2::new(rect.width(),20.0));
                        crate::child_ui(ui,egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)),|ui| {
                            labeled_switch(ui,&mut settings.flip_x,"Mirror",ctrl);
                        });
                        let row=egui::Rect::from_min_size(rect.min+Vec2::new(rect.width()*0.475,134.0),Vec2::new(rect.width()*0.525,20.0));
                        crate::child_ui(ui,egui::UiBuilder::new().max_rect(row),|ui|{labeled_switch(ui,&mut settings.smoothing,"Smooth scaling",ctrl);});
                        ui.painter().line_segment([rect.min+Vec2::new(0.0,120.0),rect.min+Vec2::new(rect.width(),120.0)],Stroke::new(1.0_f32,BORDER));
                    }
                    _=>{
                        let row=egui::Rect::from_min_size(rect.min+Vec2::new(0.0,40.0),Vec2::new(rect.width(),20.0));
                        crate::child_ui(ui,egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)),|ui|{
                            labeled_switch(ui,&mut settings.playing,"Play",animated&&ctrl);
                        });
                        let play_w=ui.painter().layout_no_wrap("Play".into(),egui::FontId::proportional(11.0),TEXT).size().x;
                        let loop_x=(36.0+8.0+play_w+24.0).max(124.0).min((rect.width()-60.0).max(0.0));
                        let row=egui::Rect::from_min_size(rect.min+Vec2::new(loop_x,40.0),Vec2::new(rect.width()-loop_x,20.0));
                        crate::child_ui(ui,egui::UiBuilder::new().max_rect(row),|ui|{labeled_switch(ui,&mut settings.looping,"Loop",animated&&ctrl);});
                        let body=egui::Rect::from_min_size(rect.min+Vec2::new(0.0,70.0),Vec2::new(rect.width(),142.0));
                        crate::child_ui(ui,egui::UiBuilder::new().max_rect(body),|ui|draw_audio_controls(ui,key,&mut audio,&mut selected_audio_track,&mut settings,&mut actor_volume,&mut volume_draft,animated,&mut audio_action,audio_interactive,busy));
                    }
                }
            });
        };
        if ui.available_width() >= 1120.0 {
            ui.spacing_mut().item_spacing.x = 16.0;
            ui.columns(3, |columns| {
                for (index, column) in columns.iter_mut().enumerate() {
                    draw_panel(column, index);
                }
            });
        } else {
            for index in 0..3 {
                draw_panel(ui, index);
                if index < 2 {
                    ui.add_space(12.0);
                }
            }
        }
        self.character_volume_draft = volume_draft;
        if audio["files"].is_array() && !audio["files"].as_array().unwrap().is_empty() {
            selected_audio_track =
                selected_audio_track.min(audio["files"].as_array().unwrap().len() - 1);
        } else {
            selected_audio_track = 0;
        }
        self.audio_track_indices
            .insert(key.to_owned(), selected_audio_track);
        match audio_action {
            Some(AudioCardAction::Add) => {
                self.choose_character_audio(key.to_owned(), ui.ctx());
            }
            Some(AudioCardAction::Configure) => {
                self.audio_dialog = Some(crate::AudioDialog {
                    actor_url: key.to_owned(),
                    track_index: selected_audio_track,
                    audio: audio.clone(),
                    volume: f64::from(actor_volume),
                });
            }
            Some(AudioCardAction::Save) => {
                self.save_character_audio(
                    key.to_owned(),
                    audio.clone(),
                    f64::from(actor_volume),
                    ui.ctx(),
                );
            }
            None => {}
        }
        self.sound_settings_dialog(ui.ctx());
        if settings != original {
            let has_worker = self
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.worker(key).is_some());
            let result = has_worker.then(|| self.update_character_overlay(key, settings));
            match result {
                Some(Err(error)) => self.status = Some(error),
                _ => {
                    self.actor_drafts.insert(key.to_owned(), settings);
                }
            }
        }
        let managed = self.managed_urls.contains(key);
        let changed =
            self.actor_drafts.get(key).copied().unwrap_or(settings) != initial_settings(asset);
        if !changed && managed {
            return;
        }
        ui.horizontal(|ui| {
            let managed = self.managed_urls.contains(key);
            let changed =
                self.actor_drafts.get(key).copied().unwrap_or(settings) != initial_settings(asset);
            let response = ui.add_enabled(
                managed && changed && !self.busy,
                egui::Button::new("Save character settings"),
            );
            if response.clicked() {
                let current = self.actor_drafts.get(key).copied().unwrap_or(settings);
                self.save_actor_settings(key.to_owned(), current, ui.ctx());
            }
            if !managed {
                ui.label(RichText::new("Import this character to save its settings.").color(MUTED));
            } else if changed {
                ui.label(RichText::new("Unsaved changes").color(MUTED));
            }
        });
    }

    fn sound_settings_dialog(&mut self, ctx: &egui::Context) {
        if self.audio_dialog.is_none() {
            return;
        }
        let mut save = false;
        let mut cancel = false;
        let mut test = false;
        let busy = self.busy;
        let dialog = self.audio_dialog.as_mut().expect("dialog checked above");
        if let Some(tracks) = dialog.audio["files"].as_array() {
            if !tracks.is_empty() {
                dialog.track_index = dialog.track_index.min(tracks.len() - 1);
            }
        }
        let track_index = dialog.track_index;
        let master_enabled = dialog.audio["enabled"].as_bool() == Some(true);
        let track_name = dialog.audio["files"][track_index]["name"]
            .as_str()
            .unwrap_or("Character audio")
            .to_owned();

        let modal = egui::Modal::new(egui::Id::new("character-sound-settings"))
            .backdrop_color(Color32::from_black_alpha(185))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(18)
                    .inner_margin(20),
            )
            .show(ctx, |ui| {
                ui.set_width(480.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.heading(RichText::new("Sound settings").size(20.0));
                        ui.label(RichText::new(track_name).size(11.0).color(MUTED));
                    });
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Min),
                        |ui| {
                            if icon_button(
                                ui,
                                "close",
                                Vec2::new(32.0, 32.0),
                                MUTED,
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                !busy,
                            )
                                .clicked()
                            {
                                cancel = true;
                            }
                        },
                    );
                });
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(10.0);

                egui::ScrollArea::vertical()
                    .max_height(390.0)
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            let track = &mut dialog.audio["files"][track_index];
                            let playback = &mut track["playback"];
                            let mut enabled = playback["enabled"].as_bool().unwrap_or(true);
                            let toggle_origin = ui.cursor().min;
                            let sound_changed = labeled_switch(ui, &mut enabled, "Enable sound", true);
                            crate::preview::record_geometry(
                                ui,
                                "Sound enabled",
                                egui::Rect::from_min_size(toggle_origin, Vec2::new(130.0, 22.0)),
                            );
                            if sound_changed {
                                playback["enabled"] = json!(enabled);
                            }

                        ui.add_space(8.0);
                        ui.label(RichText::new("Playback trigger").color(MUTED));
                        let click_available = crate::x11_click::available_in_session();
                        let mut mode = playback["mode"].as_str().unwrap_or("once").to_owned();
                        let mode_label = match mode.as_str() {
                            "loop" => "Continuous",
                            "interval" => "At intervals",
                            "click" if click_available => "When clicking the character",
                            "click" => "When clicking the character · unavailable",
                            _ => "When activated",
                        };
                        let combo_width = ui.available_width();
                        let mode_label = crate::widgets::ellipsize(
                            ui.painter(),
                            mode_label,
                            &egui::TextStyle::Button.resolve(ui.style()),
                            (combo_width - 36.0).max(40.0),
                        );
                        egui::ComboBox::from_id_salt("character-audio-mode")
            .icon(crate::widgets::combo_chevron)
            .selected_text(mode_label)
                            .width(combo_width)
                            .show_ui(ui, |ui| {
                                ui.set_min_width(combo_width.max(260.0));
                                for (value, label) in [
                                    ("once", "When activated"),
                                    ("loop", "Continuous"),
                                    ("interval", "At intervals"),
                                    ("click", "When clicking the character"),
                                ] {
                                    if value == "click" && !click_available {
                                        ui.add_enabled(
                                            false,
                                            egui::Button::new(format!("{label} · unavailable"))
                                                .selected(mode == value),
                                        );
                                    } else {
                                        ui.selectable_value(&mut mode, value.to_owned(), label);
                                    }
                                }
                            });
                        playback["mode"] = json!(mode);

                        if mode == "interval" {
                            let mut seconds = playback["intervalSeconds"]
                                .as_u64()
                                .unwrap_or(30)
                                .clamp(1, 86_400);
                            if ui
                                .add(
                                    egui::Slider::new(&mut seconds, 1..=86_400)
                                        .text("Interval in seconds"),
                                )
                                .changed()
                            {
                                playback["intervalSeconds"] = json!(seconds);
                            }
                            let mut start_immediately = playback["startImmediately"]
                                .as_bool()
                                .unwrap_or(true);
                            if ui
                                .checkbox(&mut start_immediately, "Play immediately on activation")
                                .changed()
                            {
                                playback["startImmediately"] = json!(start_immediately);
                            }
                        }

                        let mut volume = playback["volume"].as_f64().unwrap_or(100.0) as f32;
                        if labeled_slider(ui,&mut volume,0.0..=100.0,"Volume","%",true).changed()
                        {
                            playback["volume"] = json!(volume.round());
                        }

                        if mode == "click" {
                            let mut radius = playback["radiusPixels"]
                                .as_f64()
                                .unwrap_or(40.0) as f32;
                            if labeled_slider(ui,&mut radius,0.0..=500.0,"Click radius"," px",true).changed()
                            {
                                playback["radiusPixels"] = json!(radius.round());
                            }
                            let mut cooldown = playback["cooldownSeconds"]
                                .as_f64()
                                .unwrap_or(2.0) as f32;
                            if labeled_slider(ui,&mut cooldown,0.0..=60.0,"Cooldown"," s",true).changed()
                            {
                                playback["cooldownSeconds"] = json!(cooldown.round());
                            }
                            let detail = if click_available {
                                "X11 observes left clicks without intercepting them; the overlay stays click-through."
                            } else {
                                "Desktop click triggers are available only in X11 sessions. Native Wayland does not expose pointer events to a click-through overlay."
                            };
                            ui.label(RichText::new(detail).size(10.0).color(if click_available {
                                MUTED
                            } else {
                                Color32::from_rgb(251, 191, 36)
                            }));
                        }

                            ui.add_space(10.0);
                            if ui
                                .add_enabled(
                                    master_enabled && playback["enabled"].as_bool() != Some(false),
                                    egui::Button::new("Test sound"),
                                )
                                .clicked()
                            {
                                test = true;
                            }
                            ui.label(
                                RichText::new("Changes are saved to this character when you choose Done.")
                                    .size(10.0)
                                    .color(MUTED),
                            );
                        });
                    });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let done_color = if busy { MUTED } else { crate::SURFACE };
                            let done = egui::Button::new(
                                RichText::new("Done")
                                    .font(egui::FontId::new(
                                        11.5,
                                        egui::FontFamily::Name("Inter SemiBold".into()),
                                    ))
                                    .color(done_color),
                            )
                            .fill(ACCENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(8);
                            let done = ui.add_enabled(!busy, done);
                            crate::preview::record_geometry(ui, "Sound Done", done.rect);
                            if done.clicked() {
                                save = true;
                            }
                            let cancel_response = ui.add_enabled(!busy, egui::Button::new("Cancel"));
                            crate::preview::record_geometry(ui, "Sound Cancel", cancel_response.rect);
                            if cancel_response.clicked() {
                                cancel = true;
                            }
                        },
                    );
                });
            });
        if !busy && modal.should_close() {
            cancel = true;
        }
        if cancel {
            self.audio_dialog = None;
            return;
        }
        if test
            && self.preview_action(crate::preview::UiAction::Operation {
                name: "test_character_audio".into(),
            })
        {
            self.status = Some("Playing character audio preview.".into());
        } else if test {
            let global_volume = self
                .settings_draft
                .as_ref()
                .and_then(|settings| settings["globalVolume"].as_f64())
                .or_else(|| self.settings["globalVolume"].as_f64())
                .unwrap_or(100.0);
            let output = self.preferred_audio_output().map(str::to_owned);
            if let Some(dialog) = self.audio_dialog.as_ref() {
                if let Some(mut actor) = self
                    .assets
                    .iter()
                    .find(|asset| asset["url"].as_str() == Some(dialog.actor_url.as_str()))
                    .cloned()
                {
                    actor["audio"] = dialog.audio.clone();
                    actor["volume"] = json!(dialog.volume);
                    match self.audio.test_track(
                        &actor,
                        dialog.track_index,
                        global_volume,
                        output.as_deref(),
                    ) {
                        Ok(()) => self.status = Some("Playing character audio preview.".into()),
                        Err(error) => self.status = Some(error),
                    }
                }
            }
        }
        if save {
            if let Some(dialog) = self.audio_dialog.as_ref() {
                self.save_character_audio(
                    dialog.actor_url.clone(),
                    dialog.audio.clone(),
                    dialog.volume,
                    ctx,
                );
            }
        }
    }
}

fn paint_actor(
    ui: &egui::Ui,
    clip: egui::Rect,
    texture: &egui::TextureHandle,
    center: egui::Pos2,
    size: Vec2,
    settings: ActorSettings,
) {
    let (sin, cos) = settings.rotation.to_radians().sin_cos();
    let corners = [
        egui::vec2(-size.x / 2.0, -size.y / 2.0),
        egui::vec2(size.x / 2.0, -size.y / 2.0),
        egui::vec2(size.x / 2.0, size.y / 2.0),
        egui::vec2(-size.x / 2.0, size.y / 2.0),
    ];
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let uvs = if settings.flip_x {
        [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]]
    } else {
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
    };
    for (corner, uv) in corners.into_iter().zip(uvs) {
        let pos = center
            + egui::vec2(
                corner.x * cos - corner.y * sin,
                corner.x * sin + corner.y * cos,
            );
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::pos2(uv[0], uv[1]),
            color: Color32::from_white_alpha((settings.opacity * 2.55).round() as u8),
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    ui.painter()
        .with_clip_rect(clip)
        .add(egui::Shape::mesh(mesh));
}

fn rotated_bounds(center: egui::Pos2, size: Vec2, rotation: f32) -> egui::Rect {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let half = Vec2::new(
        (size.x * cos.abs() + size.y * sin.abs()) / 2.0,
        (size.x * sin.abs() + size.y * cos.abs()) / 2.0,
    );
    egui::Rect::from_center_size(center, half * 2.0)
}

fn paint_dashed_rect(
    painter: egui::Painter,
    rect: egui::Rect,
    color: Color32,
    thickness: f32,
    dash: f32,
    gap: f32,
) {
    let stroke = Stroke::new(thickness, color);
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    for index in 0..corners.len() {
        let start = corners[index];
        let end = corners[(index + 1) % corners.len()];
        let delta = end - start;
        let length = delta.length();
        if length <= 0.0 {
            continue;
        }
        let direction = delta / length;
        let mut offset = 0.0;
        while offset < length {
            let segment_end = (offset + dash).min(length);
            painter.line_segment(
                [start + direction * offset, start + direction * segment_end],
                stroke,
            );
            offset += dash + gap;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_resets_restore_only_their_defaults_and_keep_audio_tracks() {
        let mut settings = ActorSettings {
            x: 0.1,
            y: 0.2,
            scale: 70.0,
            opacity: 20.0,
            rotation: 30.0,
            flip_x: true,
            smoothing: false,
            speed: 150.0,
            playing: false,
            looping: false,
        };
        let defaults = ActorSettings::default();
        reset_actor_group(&mut settings, ResetGroup::Transform);
        assert_eq!(
            (settings.x, settings.y, settings.scale),
            (defaults.x, defaults.y, defaults.scale)
        );
        assert_eq!(
            (
                settings.opacity,
                settings.rotation,
                settings.flip_x,
                settings.smoothing
            ),
            (20.0, 30.0, true, false)
        );
        reset_actor_group(&mut settings, ResetGroup::Appearance);
        assert_eq!(
            (
                settings.opacity,
                settings.rotation,
                settings.flip_x,
                settings.smoothing
            ),
            (
                defaults.opacity,
                defaults.rotation,
                defaults.flip_x,
                defaults.smoothing
            )
        );
        assert_eq!(
            (settings.speed, settings.playing, settings.looping),
            (150.0, false, false)
        );
        reset_actor_group(&mut settings, ResetGroup::Playback);
        assert_eq!(settings, defaults);

        let mut audio = json!({"enabled": true, "files": [{"name": "Footsteps"}]});
        let mut volume = 35.0;
        assert!(reset_audio_controls(&mut audio, &mut volume));
        assert_eq!(audio["enabled"], false);
        assert_eq!(audio["files"][0]["name"], "Footsteps");
        assert_eq!(volume, 50.0);
        assert!(!reset_audio_controls(&mut audio, &mut volume));
    }

    #[test]
    fn labeled_slider_fits_the_row_and_responds_to_drag() {
        let ctx = egui::Context::default();
        let mut value = 50.0;
        let mut frame = |events: Vec<egui::Event>| {
            let mut result = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(316.0, 100.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let start = ui.cursor().min;
                    let response =
                        labeled_slider(ui, &mut value, 0.0..=100.0, "Opacity", "%", true);
                    result = Some((start, response.rect, value));
                });
            });
            result.unwrap()
        };
        let (start, rect, _) = frame(vec![]);
        assert!(rect.left() >= start.x + 76.0);
        assert!(rect.right() <= 308.0, "start={start:?}, slider={rect:?}");
        let pos = egui::pos2(rect.center().x + 30.0, rect.center().y);
        frame(vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        let (_, _, changed_value) = frame(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(changed_value > 70.0);
    }

    #[test]
    fn labeled_switch_accepts_switch_and_label_clicks_but_respects_disabled_state() {
        let ctx = egui::Context::default();
        let mut value = false;
        let mut frame = |enabled: bool, events: Vec<egui::Event>| {
            let mut result = None;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(400.0, 100.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let start = ui.cursor().min;
                    let changed = labeled_switch(ui, &mut value, "Mirror", enabled);
                    result = Some((start, value, changed));
                });
            });
            result.unwrap()
        };
        let (start, _, _) = frame(true, vec![]);
        let switch = start + Vec2::new(18.0, 10.0);
        let label = start + Vec2::new(55.0, 10.0);
        let event = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            true,
            vec![egui::Event::PointerMoved(switch), event(switch, true)],
        );
        assert_eq!(frame(true, vec![event(switch, false)]).1, true);
        frame(
            true,
            vec![egui::Event::PointerMoved(label), event(label, true)],
        );
        let (_, state, changed) = frame(true, vec![event(label, false)]);
        assert!(!state && changed);
        frame(
            false,
            vec![egui::Event::PointerMoved(switch), event(switch, true)],
        );
        let (_, state, changed) = frame(false, vec![event(switch, false)]);
        assert!(!state && !changed);
    }

    #[test]
    fn reads_legacy_character_controls_with_safe_ranges() {
        let asset = serde_json::json!({"posX":72,"posY":92,"scale":78,"opacity":150,"rotation":-10,"speed":0,"flipX":true});
        let settings = initial_settings(&asset);
        assert_eq!(settings.x, 0.72);
        assert_eq!(settings.y, 0.92);
        assert_eq!(settings.opacity, 100.0);
        assert_eq!(settings.speed, 10.0);
        assert!(settings.flip_x);
    }
}
