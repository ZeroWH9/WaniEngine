//! Deterministic UI data and framebuffer capture. No live desktop service is started.
use crate::{App, DiscoverFilter, Filter, ImportKind, Page, location::LibraryLocation};
use eframe::egui::{self, Vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};
use waniengine_core::library::file_url;
use waniengine_egui::overlay::{Monitor, WorkerStatus};

#[derive(Clone, Default, Serialize)]
pub(crate) struct GeometryLog(pub std::collections::BTreeMap<String, GeometrySample>);
#[derive(Clone, Serialize)]
pub(crate) struct GeometrySample {
    pub bounds: [f32; 4],
    pub clip: [f32; 4],
}
fn rect_values(rect: egui::Rect) -> [f32; 4] {
    [rect.left(), rect.top(), rect.width(), rect.height()]
}
pub(crate) fn record_geometry(ui: &egui::Ui, name: impl Into<String>, rect: egui::Rect) {
    let id = egui::Id::new("ui-geometry-log");
    if ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("ui-geometry-enabled")))
        != Some(true)
    {
        return;
    }
    ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<GeometryLog>(id).0.insert(
            name.into(),
            GeometrySample {
                bounds: rect_values(rect),
                clip: rect_values(ui.clip_rect()),
            },
        );
    });
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiCapabilities {
    pub wallpaper: bool,
    pub video: bool,
    pub audio: bool,
    pub overlay: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub library_count: usize,
    pub collection_wallpaper_count: usize,
    pub import_selection: Vec<UiImportFile>,
    pub assets: Vec<Value>,
    pub settings: Value,
    pub playlist: Vec<String>,
    pub active_actor: Option<String>,
    pub active_actors: std::collections::HashMap<String, String>,
    pub capabilities: UiCapabilities,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiImportFile {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
    pub bytes: u64,
    pub video: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UiAction {
    Operation {
        name: String,
    },
    Favorite {
        url: String,
        value: bool,
    },
    AssetProperty {
        url: String,
        key: String,
        value: Value,
    },
    Apply {
        url: String,
        rotation: bool,
    },
    ToggleActor {
        url: String,
    },
    ActorSettings {
        url: String,
        value: Value,
    },
    CharacterAudio {
        url: String,
        audio: Value,
        volume: f64,
    },
    Playlist {
        urls: Vec<String>,
    },
    Remove {
        url: String,
    },
    OpenFolder {
        path: String,
    },
    Import,
    Settings {
        value: Value,
    },
}

#[derive(Clone)]
pub struct PreviewConfig {
    pub audit_actions: bool,
    pub output: PathBuf,
    pub scenarios: Vec<(String, u32, u32)>,
}
impl PreviewConfig {
    pub fn parse(args: &[OsString]) -> Option<Self> {
        if !args.iter().any(|a| a == "--ui-preview") {
            return None;
        }
        let arg = |key: &str| {
            args.iter()
                .position(|a| a == key)
                .and_then(|i| args.get(i + 1))
                .map(|s| s.to_string_lossy().to_string())
        };
        let output = arg("--ui-output-dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/ui-preview"));
        let names=arg("--ui-scenarios").unwrap_or_else(||"library,discover,animations,settings,settings-audio,settings-general,settings-library,settings-about,import,playlist,remove,filters,context,empty,no-results,loading,error".into());
        let sizes = arg("--ui-sizes").unwrap_or_else(|| "1600x1000,1280x800,960x720".into());
        let mut scenarios = Vec::new();
        for size in sizes.split(',') {
            let (w, h) = size.split_once('x').expect("--ui-sizes WIDTHxHEIGHT");
            for name in names.split(',') {
                scenarios.push((
                    name.to_owned(),
                    w.parse().expect("width"),
                    h.parse().expect("height"),
                ));
            }
        }
        Some(Self {
            audit_actions: args.iter().any(|a| a == "--ui-audit-actions"),
            output,
            scenarios,
        })
    }
}
pub struct PreviewState {
    pub config: PreviewConfig,
    pub index: usize,
    frames: usize,
    pending: bool,
    started: Instant,
    pub snapshot: UiSnapshot,
    pub video_playing: bool,
    pub audit_textures: Vec<(egui::TextureId, egui::epaint::ImageDelta)>,
    pub actions: Vec<UiAction>,
}
impl PreviewState {
    pub fn new(config: PreviewConfig) -> Self {
        Self {
            config,
            index: 0,
            frames: 0,
            pending: false,
            started: Instant::now(),
            snapshot: fixture(),
            video_playing: false,
            audit_textures: Vec::new(),
            actions: Vec::new(),
        }
    }
}

fn asset_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/preview")
}
fn fixture() -> UiSnapshot {
    let data = [
        (
            "Aurora Veil",
            "wp_aurora.jpg",
            true,
            false,
            3840,
            2160,
            24,
            "4K",
        ),
        (
            "Sakura Drift",
            "wp_sakura.jpg",
            true,
            false,
            2560,
            1440,
            18,
            "2K",
        ),
        (
            "Neon Tide",
            "wp_neon.jpg",
            true,
            false,
            3840,
            2160,
            32,
            "4K",
        ),
        ("Mochi", "char_mochi.png", false, true, 240, 320, 0, "LIVE"),
        (
            "Ember Dunes",
            "wp_dunes.jpg",
            false,
            false,
            5120,
            2880,
            0,
            "5K",
        ),
        (
            "Moonlit Pines",
            "wp_pines.jpg",
            true,
            false,
            3840,
            2160,
            72,
            "4K",
        ),
        (
            "Ribbon Dreams",
            "wp_ribbon.jpg",
            true,
            false,
            3840,
            2160,
            15,
            "4K",
        ),
        ("Koi Pond", "wp_koi.jpg", false, false, 1920, 1080, 0, "GIF"),
        ("Yuki", "char_ghost.png", false, true, 240, 320, 0, "LIVE"),
        (
            "Solstice Bloom",
            "wp_bloom.jpg",
            false,
            false,
            3840,
            2160,
            0,
            "4K",
        ),
        (
            "Quiet Hours",
            "wp_solstice.jpg",
            false,
            false,
            5120,
            2880,
            0,
            "5K",
        ),
        ("Sora", "char_fox.png", false, true, 240, 320, 0, "LIVE"),
    ];
    let mut assets=data.iter().enumerate().map(|(i,(name,file,video,character,w,h,duration,badge))| {
        let path=asset_dir().join(file);
        json!({"name":name,"url":file_url(&path),"thumbnailUrl":file_url(&path),"kind":if *character {"character"} else {"wallpaper"},"video":video,"animated":character,"extension":if *video {"mp4"}else if *character {"png"}else{"jpg"},"width":w,"height":h,"nativeFps":if *video || *character {60}else{0},"duration":duration,"fileSize":24800000,"favorite":matches!(i,1|3|11),"addedAt":100-i,"missing":false,"hasAudio":video,"audioEnabled":true,"volume":if *character {35}else{50},"loop":true,"fit":"cover","scale":78,"posX":72,"posY":92,"resolutionLabel":badge,"stageColor":if i==3 {"mochi"}else if i==8 {"yuki"}else{"sora"},"characterLoop":if i==3 {"Idle loop"}else if i==8 {"Float loop"}else{"Tail wag"},"audio":{"enabled":true,"files":[{"name":"Footsteps","url":"file:///preview/footsteps.wav","playback":{"mode":"loop"}}]}})
    }).collect::<Vec<_>>();
    for (rank, index) in [2, 1, 5, 6, 4, 10].into_iter().enumerate() {
        assets[index]["discoverRank"] = json!(rank);
    }
    assets[7]["extension"] = json!("gif");
    let playlist = [0, 1, 2, 5, 6, 4, 7, 9, 10, 0, 1, 2]
        .iter()
        .map(|i| assets[*i]["url"].as_str().unwrap().to_owned())
        .collect();
    UiSnapshot {
        library_count: 24,
        collection_wallpaper_count: 7,
        import_selection: [
            ("wp_ribbon.jpg", "aurora-veil.mp4", "MP4", 24_800_000, true),
            (
                "wp_sakura.jpg",
                "sakura-drift.webm",
                "WEBM",
                18_600_000,
                true,
            ),
            ("wp_neon.jpg", "neon-tide.webp", "WEBP", 40_800_000, false),
        ]
        .into_iter()
        .map(|(file, name, extension, bytes, video)| UiImportFile {
            path: asset_dir().join(file),
            name: name.into(),
            extension: extension.into(),
            bytes,
            video,
        })
        .collect(),
        active_actor: Some(assets[3]["url"].as_str().unwrap().to_owned()),
        active_actors: [(
            assets[3]["url"].as_str().unwrap().to_owned(),
            "Display 1".to_owned(),
        )]
        .into_iter()
        .collect(),
        assets,
        playlist,
        settings: json!({"pauseFullscreen":true,"pauseMaximized":false,"frameRateLimit":60,"msaaSamples":4,"previewQuality":"High","globalVolume":100,"playlistInterval":30,"playlistRandom":true,"closeToTray":true,"restoreOnLaunch":true}),
        capabilities: UiCapabilities {
            wallpaper: true,
            video: true,
            audio: true,
            overlay: true,
        },
    }
}

