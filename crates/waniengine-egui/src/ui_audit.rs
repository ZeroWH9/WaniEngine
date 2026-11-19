//! Pointer-driven checks of the real frontend with the in-memory dispatcher.
//! Only --ui-preview --ui-audit-actions invokes these; no desktop service is used.
use crate::{App, DiscoverFilter, Filter, Page};
use eframe::egui::{self, Vec2};
use serde_json::json;
use std::time::{Duration, Instant};

impl App {
    fn audit_frame(&mut self, ctx: &egui::Context, events: Vec<egui::Event>) {
        self.audit_frame_with_modifiers(ctx, events, egui::Modifiers::NONE);
    }
    fn audit_frame_with_modifiers(
        &mut self,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        self.audit_frame_at_size(ctx, events, modifiers, Vec2::new(1600.0, 1000.0));
    }
    fn audit_frame_at_size(
        &mut self,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
        size: Vec2,
    ) {
        // Share the native frame's clock origin. Advancing synthetic frames by
        // 1/60s can otherwise put egui's history ahead of eframe after the audit.
        let clock_id = egui::Id::new("ui-audit-clock");
        let native_time = ctx.input(|input| input.time);
        let clock = ctx.data_mut(|data| {
            if let Some(clock) = data.get_temp::<(Instant, f64)>(clock_id) {
                clock
            } else {
                let clock = (Instant::now(), native_time);
                data.insert_temp(clock_id, clock);
                clock
            }
        });
        let input = egui::RawInput {
            time: Some(clock.1 + clock.0.elapsed().as_secs_f64()),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            modifiers,
            ..Default::default()
        };
        let output = ctx.run(input, |ctx| self.draw_interface(ctx));
        // Standalone audit frames are not presented by eframe. Preserve their
        // uploads for the first real renderer frame, including the font atlas.
        if let Some(preview) = self.preview.as_mut() {
            preview.audit_textures.extend(output.textures_delta.set);
        }
    }
    fn audit_click(&mut self, ctx: &egui::Context, x: f32, y: f32) {
        let pos = egui::pos2(x, y);
        for pressed in [true, false] {
            self.audit_frame(
                ctx,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        self.audit_frame(ctx, vec![egui::Event::PointerGone]);
        self.audit_frame(ctx, vec![]);
    }
    fn audit_drag(&mut self, ctx: &egui::Context, from: egui::Pos2, to: egui::Pos2, shift: bool) {
        let modifiers = egui::Modifiers {
            shift,
            ..egui::Modifiers::NONE
        };
        self.audit_frame_with_modifiers(
            ctx,
            vec![
                egui::Event::PointerMoved(from),
                egui::Event::PointerButton {
                    pos: from,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers,
                },
            ],
            modifiers,
        );
        self.audit_frame_with_modifiers(
            ctx,
            vec![egui::Event::PointerMoved(from + (to - from) * 0.1)],
            modifiers,
        );
        self.audit_frame_with_modifiers(ctx, vec![egui::Event::PointerMoved(to)], modifiers);
        self.audit_frame_with_modifiers(
            ctx,
            vec![egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            }],
            modifiers,
        );
        self.audit_frame(ctx, vec![egui::Event::PointerGone]);
        self.audit_frame(ctx, vec![]);
    }
    fn audit_reset(&mut self, ctx: &egui::Context) {
        self.seed_preview(ctx);
        self.page = Page::Library;
        self.import_files = None;
        self.audio_dialog = None;
        self.playlist_dialog_open = false;
        self.hero_menu_open = false;
        self.filter_menu_open = false;
        self.grid_view = true;
        ctx.memory_mut(|m| m.close_popup());
        self.audit_frame(ctx, vec![]);
        self.audit_frame(ctx, vec![]);
    }
    fn audit_click_recorded(&mut self, ctx: &egui::Context, name: &str) {
        let bounds = ctx
            .data(|d| d.get_temp::<crate::preview::GeometryLog>(egui::Id::new("ui-geometry-log")))
            .expect("UI geometry available")
            .0[name]
            .bounds;
        self.audit_click(
            ctx,
            bounds[0] + bounds[2] / 2.0,
            bounds[1] + bounds[3] / 2.0,
        );
    }
    fn audit_scroll_at_size(&mut self, ctx: &egui::Context, size: Vec2, delta: f32) {
        // Point events smaller than eight points follow egui's trackpad path,
        // avoiding timed wheel smoothing in this deterministic input audit.
        let mut events = vec![egui::Event::PointerMoved(egui::pos2(
            size.x * 0.6,
            size.y * 0.5,
        ))];
        events.extend(
            (0..(delta.abs() / 7.0).ceil() as usize).map(|_| egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 7.0 * delta.signum()),
                modifiers: egui::Modifiers::NONE,
            }),
        );
        self.audit_frame_at_size(ctx, events, egui::Modifiers::NONE, size);
        for _ in 0..3 {
            self.audit_frame_at_size(ctx, vec![], egui::Modifiers::NONE, size);
        }
    }
    fn audit_component_visible(ctx: &egui::Context, name: &str) -> bool {
        ctx.data(|d| d.get_temp::<crate::preview::GeometryLog>(egui::Id::new("ui-geometry-log")))
            .and_then(|log| log.0.get(name).cloned())
            .is_some_and(|sample| {
                let [x, y, w, h] = sample.bounds;
                let [cx, cy, cw, ch] = sample.clip;
                x >= cx - 1.0 && y >= cy - 1.0 && x + w <= cx + cw + 1.0 && y + h <= cy + ch + 1.0
            })
    }
    pub(crate) fn audit_pointer_actions(&mut self, ctx: &egui::Context) {
        if !self
            .preview
            .as_ref()
            .is_some_and(|p| p.config.audit_actions)
        {
            return;
        }
        let mut checks = Vec::new();
        macro_rules! check {($name:expr,$ok:expr)=>{checks.push(json!({"action":$name,"passed":$ok,"evidence":"real egui pointer events, in-memory dispatcher"}));};}
        self.audit_reset(ctx);
        self.audit_click(ctx, 436.0, 35.0);
        check!("navigation Discover", self.page == Page::Discover);
        self.audit_click(ctx, 1439.0, 144.0);
        check!(
            "Discover video filter",
            self.discover_filter == DiscoverFilter::Video
        );
        self.audit_click(ctx, 1525.0, 144.0);
        check!(
            "Discover image filter",
            self.discover_filter == DiscoverFilter::Image
        );
        self.audit_click(ctx, 1525.0, 490.0);
        check!(
            "Discover View all",
            self.page == Page::Library && self.filter == Filter::Wallpapers
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1490.0, 138.0);
        check!("hero favorite", self.assets[0]["favorite"] == true);
        self.audit_click(ctx, 1466.0, 330.0);
        check!("hero audio", self.assets[0]["audioEnabled"] == false);
        self.audit_click(ctx, 1524.0, 332.0);
        check!(
            "hero video preview",
            self.preview.as_ref().unwrap().video_playing
        );
        self.audit_click(ctx, 1134.0, 424.0);
        check!("wallpaper loop", self.assets[0]["loop"] == false);
        self.audit_click(ctx, 1388.0, 492.0);
        check!("library list view", !self.grid_view);
        self.audit_reset(ctx);
        self.audit_click(ctx, 350.0, 492.0);
        self.audit_frame(ctx, vec![egui::Event::Text("pines".into())]);
        check!(
            "library search",
            self.search == "pines" && self.assets.iter().filter(|a| self.matches(a)).count() == 1
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1470.0, 492.0);
        check!("import opens review", self.import_files.is_some());
        self.import_files = None;
        self.audit_frame(ctx, vec![]);
        self.busy = true;
        self.audit_click(ctx, 1470.0, 492.0);
        check!("busy import is disabled", self.import_files.is_none());
        self.audit_reset(ctx);
        self.audit_click(ctx, 560.0, 35.0);
        check!("navigation Animations", self.page == Page::Animations);
        let actor = self.selected_actor.clone().unwrap();
        self.audit_click(ctx, 1140.0, 462.0);
        check!("character disable", self.actor_ui_state(&actor).is_none());
        self.audit_click(ctx, 582.0, 646.0);
        check!(
            "character Mirror",
            self.actor_drafts.get(&actor).is_some_and(|s| s.flip_x)
        );
        self.audit_click(ctx, 1010.0, 524.0);
        check!(
            "appearance Reset",
            self.actor_drafts.get(&actor).is_some_and(|s| !s.flip_x)
        );
        self.audit_click(ctx, 1426.0, 36.0);
        self.settings_backdrop_requested = Some(Instant::now() - Duration::from_secs(2));
        self.audit_frame(ctx, vec![]);
        self.audit_frame(ctx, vec![]);
        check!(
            "Settings opens draft",
            self.page == Page::Settings && self.settings_draft.is_some()
        );
        self.audit_click(ctx, 1268.0, 480.0);
        check!(
            "frame-rate stepper",
            self.settings_draft
                .as_ref()
                .is_some_and(|s| s["frameRateLimit"] == 75)
        );
        self.audit_click(ctx, 1266.0, 354.0);
        check!(
            "pause fullscreen switch",
            self.settings_draft
                .as_ref()
                .is_some_and(|s| s["pauseFullscreen"] == false)
        );
        self.audit_click(ctx, 365.0, 309.0);
        check!("Settings Audio navigation", self.settings_tab == 1);
        self.audit_click(ctx, 365.0, 356.0);
        check!("Settings General navigation", self.settings_tab == 2);
        self.audit_click(ctx, 365.0, 402.0);
        check!("Settings Library navigation", self.settings_tab == 3);
        self.audit_click(ctx, 365.0, 448.0);
        check!("Settings About navigation", self.settings_tab == 4);
        self.audit_click(ctx, 1086.0, 835.0);
        check!(
            "Settings Cancel preserves committed values",
            self.page == Page::Animations
                && self.settings_draft.is_none()
                && self.settings["frameRateLimit"] == 60
        );
        self.audit_click(ctx, 1426.0, 36.0);
        self.settings_tab = 0;
        self.settings_backdrop_requested = Some(Instant::now() - Duration::from_secs(2));
        self.audit_frame(ctx, vec![]);
        self.audit_frame(ctx, vec![]);
        self.audit_click(ctx, 1268.0, 480.0);
        self.audit_click(ctx, 1216.0, 835.0);
        check!(
            "Settings Save commits draft",
            self.settings_draft.is_none()
                && self.settings["frameRateLimit"] == 75
                && self.page == Page::Animations
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1354.0, 956.0);
        check!("playlist pause", !self.rotation_running);
        self.audit_reset(ctx);
        self.audit_click(ctx, 958.0, 492.0);
        check!("advanced filters open", self.filter_menu_open);
        self.audit_click(ctx, 970.0, 635.0);
        self.audit_click(ctx, 1240.0, 702.0);
        check!(
            "type and favorites filters combine",
            self.filter == Filter::Wallpapers && self.favorites_only
        );
        self.audit_click(ctx, 1100.0, 780.0);
        check!("filter results close popup", !self.filter_menu_open);
        self.audit_click(ctx, 958.0, 492.0);
        self.audit_click(ctx, 1240.0, 545.0);
        check!(
            "Clear all resets filters",
            self.filter == Filter::All && !self.favorites_only
        );
        self.audit_reset(ctx);
        let count = self.assets.len();
        self.audit_click(ctx, 1470.0, 492.0);
        self.audit_click(ctx, 1030.0, 540.0);
        check!(
            "import character choice",
            self.import_kind == crate::ImportKind::Character
        );
        self.audit_click(ctx, 954.0, 697.0);
        check!(
            "import Cancel preserves library",
            self.import_files.is_none() && self.assets.len() == count
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 360.0, 956.0);
        check!(
            "Manage playlist opens draft",
            self.playlist_dialog_open && self.playlist_draft == self.playlist
        );
        let before = self.playlist_draft.clone();
        self.audit_click(ctx, 1050.0, 366.0);
        check!(
            "playlist move down",
            self.playlist_draft.get(1) == before.first()
        );
        self.audit_click(ctx, 1106.0, 366.0);
        check!(
            "playlist remove edits draft only",
            self.playlist_draft.len() + 1 == before.len() && self.playlist == before
        );
        self.audit_click(ctx, 622.0, 636.0);
        check!(
            "playlist interval stepper",
            self.playlist_interval_draft == 31
        );
        self.audit_click(ctx, 1071.0, 749.0);
        check!(
            "playlist Save commits draft",
            !self.playlist_dialog_open
                && self.playlist.len() + 1 == before.len()
                && self.settings["playlistInterval"] == 31
        );
        self.audit_reset(ctx);
        self.selected = self.assets[2]["url"].as_str().map(str::to_owned);
        self.audit_frame(ctx, vec![]);
        self.audit_frame(ctx, vec![]);
        self.audit_click(ctx, 1534.0, 138.0);
        self.audit_click(ctx, 1400.0, 351.0);
        check!(
            "context Remove requests confirmation",
            self.remove_confirmation.is_some()
        );
        self.audit_click(ctx, 823.0, 578.0);
        check!(
            "removal Cancel preserves library",
            self.remove_confirmation.is_none() && self.assets.len() == count
        );
        self.audit_click(ctx, 1534.0, 138.0);
        self.audit_click(ctx, 1400.0, 351.0);
        self.audit_click(ctx, 917.0, 578.0);
        check!(
            "removal confirmation dispatches action",
            self.assets.len() + 1 == count
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1426.0, 36.0);
        self.settings_backdrop_requested = Some(Instant::now() - Duration::from_secs(2));
        self.audit_frame(ctx, vec![]);
        self.audit_frame(ctx, vec![]);
        self.audit_click(ctx, 1268.0, 480.0);
        self.audit_click(ctx, 380.0, 832.0);
        check!(
            "Settings Reset updates draft only",
            self.settings_draft
                .as_ref()
                .is_some_and(|d| d["frameRateLimit"] == 60)
                && self.settings["frameRateLimit"] == 60
        );
        self.audit_frame(
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        check!(
            "Settings Escape cancels draft",
            self.page == Page::Library && self.settings_draft.is_none()
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 875.0, 424.0);
        check!(
            "wallpaper volume commits on release",
            self.assets[0]["volume"].as_f64().is_some_and(|v| v > 60.0)
        );
        self.audit_reset(ctx);
        let queue = self.playlist.clone();
        self.audit_click(ctx, 1258.0, 956.0);
        check!(
            "shuffle preserves playlist",
            self.playlist == queue && self.settings["playlistRandom"] == false
        );
        self.audit_click(ctx, 560.0, 35.0);
        self.audit_click(ctx, 1360.0, 405.0);
        check!("inspector sound opens editor", self.audio_dialog.is_some());
        self.audit_frame(
            ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        self.audit_frame(ctx, vec![]);
        check!(
            "sound Escape preserves tracks",
            self.audio_dialog.is_none()
                && self.assets[3]["audio"]["files"]
                    .as_array()
                    .is_some_and(|f| f.len() == 1)
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 560.0, 35.0);
        let first = self.assets[3]["url"].as_str().unwrap().to_owned();
        let second = self.assets[8]["url"].as_str().unwrap().to_owned();
        self.audit_click(ctx, 990.0, 873.0);
        check!(
            "gallery enables another character",
            self.actor_ui_state(&first).is_some() && self.actor_ui_state(&second).is_some()
        );
        self.audit_click(ctx, 800.0, 810.0);
        self.audit_click(ctx, 1140.0, 462.0);
        check!(
            "inspector disables selected character only",
            self.actor_ui_state(&first).is_some() && self.actor_ui_state(&second).is_none()
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1252.0, 492.0);
        self.audit_click(ctx, 1252.0, 578.0);
        check!("sort Name A–Z", self.sort_by_name);
        self.audit_reset(ctx);
        self.audit_click(ctx, 360.0, 430.0);
        let option = ctx
            .data(|d| d.get_temp::<crate::preview::GeometryLog>(egui::Id::new("ui-geometry-log")))
            .unwrap()
            .0["Fit option contain"]
            .bounds;
        self.audit_click(
            ctx,
            option[0] + option[2] / 2.0,
            option[1] + option[3] / 2.0,
        );
        check!(
            "Fit saves selected mode",
            self.assets[0]["fit"] == "contain"
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1534.0, 138.0);
        self.audit_click(ctx, 1400.0, 252.0);
        check!(
            "context favorite closes menu",
            self.assets[0]["favorite"] == true && !self.hero_menu_open
        );
        self.audit_reset(ctx);
        self.playlist.truncate(1);
        self.selected = self.assets[2]["url"].as_str().map(str::to_owned);
        let add_url = self.selected.clone().unwrap();
        self.audit_frame(ctx, vec![]);
        self.audit_click(ctx, 1534.0, 138.0);
        self.audit_click(ctx, 1400.0, 216.0);
        check!(
            "context Add to playlist",
            self.playlist.len() == 2 && self.playlist.last() == Some(&add_url)
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 360.0, 956.0);
        let before = self.playlist_draft.clone();
        self.audit_drag(
            ctx,
            egui::pos2(550.0, 366.0),
            egui::pos2(620.0, 470.0),
            false,
        );
        check!(
            "playlist drag reorders draft only",
            self.playlist_draft.get(2) == before.first() && self.playlist == before
        );
        self.audit_click(ctx, 954.0, 749.0);
        check!(
            "playlist Cancel preserves committed order",
            !self.playlist_dialog_open && self.playlist == before
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 560.0, 35.0);
        let actor = self.selected_actor.clone().unwrap();
        self.audit_drag(
            ctx,
            egui::pos2(748.0, 350.0),
            egui::pos2(695.0, 310.0),
            true,
        );
        let pose = self.actor_drafts[&actor];
        check!(
            "preview drag updates normalized position",
            pose.x < 0.72 && pose.y < 0.92
        );
        check!(
            "Shift drag snaps to four-point grid",
            (pose.x * 1000.0 / 4.0).fract().abs() < 0.001
                && (pose.y * 288.0 / 4.0).fract().abs() < 0.001
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 560.0, 35.0);
        self.audit_click(ctx, 944.0, 456.0);
        check!(
            "preview pause updates character settings",
            self.actor_drafts[&actor].playing == false
        );
        self.audit_click(ctx, 992.0, 456.0);
        check!(
            "preview mute saves character audio",
            self.assets[3]["audio"]["enabled"] == false
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 1410.0, 956.0);
        check!(
            "dock Next advances rotation",
            self.rotation_current_url.as_deref() == self.assets[1]["url"].as_str()
                && self.rotation_running
        );
        self.audit_click(ctx, 1302.0, 956.0);
        check!(
            "dock Previous restores preceding wallpaper",
            self.rotation_current_url.as_deref() == self.assets[0]["url"].as_str()
        );
        self.selected = self.assets[2]["url"].as_str().map(str::to_owned);
        self.audit_frame(ctx, vec![]);
        self.audit_click(ctx, 1470.0, 424.0);
        check!(
            "Apply button uses selected wallpaper",
            self.wallpaper_assignments["Display 1"] == self.assets[2]["url"].as_str().unwrap()
                && !self.rotation_running
        );
        self.audit_reset(ctx);
        self.audit_click(ctx, 560.0, 35.0);
        self.audit_click(ctx, 1360.0, 405.0);
        self.audit_click_recorded(ctx, "Sound enabled");
        check!(
            "sound editor preserves draft until Done",
            self.audio_dialog
                .as_ref()
                .is_some_and(|d| d.audio["files"][0]["playback"]["enabled"] == false)
                && self.assets[3]["audio"]["files"][0]["playback"]["enabled"] != false
        );
        self.audit_click_recorded(ctx, "Sound Done");
        check!(
            "sound Done commits and closes editor",
            self.audio_dialog.is_none()
                && self.assets[3]["audio"]["files"][0]["playback"]["enabled"] == false
        );
        self.audit_click(ctx, 1360.0, 405.0);
        self.audit_click_recorded(ctx, "Sound enabled");
        self.audit_click_recorded(ctx, "Sound Cancel");
        check!(
            "sound Cancel discards edited draft",
            self.audio_dialog.is_none()
                && self.assets[3]["audio"]["files"][0]["playback"]["enabled"] == false
        );
        for size in [Vec2::new(1280.0, 800.0), Vec2::new(960.0, 720.0)] {
            self.audit_reset(ctx);
            self.page = Page::Animations;
            for _ in 0..3 {
                self.audit_frame_at_size(ctx, vec![], egui::Modifiers::NONE, size);
            }
            self.audit_scroll_at_size(ctx, size, -5000.0);
            check!(
                format!("Animations gallery reachable at {}×{}", size.x, size.y),
                Self::audit_component_visible(ctx, "Sora Character Row")
            );
            check!(
                format!("Playback controls reachable at {}×{}", size.x, size.y),
                Self::audit_component_visible(ctx, "Playback Panel")
            );
            self.audit_scroll_at_size(ctx, size, 5000.0);
            self.open_settings(ctx);
            self.settings_backdrop_requested = Some(Instant::now() - Duration::from_secs(2));
            for _ in 0..3 {
                self.audit_frame_at_size(ctx, vec![], egui::Modifiers::NONE, size);
            }
            self.audit_scroll_at_size(ctx, size, -5000.0);
            check!(
                format!("Settings lower controls reachable at {}×{}", size.x, size.y),
                Self::audit_component_visible(ctx, "Preview quality select")
            );
            self.audit_scroll_at_size(ctx, size, 5000.0);
            self.cancel_settings();
        }
        let output = self
            .preview
            .as_ref()
            .unwrap()
            .config
            .output
            .join("pointer-audit.json");
        std::fs::write(output, serde_json::to_vec_pretty(&checks).unwrap()).unwrap();
        let passed = checks.iter().filter(|c| c["passed"] == true).count();
        eprintln!("UI_POINTER_AUDIT {passed}/{}", checks.len());
        self.audit_reset(ctx);
        let uploads = std::mem::take(&mut self.preview.as_mut().unwrap().audit_textures);
        let textures = ctx.tex_manager();
        let mut textures = textures.write();
        for (id, delta) in uploads {
            if textures.meta(id).is_some() {
                textures.set(id, delta);
            }
        }
    }
}