pub fn run(config: PreviewConfig) -> eframe::Result {
    std::fs::create_dir_all(&config.output).expect("preview output directory");
    let (_, w, h) = config.scenarios.first().expect("preview scenarios");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("org.waniengine.UiPreview")
            .with_inner_size([*w as f32, *h as f32])
            .with_transparent(true)
            .with_decorations(false),
        ..Default::default()
    };
    let root = config.output.join("isolated-library");
    let location = LibraryLocation {
        root: root.clone(),
        legacy: root.clone(),
        installed_scene: root.join("scene.json"),
        migrate: false,
        start_in_tray: false,
    };
    eframe::run_native(
        "WaniEngine UI preview",
        options,
        Box::new(move |cc| {
            Ok(Box::new(App::new(
                cc,
                location,
                None,
                None,
                false,
                Some(config),
            )))
        }),
    )
}

impl App {
    pub(crate) fn audit_preview_actions(&mut self, ctx: &egui::Context) {
        if !self
            .preview
            .as_ref()
            .is_some_and(|p| p.config.audit_actions)
        {
            return;
        }
        let original = self.assets.clone();
        let url = original[0]["url"].as_str().unwrap().to_owned();
        let actor = original[3]["url"].as_str().unwrap().to_owned();
        let mut checks = Vec::new();
        self.toggle_favorite(url.clone(), true, ctx);
        checks.push(json!({"action":"favorite","passed":self.assets[0]["favorite"]==true}));
        self.update_asset_property(url.clone(), "volume", json!(67), ctx);
        checks.push(json!({"action":"wallpaper volume","passed":self.assets[0]["volume"]==67}));
        self.apply_wallpaper(&original[2], ctx, false);
        checks.push(json!({"action":"apply wallpaper","passed":self.wallpaper_assignments["Display 1"]==original[2]["url"].as_str().unwrap()}));
        self.toggle_character(&original[3], ctx);
        checks.push(
            json!({"action":"disable character","passed":self.actor_ui_state(&actor).is_none()}),
        );
        self.toggle_character(&original[3], ctx);
        checks.push(
            json!({"action":"enable character","passed":self.actor_ui_state(&actor).is_some()}),
        );
        let second_actor = original[8]["url"].as_str().unwrap().to_owned();
        self.toggle_character(&original[8], ctx);
        checks.push(json!({"action":"enable multiple characters","passed":self.actor_ui_state(&actor).is_some()&&self.actor_ui_state(&second_actor).is_some()}));
        self.toggle_character(&original[8], ctx);
        checks.push(json!({"action":"disable one character preserves others","passed":self.actor_ui_state(&actor).is_some()&&self.actor_ui_state(&second_actor).is_none()}));
        let mut settings = crate::animations::initial_settings(&original[3]);
        settings.x = 0.41;
        settings.y = 0.83;
        settings.flip_x = true;
        settings.looping = false;
        self.save_actor_settings(actor.clone(), settings, ctx);
        checks.push(json!({"action":"save character settings round trip","passed":self.assets.iter().find(|a|a["url"].as_str()==Some(&actor)).is_some_and(|a|crate::animations::initial_settings(a)==settings)}));
        self.save_character_audio(
            actor.clone(),
            json!({"enabled":false,"files":[]}),
            22.0,
            ctx,
        );
        checks.push(json!({"action":"save character audio","passed":self.assets.iter().find(|a|a["url"].as_str()==Some(&actor)).unwrap()["volume"]==22.0}));
        let playlist = vec![url.clone()];
        self.save_playlist(playlist.clone(), ctx);
        checks.push(json!({"action":"save playlist","passed":self.playlist==playlist}));
        self.settings_draft = Some(json!({"frameRateLimit":90}));
        self.save_settings(ctx);
        checks.push(json!({"action":"save settings","passed":self.settings["frameRateLimit"]==90&&self.settings_draft.is_none()}));
        self.open_folder(std::path::Path::new("/preview/isolated"));
        checks.push(json!({"action":"open folder boundary","passed":self.status.as_deref()==Some("Opening containing folder.")}));
        let removed = original[2]["url"].as_str().unwrap().to_owned();
        self.remove_selected(removed.clone(), ctx);
        checks.push(json!({"action":"remove content","passed":!self.assets.iter().any(|a|a["url"].as_str()==Some(&removed))}));
        let path = self
            .preview
            .as_ref()
            .unwrap()
            .config
            .output
            .join("action-audit.json");
        std::fs::write(path, serde_json::to_vec_pretty(&checks).unwrap()).unwrap();
        assert!(
            checks.iter().all(|c| c["passed"] == true),
            "UI action boundary audit failed: {checks:?}"
        );
        self.seed_preview(ctx);
    }

    pub(crate) fn seed_preview(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        preview.snapshot = fixture();
        ctx.data_mut(|d| {
            d.insert_temp(egui::Id::new("ui-geometry-enabled"), true);
            d.insert_temp(egui::Id::new("ui-geometry-log"), GeometryLog::default());
        });
        preview.video_playing = false;
        let name = preview.config.scenarios[preview.index].0.clone();
        self.assets = preview.snapshot.assets.clone();
        self.settings = preview.snapshot.settings.clone();
        self.settings_draft = None;
        self.playlist = preview.snapshot.playlist.clone();
        self.playlist_name = "Evening rotation".into();
        self.managed_urls = self
            .assets
            .iter()
            .filter_map(|a| a["url"].as_str().map(str::to_owned))
            .collect::<HashSet<_>>();
        self.selected = self
            .assets
            .first()
            .and_then(|a| a["url"].as_str().map(str::to_owned));
        self.selected_actor = preview.snapshot.active_actor.clone();
        self.saved_actor_urls = self.selected_actor.iter().cloned().collect();
        self.actor_drafts.clear();
        self.actor_search.clear();
        self.search.clear();
        self.filter = Filter::All;
        self.favorites_only = false;
        self.sort_by_name = false;
        self.grid_view = true;
        self.discover_filter = DiscoverFilter::All;
        self.page = Page::Library;
        self.settings_return_page = Page::Library;
        self.settings_backdrop = None;
        self.settings_backdrop_requested = None;
        self.settings_tab = 0;
        self.import_files = None;
        self.remove_confirmation = None;
        self.playlist_dialog_open = false;
        self.playlist_drag_index = None;
        self.playlist_dirty = false;
        self.wallpaper_volume_draft = None;
        self.character_volume_draft = None;
        self.hero_menu_open = false;
        self.filter_menu_open = false;
        self.audio_dialog = None;
        self.selected_output = Some("Display 1".into());
        self.wallpaper_assignments.clear();
        self.wallpaper_assignments
            .insert("Display 1".into(), self.selected.clone().unwrap());
        self.rotation_running = true;
        self.rotation_waiting = false;
        self.rotation_deadline = Some(Instant::now() + Duration::from_secs(1104));
        self.rotation_current_url = self.selected.clone();
        self.restore_actors_pending = false;
        self.restore_wallpapers_pending = false;
        self.busy = false;
        self.status = None;
        self.error = None;
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.monitors = vec![Monitor {
                name: Some("Display 1".into()),
                description: Some("Display 1".into()),
                logical_position: Some((0, 0)),
                logical_size: Some((3840, 2160)),
                scale: 1,
            }];
            overlay.loading_monitors = false;
            overlay.notice = None;
        }
        if preview.index == 0
            && !self
                .thumbnails
                .has_cached(self.assets[0]["thumbnailUrl"].as_str().unwrap())
        {
            self.thumbnails.clear();
            for a in &self.assets {
                let url = a["thumbnailUrl"].as_str().unwrap();
                if let Ok(path) = waniengine_core::library::file_path(url)
                    && let Ok(img) = image::open(path)
                {
                    self.thumbnails.insert_preview(ctx, url, &img.to_rgba8());
                }
            }
        }
        match name.as_str() {
            "discover" => self.page = Page::Discover,
            "animations" => {
                self.page = Page::Animations;
                self.selected = self
                    .assets
                    .get(5)
                    .and_then(|a| a["url"].as_str().map(str::to_owned));
            }
            n if n.starts_with("settings") => {
                self.page = Page::Settings;
                self.settings_draft = Some(self.settings.clone());
                self.settings_tab = match n {
                    "settings-audio" => 1,
                    "settings-general" => 2,
                    "settings-library" => 3,
                    "settings-about" => 4,
                    _ => 0,
                };
            }
            "sound" => {
                self.page = Page::Animations;
                let actor = self.assets[3].clone();
                self.audio_dialog = Some(crate::AudioDialog {
                    actor_url: actor["url"].as_str().unwrap().into(),
                    track_index: 0,
                    audio: crate::animations::audio_defaults(&actor),
                    volume: 35.0,
                });
            }
            "unavailable" => {
                preview.snapshot.capabilities = UiCapabilities {
                    wallpaper: false,
                    video: false,
                    audio: false,
                    overlay: false,
                };
            }
            "import" => {
                self.import_files = Some(vec![
                    asset_dir().join("wp_ribbon.jpg"),
                    asset_dir().join("wp_sakura.jpg"),
                    asset_dir().join("wp_neon.jpg"),
                ]);
                self.import_kind = ImportKind::Wallpaper;
            }
            "playlist" => {
                self.playlist_dialog_open = true;
                self.playlist_draft = self.playlist.iter().take(5).cloned().collect();
                self.playlist_interval_draft = 30;
                self.playlist_random_draft = true;
            }
            "remove" => {
                self.remove_confirmation =
                    Some((self.selected.clone().unwrap(), "Aurora Veil".into()))
            }
            "filters" => {
                self.filter_menu_open = true;
                self.favorites_only = true;
            }
            "context" => self.hero_menu_open = true,
            "empty" => {
                preview.snapshot.library_count = 0;
                self.assets.clear();
                self.selected = None;
                self.playlist.clear();
                self.rotation_running = false;
            }
            "no-results" => self.search = "No matching wallpaper".into(),
            "loading" => {
                self.busy = true;
                self.status = Some("Loading content…".into());
            }
            "error" => self.status = Some("Could not save changes. Try again.".into()),
            _ => {}
        }
        ctx.set_pixels_per_point(1.0);
    }

    pub(crate) fn preview_tick(&mut self, ctx: &egui::Context) {
        let Some(preview) = self.preview.as_mut() else {
            return;
        };
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot {
                    image, user_data, ..
                } = e
                {
                    if user_data.data.is_some() {
                        return None;
                    }
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let (name, w, h) = &preview.config.scenarios[preview.index];
            let path = preview.config.output.join(format!("{name}-{w}x{h}.png"));
            if let Some(geometry) =
                ctx.data(|d| d.get_temp::<GeometryLog>(egui::Id::new("ui-geometry-log")))
            {
                std::fs::write(
                    preview
                        .config
                        .output
                        .join(format!("{name}-{w}x{h}-geometry.json")),
                    serde_json::to_vec_pretty(&geometry.0).unwrap(),
                )
                .unwrap();
            }
            let pixels = image
                .pixels
                .iter()
                .flat_map(|p| p.to_array())
                .collect::<Vec<_>>();
            assert_eq!(
                image.size,
                [*w as usize, *h as usize],
                "preview framebuffer size must match the reference"
            );
            image::save_buffer(
                &path,
                &pixels,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .expect("save UI framebuffer");
            eprintln!("UI_FRAME {}", path.display());
            preview.index += 1;
            preview.frames = 0;
            preview.pending = false;
            preview.started = Instant::now();
            if preview.index == preview.config.scenarios.len() {
                std::fs::write(
                    preview.config.output.join("actions.json"),
                    serde_json::to_vec_pretty(&preview.actions).unwrap(),
                )
                .unwrap();
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            let (_, w, h) = preview.config.scenarios[preview.index];
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                w as f32, h as f32,
            )));
            self.seed_preview(ctx);
            return;
        }
        if preview.started.elapsed() > Duration::from_secs(60) {
            panic!("UI preview timed out waiting for framebuffer");
        }
        preview.frames += 1;
        if preview.frames >= 6 && !preview.pending {
            preview.pending = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }

    pub(crate) fn ui_capabilities(&self) -> UiCapabilities {
        self.preview
            .as_ref()
            .map(|p| p.snapshot.capabilities.clone())
            .unwrap_or_else(|| UiCapabilities {
                wallpaper: self.wallpaper.is_gnome() || self.selected_output.is_some(),
                video: self.wallpaper.supports_video(),
                audio: !self.wallpaper.is_gnome() || self.wallpaper.supports_video(),
                overlay: self.overlay.is_some() && self.selected_output.is_some(),
            })
    }

    pub(crate) fn actor_ui_state(&self, url: &str) -> Option<(WorkerStatus, String)> {
        if let Some(preview) = self.preview.as_ref() {
            return preview
                .snapshot
                .active_actors
                .get(url)
                .map(|monitor| (WorkerStatus::Running, monitor.clone()));
        }
        self.overlay
            .as_ref()
            .and_then(|o| o.worker(url))
            .map(|w| (w.status, w.monitor.clone()))
    }

    pub(crate) fn preview_action(&mut self, action: UiAction) -> bool {
        let Some(preview) = self.preview.as_mut() else {
            return false;
        };
        preview.actions.push(action.clone());
        match action {
            UiAction::Operation { name } => {
                if name == "video_preview" {
                    preview.video_playing = !preview.video_playing;
                }
                if name == "save_playlist_configuration" {
                    self.settings["playlistInterval"] = json!(self.playlist_interval_draft);
                    self.settings["playlistRandom"] = json!(self.playlist_random_draft);
                    self.playlist_dialog_open = false;
                }
                if name == "import_confirm" {
                    self.import_files = None;
                }
                self.status = Some(format!("{name} completed."));
            }
            UiAction::Favorite { url, value } => {
                if let Some(a) = self
                    .assets
                    .iter_mut()
                    .find(|a| a["url"].as_str() == Some(&url))
                {
                    a["favorite"] = json!(value);
                }
                self.status = Some("Favorite updated.".into());
            }
            UiAction::AssetProperty { url, key, value } => {
                if let Some(a) = self
                    .assets
                    .iter_mut()
                    .find(|a| a["url"].as_str() == Some(&url))
                {
                    a[key] = value;
                }
                self.status = Some("Wallpaper settings saved.".into());
                self.wallpaper_volume_draft = None;
            }
            UiAction::Apply { url, rotation } => {
                self.wallpaper_assignments.insert(
                    self.selected_output
                        .clone()
                        .unwrap_or_else(|| "Display 1".into()),
                    url.clone(),
                );
                self.rotation_current_url = Some(url);
                self.rotation_waiting = false;
                self.rotation_running = rotation;
                self.status = Some("Wallpaper applied to the selected display.".into());
            }
            UiAction::ToggleActor { url } => {
                if preview.snapshot.active_actors.remove(&url).is_none() {
                    preview.snapshot.active_actors.insert(
                        url.clone(),
                        self.selected_output
                            .clone()
                            .unwrap_or_else(|| "Display 1".into()),
                    );
                }
                self.saved_actor_urls = preview.snapshot.active_actors.keys().cloned().collect();
                self.status = Some("Character overlay updated.".into());
            }
            UiAction::ActorSettings { url, value } => {
                if let Some(a) = self
                    .assets
                    .iter_mut()
                    .find(|a| a["url"].as_str() == Some(&url))
                {
                    if let Some(object) = value.as_object() {
                        for (k, v) in object {
                            if k == "x" || k == "y" {
                                a[if k == "x" { "posX" } else { "posY" }] =
                                    json!(v.as_f64().unwrap_or_default() * 100.0);
                            } else {
                                a[k] = v.clone();
                            }
                        }
                    }
                }
                self.status = Some("Character settings saved.".into());
            }
            UiAction::CharacterAudio { url, audio, volume } => {
                if let Some(a) = self
                    .assets
                    .iter_mut()
                    .find(|a| a["url"].as_str() == Some(&url))
                {
                    a["audio"] = audio;
                    a["volume"] = json!(volume);
                }
                self.character_volume_draft = None;
                self.audio_dialog = None;
                self.status = Some("Character sound settings saved.".into());
            }
            UiAction::Playlist { urls } => {
                self.playlist = urls;
                self.playlist_dialog_open = false;
                self.status = Some("Playlist saved.".into());
            }
            UiAction::Remove { url } => {
                preview.snapshot.library_count = preview.snapshot.library_count.saturating_sub(1);
                self.assets.retain(|a| a["url"].as_str() != Some(&url));
                self.playlist.retain(|u| u != &url);
                self.status =
                    Some("Removed from the library. The original file was preserved.".into());
            }
            UiAction::OpenFolder { .. } => self.status = Some("Opening containing folder.".into()),
            UiAction::Import => {
                self.import_files = Some(vec![asset_dir().join("wp_ribbon.jpg")]);
            }
            UiAction::Settings { value } => {
                self.settings = value;
                self.settings_draft = None;
                self.page = self.settings_return_page;
                self.status = Some("Settings saved.".into());
            }
        }
        true
    }
}
