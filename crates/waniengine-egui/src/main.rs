mod animations;
mod audio;
mod audio_output;
mod autostart;
mod backend_check;
mod control;
mod controls;
pub(crate) use controls::{
    icon_button, icon_text_button, record_volume_response, volume_draft_value,
};
mod dialogs;
mod discover;
mod dock;
mod icon_set;
mod icons;
mod library_actions;
mod location;
mod metadata;
mod preview;
mod settings;
mod thumbnails;
mod tray;
mod ui_audit;
mod ui_session;
mod video_preview;
mod video_wallpaper;
mod widgets;
mod window_state;
mod x11_click;

use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use library_actions::{ImportKind, ImportResult};
use location::LibraryLocation;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Command,
    rc::Rc,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use waniengine_core::library::{file_path, file_url};
use waniengine_egui::{
    desktop::GNOME_OUTPUT,
    overlay::{ActorSettings, OverlayManager, WorkerStatus},
};

const SURFACE: Color32 = Color32::from_rgb(12, 10, 17);
const NAV: Color32 = Color32::from_rgb(18, 16, 23);
const PANEL: Color32 = Color32::from_rgb(23, 20, 31);
const BORDER: Color32 = Color32::from_rgb(42, 37, 52);
const TEXT: Color32 = Color32::from_rgb(245, 242, 250);
const SECONDARY: Color32 = Color32::from_rgb(201, 195, 214);
const MUTED: Color32 = Color32::from_rgb(143, 135, 158);
const ACCENT: Color32 = Color32::from_rgb(140, 130, 240);

/// Draw inside an already allocated rectangle without moving the parent's cursor.
pub(crate) fn child_ui<R>(
    ui: &mut egui::Ui,
    builder: egui::UiBuilder,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let mut child = ui.new_child(builder);
    let inner = add(&mut child);
    egui::InnerResponse {
        inner,
        response: child.response(),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Library,
    Discover,
    Animations,
    Settings,
}
#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Wallpapers,
    Animations,
    Favorites,
}
#[derive(Clone, Copy, PartialEq)]
enum DiscoverFilter {
    All,
    Video,
    Image,
}

enum PlaylistAfterSave {
    Start(String),
    Notify(String),
}

enum TaskResult {
    VideoConfigured(Result<(), String>),
    Files(Vec<PathBuf>),
    AudioFiles {
        actor_url: String,
        files: Vec<PathBuf>,
    },
    Imported {
        result: Result<ImportResult, String>,
        kind: ImportKind,
        start_after: bool,
    },
    AudioAdded {
        actor_url: String,
        result: Result<usize, String>,
    },
    AudioSaved {
        actor_url: String,
        result: Result<(), String>,
    },
    AudioOutputs(Result<Vec<audio::OutputDevice>, String>),
    Favorite(Result<(), String>),
    Removed(Result<(), String>),
    AssetProperty {
        result: Result<(), String>,
        url: String,
        key: &'static str,
    },
    ActorSettings(Result<(), String>),
    Settings(Result<(Value, bool), String>),
    Playlist(Result<(), String>),
    PlaylistConfig(Result<(), String>),
    ActiveActors(Result<(), String>),
    NamedPlaylistSaved(Result<Option<String>, String>),
    NamedPlaylistLoaded(Result<Option<library_actions::LoadedPlaylist>, String>),
    Wallpaper {
        result: Result<(), String>,
        assignment_saved: Result<(), String>,
        rotation: bool,
        restoration: bool,
        select_monitor: bool,
        reset_audio: bool,
        output_name: String,
        url: String,
    },
    WallpaperAnimation {
        result: Result<(), String>,
        enabled: bool,
    },
    Migration(Result<usize, String>),
    WorkerPanicked(String),
}

/// Reports a panicking background worker so the UI never stays busy forever.
struct PanicGuard {
    tx: mpsc::Sender<TaskResult>,
    ctx: egui::Context,
}

impl PanicGuard {
    fn new(tx: mpsc::Sender<TaskResult>, ctx: egui::Context) -> Self {
        Self { tx, ctx }
    }
}

impl Drop for PanicGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = self
                .tx
                .send(TaskResult::WorkerPanicked("worker panicked".into()));
            self.ctx.request_repaint();
        }
    }
}

struct App {
    location: LibraryLocation,
    page: Page,
    filter: Filter,
    favorites_only: bool,
    filter_menu_open: bool,
    discover_filter: DiscoverFilter,
    sort_by_name: bool,
    grid_view: bool,
    search: String,
    focus_library_search: bool,
    assets: Vec<Value>,
    managed_urls: HashSet<String>,
    error: Option<String>,
    selected: Option<String>,
    hero_menu_open: bool,
    selected_actor: Option<String>,
    actor_search: String,
    actor_drafts: HashMap<String, ActorSettings>,
    wallpaper_volume_draft: Option<(String, f32)>,
    character_volume_draft: Option<(String, f32)>,
    audio_track_indices: HashMap<String, usize>,
    audio_dialog: Option<AudioDialog>,
    audio_outputs: Vec<audio::OutputDevice>,
    audio_outputs_loading: bool,
    audio_outputs_error: Option<String>,
    audio_outputs_last_refresh: Option<Instant>,
    selected_output: Option<String>,
    settings_tab: usize,
    settings: Value,
    settings_draft: Option<Value>,
    settings_return_page: Page,
    settings_backdrop: Option<egui::TextureHandle>,
    settings_backdrop_requested: Option<Instant>,
    playlist: Vec<String>,
    playlist_name: String,
    saved_actor_urls: Vec<String>,
    wallpaper_assignments: HashMap<String, String>,
    attempted_wallpaper_restores: HashSet<(String, String)>,
    /// Video surfaces lost to a monitor powering off: attempts and next try.
    wallpaper_retries: HashMap<String, (u32, Instant)>,
    pending_video_reapply: std::collections::VecDeque<(String, String)>,
    last_display_refresh: Instant,
    pending_actor_save: Option<Vec<String>>,
    restore_actors_pending: bool,
    restore_wallpapers_pending: bool,
    start_with_session: bool,
    playlist_dirty: bool,
    playlist_dialog_open: bool,
    playlist_draft: Vec<String>,
    playlist_interval_draft: u64,
    playlist_random_draft: bool,
    playlist_drag_index: Option<usize>,
    playlist_after_save: Option<PlaylistAfterSave>,
    rotation_running: bool,
    rotation_waiting: bool,
    rotation_current_url: Option<String>,
    rotation_deadline: Option<Instant>,
    audio: audio::AudioManager,
    last_audio_sync: Instant,
    wallpaper_video_audio: video_wallpaper::WallpaperAudio,
    video_preview: video_preview::VideoPreview,
    autoplay_video_preview: bool,
    video_texture: Option<egui::TextureHandle>,
    overlay: Option<OverlayManager>,
    wallpaper: ui_session::WallpaperUi,
    wallpaper_animation_sync_pending: bool,
    wallpaper_pause_applied: Option<bool>,
    wallpaper_pause_retry_at: Option<Instant>,
    reconcile_pending: bool,
    pending_import_status: Option<String>,
    window_state: ui_session::WindowMonitorUi,
    automatic_pause: bool,
    pause_saved_playing: HashMap<String, bool>,
    thumbnails: thumbnails::Thumbnails,
    logo: Option<egui::TextureHandle>,
    discover_background: Option<egui::TextureHandle>,
    task_tx: mpsc::Sender<TaskResult>,
    task_rx: mpsc::Receiver<TaskResult>,
    busy: bool,
    import_files: Option<Vec<PathBuf>>,
    import_start_playing: bool,
    remove_confirmation: Option<(String, String)>,
    import_kind: ImportKind,
    status: Option<String>,
    status_snapshot: Option<String>,
    status_since: Option<Instant>,
    control: Option<control::ManagerControl>,
    tray: Option<tray::TrayController>,
    window_hidden: bool,
    force_exit: bool,
    preview: Option<preview::PreviewState>,
}

pub(crate) struct AudioDialog {
    pub(crate) actor_url: String,
    pub(crate) track_index: usize,
    pub(crate) audio: Value,
    pub(crate) volume: f64,
}

impl App {
    /// `ctx` paints the first window (or is `service` when starting in the
    /// tray). Long-lived workers wake `service`, which outlives every window.
    fn new(
        ctx: &egui::Context,
        service: egui::Context,
        location: LibraryLocation,
        control: Option<control::ManagerControl>,
        tray: Option<tray::TrayController>,
        start_in_tray: bool,
        preview_config: Option<preview::PreviewConfig>,
    ) -> Self {
        let previewing = preview_config.is_some();
        install_theme(ctx);
        let (logo, discover_background) = window_textures(ctx);
        let (task_tx, task_rx) = mpsc::channel();
        let mut app = Self {
            location,
            page: Page::Library,
            filter: Filter::All,
            favorites_only: false,
            filter_menu_open: false,
            discover_filter: DiscoverFilter::All,
            sort_by_name: false,
            grid_view: true,
            search: String::new(),
            focus_library_search: false,
            assets: Vec::new(),
            managed_urls: HashSet::new(),
            error: None,
            selected: None,
            hero_menu_open: false,
            selected_actor: None,
            actor_search: String::new(),
            actor_drafts: HashMap::new(),
            wallpaper_volume_draft: None,
            character_volume_draft: None,
            audio_track_indices: HashMap::new(),
            audio_dialog: None,
            audio_outputs: Vec::new(),
            audio_outputs_loading: false,
            audio_outputs_error: None,
            audio_outputs_last_refresh: None,
            selected_output: None,
            settings_tab: 0,
            settings: Value::Object(Default::default()),
            settings_draft: None,
            settings_return_page: Page::Library,
            settings_backdrop: None,
            settings_backdrop_requested: None,
            playlist: Vec::new(),
            playlist_name: "Unsaved".into(),
            saved_actor_urls: Vec::new(),
            wallpaper_assignments: HashMap::new(),
            attempted_wallpaper_restores: HashSet::new(),
            wallpaper_retries: HashMap::new(),
            pending_video_reapply: std::collections::VecDeque::new(),
            last_display_refresh: Instant::now(),
            pending_actor_save: None,
            restore_actors_pending: true,
            restore_wallpapers_pending: true,
            start_with_session: !previewing && autostart::enabled(),
            playlist_dirty: false,
            playlist_dialog_open: false,
            playlist_draft: Vec::new(),
            playlist_interval_draft: 30,
            playlist_random_draft: false,
            playlist_drag_index: None,
            playlist_after_save: None,
            rotation_running: false,
            rotation_waiting: false,
            rotation_current_url: None,
            rotation_deadline: None,
            audio: audio::AudioManager::default(),
            last_audio_sync: Instant::now(),
            wallpaper_video_audio: video_wallpaper::WallpaperAudio::default(),
            video_preview: video_preview::VideoPreview::default(),
            autoplay_video_preview: false,
            video_texture: None,
            overlay: std::env::current_exe()
                .ok()
                .map(|path| OverlayManager::new(path, service.clone())),
            wallpaper: ui_session::WallpaperUi::new(previewing),
            wallpaper_animation_sync_pending: false,
            wallpaper_pause_applied: None,
            wallpaper_pause_retry_at: None,
            reconcile_pending: false,
            pending_import_status: None,
            window_state: ui_session::WindowMonitorUi::new(service.clone(), previewing),
            automatic_pause: false,
            pause_saved_playing: HashMap::new(),
            thumbnails: thumbnails::Thumbnails::new(service),
            logo,
            discover_background,
            task_tx,
            task_rx,
            busy: false,
            import_files: None,
            import_start_playing: true,
            remove_confirmation: None,
            import_kind: ImportKind::Wallpaper,
            status: None,
            status_snapshot: None,
            status_since: None,
            control,
            tray,
            window_hidden: start_in_tray,
            force_exit: false,
            preview: preview_config.map(preview::PreviewState::new),
        };
        if previewing {
            app.seed_preview(ctx);
            app.audit_preview_actions(ctx);
            app.audit_pointer_actions(ctx);
            return app;
        }
        app.reload();
        if let Some(overlay) = &mut app.overlay {
            overlay.refresh_monitors();
        }
        app.refresh_audio_outputs(ctx, false);
        if app.location.migrate && app.location.legacy.is_dir() {
            app.busy = true;
            let location = app.location.clone();
            let tx = app.task_tx.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
                let result = location
                    .store()
                    .migrate_from(&location.legacy)
                    .map_err(|error| error.to_string());
                let _ = tx.send(TaskResult::Migration(result));
                ctx.request_repaint();
            });
        }
        app
    }

    fn reload(&mut self) {
        if self.preview_action(preview::UiAction::Operation {
            name: "refresh_library".into(),
        }) {
            return;
        }
        self.thumbnails.clear();
        match self.location.store().load_scene() {
            Ok(scene) => {
                self.settings = scene["settings"].clone();
                self.playlist_name = scene["playlistName"]
                    .as_str()
                    .unwrap_or("Unsaved")
                    .to_owned();
                self.playlist_dirty = scene["playlistDirty"].as_bool().unwrap_or(false);
                self.playlist = scene["playlist"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                self.managed_urls = scene["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|asset| asset["url"].as_str().map(str::to_owned))
                    .collect();
                self.assets = ["assets", "discovered"]
                    .into_iter()
                    .flat_map(|key| scene[key].as_array().into_iter().flatten().cloned())
                    .collect();
                self.saved_actor_urls = scene["actorUrls"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .filter(|url| {
                        self.assets.iter().any(|asset| {
                            asset["url"].as_str() == Some(*url)
                                && asset["kind"].as_str() == Some("character")
                                && asset["missing"].as_bool() != Some(true)
                        })
                    })
                    .map(str::to_owned)
                    .collect();
                self.wallpaper_assignments = scene["wallpapersByMonitor"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter_map(|(monitor, entry)| {
                        if self.wallpaper.is_gnome() && monitor != GNOME_OUTPUT {
                            return None;
                        }
                        entry["url"]
                            .as_str()
                            .map(|url| (monitor.clone(), url.to_owned()))
                    })
                    .collect();
                self.attempted_wallpaper_restores
                    .retain(|(monitor, url)| self.wallpaper_assignments.get(monitor) == Some(url));
                self.restore_wallpapers_pending = true;
                if self.selected_output.is_none() {
                    self.selected_output = scene["selectedMonitor"]
                        .as_str()
                        .filter(|monitor| *monitor != GNOME_OUTPUT)
                        .map(str::to_owned);
                }
                if !self.assets.iter().any(|asset| {
                    asset["url"]
                        .as_str()
                        .is_some_and(|url| self.selected.as_deref() == Some(url))
                }) {
                    self.selected = self
                        .assets
                        .iter()
                        .find(|asset| asset["missing"].as_bool() != Some(true))
                        .and_then(|asset| asset["url"].as_str())
                        .map(str::to_owned);
                }
                self.error = None;
            }
            Err(error) => {
                self.assets.clear();
                self.managed_urls.clear();
                self.selected = None;
                self.error = Some(error.to_string());
            }
        }
    }

    fn choose_files(&mut self, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Import) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let files = rfd::FileDialog::new()
                .set_title("Import content")
                .add_filter(
                    "Supported media",
                    &[
                        "png", "jpg", "jpeg", "webp", "gif", "apng", "bmp", "svg", "mp4", "mkv",
                        "webm", "mov", "avi", "m4v",
                    ],
                )
                .pick_files()
                .unwrap_or_default();
            let _ = tx.send(TaskResult::Files(files));
            ctx.request_repaint();
        });
    }

    pub(crate) fn choose_character_audio(&mut self, actor_url: String, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "choose_character_audio".into(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let files = rfd::FileDialog::new()
                .set_title("Add character audio")
                .add_filter(
                    "Audio",
                    &["wav", "mp3", "ogg", "flac", "m4a", "aac", "opus"],
                )
                .pick_files()
                .unwrap_or_default();
            let _ = tx.send(TaskResult::AudioFiles { actor_url, files });
            ctx.request_repaint();
        });
    }

    fn add_character_audio(&mut self, actor_url: String, files: Vec<PathBuf>, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "add_character_audio".into(),
        }) {
            return;
        }

        if self.busy || files.is_empty() {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result =
                library_actions::add_character_audio(&location.store(), &actor_url, &files)
                    .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::AudioAdded { actor_url, result });
            ctx.request_repaint();
        });
    }

    pub(crate) fn save_character_audio(
        &mut self,
        actor_url: String,
        audio: Value,
        volume: f64,
        ctx: &egui::Context,
    ) {
        if self.preview_action(preview::UiAction::CharacterAudio {
            url: actor_url.clone(),
            audio: audio.clone(),
            volume,
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result =
                library_actions::save_character_audio(&location.store(), &actor_url, audio, volume)
                    .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::AudioSaved { actor_url, result });
            ctx.request_repaint();
        });
    }

    fn import_selected(&mut self, files: Vec<PathBuf>, start_after: bool, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "import_confirm".into(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let kind = self.import_kind;
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let store = location.store();
            let result =
                library_actions::import(&store, &files, kind).map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::Imported {
                result,
                kind,
                start_after,
            });
            ctx.request_repaint();
        });
    }

    fn toggle_favorite(&mut self, url: String, value: bool, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Favorite {
            url: url.clone(),
            value,
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let store = location.store();
            let result = library_actions::set_favorite(&store, &url, value)
                .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::Favorite(result));
            ctx.request_repaint();
        });
    }

    fn update_asset_property(
        &mut self,
        url: String,
        key: &'static str,
        value: Value,
        ctx: &egui::Context,
    ) {
        if self.preview_action(preview::UiAction::AssetProperty {
            url: url.clone(),
            key: key.into(),
            value: value.clone(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        let result_url = url.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result =
                library_actions::update_asset_property(&location.store(), &url, key, value)
                    .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::AssetProperty {
                result,
                url: result_url,
                key,
            });
            ctx.request_repaint();
        });
    }

    fn reapply_active_video(&mut self, url: &str, _ctx: &egui::Context) {
        for (output, active) in &self.wallpaper_assignments {
            if active == url
                && !self
                    .pending_video_reapply
                    .iter()
                    .any(|(o, u)| o == output && u == url)
            {
                self.pending_video_reapply
                    .push_back((output.clone(), url.to_owned()));
            }
        }
    }
    fn dispatch_video_updates(&mut self, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        while let Some((output, url)) = self.pending_video_reapply.pop_front() {
            if self.wallpaper_assignments.get(&output) != Some(&url) {
                continue;
            }
            let Some(asset) = self
                .assets
                .iter()
                .find(|a| a["url"].as_str() == Some(url.as_str()) && a["video"] == true)
                .cloned()
            else {
                continue;
            };
            let rotation = self.rotation_running;
            if self.apply_wallpaper_on(&asset, output, ctx, rotation, false, true) {
                break;
            }
        }
    }

    fn sync_wallpaper_video_audio(&mut self, global_volume: f64) {
        let output = if self.wallpaper.is_gnome() {
            Some(GNOME_OUTPUT)
        } else {
            self.selected_output.as_deref()
        };
        let active_url = output
            .and_then(|output| self.wallpaper_assignments.get(output))
            .cloned();
        let finished = output.is_some_and(|output| self.wallpaper.video_finished(output));
        let active_asset = active_url.filter(|_| !finished).and_then(|url| {
            self.assets
                .iter()
                .find(|asset| asset["url"].as_str() == Some(url.as_str()))
        });
        let position = output.and_then(|output| self.wallpaper.video_position(output));
        if let Some(error) = self.wallpaper_video_audio.sync(
            active_asset,
            global_volume,
            self.automatic_pause,
            position,
        ) {
            self.status = Some(error);
        }
    }

    fn remove_selected(&mut self, url: String, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Remove { url: url.clone() }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result =
                library_actions::remove(&location.store(), &url).map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::Removed(result));
            ctx.request_repaint();
        });
    }

    fn save_actor_settings(&mut self, url: String, settings: ActorSettings, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::ActorSettings {
            url: url.clone(),
            value: serde_json::to_value(settings).unwrap(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result =
                library_actions::save_character_settings(&location.store(), &url, settings)
                    .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::ActorSettings(result));
            ctx.request_repaint();
        });
    }

    fn open_settings(&mut self, ctx: &egui::Context) {
        if self.page == Page::Settings {
            return;
        }
        self.settings_backdrop = None;
        self.settings_backdrop_requested = None;
        self.settings_return_page = self.page;
        let mut draft = self.settings.clone();
        draft["startWithSession"] = serde_json::json!(self.start_with_session);
        draft["closeToTray"] =
            serde_json::json!(self.settings["closeToTray"].as_bool().unwrap_or(true));
        draft["msaaSamples"] =
            serde_json::json!(self.settings["msaaSamples"].as_u64().unwrap_or(4));
        self.settings_draft = Some(draft);
        self.page = Page::Settings;
        if self.preview.is_none() {
            self.refresh_audio_outputs(ctx, false);
        }
    }

    pub(crate) fn open_folder(&mut self, path: &std::path::Path) {
        if self.preview_action(preview::UiAction::OpenFolder {
            path: path.display().to_string(),
        }) {
            return;
        }

        self.status = Some(match Command::new("xdg-open").arg(path).spawn() {
            Ok(_) => format!("Opening folder: {}", path.display()),
            Err(error) => format!("Could not open folder: {error}"),
        });
    }

    fn open_or_apply_asset(&mut self, asset: &Value, ctx: &egui::Context) {
        let Some(url) = asset["url"].as_str().map(str::to_owned) else {
            self.status = Some("This item has no file location.".into());
            return;
        };
        self.selected = Some(url.clone());
        if asset["kind"].as_str() == Some("character") {
            self.selected_actor = Some(url);
            self.page = Page::Animations;
            if let Some(overlay) = &mut self.overlay {
                overlay.refresh_monitors();
            }
        } else if asset["missing"].as_bool() == Some(true) {
            self.status = Some("This wallpaper is unavailable.".into());
        } else {
            self.apply_wallpaper(asset, ctx, false);
        }
    }

    fn tray_close_enabled(&self) -> bool {
        self.tray.as_ref().is_some_and(|tray| tray.is_running())
            && self.settings_draft.as_ref().unwrap_or(&self.settings)["closeToTray"]
                .as_bool()
                .unwrap_or(true)
    }

    fn request_window_close(&mut self, ctx: &egui::Context) {
        if self.tray_close_enabled() {
            self.window_hidden = true;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Rebinds window-only state to a newly opened window's context.
    fn attach_window(&mut self, ctx: &egui::Context) {
        install_theme(ctx);
        (self.logo, self.discover_background) = window_textures(ctx);
        self.window_hidden = false;
    }

    /// Releases what only the closed window could show.
    fn detach_window(&mut self) {
        self.logo = None;
        self.discover_background = None;
        self.settings_backdrop = None;
        self.settings_backdrop_requested = None;
        self.video_preview.stop();
        self.video_texture = None;
        self.thumbnails.clear();
    }

    /// Keeps the desktop services running while the window is closed to the
    /// tray, until the tray or a second launch asks for the window again.
    fn run_in_tray(&mut self, service: &egui::Context, wake: &mpsc::Receiver<()>) {
        while self.window_hidden && !self.force_exit {
            let _ = service.run(egui::RawInput::default(), |ctx| self.run_frame(ctx, false));
            let _ = wake.recv_timeout(Duration::from_millis(250));
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.window_hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    pub(crate) fn start_character_overlay(
        &mut self,
        key: &str,
        path: &std::path::Path,
        monitor: &str,
        mut settings: ActorSettings,
    ) -> Result<(), String> {
        if self.automatic_pause {
            self.pause_saved_playing
                .insert(key.to_owned(), settings.playing);
            settings.playing = false;
        }
        let result = self
            .overlay
            .as_mut()
            .ok_or_else(|| "Character overlay is unavailable".to_owned())?
            .start(key, path, monitor, settings);
        if result.is_err() {
            self.pause_saved_playing.remove(key);
        }
        result
    }

    pub(crate) fn update_character_overlay(
        &mut self,
        key: &str,
        mut settings: ActorSettings,
    ) -> Result<(), String> {
        if self.preview.is_some() {
            self.actor_drafts.insert(key.to_owned(), settings);
            return Ok(());
        }

        if self.automatic_pause {
            self.pause_saved_playing
                .insert(key.to_owned(), settings.playing);
            settings.playing = false;
        }
        self.overlay
            .as_mut()
            .ok_or_else(|| "Character overlay is unavailable".to_owned())?
            .update(key, settings)
    }

    fn refresh_automatic_pause(&mut self, ctx: &egui::Context) {
        let snapshot = self.window_state.snapshot();
        let settings = self.settings_draft.as_ref().unwrap_or(&self.settings);
        self.automatic_pause = (settings["pauseFullscreen"].as_bool().unwrap_or(true)
            && snapshot.active.fullscreen)
            || (settings["pauseMaximized"].as_bool().unwrap_or(false) && snapshot.active.maximized)
            || self.window_state.session_locked();
        self.sync_wallpaper_animation(ctx);
        self.sync_actor_pause_state();
    }

    fn sync_wallpaper_animation(&mut self, ctx: &egui::Context) {
        let enabled = !self.automatic_pause;
        if self.wallpaper_animation_sync_pending
            || self.busy
            || self.wallpaper_pause_applied == Some(enabled)
            || self
                .wallpaper_pause_retry_at
                .is_some_and(|at| Instant::now() < at)
        {
            return;
        }
        let monitors = self
            .overlay
            .as_ref()
            .map(|overlay| overlay.monitors.clone())
            .unwrap_or_default();
        let assignments = self.wallpaper_assignments.clone();
        let assets = self.assets.clone();
        let outputs = assignments
            .into_iter()
            .filter_map(|(output, url)| {
                let asset = assets
                    .iter()
                    .find(|asset| asset["url"].as_str() == Some(url.as_str()))?;
                let video = asset["video"].as_bool() == Some(true);
                if !video && asset["animated"].as_bool() != Some(true) {
                    return None;
                }
                if output == GNOME_OUTPUT {
                    return Some((output, (0, 0), (0, 0), video));
                }
                let monitor = monitors
                    .iter()
                    .find(|monitor| monitor.name.as_deref() == Some(output.as_str()))?;
                Some((
                    output,
                    monitor.logical_position?,
                    monitor.logical_size?,
                    video,
                ))
            })
            .collect::<Vec<_>>();
        if outputs.is_empty() {
            self.wallpaper_pause_applied = Some(enabled);
            return;
        }
        let wallpaper = self.wallpaper.clone();
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        self.wallpaper_animation_sync_pending = true;
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let mut errors = Vec::new();
            for (output, position, size, video) in outputs {
                let result = if video {
                    wallpaper.set_video_playing(&output, enabled)
                } else {
                    wallpaper.set_animation_enabled(&output, position, size, enabled)
                };
                if let Err(error) = result {
                    errors.push(error);
                }
            }
            let result = if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            };
            let _ = tx.send(TaskResult::WallpaperAnimation { result, enabled });
            ctx.request_repaint();
        });
    }

    fn sync_actor_pause_state(&mut self) {
        let worker_states = self
            .saved_actor_urls
            .iter()
            .filter_map(|key| {
                self.overlay
                    .as_ref()
                    .and_then(|overlay| overlay.worker(key))
                    .map(|worker| (key.clone(), worker.status, worker.settings))
            })
            .collect::<Vec<_>>();
        let mut updates = Vec::new();
        for (key, status, settings) in worker_states {
            if self.automatic_pause {
                let desired = self
                    .pause_saved_playing
                    .get(&key)
                    .copied()
                    .or_else(|| self.actor_drafts.get(&key).map(|settings| settings.playing))
                    .unwrap_or(settings.playing);
                self.pause_saved_playing
                    .entry(key.clone())
                    .or_insert(desired);
                if status == WorkerStatus::Running && settings.playing {
                    let mut paused = settings;
                    paused.playing = false;
                    updates.push((key, paused));
                }
            } else if let Some(desired) = self.pause_saved_playing.get(&key).copied() {
                if settings.playing == desired {
                    self.pause_saved_playing.remove(&key);
                } else if status == WorkerStatus::Running {
                    let mut resumed = settings;
                    resumed.playing = desired;
                    updates.push((key, resumed));
                }
            }
        }
        for (key, settings) in updates {
            let result = self
                .overlay
                .as_mut()
                .ok_or_else(|| "Character overlay is unavailable".to_owned())
                .and_then(|overlay| overlay.update(&key, settings));
            match result {
                Ok(()) if !self.automatic_pause => {
                    self.pause_saved_playing.remove(&key);
                }
                Err(error) => self.status = Some(error.to_owned()),
                _ => {}
            }
        }
    }

    fn refresh_audio_outputs(&mut self, ctx: &egui::Context, force: bool) {
        if self.preview_action(preview::UiAction::Operation {
            name: "refresh_audio_outputs".into(),
        }) {
            return;
        }

        if self.audio_outputs_loading
            || (!force
                && self
                    .audio_outputs_last_refresh
                    .is_some_and(|last| last.elapsed() < Duration::from_secs(15)))
        {
            return;
        }
        self.audio_outputs_loading = true;
        self.audio_outputs_last_refresh = Some(Instant::now());
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result = audio::discover_outputs();
            let _ = tx.send(TaskResult::AudioOutputs(result));
            ctx.request_repaint();
        });
    }

    fn preferred_audio_output(&self) -> Option<&str> {
        let name = self
            .settings_draft
            .as_ref()
            .and_then(|settings| settings["audioOutput"].as_str())
            .or_else(|| self.settings["audioOutput"].as_str())
            .filter(|name| !name.is_empty())?;
        // Until the device list is known, trust the saved name: PulseAudio falls back to
        // the default sink for an unknown one, and this avoids a first sound on the wrong device.
        (self.audio_outputs.is_empty()
            || self.audio_outputs.iter().any(|output| output.name == name))
        .then_some(name)
    }

    fn sync_video_preview(&mut self, ctx: &egui::Context) {
        let autoplay = std::mem::take(&mut self.autoplay_video_preview);
        let library_visible = self.page == Page::Library
            || (self.page == Page::Settings && self.settings_return_page == Page::Library);
        let asset = library_visible.then(|| {
            self.assets
                .iter()
                .find(|item| {
                    item["url"]
                        .as_str()
                        .is_some_and(|url| self.selected.as_deref() == Some(url))
                        && item["video"].as_bool() == Some(true)
                        && item["missing"].as_bool() != Some(true)
                })
                .cloned()
        });
        let Some(Some(asset)) = asset else {
            self.video_preview.stop();
            self.video_texture = None;
            return;
        };
        let Some(url) = asset["url"].as_str() else {
            self.video_preview.stop();
            self.video_texture = None;
            return;
        };
        let path = match file_path(url) {
            Ok(path) => path,
            Err(error) => {
                self.video_preview.stop();
                self.video_texture = None;
                self.status = Some(format!("Could not open video preview: {error}"));
                return;
            }
        };
        let width = asset["width"].as_u64().unwrap_or(1280).min(u32::MAX as u64) as u32;
        let height = asset["height"].as_u64().unwrap_or(720).min(u32::MAX as u64) as u32;
        if self
            .video_preview
            .set_source(url, &path, (width, height), ctx.clone())
        {
            self.video_texture = None;
        }
        if autoplay && self.video_preview.is_available() {
            if let Err(error) = self.video_preview.toggle_playback() {
                self.status = Some(format!("Could not start video preview: {error}"));
            }
        }
        if let Some(frame) = self.video_preview.poll() {
            let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
            if let Some(texture) = &mut self.video_texture {
                texture.set(image, egui::TextureOptions::LINEAR);
            } else {
                self.video_texture = Some(ctx.load_texture(
                    "selected-video-preview",
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        if let Some(error) = self.video_preview.take_error() {
            self.status = Some(error);
        }
    }

    fn cancel_settings(&mut self) {
        self.settings_draft = None;
        let quality = self.settings["previewQuality"].as_str().unwrap_or("High");
        self.thumbnails.set_quality(match quality {
            "Low" => 240,
            "Medium" => 360,
            _ => 480,
        });
        self.page = self.settings_return_page;
    }

    fn save_settings(&mut self, ctx: &egui::Context) {
        if self.preview.is_some() {
            let value = self
                .settings_draft
                .clone()
                .unwrap_or_else(|| self.settings.clone());
            self.preview_action(preview::UiAction::Settings { value });
            return;
        }

        if self.busy {
            return;
        }
        let Some(draft) = self.settings_draft.clone() else {
            return;
        };
        let start_with_session = draft["startWithSession"]
            .as_bool()
            .unwrap_or(self.start_with_session);
        let previous_start_with_session = self.start_with_session;
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let mut autostart_changed = false;
            let mut result = if start_with_session != previous_start_with_session {
                std::env::current_exe()
                    .map_err(|error| error.to_string())
                    .and_then(|executable| {
                        crate::autostart::set_enabled(start_with_session, &executable)
                    })
                    .map(|()| autostart_changed = true)
            } else {
                Ok(())
            };
            if result.is_ok() {
                result = library_actions::save_app_settings(&location.store(), &draft)
                    .map_err(|error| error.to_string());
            }
            if let Err(error) = result {
                if autostart_changed {
                    let rollback = std::env::current_exe()
                        .map_err(|rollback| rollback.to_string())
                        .and_then(|executable| {
                            crate::autostart::set_enabled(previous_start_with_session, &executable)
                        });
                    let error = match rollback {
                        Ok(()) => error,
                        Err(rollback) => {
                            format!(
                                "{error}; could not restore the previous autostart entry: {rollback}"
                            )
                        }
                    };
                    let _ = tx.send(TaskResult::Settings(Err(error)));
                } else {
                    let _ = tx.send(TaskResult::Settings(Err(error)));
                }
            } else {
                let _ = tx.send(TaskResult::Settings(Ok((draft, start_with_session))));
            }
            ctx.request_repaint();
        });
    }

    fn save_playlist(&mut self, playlist: Vec<String>, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Playlist {
            urls: playlist.clone(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result = library_actions::save_playlist(&location.store(), &playlist)
                .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::Playlist(result));
            ctx.request_repaint();
        });
    }

    fn persist_active_actors(&mut self, urls: Vec<String>, ctx: &egui::Context) {
        if self.busy {
            self.saved_actor_urls = urls.clone();
            self.pending_actor_save = Some(urls);
            return;
        }
        self.busy = true;
        self.saved_actor_urls = urls.clone();
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result = library_actions::save_active_actors(&location.store(), &urls)
                .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::ActiveActors(result));
            ctx.request_repaint();
        });
    }

    fn restore_saved_actors(&mut self) {
        if !self.restore_actors_pending {
            return;
        }
        let Some(overlay) = &mut self.overlay else {
            self.restore_actors_pending = false;
            return;
        };
        if overlay.loading_monitors || overlay.monitors.is_empty() {
            return;
        }
        let monitor = self
            .selected_output
            .as_deref()
            .filter(|selected| {
                overlay
                    .monitors
                    .iter()
                    .any(|item| item.name.as_deref() == Some(*selected))
            })
            .map(str::to_owned)
            .or_else(|| overlay.monitors.iter().find_map(|item| item.name.clone()));
        let Some(monitor) = monitor else {
            return;
        };
        self.selected_output = Some(monitor.clone());
        let mut errors = Vec::new();
        for url in self.saved_actor_urls.clone() {
            if self
                .overlay
                .as_ref()
                .is_some_and(|overlay| overlay.worker(&url).is_some())
            {
                continue;
            }
            let Some(asset) = self
                .assets
                .iter()
                .find(|asset| asset["url"].as_str() == Some(url.as_str()))
                .cloned()
            else {
                continue;
            };
            let result = file_path(&url)
                .map_err(|error| error.to_string())
                .and_then(|path| {
                    self.start_character_overlay(
                        &url,
                        &path,
                        &monitor,
                        animations::initial_settings(&asset),
                    )
                });
            if let Err(error) = result {
                errors.push(error);
            }
        }
        self.restore_actors_pending = false;
        self.status = if errors.is_empty() && !self.saved_actor_urls.is_empty() {
            Some("Saved characters restored on this display.".into())
        } else if errors.is_empty() {
            None
        } else {
            Some(format!(
                "Could not restore characters: {}",
                errors.join("; ")
            ))
        };
    }

    fn save_playlist_configuration(&mut self, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "save_playlist_configuration".into(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        let playlist = self.playlist.clone();
        let interval = self.playlist_interval_draft;
        let random = self.playlist_random_draft;
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result = library_actions::save_playlist_configuration(
                &location.store(),
                &playlist,
                interval,
                random,
            )
            .map_err(|error| error.to_string());
            let _ = tx.send(TaskResult::PlaylistConfig(result));
            ctx.request_repaint();
        });
    }

    fn save_named_playlist(&mut self, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "save_named_playlist".into(),
        }) {
            return;
        }

        if self.busy || self.playlist.is_empty() {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        let playlist = self.playlist.clone();
        let interval = self.settings["playlistInterval"]
            .as_u64()
            .unwrap_or(30)
            .clamp(1, 1440);
        let random = self.settings["playlistRandom"].as_bool().unwrap_or(false);
        let default_stem = self
            .playlist_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' {
                    character.to_ascii_lowercase()
                } else if character.is_whitespace() {
                    '-'
                } else {
                    '_'
                }
            })
            .collect::<String>();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let directory = location.root.join("Playlists");
            let result = std::fs::create_dir_all(&directory)
                .map_err(|error| error.to_string())
                .and_then(|_| {
                    let Some(mut path) = rfd::FileDialog::new()
                        .set_title("Save playlist")
                        .set_directory(&directory)
                        .set_file_name(format!("{}.json", default_stem.trim_matches('-')))
                        .add_filter("WaniEngine playlist", &["json"])
                        .save_file()
                    else {
                        return Ok(None);
                    };
                    if path.extension().is_none() {
                        path.set_extension("json");
                    }
                    let name = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("Playlist");
                    library_actions::save_named_playlist(
                        &location.store(),
                        &path,
                        name,
                        &playlist,
                        interval,
                        random,
                    )
                    .map(|()| Some(path.display().to_string()))
                    .map_err(|error| error.to_string())
                });
            let _ = tx.send(TaskResult::NamedPlaylistSaved(result));
            ctx.request_repaint();
        });
    }

    fn load_named_playlist(&mut self, ctx: &egui::Context) {
        if self.preview_action(preview::UiAction::Operation {
            name: "load_named_playlist".into(),
        }) {
            return;
        }

        if self.busy {
            return;
        }
        self.busy = true;
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let location = self.location.clone();
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let directory = location.root.join("Playlists");
            let result = match rfd::FileDialog::new()
                .set_title("Load playlist")
                .set_directory(&directory)
                .add_filter("WaniEngine playlist", &["json"])
                .pick_file()
            {
                Some(path) => library_actions::load_named_playlist(&location.store(), &path)
                    .map(Some)
                    .map_err(|error| error.to_string()),
                None => Ok(None),
            };
            let _ = tx.send(TaskResult::NamedPlaylistLoaded(result));
            ctx.request_repaint();
        });
    }

    fn apply_wallpaper(&mut self, asset: &Value, ctx: &egui::Context, rotation: bool) -> bool {
        if self.preview_action(preview::UiAction::Apply {
            url: asset["url"].as_str().unwrap_or_default().into(),
            rotation,
        }) {
            return true;
        }

        let output_name = if self.wallpaper.is_gnome() {
            GNOME_OUTPUT.to_owned()
        } else if let Some(output_name) = self.selected_output.clone() {
            output_name
        } else {
            self.status = Some("Select a connected display before applying wallpaper.".into());
            if rotation {
                self.stop_rotation();
            }
            return false;
        };
        self.apply_wallpaper_on(asset, output_name, ctx, rotation, false, false)
    }

    pub(crate) fn wallpaper_format_supported(&self, asset: &Value) -> bool {
        if asset["kind"].as_str() == Some("character") {
            return false;
        }
        if asset["video"].as_bool() == Some(true) {
            return self.wallpaper.supports_video();
        }
        if !self.wallpaper.supports_images() {
            return false;
        }
        let animated = asset["animated"].as_bool() == Some(true);
        if self.wallpaper.is_gnome() {
            !animated
        } else {
            !animated || self.wallpaper.supports_animated()
        }
    }

    fn apply_wallpaper_on(
        &mut self,
        asset: &Value,
        output_name: String,
        ctx: &egui::Context,
        rotation: bool,
        restoration: bool,
        reconfiguration: bool,
    ) -> bool {
        if self.busy || self.wallpaper_animation_sync_pending {
            return false;
        }
        let is_gnome = self.wallpaper.is_gnome();
        let is_video = asset["video"].as_bool() == Some(true);
        let animated = asset["animated"].as_bool().unwrap_or(false);
        let unsupported = !self.wallpaper_format_supported(asset);
        if unsupported {
            self.status = Some(if is_gnome {
                "This GNOME session supports still images and video through XWayland; animated image wallpapers require another adapter. Check that XWayland is available for video.".into()
            } else {
                "This wallpaper format is unavailable in the current desktop session.".into()
            });
            if rotation {
                self.stop_rotation();
            }
            return false;
        }
        let Some(url) = asset["url"].as_str() else {
            self.status = Some("The selected item has no local file.".into());
            if rotation {
                self.stop_rotation();
            }
            return false;
        };
        let path = match file_path(url) {
            Ok(path) if path.is_file() => path,
            _ => {
                self.status = Some("The selected wallpaper file is unavailable.".into());
                if rotation {
                    self.stop_rotation();
                }
                return false;
            }
        };
        let (position, size) = if is_gnome {
            ((0, 0), (0, 0))
        } else {
            let Some(monitor) = self.overlay.as_ref().and_then(|manager| {
                manager
                    .monitors
                    .iter()
                    .find(|monitor| monitor.name.as_deref() == Some(&output_name))
                    .cloned()
            }) else {
                self.status = Some(
                    "The selected display is no longer connected. Refresh the display list.".into(),
                );
                if rotation {
                    self.stop_rotation();
                }
                return false;
            };
            let Some(position) = monitor.logical_position else {
                self.status =
                    Some("The display server did not provide this monitor's position.".into());
                if rotation {
                    self.stop_rotation();
                }
                return false;
            };
            let Some(size) = monitor.logical_size else {
                self.status =
                    Some("The display server did not provide this monitor's size.".into());
                if rotation {
                    self.stop_rotation();
                }
                return false;
            };
            (position, size)
        };
        let fit = asset["fit"].as_str().unwrap_or("cover").to_owned();
        let looping = asset["loop"].as_bool().unwrap_or(true);
        let speed = asset["speed"].as_f64().unwrap_or(100.0);
        let duration = asset["duration"].as_f64();
        let video_fps = self.settings["frameRateLimit"]
            .as_u64()
            .unwrap_or(60)
            .clamp(15, 240) as u32;
        let video_fps = asset["nativeFps"]
            .as_f64()
            .filter(|fps| fps.is_finite() && *fps > 0.0)
            .map(|fps| ((fps * speed / 100.0).ceil() as u32).clamp(1, video_fps))
            .unwrap_or(video_fps);
        let animation_enabled = (animated || is_video) && !self.automatic_pause;
        let controller = self.wallpaper.clone();
        let location = self.location.clone();
        let tx = self.task_tx.clone();
        let ctx = ctx.clone();
        let url = url.to_owned();
        let assigned_output = output_name.clone();
        let select_monitor = !restoration && !is_gnome && !reconfiguration;
        let reset_audio = is_gnome || self.selected_output.as_deref() == Some(output_name.as_str());
        if is_video {
            self.status = Some("Starting the video wallpaper…".into());
        }
        self.busy = true;
        thread::spawn(move || {
            let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
            let result = if is_video {
                controller.apply_video(
                    &output_name,
                    position,
                    size,
                    &path,
                    &fit,
                    waniengine_egui::native_wallpaper::VideoSettings {
                        duration,
                        speed,
                        fps: video_fps,
                        looping,
                        playing: animation_enabled,
                    },
                )
            } else {
                controller.apply(&output_name, position, size, &path, &fit, animation_enabled)
            };
            let assignment_saved = if result.is_ok() {
                library_actions::save_wallpaper_assignment(
                    &location.store(),
                    &assigned_output,
                    &url,
                    select_monitor,
                )
                .map_err(|error| error.to_string())
            } else {
                Ok(())
            };
            let _ = tx.send(TaskResult::Wallpaper {
                result,
                assignment_saved,
                rotation,
                restoration,
                select_monitor,
                reset_audio,
                output_name: assigned_output,
                url,
            });
            ctx.request_repaint();
        });
        true
    }

    fn restore_saved_wallpapers(&mut self, ctx: &egui::Context) {
        // apply_wallpaper_on refuses while an animation sync runs, and every
        // restore marks its monitor attempted first: wait, or it is lost.
        if !self.restore_wallpapers_pending
            || self.busy
            || self.wallpaper_animation_sync_pending
            || !self.wallpaper.capabilities_ready()
        {
            return;
        }
        let is_gnome = self.wallpaper.is_gnome();
        let connected_outputs: HashSet<String> = if is_gnome {
            HashSet::from([GNOME_OUTPUT.to_owned()])
        } else {
            let Some(overlay) = &self.overlay else {
                self.restore_wallpapers_pending = false;
                return;
            };
            if overlay.loading_monitors {
                return;
            }
            overlay
                .monitors
                .iter()
                .filter_map(|monitor| monitor.name.clone())
                .collect()
        };
        let mut disconnected = false;
        let mut assignments: Vec<_> = self
            .wallpaper_assignments
            .iter()
            .filter(|(monitor, _)| !is_gnome || monitor.as_str() == GNOME_OUTPUT)
            .map(|(monitor, url)| (monitor.clone(), url.clone()))
            .collect();
        assignments.sort();
        for (monitor_name, url) in assignments {
            if self
                .attempted_wallpaper_restores
                .contains(&(monitor_name.clone(), url.clone()))
            {
                continue;
            }
            if !connected_outputs.contains(&monitor_name) {
                disconnected = true;
                continue;
            }
            if let Some((_, at)) = self.wallpaper_retries.get(&monitor_name) {
                let wait = at.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    ctx.request_repaint_after(wait);
                    disconnected = true;
                    continue;
                }
            }
            let Some(asset) = self
                .assets
                .iter()
                .find(|asset| {
                    asset["url"].as_str() == Some(url.as_str())
                        && asset["kind"].as_str() == Some("wallpaper")
                        && asset["missing"].as_bool() != Some(true)
                })
                .cloned()
            else {
                self.attempted_wallpaper_restores
                    .insert((monitor_name, url));
                continue;
            };
            if !self.wallpaper_format_supported(&asset) {
                self.attempted_wallpaper_restores
                    .insert((monitor_name, url));
                self.status = Some(
                    "A saved wallpaper was skipped because its desktop adapter is unavailable."
                        .into(),
                );
                continue;
            }
            self.attempted_wallpaper_restores
                .insert((monitor_name.clone(), url));
            if self.apply_wallpaper_on(&asset, monitor_name, ctx, false, true, false) {
                return;
            }
        }
        self.restore_wallpapers_pending = disconnected;
    }

    /// Restores a video wallpaper whose surface the compositor dropped, after
    /// its monitor has had time to come back. Returns false once retries run out.
    fn schedule_wallpaper_retry(&mut self, output: &str) -> bool {
        const DELAYS: [u64; 5] = [3, 5, 10, 30, 60];
        let attempts = self.wallpaper_retries.get(output).map_or(0, |(n, _)| *n);
        let Some(delay) = DELAYS.get(attempts as usize) else {
            return false;
        };
        self.wallpaper_retries.insert(
            output.to_owned(),
            (attempts + 1, Instant::now() + Duration::from_secs(*delay)),
        );
        self.attempted_wallpaper_restores
            .retain(|(name, _)| name != output);
        self.restore_wallpapers_pending = true;
        true
    }

    fn stop_rotation(&mut self) {
        self.rotation_running = false;
        self.rotation_waiting = false;
        self.rotation_deadline = None;
    }

    fn rotation_assets(&self) -> Vec<(String, Value)> {
        self.playlist
            .iter()
            .filter_map(|url| {
                let asset = self
                    .assets
                    .iter()
                    .find(|asset| asset["url"].as_str() == Some(url.as_str()))?;
                let extension = asset["extension"]
                    .as_str()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let video = asset["video"].as_bool() == Some(true)
                    || matches!(
                        extension.as_str(),
                        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"
                    );
                if asset["missing"].as_bool() == Some(true)
                    || asset["kind"].as_str() == Some("character")
                    || (video && !self.wallpaper.supports_video())
                    || (!video && !self.wallpaper_format_supported(asset))
                {
                    return None;
                }
                Some((url.clone(), asset.clone()))
            })
            .collect()
    }

    fn rotation_interval(&self) -> Duration {
        let minutes = self.settings["playlistInterval"]
            .as_u64()
            .unwrap_or(30)
            .clamp(1, 1440);
        Duration::from_secs(minutes * 60)
    }

    fn toggle_rotation(&mut self, ctx: &egui::Context) {
        if self.rotation_running {
            self.stop_rotation();
            self.status = Some("Rotation paused. The current wallpaper stays applied.".into());
            return;
        }
        if self.busy {
            return;
        }
        let assets = self.rotation_assets();
        if assets.is_empty() {
            self.status = Some(if self.playlist.is_empty() {
                "Add an image wallpaper to the playlist first.".into()
            } else {
                "The playlist has no available still or animated images to rotate yet.".into()
            });
            return;
        }
        let current = self.rotation_current_url.as_deref();
        let mut index = current
            .and_then(|url| assets.iter().position(|(candidate, _)| candidate == url))
            .map(|index| (index + 1) % assets.len())
            .unwrap_or(0);
        if self.settings["playlistRandom"].as_bool().unwrap_or(false) && assets.len() > 1 {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as usize;
            index = seed % assets.len();
            if current.is_some_and(|url| assets[index].0 == url) {
                index = (index + 1) % assets.len();
            }
        }
        self.rotation_running = true;
        self.rotation_waiting = true;
        self.rotation_deadline = None;
        if !self.apply_wallpaper(&assets[index].1, ctx, true) {
            self.stop_rotation();
        }
    }

    fn tick_rotation(&mut self, ctx: &egui::Context) {
        if !self.rotation_running || self.rotation_waiting || self.busy {
            return;
        }
        let Some(deadline) = self.rotation_deadline else {
            return;
        };
        let now = Instant::now();
        if now < deadline {
            ctx.request_repaint_after(deadline.saturating_duration_since(now));
            return;
        }
        let assets = self.rotation_assets();
        if assets.is_empty() {
            self.stop_rotation();
            self.status =
                Some("Rotation stopped because no supported images are available.".into());
            return;
        }
        let current = self.rotation_current_url.as_deref();
        let mut index = current
            .and_then(|url| assets.iter().position(|(candidate, _)| candidate == url))
            .map(|index| (index + 1) % assets.len())
            .unwrap_or(0);
        if self.settings["playlistRandom"].as_bool().unwrap_or(false) && assets.len() > 1 {
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as usize;
            index = seed % assets.len();
            if current.is_some_and(|url| assets[index].0 == url) {
                index = (index + 1) % assets.len();
            }
        }
        self.rotation_waiting = true;
        self.rotation_deadline = None;
        if !self.apply_wallpaper(&assets[index].1, ctx, true) {
            self.stop_rotation();
        }
    }

    fn step_rotation(&mut self, direction: isize, ctx: &egui::Context) {
        if self.busy {
            return;
        }
        let assets = self.rotation_assets();
        if assets.is_empty() {
            self.status = Some("Add an available wallpaper to the playlist first.".into());
            return;
        }
        let len = assets.len();
        let current = self
            .rotation_current_url
            .as_deref()
            .and_then(|url| assets.iter().position(|(candidate, _)| candidate == url));
        let index = match current {
            Some(index) => (index as isize + direction).rem_euclid(len as isize) as usize,
            None if direction < 0 => len - 1,
            None => 0,
        };
        let continue_rotation = self.rotation_running;
        if continue_rotation {
            self.rotation_waiting = true;
            self.rotation_deadline = None;
        }
        if !self.apply_wallpaper(&assets[index].1, ctx, continue_rotation) && continue_rotation {
            self.stop_rotation();
        }
    }

    fn poll_tasks(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.task_rx.try_recv() {
            match &result {
                TaskResult::AssetProperty {
                    url, key: "volume", ..
                } if self
                    .wallpaper_volume_draft
                    .as_ref()
                    .is_some_and(|(draft_url, _)| draft_url == url) =>
                {
                    self.wallpaper_volume_draft = None;
                }
                TaskResult::AudioSaved { actor_url, .. }
                    if self
                        .character_volume_draft
                        .as_ref()
                        .is_some_and(|(draft_url, _)| draft_url == actor_url) =>
                {
                    self.character_volume_draft = None;
                }
                _ => {}
            }
            if !matches!(
                &result,
                TaskResult::AudioOutputs(_)
                    | TaskResult::WallpaperAnimation { .. }
                    | TaskResult::VideoConfigured(_)
            ) {
                self.busy = false;
            }
            match result {
                TaskResult::WorkerPanicked(detail) => {
                    eprintln!("WaniEngine background task failed: {detail}");
                    self.wallpaper_animation_sync_pending = false;
                    self.reconcile_pending = false;
                    self.audio_outputs_loading = false;
                    self.playlist_after_save = None;
                    self.pending_import_status = None;
                    self.reload();
                    self.status = Some("A background task failed unexpectedly.".into());
                }
                TaskResult::VideoConfigured(Err(error)) => {
                    self.reconcile_pending = false;
                    self.status = Some(error);
                }
                TaskResult::VideoConfigured(Ok(())) => {
                    self.reconcile_pending = false;
                }
                TaskResult::Files(files) => {
                    if !files.is_empty() {
                        self.import_files = Some(files);
                    }
                }
                TaskResult::AudioFiles { actor_url, files } => {
                    self.add_character_audio(actor_url, files, ctx);
                }
                TaskResult::Imported {
                    result: Ok(result),
                    kind,
                    start_after,
                } => {
                    self.reload();
                    if let Some(url) = result.first_url.clone() {
                        if kind == ImportKind::Character {
                            self.selected_actor = Some(url.clone());
                        }
                        self.selected = Some(url);
                    }
                    let mut message = format!(
                        "Imported {} · skipped {}{}",
                        result.added,
                        result.skipped,
                        if result.errors.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", result.errors.join("; "))
                        }
                    );
                    if kind == ImportKind::Wallpaper && result.added > 0 {
                        let mut next_playlist = self.playlist.clone();
                        let mut playlist_added = 0;
                        for url in &result.imported_urls {
                            if next_playlist.len() >= 1000 {
                                break;
                            }
                            if !next_playlist.contains(url) {
                                next_playlist.push(url.clone());
                                playlist_added += 1;
                            }
                        }
                        if next_playlist != self.playlist {
                            self.playlist = next_playlist.clone();
                            self.playlist_dirty = true;
                            self.playlist_after_save = if start_after {
                                let first_playable = result.imported_urls.iter().find(|url| {
                                    self.rotation_assets()
                                        .iter()
                                        .any(|(candidate, _)| candidate == *url)
                                });
                                first_playable
                                    .cloned()
                                    .map(PlaylistAfterSave::Start)
                                    .or_else(|| {
                                        Some(PlaylistAfterSave::Notify(
                                            "Added wallpapers to the playlist, but it has no supported images to start."
                                                .into(),
                                        ))
                                    })
                            } else {
                                None
                            };
                            message.push_str(&format!(
                                " · {playlist_added} wallpaper(s) added to playlist"
                            ));
                            if playlist_added < result.imported_urls.len() {
                                message.push_str(" · playlist limit reached");
                            }
                            self.status = Some(message.clone());
                            self.pending_import_status = Some(message);
                            self.save_playlist(next_playlist, ctx);
                            if !self.busy {
                                // The save never started: do not leave stale state.
                                self.playlist_after_save = None;
                                self.pending_import_status = None;
                                self.playlist_dirty = false;
                            }
                            continue;
                        } else if result.imported_urls.len() > 0 && self.playlist.len() >= 1000 {
                            message.push_str(" · playlist is full; imported files were kept");
                        }
                    }
                    self.status = Some(message);
                }
                TaskResult::AudioAdded {
                    actor_url,
                    result: Ok(count),
                } => {
                    self.reload();
                    self.selected_actor = Some(actor_url.clone());
                    self.audio_track_indices.insert(actor_url, 0);
                    self.status = Some(format!("Added {count} audio track(s) to the character."));
                }
                TaskResult::AudioSaved {
                    actor_url,
                    result: Ok(()),
                } => {
                    self.reload();
                    if self
                        .audio_dialog
                        .as_ref()
                        .is_some_and(|dialog| dialog.actor_url == actor_url)
                    {
                        self.audio_dialog = None;
                    }
                    self.status = Some("Character sound settings saved.".into());
                }
                TaskResult::AudioOutputs(Ok(outputs)) => {
                    self.audio_outputs = outputs;
                    self.audio_outputs_loading = false;
                    self.audio_outputs_error = None;
                }
                TaskResult::AudioOutputs(Err(error)) => {
                    self.audio_outputs_loading = false;
                    self.audio_outputs.clear();
                    self.audio_outputs_error = Some(error);
                }
                TaskResult::Migration(Ok(count)) => {
                    self.reload();
                    if count > 0 {
                        self.status = Some(format!("Copied {count} legacy projects."));
                    }
                }
                TaskResult::Migration(Err(error)) => {
                    self.reload();
                    self.status = Some(format!("Could not migrate legacy library: {error}"));
                }
                TaskResult::Favorite(Ok(())) => {
                    self.reload();
                    self.status = Some("Favorite updated.".into());
                }
                TaskResult::AssetProperty {
                    result: Ok(()),
                    url,
                    key,
                } => {
                    self.reload();
                    self.status = Some("Wallpaper settings saved.".into());
                    if matches!(key, "loop" | "speed" | "fit") {
                        self.reapply_active_video(&url, ctx);
                    }
                }
                TaskResult::Removed(Ok(())) => {
                    self.reload();
                    self.status =
                        Some("Removed from the library. The original file was preserved.".into());
                }
                TaskResult::ActorSettings(Ok(())) => {
                    self.reload();
                    self.status = Some("Character settings saved.".into());
                }
                TaskResult::Settings(Ok((draft, start_with_session))) => {
                    let old_fps = self.settings["frameRateLimit"].as_u64().unwrap_or(60);
                    let new_fps = draft["frameRateLimit"].as_u64().unwrap_or(60);
                    self.settings = draft;
                    if old_fps != new_fps {
                        // Re-applying clamps each video by its own native FPS; a raw
                        // set_fps would push the global limit past slower videos.
                        let urls: Vec<String> = self
                            .wallpaper_assignments
                            .values()
                            .cloned()
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        for url in urls {
                            self.reapply_active_video(&url, ctx);
                        }
                    }
                    self.start_with_session = start_with_session;
                    self.settings_draft = None;
                    self.page = self.settings_return_page;
                    self.reload();
                    let quality = self.settings["previewQuality"].as_str().unwrap_or("High");
                    self.thumbnails.set_quality(match quality {
                        "Low" => 240,
                        "Medium" => 360,
                        _ => 480,
                    });
                    self.status = Some("Settings saved.".into());
                }
                TaskResult::Playlist(Ok(())) => {
                    self.reload();
                    match self.playlist_after_save.take() {
                        Some(PlaylistAfterSave::Start(url)) => {
                            if let Some(asset) = self
                                .assets
                                .iter()
                                .find(|asset| asset["url"].as_str() == Some(url.as_str()))
                                .cloned()
                            {
                                self.rotation_running = true;
                                self.rotation_waiting = true;
                                self.rotation_deadline = None;
                                self.apply_wallpaper(&asset, ctx, true);
                            } else {
                                self.status = Some(
                                    "Imported wallpaper is no longer available to start.".into(),
                                );
                            }
                        }
                        Some(PlaylistAfterSave::Notify(message)) => {
                            self.status = Some(message);
                        }
                        None => self.status = Some("Playlist saved.".into()),
                    }
                    if let Some(import) = self.pending_import_status.take() {
                        self.status = Some(match self.status.take() {
                            Some(status) if status != "Playlist saved." => {
                                format!("{import} · {status}")
                            }
                            _ => import,
                        });
                    }
                }
                TaskResult::PlaylistConfig(Ok(())) => {
                    self.playlist_dialog_open = false;
                    self.playlist_drag_index = None;
                    self.reload();
                    if self.rotation_running && !self.rotation_waiting {
                        self.rotation_deadline = Some(Instant::now() + self.rotation_interval());
                    }
                    self.status = Some("Playlist settings saved.".into());
                }
                TaskResult::ActiveActors(Ok(())) => {
                    self.busy = false;
                    self.status = Some("Character launch state saved.".into());
                }
                TaskResult::NamedPlaylistSaved(Ok(Some(path))) => {
                    self.reload();
                    self.status = Some(format!("Playlist saved: {path}"));
                }
                TaskResult::NamedPlaylistSaved(Ok(None)) => {}
                TaskResult::NamedPlaylistLoaded(Ok(Some(loaded))) => {
                    self.stop_rotation();
                    self.rotation_current_url = None;
                    self.reload();
                    self.status = Some(if loaded.unavailable > 0 {
                        format!(
                            "Loaded {} · {} unavailable item(s) skipped.",
                            loaded.name, loaded.unavailable
                        )
                    } else {
                        format!("Loaded {} · {} item(s).", loaded.name, loaded.urls.len())
                    });
                }
                TaskResult::NamedPlaylistLoaded(Ok(None)) => {}
                TaskResult::Wallpaper {
                    result: Ok(()),
                    assignment_saved,
                    rotation,
                    restoration,
                    select_monitor,
                    reset_audio,
                    output_name,
                    url,
                } => {
                    self.wallpaper_pause_applied = None;
                    // A replacement starts the video at zero. Restart its
                    // separate audio stream on the next bounded audio sync.
                    if reset_audio {
                        self.wallpaper_video_audio.stop();
                    }
                    if select_monitor {
                        self.selected_output = Some(output_name.clone());
                    }
                    self.wallpaper_retries.remove(&output_name);
                    self.wallpaper_assignments
                        .insert(output_name.clone(), url.clone());
                    if assignment_saved.is_ok() {
                        self.attempted_wallpaper_restores
                            .insert((output_name.clone(), url.clone()));
                    }
                    self.rotation_current_url = Some(url);
                    let assignment_error = assignment_saved.err();
                    if restoration {
                        self.restore_wallpapers_pending = true;
                    }
                    let message = if restoration {
                        if self.wallpaper.is_gnome() {
                            "Restored the GNOME wallpaper.".into()
                        } else {
                            format!("Restored wallpaper on {output_name}.")
                        }
                    } else if rotation && self.rotation_running {
                        self.rotation_waiting = false;
                        self.rotation_deadline = Some(Instant::now() + self.rotation_interval());
                        if self.wallpaper.is_gnome() {
                            "Playlist rotation is running across the GNOME desktop.".into()
                        } else {
                            "Playlist rotation is running on the selected display.".into()
                        }
                    } else {
                        self.stop_rotation();
                        if self.wallpaper.is_gnome() {
                            "Wallpaper applied across all GNOME displays.".into()
                        } else {
                            "Wallpaper applied to the selected display.".into()
                        }
                    };
                    self.status = Some(match assignment_error {
                        Some(error) => {
                            format!("{message} Its launch assignment could not be saved: {error}")
                        }
                        None => message,
                    });
                }
                TaskResult::WallpaperAnimation { result, enabled } => {
                    self.wallpaper_animation_sync_pending = false;
                    match result {
                        Ok(()) => {
                            self.wallpaper_pause_applied = Some(enabled);
                            self.wallpaper_pause_retry_at = None;
                        }
                        Err(error) => {
                            self.wallpaper_pause_applied = None;
                            self.wallpaper_pause_retry_at =
                                Some(Instant::now() + Duration::from_secs(5));
                            ctx.request_repaint_after(Duration::from_secs(5));
                            self.status = Some(error);
                        }
                    }
                }
                TaskResult::Imported {
                    result: Err(error), ..
                }
                | TaskResult::AudioAdded {
                    result: Err(error), ..
                }
                | TaskResult::AudioSaved {
                    result: Err(error), ..
                }
                | TaskResult::Favorite(Err(error))
                | TaskResult::Removed(Err(error))
                | TaskResult::AssetProperty {
                    result: Err(error), ..
                }
                | TaskResult::ActorSettings(Err(error)) => {
                    self.reload();
                    self.status = Some(error);
                }
                TaskResult::Settings(Err(error)) => self.status = Some(error),
                TaskResult::Playlist(Err(error)) => {
                    self.playlist_after_save = None;
                    self.pending_import_status = None;
                    self.reload();
                    self.status = Some(error);
                }
                TaskResult::PlaylistConfig(Err(error)) => {
                    self.reload();
                    self.status = Some(error);
                }
                TaskResult::ActiveActors(Err(error)) => {
                    self.reload();
                    self.status = Some(error);
                }
                TaskResult::NamedPlaylistSaved(Err(error))
                | TaskResult::NamedPlaylistLoaded(Err(error)) => {
                    self.status = Some(error);
                }
                TaskResult::Wallpaper {
                    result: Err(error),
                    rotation,
                    restoration,
                    output_name,
                    ..
                } => {
                    if rotation {
                        self.stop_rotation();
                    }
                    let recovering =
                        restoration && self.wallpaper_retries.contains_key(&output_name);
                    if !(recovering && self.schedule_wallpaper_retry(&output_name)) {
                        self.wallpaper_retries.remove(&output_name);
                        self.status = Some(error);
                    }
                }
            }
        }
        if !self.busy {
            if let Some(urls) = self.pending_actor_save.take() {
                self.persist_active_actors(urls, ctx);
            }
        }
    }

    fn matches(&self, asset: &Value) -> bool {
        library_asset_matches(asset, self.filter, self.favorites_only, &self.search)
    }

    fn hero(&mut self, ui: &mut egui::Ui) {
        let asset = self
            .assets
            .iter()
            .find(|item| {
                item["url"]
                    .as_str()
                    .is_some_and(|url| self.selected.as_deref() == Some(url))
            })
            .cloned();
        let Some(asset) = asset else {
            return;
        };
        let is_video = asset["video"].as_bool() == Some(true);
        let is_gnome = self.wallpaper.is_gnome();
        let has_audio = asset["hasAudio"].as_bool().unwrap_or(false);
        let hero_audio_enabled = asset["audioEnabled"].as_bool().unwrap_or(has_audio);
        let can_control_video =
            is_video && self.wallpaper.supports_video() && asset["missing"].as_bool() != Some(true);
        let can_control_audio =
            can_control_video && has_audio && self.ui_capabilities().audio && !self.busy;
        let hero_height = (ui.available_width() * 0.18135).clamp(190.0, 280.0);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), hero_height),
            egui::Sense::hover(),
        );
        preview::record_geometry(ui, "Hero", rect);
        ui.painter().rect_filled(rect, 18.0, PANEL);
        let url = asset["thumbnailUrl"]
            .as_str()
            .or_else(|| asset["url"].as_str());
        let video_texture = (asset["video"].as_bool() == Some(true))
            .then_some(self.video_texture.as_ref())
            .flatten();
        if let Some(texture) =
            video_texture.or_else(|| url.and_then(|url| self.thumbnails.get(url)))
        {
            paint_cover_rounded(ui, rect, texture, [18.0; 4]);
        }
        ui.painter()
            .rect_filled(rect, 18.0, Color32::from_black_alpha(22));
        paint_rounded_vertical_gradient(
            ui.painter(),
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 100.0)),
            [18.0, 18.0, 0.0, 0.0],
            Color32::from_rgba_unmultiplied(9, 6, 13, 153),
            Color32::from_rgba_unmultiplied(9, 6, 13, 0),
        );
        let fade_top = rect.bottom() - 160.0;
        paint_rounded_vertical_gradient(
            ui.painter(),
            egui::Rect::from_min_max(egui::pos2(rect.left(), fade_top), rect.max),
            [0.0, 0.0, 18.0, 18.0],
            Color32::from_rgba_unmultiplied(12, 10, 17, 0),
            Color32::from_rgba_unmultiplied(12, 10, 17, 230),
        );
        ui.painter().rect_stroke(
            rect,
            18.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
        let label = egui::pos2(rect.left() + 28.0, rect.bottom() - 100.0);
        widgets::tracked_text(
            ui.painter(),
            label,
            egui::Align2::LEFT_TOP,
            asset["name"].as_str().unwrap_or("Content"),
            egui::FontId::new(36.0, egui::FontFamily::Name("Inter SemiBold".into())),
            TEXT,
            -1.5,
        );
        ui.painter().text(
            egui::pos2(label.x + 2.0, label.y + 52.0),
            egui::Align2::LEFT_TOP,
            match asset["size"]
                .as_u64()
                .or_else(|| asset["fileSize"].as_u64())
            {
                Some(bytes) => format!("{}  ·  {}", asset_details(&asset), format_file_size(bytes)),
                None => asset_details(&asset),
            },
            egui::FontId::proportional(12.5),
            Color32::from_rgb(213, 207, 225),
        );
        let url = asset["url"].as_str().unwrap_or_default();
        let applied_to = if self.wallpaper.is_gnome() {
            self.wallpaper_assignments
                .get(GNOME_OUTPUT)
                .filter(|assigned| assigned.as_str() == url)
                .map(|_| GNOME_OUTPUT)
        } else {
            self.wallpaper_assignments
                .iter()
                .find_map(|(monitor, assigned)| (assigned == url).then_some(monitor.as_str()))
        };
        if let Some(monitor) = applied_to {
            let title = if monitor == GNOME_OUTPUT {
                "APPLIED  ·  ALL DISPLAYS".to_owned()
            } else {
                format!("APPLIED  ·  {}", monitor.to_uppercase())
            };
            let title_width = ui
                .painter()
                .layout_no_wrap(
                    title.clone(),
                    egui::FontId::proportional(9.5),
                    Color32::from_rgb(52, 211, 153),
                )
                .size()
                .x;
            let chip = egui::Rect::from_min_size(
                rect.min + egui::vec2(24.0, 20.0),
                Vec2::new((title_width + 42.0).clamp(186.0, 240.0), 28.0),
            );
            ui.painter()
                .rect_filled(chip, 14.0, Color32::from_rgba_unmultiplied(18, 16, 23, 140));
            ui.painter().rect_stroke(
                chip,
                14.0,
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(58, 52, 72, 77)),
                egui::StrokeKind::Inside,
            );
            let group_width = 5.0 + 6.0 + title_width;
            let group_left = chip.center().x - group_width * 0.5;
            ui.painter().circle_filled(
                egui::pos2(group_left + 2.5, chip.center().y),
                2.5,
                Color32::from_rgb(52, 211, 153),
            );
            ui.painter().text(
                egui::pos2(group_left + 11.0, chip.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                egui::FontId::proportional(9.5),
                SECONDARY,
            );
        }
        let favorite = asset["favorite"].as_bool().unwrap_or(false);
        let favorite_enabled = self.managed_urls.contains(url) && !self.busy;
        let heart_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 100.0, rect.top() + 20.0),
            Vec2::splat(36.0),
        );
        ui.painter()
            .circle_filled(heart_rect.center(), 18.0, Color32::from_black_alpha(150));
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
        let heart = ui.interact(
            heart_rect,
            egui::Id::new("hero-favorite"),
            if favorite_enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let heart = heart.on_hover_text(if self.busy {
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
        if heart.clicked() {
            self.toggle_favorite(url.to_owned(), !favorite, ui.ctx());
        }
        let more_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 56.0, rect.top() + 20.0),
            Vec2::splat(36.0),
        );
        ui.painter()
            .circle_filled(more_rect.center(), 18.0, Color32::from_black_alpha(150));
        paint_action_icon(ui.painter(), more_rect.center(), "more", TEXT);
        if ui
            .interact(more_rect, egui::Id::new("hero-more"), egui::Sense::click())
            .clicked()
        {
            self.hero_menu_open = !self.hero_menu_open;
        }
        let audio_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 128.0, rect.bottom() - 72.0),
            Vec2::splat(44.0),
        );
        let audio_fill = if can_control_audio {
            Color32::from_black_alpha(140)
        } else {
            Color32::from_black_alpha(112)
        };
        ui.painter().rect_filled(audio_rect, 22.0, audio_fill);
        ui.painter().rect_stroke(
            audio_rect,
            22.0,
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(58, 52, 72, 77)),
            egui::StrokeKind::Inside,
        );
        paint_speaker_icon(ui.painter(), audio_rect, hero_audio_enabled);
        let audio_response = ui.interact(
            audio_rect,
            egui::Id::new("hero-audio-toggle"),
            if can_control_audio {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let audio_response = audio_response.on_hover_text(if can_control_audio {
            if hero_audio_enabled {
                "Mute wallpaper audio"
            } else {
                "Enable wallpaper audio"
            }
        } else if self.busy {
            "Wait for the current operation to finish."
        } else if !has_audio {
            "This video has no audio track."
        } else {
            "Wallpaper audio control is unavailable in this session."
        });
        if audio_response.clicked()
            && let Some(url) = asset["url"].as_str()
        {
            self.update_asset_property(
                url.to_owned(),
                "audioEnabled",
                serde_json::json!(!hero_audio_enabled),
                ui.ctx(),
            );
        }
        let play_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 72.0, rect.bottom() - 72.0),
            Vec2::splat(48.0),
        );
        let can_preview = self.ui_capabilities().video
            && is_video
            && asset["missing"].as_bool() != Some(true)
            && self.video_preview.is_available();
        let can_apply = self.ui_capabilities().wallpaper
            && asset["missing"].as_bool() != Some(true)
            && self.wallpaper_format_supported(&asset)
            && (is_gnome || self.selected_output.is_some());
        let hero_action_enabled = (can_preview || can_apply) && !self.busy;
        let play = ui.interact(
            play_rect,
            egui::Id::new("hero-preview"),
            if hero_action_enabled {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        ui.painter().circle_filled(
            play_rect.center(),
            24.0,
            if hero_action_enabled { ACCENT } else { MUTED },
        );
        if is_video
            && self
                .preview
                .as_ref()
                .map_or_else(|| self.video_preview.is_playing(), |p| p.video_playing)
        {
            for offset_x in [-4.0, 4.0] {
                let bar = egui::Rect::from_center_size(
                    play_rect.center() + egui::vec2(offset_x, 0.0),
                    Vec2::new(4.0, 17.0),
                );
                ui.painter().rect_filled(bar, 1.0, Color32::BLACK);
            }
        } else {
            paint_play_outline(
                ui.painter(),
                play_rect.center(),
                Color32::BLACK,
                Vec2::new(18.0, 20.25),
                2.2,
            );
        }
        if play.clicked() {
            if self.preview_action(preview::UiAction::Operation {
                name: "video_preview".into(),
            }) {
            } else if can_preview {
                if let Err(error) = self.video_preview.toggle_playback() {
                    self.status = Some(format!("Could not control video preview: {error}"));
                }
            } else if can_apply {
                self.apply_wallpaper(&asset, ui.ctx(), false);
            }
        }
        ui.painter().rect_stroke(
            rect,
            16.0,
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );

        if self.hero_menu_open {
            let menu_pos = egui::pos2(rect.right() - 272.0, rect.top() + 56.0);
            let can_add = asset["kind"].as_str() != Some("character")
                && !self.playlist.iter().any(|item| item == url)
                && self.playlist.len() < 1000
                && self.managed_urls.contains(url)
                && asset["missing"].as_bool() != Some(true);
            let can_favorite =
                self.managed_urls.contains(url) && asset["missing"].as_bool() != Some(true);
            let can_open = file_path(url)
                .ok()
                .and_then(|path| path.parent().map(PathBuf::from))
                .is_some();
            let can_remove = self.managed_urls.contains(url)
                && asset["missing"].as_bool() != Some(true)
                && self
                    .overlay
                    .as_ref()
                    .and_then(|overlay| overlay.worker(url))
                    .is_none()
                && !self.saved_actor_urls.iter().any(|active| active == url)
                && !self
                    .wallpaper_assignments
                    .values()
                    .any(|assigned| assigned.as_str() == url);
            let mut action = None;
            let context_menu = egui::Area::new(egui::Id::new("hero-quick-menu"))
                .order(egui::Order::Foreground)
                .fixed_pos(menu_pos)
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::default()
                        .fill(Color32::from_rgb(33, 30, 43))
                        .stroke(Stroke::new(1.0_f32, Color32::from_rgb(55, 51, 65)))
                        .corner_radius(12)
                        .inner_margin(6)
                        .show(ui, |ui| {
                            ui.set_width(246.0);
                            ui.set_min_height(226.0);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if context_menu_item(
                                ui,
                                "play",
                                "Apply",
                                can_apply && !self.busy,
                                SECONDARY,
                            ) {
                                action = Some(0);
                            }
                            if context_menu_item(
                                ui,
                                "music",
                                "Add to playlist",
                                can_add && !self.busy,
                                SECONDARY,
                            ) {
                                action = Some(1);
                            }
                            let favorite_label = if asset["favorite"].as_bool().unwrap_or(false) {
                                "Remove favorite"
                            } else {
                                "Add favorite"
                            };
                            if context_menu_item(
                                ui,
                                "heart",
                                favorite_label,
                                can_favorite && !self.busy,
                                SECONDARY,
                            ) {
                                action = Some(2);
                            }
                            if context_menu_item(
                                ui,
                                "folder",
                                "Open containing folder",
                                can_open,
                                SECONDARY,
                            ) {
                                action = Some(3);
                            }
                            if context_menu_item(ui, "library", "Show in library", true, SECONDARY)
                            {
                                action = Some(4);
                            }
                            let (divider, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), 1.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().line_segment(
                                [divider.min, divider.right_top()],
                                Stroke::new(1.0_f32, BORDER),
                            );
                            ui.add_space(6.0);
                            if context_menu_item(
                                ui,
                                "trash",
                                "Remove from library",
                                can_remove && !self.busy,
                                Color32::from_rgb(251, 113, 133),
                            ) {
                                action = Some(5);
                            }
                        });
                });
            if let Some(action) = action {
                self.hero_menu_open = false;
                match action {
                    0 => {
                        self.apply_wallpaper(&asset, ui.ctx(), false);
                    }
                    1 => {
                        let mut playlist = self.playlist.clone();
                        playlist.push(url.to_owned());
                        self.playlist = playlist.clone();
                        self.save_playlist(playlist, ui.ctx());
                    }
                    2 => {
                        self.toggle_favorite(
                            url.to_owned(),
                            !asset["favorite"].as_bool().unwrap_or(false),
                            ui.ctx(),
                        );
                    }
                    3 => match file_path(url) {
                        Ok(path) => match path.parent() {
                            Some(parent) => self.open_folder(parent),
                            None => self.status = Some("The item has no containing folder.".into()),
                        },
                        Err(error) => {
                            self.status = Some(format!("Could not locate this item: {error}"))
                        }
                    },
                    4 => {
                        self.page = Page::Library;
                        self.filter = Filter::All;
                        self.favorites_only = false;
                        self.search.clear();
                        self.selected = Some(url.to_owned());
                    }
                    5 => {
                        self.remove_confirmation = Some((
                            url.to_owned(),
                            asset["name"].as_str().unwrap_or("Content").to_owned(),
                        ));
                    }
                    _ => {}
                }
            } else {
                let dismiss = ui.ctx().input(|input| {
                    input.key_pressed(egui::Key::Escape)
                        || (input.pointer.any_click()
                            && input.pointer.interact_pos().is_some_and(|pos| {
                                !context_menu.response.rect.contains(pos)
                                    && !more_rect.contains(pos)
                            }))
                });
                if dismiss {
                    self.hero_menu_open = false;
                }
            }
        }
        self.wallpaper_controls(ui, &asset);
    }

    fn asset_context_menu(
        &mut self,
        response: &egui::Response,
        asset: &Value,
        ctx: &egui::Context,
    ) {
        let Some(url) = asset["url"].as_str().map(str::to_owned) else {
            return;
        };
        let is_character = asset["kind"].as_str() == Some("character");
        let missing = asset["missing"].as_bool() == Some(true);
        let can_apply = !missing
            && (is_character
                || (self.wallpaper_format_supported(&asset)
                    && (self.wallpaper.is_gnome() || self.selected_output.is_some())));
        let can_add = !is_character
            && !missing
            && self.managed_urls.contains(&url)
            && !self.playlist.iter().any(|item| item == &url)
            && self.playlist.len() < 1000;
        let can_favorite = !missing && self.managed_urls.contains(&url);
        let can_open = file_path(&url)
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
            .is_some();
        let can_remove = !missing
            && self.managed_urls.contains(&url)
            && self
                .overlay
                .as_ref()
                .and_then(|overlay| overlay.worker(&url))
                .is_none()
            && !self.saved_actor_urls.iter().any(|active| active == &url)
            && !self
                .wallpaper_assignments
                .values()
                .any(|assigned| assigned.as_str() == url);
        let mut action = None;

        response.context_menu(|menu_ui| {
            egui::Frame::default()
                .fill(Color32::from_rgb(33, 30, 43))
                .stroke(Stroke::new(1.0_f32, Color32::from_rgb(55, 51, 65)))
                .corner_radius(12)
                .inner_margin(6)
                .show(menu_ui, |ui| {
                    ui.set_width(246.0);
                    ui.set_min_height(226.0);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (index, (icon, label, enabled, color)) in [
                        ("play", "Apply", can_apply && !self.busy, SECONDARY),
                        ("music", "Add to playlist", can_add && !self.busy, SECONDARY),
                        (
                            "heart",
                            if asset["favorite"].as_bool().unwrap_or(false) {
                                "Remove favorite"
                            } else {
                                "Add favorite"
                            },
                            can_favorite && !self.busy,
                            SECONDARY,
                        ),
                        ("folder", "Open containing folder", can_open, SECONDARY),
                        ("library", "Show in library", true, SECONDARY),
                        (
                            "trash",
                            "Remove from library",
                            can_remove && !self.busy,
                            Color32::from_rgb(251, 113, 133),
                        ),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if index == 5 {
                            ui.add_space(2.0);
                            ui.separator();
                        }
                        if context_menu_item(ui, icon, label, enabled, color) {
                            action = Some(index);
                            ui.close_menu();
                        }
                    }
                });
        });

        let Some(action) = action else { return };
        match action {
            0 => self.open_or_apply_asset(asset, ctx),
            1 => {
                let mut playlist = self.playlist.clone();
                playlist.push(url.clone());
                self.playlist = playlist.clone();
                self.save_playlist(playlist, ctx);
            }
            2 => self.toggle_favorite(
                url.clone(),
                !asset["favorite"].as_bool().unwrap_or(false),
                ctx,
            ),
            3 => match file_path(&url) {
                Ok(path) => match path.parent() {
                    Some(parent) => self.open_folder(parent),
                    None => self.status = Some("The item has no containing folder.".into()),
                },
                Err(error) => self.status = Some(format!("Could not locate this item: {error}")),
            },
            4 => {
                self.page = Page::Library;
                self.filter = Filter::All;
                self.favorites_only = false;
                self.search.clear();
                self.selected = Some(url);
            }
            5 => {
                self.remove_confirmation =
                    Some((url, asset["name"].as_str().unwrap_or("Content").to_owned()));
            }
            _ => {}
        }
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut items: Vec<Value> = self
            .assets
            .iter()
            .filter(|asset| self.matches(asset))
            .cloned()
            .collect();
        if self.sort_by_name {
            items.sort_by(|left, right| {
                left["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&right["name"].as_str().unwrap_or_default().to_lowercase())
            });
        } else {
            items.sort_by(|left, right| {
                right["addedAt"]
                    .as_i64()
                    .unwrap_or_default()
                    .cmp(&left["addedAt"].as_i64().unwrap_or_default())
            });
        }
        // The hero follows the filtered items: drop a selection that is filtered out.
        let selected_visible = self.selected.as_deref().is_some_and(|selected| {
            items
                .iter()
                .any(|asset| asset["url"].as_str() == Some(selected))
        });
        if !selected_visible {
            self.selected = items
                .iter()
                .find(|asset| asset["missing"].as_bool() != Some(true))
                .and_then(|asset| asset["url"].as_str())
                .map(str::to_owned);
        }
        // One vertical scroll area only: short windows scroll the whole page,
        // tall windows keep the hero and toolbar fixed and scroll the grid.
        if ui.available_height() < 700.0 {
            egui::ScrollArea::vertical()
                .id_salt("library-page")
                .show(ui, |ui| {
                    self.library_header(ui);
                    self.library_body(ui, &items);
                });
        } else {
            self.library_header(ui);
            egui::ScrollArea::vertical()
                .id_salt("library-grid")
                .show(ui, |ui| self.library_body(ui, &items));
        }
    }

    fn library_header(&mut self, ui: &mut egui::Ui) {
        self.hero(ui);
        ui.add_space(16.0);
        let filter_button = self.library_toolbar(ui);
        if self.filter_menu_open {
            let ctx = ui.ctx().clone();
            let mut content_type = match self.filter {
                Filter::Wallpapers => Filter::Wallpapers,
                Filter::Animations => Filter::Animations,
                Filter::All | Filter::Favorites => Filter::All,
            };
            let mut favorites_only = self.favorites_only || self.filter == Filter::Favorites;
            let mut clear_all = false;
            let mut show_results = false;
            let popup_pos = filter_button.rect.left_bottom() + egui::vec2(0.0, 8.0);
            let popup = egui::Area::new(egui::Id::new("library-filter-popover"))
                .order(egui::Order::Foreground)
                .fixed_pos(popup_pos)
                .constrain(true)
                .show(&ctx, |area_ui| {
                    (clear_all, show_results) = widgets::filter_popover(
                        area_ui,
                        &self.assets,
                        &self.search,
                        &mut content_type,
                        &mut favorites_only,
                    );
                });

            if clear_all {
                self.filter = Filter::All;
                self.favorites_only = false;
                self.filter_menu_open = false;
            } else {
                self.filter = content_type;
                self.favorites_only = favorites_only;
                if show_results {
                    self.filter_menu_open = false;
                }
            }

            let dismiss = ctx.input(|input| {
                input.key_pressed(egui::Key::Escape)
                    || (input.pointer.any_click()
                        && input.pointer.interact_pos().is_some_and(|pos| {
                            !popup.response.rect.contains(pos) && !filter_button.rect.contains(pos)
                        }))
            });
            if dismiss {
                self.filter_menu_open = false;
            }
        }
        // Fixed-height status row so loading/error text never shifts the grid.
        let error = self.error.clone();
        let busy = self.busy;
        let mut retry = false;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 24.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if let Some(error) = &error {
                    if ui.button(RichText::new("Retry").color(ACCENT)).clicked() {
                        retry = true;
                    }
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("Could not load library: {error}"))
                                .color(Color32::from_rgb(251, 113, 133)),
                        )
                        .truncate(),
                    );
                } else if busy {
                    ui.label(RichText::new("Working…").color(MUTED));
                }
            },
        );
        if retry {
            self.reload();
        }
    }

    /// Paints the favorite heart, status badges and duration pill over a thumbnail.
    /// Missing files get a dimmed thumbnail and a MISSING badge instead.
    fn paint_asset_overlays(
        &self,
        ui: &mut egui::Ui,
        asset: &Value,
        artwork: egui::Rect,
        radius: egui::CornerRadius,
        compact: bool,
    ) -> Option<egui::Response> {
        let painter = ui.painter().clone();
        let measure = |text: &str, font: &egui::FontId| -> f32 {
            painter
                .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x
        };
        let badge_font = egui::FontId::new(9.0, egui::FontFamily::Name("Inter SemiBold".into()));
        let badge_bg = Color32::from_black_alpha(170);
        if asset["missing"].as_bool() == Some(true) {
            painter.rect_filled(artwork, radius, Color32::from_black_alpha(160));
            let pill = egui::Rect::from_min_size(
                artwork.min + egui::vec2(8.0, 8.0),
                Vec2::new(measure("MISSING", &badge_font) + 18.0, 22.0),
            );
            painter.rect_filled(pill, 11.0, Color32::from_black_alpha(205));
            painter.text(
                pill.center(),
                egui::Align2::CENTER_CENTER,
                "MISSING",
                badge_font,
                Color32::from_rgb(251, 113, 133),
            );
            return None;
        }
        let favorite = asset["favorite"].as_bool().unwrap_or(false);
        let asset_url = asset["url"].as_str().unwrap_or_default();
        let favorite_enabled = self.managed_urls.contains(asset_url) && !self.busy;
        let (heart_size, heart_offset) = if compact { (22.0, 5.0) } else { (26.0, 8.0) };
        let heart = egui::Rect::from_min_size(
            artwork.min + egui::vec2(heart_offset, heart_offset),
            Vec2::splat(heart_size),
        );
        painter.rect_filled(heart, heart_size / 2.0, badge_bg);
        paint_action_icon(
            &painter,
            heart.center(),
            if favorite { "heart-filled" } else { "♡" },
            if favorite {
                Color32::from_rgb(251, 113, 133)
            } else if favorite_enabled {
                TEXT
            } else {
                MUTED
            },
        );
        let favorite_response = ui
            .interact(
                heart,
                egui::Id::new(("library-favorite", asset_url, compact)),
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
        let applied = self
            .wallpaper_assignments
            .values()
            .any(|url| url == asset_url);
        let badge: String = if applied {
            "Applied".into()
        } else {
            asset_badge(asset)
        };
        let live = badge == "LIVE";
        let green = applied || live;
        let text_start = if applied {
            28.0
        } else if live {
            18.0
        } else {
            9.0
        };
        let badge_width = text_start + measure(&badge, &badge_font) + 10.0;
        let badge_rect = egui::Rect::from_min_size(
            egui::pos2(
                artwork.right() - badge_width - heart_offset,
                artwork.top() + heart_offset,
            ),
            Vec2::new(badge_width, 22.0),
        );
        painter.rect_filled(
            badge_rect,
            11.0,
            if green {
                Color32::from_rgb(30, 58, 50)
            } else {
                badge_bg
            },
        );
        painter.text(
            egui::pos2(badge_rect.left() + text_start, badge_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &badge,
            badge_font,
            if green {
                Color32::from_rgb(110, 231, 183)
            } else {
                TEXT
            },
        );
        if applied {
            icons::paint_node(
                &painter,
                badge_rect.left_center() + Vec2::new(18.0, 0.0),
                962,
                Color32::from_rgb(110, 231, 183),
                Some(Vec2::new(11.0, 7.5)),
            );
        } else if live {
            painter.circle_filled(
                badge_rect.left_center() + Vec2::new(10.0, 0.0),
                2.5,
                Color32::from_rgb(52, 211, 153),
            );
        }
        if asset["video"].as_bool() == Some(true)
            && let Some(duration) = asset["duration"]
                .as_f64()
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        {
            let duration = format_duration(duration.round() as u64);
            let duration_font = egui::FontId::proportional(9.0);
            let pill_width = (18.0 + measure(&duration, &duration_font) + 10.0).max(46.0);
            let pill = egui::Rect::from_min_size(
                egui::pos2(artwork.right() - pill_width - 8.0, artwork.bottom() - 28.0),
                Vec2::new(pill_width, 20.0),
            );
            painter.rect_filled(pill, 10.0, Color32::from_black_alpha(185));
            paint_play_outline(
                &painter,
                pill.left_center() + Vec2::new(10.0, 0.0),
                TEXT,
                Vec2::new(8.0, 9.0),
                1.3,
            );
            painter.text(
                egui::pos2(pill.left() + 18.0, pill.center().y),
                egui::Align2::LEFT_CENTER,
                duration,
                duration_font,
                TEXT,
            );
        }
        Some(favorite_response)
    }

    fn library_body(&mut self, ui: &mut egui::Ui, items: &[Value]) {
        if self.error.is_some() {
            // The status row above explains the failure and offers Retry.
            return;
        }
        if items.is_empty() {
            let library_empty = self.assets.is_empty();
            if library_empty && self.busy {
                ui.vertical_centered(|ui| {
                    ui.add_space(48.0);
                    ui.label(RichText::new("Loading…").color(MUTED));
                });
                return;
            }
            let query = self.search.trim();
            let no_results = if query.is_empty() {
                "No results for the current filters".to_owned()
            } else {
                let shown: String = query.chars().take(40).collect();
                let ellipsis = if query.chars().count() > 40 {
                    "…"
                } else {
                    ""
                };
                format!("No results for “{shown}{ellipsis}”")
            };
            let mut import_clicked = false;
            let mut browse_clicked = false;
            let mut clear_clicked = false;
            ui.vertical_centered(|ui| {
                egui::Frame::default()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(if library_empty { 14 } else { 12 })
                    .inner_margin(if library_empty { 24 } else { 20 })
                    .show(ui, |ui| {
                        ui.set_min_width(if library_empty { 392.0 } else { 360.0 });
                        if library_empty {
                            ui.set_min_height(232.0);
                            ui.vertical_centered(|ui| {
                                let (icon_rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(48.0), egui::Sense::hover());
                                ui.painter().rect_filled(
                                    icon_rect,
                                    12.0,
                                    Color32::from_rgb(42, 36, 63),
                                );
                                paint_image_icon(ui.painter(), icon_rect.center(), ACCENT);
                                ui.add_space(10.0);
                                ui.heading("Your library starts here");
                                ui.label(
                                    RichText::new(
                                        "Import a wallpaper or character to personalize your desktop.",
                                    )
                                    .color(MUTED),
                                );
                                ui.add_space(10.0);
                                let import = icon_text_button(
                                    ui,
                                    "upload",
                                    "Import content",
                                    Vec2::new(168.0, 40.0),
                                    !self.busy,
                                    ACCENT,
                                    SURFACE,
                                    Color32::TRANSPARENT,
                                );
                                import_clicked = import.clicked();
                                ui.add_space(14.0);
                                browse_clicked = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("or browse Discover").color(ACCENT),
                                        )
                                        .min_size(Vec2::new(160.0, 34.0)),
                                    )
                                    .clicked();
                            });
                        } else {
                            ui.vertical_centered(|ui| {
                                ui.heading(&no_results);
                                ui.label(
                                    RichText::new("Try a different search or clear your filters.")
                                        .color(MUTED),
                                );
                                ui.add_space(8.0);
                                clear_clicked = ui
                                    .button(RichText::new("Clear filters").color(ACCENT))
                                    .clicked();
                            });
                        }
                    });
            });
            if import_clicked {
                self.choose_files(ui.ctx());
            }
            if browse_clicked {
                self.page = Page::Discover;
            }
            if clear_clicked {
                self.search.clear();
                self.filter = Filter::All;
                self.favorites_only = false;
            }
            return;
        }
        if !self.grid_view {
            for asset in items {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), 82.0),
                    egui::Sense::click(),
                );
                let selected = asset["url"]
                    .as_str()
                    .is_some_and(|url| self.selected.as_deref() == Some(url));
                let missing = asset["missing"].as_bool() == Some(true);
                let hovered = response.hovered();
                if hovered {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                ui.painter().rect_filled(
                    rect,
                    10.0,
                    if hovered {
                        Color32::from_rgb(29, 25, 39)
                    } else {
                        PANEL
                    },
                );
                let art = egui::Rect::from_min_size(
                    rect.min + egui::vec2(6.0, 6.0),
                    Vec2::new(124.0, 70.0),
                );
                if let Some(texture) = asset["thumbnailUrl"]
                    .as_str()
                    .or_else(|| asset["url"].as_str())
                    .and_then(|url| self.thumbnails.get(url))
                {
                    paint_cover_rounded(ui, art, texture, [6.0; 4]);
                }
                let favorite_response =
                    self.paint_asset_overlays(ui, asset, art, egui::CornerRadius::same(6), true);
                let painter = ui.painter().with_clip_rect(rect);
                let text_width = rect.right() - art.right() - 30.0;
                let (name_color, detail_color) = if missing {
                    (SECONDARY, Color32::from_rgb(110, 104, 124))
                } else {
                    (TEXT, MUTED)
                };
                painter.text(
                    egui::pos2(art.right() + 14.0, rect.top() + 20.0),
                    egui::Align2::LEFT_TOP,
                    crate::widgets::ellipsize(
                        &painter,
                        asset["name"].as_str().unwrap_or("Content"),
                        &egui::FontId::proportional(15.0),
                        text_width,
                    ),
                    egui::FontId::proportional(15.0),
                    name_color,
                );
                painter.text(
                    egui::pos2(art.right() + 14.0, rect.top() + 44.0),
                    egui::Align2::LEFT_TOP,
                    crate::widgets::ellipsize(
                        &painter,
                        &if missing {
                            "Unavailable".to_owned()
                        } else {
                            asset_details(asset)
                        },
                        &egui::FontId::proportional(11.0),
                        text_width,
                    ),
                    egui::FontId::proportional(11.0),
                    detail_color,
                );
                ui.painter().rect_stroke(
                    rect,
                    10.0,
                    if selected {
                        Stroke::new(1.5_f32, ACCENT)
                    } else {
                        Stroke::new(
                            1.0_f32,
                            if hovered {
                                Color32::from_rgb(62, 56, 80)
                            } else {
                                BORDER
                            },
                        )
                    },
                    egui::StrokeKind::Inside,
                );
                let favorite_hovered = favorite_response.as_ref().is_some_and(|r| r.hovered());
                if favorite_response.as_ref().is_some_and(|r| r.clicked()) {
                    let asset_url = asset["url"].as_str().unwrap_or_default().to_owned();
                    let favorite = asset["favorite"].as_bool().unwrap_or(false);
                    self.toggle_favorite(asset_url, !favorite, ui.ctx());
                }
                self.asset_context_menu(&response, asset, ui.ctx());
                if response.double_clicked() && !favorite_hovered {
                    self.open_or_apply_asset(asset, ui.ctx());
                } else if response.clicked() && !favorite_hovered {
                    self.selected = asset["url"].as_str().map(str::to_owned);
                }
                ui.add_space(8.0);
            }
            return;
        }
        let card_gap = 16.0;
        let columns = ((ui.available_width() + card_gap) / 260.0).floor().max(1.0) as usize;
        let width = ((ui.available_width() - (columns.saturating_sub(1) as f32 * card_gap))
            / columns as f32)
            .max(120.0);
        ui.spacing_mut().item_spacing = egui::vec2(card_gap, card_gap);
        for row in items.chunks(columns) {
            ui.horizontal_top(|ui| {
                for asset in row {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(width, width * 0.56 + 47.0),
                        egui::Sense::click(),
                    );
                    let selected = asset["url"]
                        .as_str()
                        .is_some_and(|url| self.selected.as_deref() == Some(url));
                    let missing = asset["missing"].as_bool() == Some(true);
                    let hovered = response.hovered();
                    if hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    ui.painter().rect_filled(
                        rect,
                        12.0,
                        if hovered {
                            Color32::from_rgb(29, 25, 39)
                        } else {
                            PANEL
                        },
                    );
                    ui.painter().rect_stroke(
                        rect,
                        12.0,
                        Stroke::new(
                            1.0_f32,
                            if hovered {
                                Color32::from_rgb(62, 56, 80)
                            } else {
                                BORDER
                            },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    preview::record_geometry(
                        ui,
                        format!("{} Card", asset["name"].as_str().unwrap_or("Content")),
                        rect,
                    );
                    let artwork =
                        egui::Rect::from_min_size(rect.min, Vec2::new(width, width * 0.56));
                    let url = asset["thumbnailUrl"]
                        .as_str()
                        .or_else(|| asset["url"].as_str());
                    if ui.is_rect_visible(artwork) {
                        if let Some(texture) = url.and_then(|url| self.thumbnails.get(url)) {
                            if asset["kind"].as_str() == Some("character") {
                                paint_character_stage(ui, artwork, texture, asset);
                            } else {
                                paint_cover_rounded(ui, artwork, texture, [12.0, 12.0, 0.0, 0.0]);
                            }
                        }
                    }
                    let favorite_response = self.paint_asset_overlays(
                        ui,
                        asset,
                        artwork,
                        egui::CornerRadius {
                            nw: 12,
                            ne: 12,
                            sw: 0,
                            se: 0,
                        },
                        false,
                    );
                    let asset_url = asset["url"].as_str().unwrap_or_default();
                    let favorite = asset["favorite"].as_bool().unwrap_or(false);
                    let name = asset["name"].as_str().unwrap_or("Content");
                    let label_painter = ui.painter().with_clip_rect(rect);
                    let name_font =
                        egui::FontId::new(13.0, egui::FontFamily::Name("Inter SemiBold".into()));
                    let name = crate::widgets::ellipsize(
                        &label_painter,
                        name,
                        &name_font,
                        rect.width() - 24.0,
                    );
                    label_painter.text(
                        artwork.left_bottom() + egui::vec2(12.0, 9.0),
                        egui::Align2::LEFT_TOP,
                        name,
                        name_font,
                        if missing { SECONDARY } else { TEXT },
                    );
                    label_painter.text(
                        artwork.left_bottom() + egui::vec2(12.0, 27.0),
                        egui::Align2::LEFT_TOP,
                        crate::widgets::ellipsize(
                            &label_painter,
                            &if missing {
                                "Unavailable".to_owned()
                            } else {
                                card_details(asset)
                            },
                            &egui::FontId::proportional(10.5),
                            rect.width() - 24.0,
                        ),
                        egui::FontId::proportional(10.5),
                        MUTED,
                    );
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            12.0,
                            Stroke::new(1.5_f32, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let favorite_hovered = favorite_response.as_ref().is_some_and(|r| r.hovered());
                    if favorite_response.as_ref().is_some_and(|r| r.clicked()) {
                        self.toggle_favorite(asset_url.to_owned(), !favorite, ui.ctx());
                    }
                    self.asset_context_menu(&response, asset, ui.ctx());
                    if response.double_clicked() && !favorite_hovered {
                        self.open_or_apply_asset(asset, ui.ctx());
                    } else if response.clicked() && !favorite_hovered {
                        self.selected = asset["url"].as_str().map(str::to_owned);
                    }
                }
            });
        }
    }
}

fn install_theme(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-Medium".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-Medium.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".into(),
        std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
            "../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    if let Some(proportional) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        proportional.insert(0, "Inter".into());
    }
    fonts
        .families
        .insert(egui::FontFamily::Name("Inter".into()), vec!["Inter".into()]);
    fonts.families.insert(
        egui::FontFamily::Name("Inter Medium".into()),
        vec!["Inter-Medium".into(), "Inter".into()],
    );
    fonts.families.insert(
        egui::FontFamily::Name("Inter SemiBold".into()),
        vec![
            "Inter-SemiBold".into(),
            "Inter-Medium".into(),
            "Inter".into(),
        ],
    );
    icon_set::register_fonts(&mut fonts);
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = SURFACE;
    style.visuals.window_fill = PANEL;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.widgets.noninteractive.bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_fill = PANEL;
    style.visuals.widgets.inactive.weak_bg_fill = PANEL;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(42, 36, 61);
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(42, 36, 61);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(58, 49, 83);
    style.visuals.selection.bg_fill = ACCENT;
    style.spacing.item_spacing = egui::vec2(12.0, 10.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size = egui::vec2(40.0, 38.0);
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        widget.corner_radius = 8.into();
    }
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(14.0, egui::FontFamily::Name("Inter Medium".into())),
    );
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(20.0, egui::FontFamily::Name("Inter SemiBold".into())),
    );
    ctx.set_style(style);
}

fn window_textures(
    ctx: &egui::Context,
) -> (Option<egui::TextureHandle>, Option<egui::TextureHandle>) {
    let logo = image::load_from_memory(include_bytes!("../../../assets/waniengine.png"))
        .ok()
        .map(|image| image.thumbnail(96, 96).to_rgba8())
        .map(|image| {
            ctx.load_texture(
                "wani-logo",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    &image,
                ),
                egui::TextureOptions::LINEAR,
            )
        });
    let discover_background =
        image::load_from_memory(include_bytes!("../assets/discover-featured-bg.png"))
            .ok()
            .map(|image| image.to_rgba8())
            .map(|image| {
                ctx.load_texture(
                    "discover-featured-background",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        &image,
                    ),
                    egui::TextureOptions::LINEAR,
                )
            });
    (logo, discover_background)
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.run_frame(ctx, true);
    }
}

impl App {
    /// One pass of the manager. Without a window (`windowed` false) only the
    /// desktop services run: nothing is drawn and no textures are loaded.
    fn run_frame(&mut self, ctx: &egui::Context, windowed: bool) {
        if windowed {
            self.poll_settings_backdrop(ctx);
            self.preview_tick(ctx);
            self.thumbnails.poll(ctx);
        }
        if self.preview.is_none() {
            let requests = self
                .control
                .as_ref()
                .map(|control| control.take_requests().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut explicit_exit =
                ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Q));
            for request in requests {
                match request {
                    control::ControlRequest::Exit => explicit_exit = true,
                    control::ControlRequest::Show => self.show_window(ctx),
                }
            }
            if explicit_exit {
                self.force_exit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            let (focus_library_search, toggle_rotation) = ctx.input(|input| {
                let command = input.modifiers.command;
                let unmodified =
                    !input.modifiers.command && !input.modifiers.shift && !input.modifiers.alt;
                (
                    command && input.key_pressed(egui::Key::K),
                    unmodified && input.key_pressed(egui::Key::Space),
                )
            });
            let modal_open = self.page == Page::Settings
                || self.import_files.is_some()
                || self.remove_confirmation.is_some()
                || self.playlist_dialog_open
                || self.audio_dialog.is_some()
                || self.filter_menu_open
                || self.hero_menu_open;
            if focus_library_search && !modal_open {
                self.page = Page::Library;
                self.focus_library_search = true;
            }
            if toggle_rotation
                && !modal_open
                && !ctx.wants_keyboard_input()
                && ctx.memory(|memory| memory.focused().is_none())
            {
                self.toggle_rotation(ctx);
            }
            let actions = self
                .tray
                .as_ref()
                .map(|tray| tray.take_actions().collect::<Vec<_>>())
                .unwrap_or_default();
            let mut tray_unavailable = false;
            for action in actions {
                match action {
                    tray::TrayAction::Activate => self.show_window(ctx),
                    tray::TrayAction::ContextMenu => {
                        self.show_window(ctx);
                        self.open_settings(ctx);
                        self.settings_tab = 2;
                    }
                    tray::TrayAction::Exit => {
                        self.force_exit = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    tray::TrayAction::Unavailable => tray_unavailable = true,
                }
            }
            if tray_unavailable {
                self.tray = None;
                if self.window_hidden {
                    self.show_window(ctx);
                    self.status = Some(
                        "The desktop tray disconnected; WaniEngine restored its window.".into(),
                    );
                }
            }
            // Wayland cannot hide a window, so closing to the tray really
            // closes it; `main` keeps the services running without one.
            if ctx.input(|input| input.viewport().close_requested())
                && self.tray_close_enabled()
                && !self.force_exit
            {
                self.window_hidden = true;
            }
            if windowed {
                handle_borderless_resize(ctx);
                self.thumbnails.poll(ctx);
            }
            self.poll_tasks(ctx);
            self.refresh_automatic_pause(ctx);
            if windowed {
                self.sync_video_preview(ctx);
            }
            self.tick_rotation(ctx);
            self.dispatch_video_updates(ctx);
            if self.last_display_refresh.elapsed() > Duration::from_secs(2) {
                self.last_display_refresh = Instant::now();
                if let Some(overlay) = &mut self.overlay {
                    overlay.refresh_monitors();
                }
                self.restore_wallpapers_pending = true;
                if !self.busy && !self.reconcile_pending && self.wallpaper.is_gnome() {
                    self.reconcile_pending = true;
                    let controller = self.wallpaper.clone();
                    let tx = self.task_tx.clone();
                    let ctx = ctx.clone();
                    thread::spawn(move || {
                        let _panic_guard = PanicGuard::new(tx.clone(), ctx.clone());
                        let _ =
                            tx.send(TaskResult::VideoConfigured(controller.reconcile_outputs()));
                        ctx.request_repaint();
                    });
                }
            }
            if let Some(overlay) = &mut self.overlay {
                overlay.poll();
            }
            if self.selected_output.is_none() {
                self.selected_output = self
                    .overlay
                    .as_ref()
                    .and_then(|overlay| overlay.monitors.first())
                    .and_then(|monitor| monitor.name.clone());
            }
            let exited_actors = self
                .overlay
                .as_mut()
                .map(OverlayManager::take_exited)
                .unwrap_or_default();
            if !exited_actors.is_empty() {
                for url in &exited_actors {
                    self.pause_saved_playing.remove(url);
                }
                let mut active = self.saved_actor_urls.clone();
                active.retain(|url| !exited_actors.contains(url));
                if active != self.saved_actor_urls {
                    self.persist_active_actors(active, ctx);
                }
            }
            self.restore_saved_actors();
            if self.last_audio_sync.elapsed() >= Duration::from_millis(100) {
                self.last_audio_sync = Instant::now();
                if let Some((output, error)) = self.wallpaper.take_video_error() {
                    // KWin closes layer surfaces when a monitor powers off,
                    // before or instead of removing its output.
                    let output_lost = error.contains("disconnected")
                        || error.contains("closed the wallpaper surface");
                    if !(output_lost && self.schedule_wallpaper_retry(&output)) {
                        self.wallpaper_assignments.remove(&output);
                        self.status = Some(error);
                    }
                    self.stop_rotation();
                }
                // Wallpaper volume follows the slider live. Character players take their volume
                // at spawn, so dragging must not restart them: they use the saved value.
                let global_volume = self
                    .settings_draft
                    .as_ref()
                    .and_then(|settings| settings["globalVolume"].as_f64())
                    .or_else(|| self.settings["globalVolume"].as_f64())
                    .unwrap_or(100.0);
                let saved_volume = self.settings["globalVolume"].as_f64().unwrap_or(100.0);
                let audio_output = self.preferred_audio_output().map(str::to_owned);
                let active_actor_urls = if self.automatic_pause {
                    &[][..]
                } else {
                    &self.saved_actor_urls
                };
                let click_bounds = self
                    .overlay
                    .as_ref()
                    .map(|overlay| overlay.click_bounds(&self.assets))
                    .unwrap_or_default();
                let audio_errors = self.audio.sync(
                    &self.assets,
                    active_actor_urls,
                    saved_volume,
                    audio_output.as_deref(),
                    &click_bounds,
                );
                if let Some(error) = audio_errors.into_iter().next() {
                    self.status = Some(error);
                }
                self.sync_wallpaper_video_audio(global_volume);
            }
            self.restore_saved_wallpapers(ctx);
        }
        if !windowed {
            return;
        }
        self.draw_interface(ctx);
        let frame_rate = self
            .settings_draft
            .as_ref()
            .and_then(|settings| settings["frameRateLimit"].as_u64())
            .or_else(|| self.settings["frameRateLimit"].as_u64())
            .unwrap_or(60)
            .clamp(15, 240);
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / frame_rate as f64));
    }
}

impl App {
    pub(crate) fn draw_interface(&mut self, ctx: &egui::Context) {
        self.import_dialog(ctx);
        self.removal_dialog(ctx);
        self.playlist_dialog(ctx);
        self.navigation(ctx);
        self.playlist_dock(ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::default()
                    .fill(SURFACE)
                    .inner_margin(egui::Margin {
                        left: 28,
                        right: 28,
                        top: 28,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                if self.page == Page::Settings {
                    match self.settings_return_page {
                        Page::Library => self.library(ui),
                        Page::Animations => self.animations_ui(ui),
                        Page::Discover => self.discover_ui(ui),
                        Page::Settings => {}
                    }
                } else {
                    match self.page {
                        Page::Library => self.library(ui),
                        Page::Animations => self.animations_ui(ui),
                        Page::Discover => self.discover_ui(ui),
                        Page::Settings => unreachable!(),
                    }
                }
            });
        if self.page == Page::Settings {
            self.settings_overlay(ctx);
        }
        self.status_toast(ctx);
        ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("window-outline"),
        ))
        .rect_stroke(
            ctx.screen_rect(),
            egui::CornerRadius {
                nw: widgets::window_radius(ctx) / 3,
                ne: widgets::window_radius(ctx) / 3,
                sw: widgets::window_radius(ctx),
                se: widgets::window_radius(ctx),
            },
            Stroke::new(1.0_f32, BORDER),
            egui::StrokeKind::Inside,
        );
    }

    fn status_toast(&mut self, ctx: &egui::Context) {
        let Some(message) = self.status.clone() else {
            self.status_snapshot = None;
            self.status_since = None;
            return;
        };

        if self.status_snapshot.as_deref() != Some(message.as_str()) {
            self.status_snapshot = Some(message.clone());
            self.status_since = Some(Instant::now());
        }
        if self
            .status_since
            .is_some_and(|started| started.elapsed() >= Duration::from_secs(7))
        {
            self.status = None;
            self.status_snapshot = None;
            self.status_since = None;
            return;
        }

        if let Some(started) = self.status_since {
            ctx.request_repaint_after(
                Duration::from_secs(7)
                    .saturating_sub(started.elapsed())
                    .max(Duration::from_millis(50)),
            );
        }
        let lowercase_message = message.to_lowercase();
        let is_error = [
            "could not",
            "couldn't",
            "failed",
            "failure",
            "error",
            "denied",
            "invalid",
            "cannot",
            "must ",
            "outside the supported",
            "no longer owns",
            "timed out",
        ]
        .iter()
        .any(|needle| lowercase_message.contains(needle));
        let is_warning = [
            "unavailable",
            "no longer available",
            "missing",
            "not supported",
            "unsupported",
            "not yet",
            "not found",
            "no local file",
            "no file",
            "no display",
            "connect or refresh",
            "select a connected",
            "add an available",
            "does not support",
        ]
        .iter()
        .any(|needle| lowercase_message.contains(needle));
        let is_progress = ["preparing", "loading", "refreshing"]
            .iter()
            .any(|needle| lowercase_message.contains(needle));
        let accent = if is_error {
            Color32::from_rgb(251, 113, 133)
        } else if is_warning {
            Color32::from_rgb(251, 191, 36)
        } else if is_progress {
            ACCENT
        } else {
            Color32::from_rgb(52, 211, 153)
        };
        let mut dismiss = false;

        egui::Area::new(egui::Id::new("app-status-toast"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-28.0, -104.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::default()
                    .fill(Color32::from_rgb(33, 30, 43))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(500.0);
                        ui.horizontal_wrapped(|ui| {
                            status_dot(ui, accent, 4.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(message).size(11.5).color(SECONDARY),
                                )
                                .wrap(),
                            );
                            if icon_button(
                                ui,
                                "close",
                                Vec2::splat(22.0),
                                MUTED,
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                true,
                            )
                            .clicked()
                            {
                                dismiss = true;
                            }
                        });
                    });
            });

        if dismiss {
            self.status = None;
            self.status_snapshot = None;
            self.status_since = None;
        }
    }
}

fn library_asset_matches(asset: &Value, filter: Filter, favorites_only: bool, query: &str) -> bool {
    let kind = asset["kind"].as_str().unwrap_or("wallpaper");
    let matches_type = match filter {
        Filter::All | Filter::Favorites => true,
        Filter::Wallpapers => kind != "character",
        Filter::Animations => kind == "character",
    };
    let favorite = asset["favorite"].as_bool().unwrap_or(false);
    matches_type
        && (!(favorites_only || filter == Filter::Favorites) || favorite)
        && asset["name"]
            .as_str()
            .unwrap_or("Content")
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
}

fn card_details(asset: &Value) -> String {
    if asset["kind"].as_str() == Some("character") {
        return format!(
            "Character  ·  {}",
            asset["characterLoop"].as_str().unwrap_or("Idle loop")
        );
    }
    let kind = if asset["extension"].as_str() == Some("gif") {
        "GIF"
    } else if asset["video"].as_bool() == Some(true) {
        "Video"
    } else {
        "Image"
    };
    match (asset["width"].as_u64(), asset["height"].as_u64()) {
        (Some(w), Some(h)) => format!("{kind}  ·  {w} × {h}"),
        _ => kind.into(),
    }
}

pub(crate) fn paint_character_stage(
    ui: &egui::Ui,
    rect: egui::Rect,
    texture: &egui::TextureHandle,
    asset: &Value,
) {
    let (top, bottom) = match asset["stageColor"].as_str() {
        Some("yuki") => (Color32::from_rgb(34, 54, 71), Color32::from_rgb(19, 24, 32)),
        Some("sora") => (Color32::from_rgb(62, 40, 37), Color32::from_rgb(30, 23, 24)),
        _ => (
            Color32::from_rgb(75, 58, 112),
            Color32::from_rgb(32, 26, 46),
        ),
    };
    let tile = rect.width() / rect.height() < 1.4;
    paint_rounded_gradient(
        ui.painter(),
        rect,
        if tile {
            [12.0, 0.0, 0.0, 12.0]
        } else {
            [12.0, 12.0, 0.0, 0.0]
        },
        top,
        bottom,
        true,
    );
    let (glow_size, glow_y, box_size, image_y, alpha) = if tile && rect.height() > 140.0 {
        (
            Vec2::new(108.0, 100.0),
            80.0,
            Vec2::new(120.0, 144.0),
            78.0,
            if asset["stageColor"].as_str() == Some("mochi") {
                23
            } else {
                10
            },
        )
    } else if tile {
        (
            Vec2::new(80.0, 84.0),
            66.0,
            Vec2::new(92.0, 122.0),
            63.0,
            15,
        )
    } else {
        (
            Vec2::new(120.0, 104.0),
            74.0,
            Vec2::new(140.0, 132.0),
            69.0,
            10,
        )
    };
    // FigCraft composites its translucent white in sRGB. Preserve that value
    // instead of ecolor's linear-light premultiplication, which brightens it.
    ui.painter().add(egui::Shape::ellipse_filled(
        egui::pos2(rect.center().x, rect.top() + glow_y),
        glow_size * 0.5,
        Color32::from_rgba_premultiplied(alpha, alpha, alpha, alpha),
    ));
    let source = texture.size_vec2();
    let scale = (box_size.x / source.x).min(box_size.y / source.y);
    let image = egui::Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + image_y),
        source * scale,
    );
    ui.painter().image(
        texture.id(),
        image,
        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

fn asset_details(asset: &Value) -> String {
    let mut pieces = Vec::new();
    if let (Some(width), Some(height)) = (asset["width"].as_u64(), asset["height"].as_u64()) {
        if width > 0 && height > 0 {
            pieces.push(format!("{width} × {height}"));
        }
    }
    pieces.push(
        if asset["kind"].as_str() == Some("character") {
            "Character"
        } else if asset["video"].as_bool() == Some(true) {
            "Video"
        } else {
            "Image"
        }
        .to_owned(),
    );
    if let Some(fps) = asset["nativeFps"]
        .as_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        pieces.push(format!("{fps:.0} FPS"));
    }
    pieces.join("  ·  ")
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_duration(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn asset_badge(asset: &Value) -> String {
    if asset["kind"].as_str() == Some("character") {
        return "LIVE".into();
    }
    if asset["animated"].as_bool() == Some(true) || asset["extension"].as_str() == Some("gif") {
        return "GIF".into();
    }
    match asset["width"].as_u64().unwrap_or_default() {
        width if width >= 5120 => "5K".into(),
        width if width >= 3840 => "4K".into(),
        width if width >= 2560 => "2K".into(),
        width if width > 0 => format!("{width}P"),
        _ => "IMAGE".into(),
    }
}

pub(crate) fn paint_cover_rounded(
    ui: &egui::Ui,
    rect: egui::Rect,
    texture: &egui::TextureHandle,
    corner_radii: [f32; 4],
) {
    ui.painter().add(egui::Shape::mesh(rounded_cover_mesh(
        rect,
        texture,
        corner_radii,
    )));
}

pub(crate) fn rounded_cover_mesh(
    rect: egui::Rect,
    texture: &egui::TextureHandle,
    corner_radii: [f32; 4],
) -> egui::Mesh {
    let [width, height] = texture.size();
    let image_aspect = width as f32 / height as f32;
    let target_aspect = rect.width() / rect.height();
    let (u, v) = if image_aspect > target_aspect {
        (target_aspect / image_aspect, 1.0)
    } else {
        (1.0, image_aspect / target_aspect)
    };
    let uv = egui::Rect::from_center_size(egui::pos2(0.5, 0.5), egui::vec2(u, v));
    let max_radius = (rect.width().min(rect.height()) * 0.5).max(0.0);
    let radii = corner_radii.map(|radius| radius.clamp(0.0, max_radius));
    let rows = rounded_mesh_rows(rect, radii);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for y in &rows {
        let left_inset = rounded_side_inset(*y, rect, radii[0], radii[3]);
        let right_inset = rounded_side_inset(*y, rect, radii[1], radii[2]);
        for x in [rect.left() + left_inset, rect.right() - right_inset] {
            let position = egui::pos2(x, *y);
            let tex_coord = egui::pos2(
                uv.left() + (x - rect.left()) / rect.width() * uv.width(),
                uv.top() + (*y - rect.top()) / rect.height() * uv.height(),
            );
            mesh.vertices.push(egui::epaint::Vertex {
                pos: position,
                uv: tex_coord,
                color: Color32::WHITE,
            });
        }
    }
    add_rounded_mesh_quads(&mut mesh, rows.len());
    mesh
}

fn paint_rounded_vertical_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    corner_radii: [f32; 4],
    top_color: Color32,
    bottom_color: Color32,
) {
    paint_rounded_gradient(painter, rect, corner_radii, top_color, bottom_color, false);
}

fn paint_rounded_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    corner_radii: [f32; 4],
    top_color: Color32,
    bottom_color: Color32,
    diagonal: bool,
) {
    let max_radius = (rect.width().min(rect.height()) * 0.5).max(0.0);
    let radii = corner_radii.map(|radius| radius.clamp(0.0, max_radius));
    let rows = rounded_mesh_rows(rect, radii);
    let mut mesh = egui::Mesh::default();
    for y in &rows {
        let progress = ((*y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        let color = lerp_color(top_color, bottom_color, progress);
        let left_inset = rounded_side_inset(*y, rect, radii[0], radii[3]);
        let right_inset = rounded_side_inset(*y, rect, radii[1], radii[2]);
        for x in [rect.left() + left_inset, rect.right() - right_inset] {
            let color = if diagonal {
                lerp_color(
                    top_color,
                    bottom_color,
                    (progress + (x - rect.left()) / rect.width()) * 0.5,
                )
            } else {
                color
            };
            mesh.colored_vertex(egui::pos2(x, *y), color);
        }
    }
    add_rounded_mesh_quads(&mut mesh, rows.len());
    painter.add(egui::Shape::mesh(mesh));
}

fn rounded_mesh_rows(rect: egui::Rect, radii: [f32; 4]) -> Vec<f32> {
    let top_radius = radii[0].max(radii[1]);
    let bottom_radius = radii[2].max(radii[3]);
    let steps = 8;
    let mut rows = if top_radius > 0.0 {
        (0..=steps)
            .map(|step| rect.top() + top_radius * step as f32 / steps as f32)
            .collect::<Vec<_>>()
    } else {
        vec![rect.top()]
    };
    let bottom_start = rect.bottom() - bottom_radius;
    if bottom_start > rect.top() + top_radius {
        rows.push(bottom_start);
    }
    if bottom_radius > 0.0 {
        rows.extend(
            (1..=steps).map(|step| bottom_start + bottom_radius * step as f32 / steps as f32),
        );
    } else if rect.bottom() > *rows.last().unwrap_or(&rect.top()) {
        rows.push(rect.bottom());
    }
    rows
}

fn rounded_side_inset(y: f32, rect: egui::Rect, top_radius: f32, bottom_radius: f32) -> f32 {
    let radius = if top_radius > 0.0 && y < rect.top() + top_radius {
        top_radius
    } else if bottom_radius > 0.0 && y > rect.bottom() - bottom_radius {
        bottom_radius
    } else {
        return 0.0;
    };
    let center_y = if y < rect.top() + top_radius {
        rect.top() + radius
    } else {
        rect.bottom() - radius
    };
    let distance = y - center_y;
    radius - (radius * radius - distance * distance).max(0.0).sqrt()
}

fn add_rounded_mesh_quads(mesh: &mut egui::Mesh, row_count: usize) {
    for row in 0..row_count.saturating_sub(1) {
        let top = (row * 2) as u32;
        let bottom = top + 2;
        mesh.add_triangle(top, top + 1, bottom);
        mesh.add_triangle(bottom, top + 1, bottom + 1);
    }
}

fn lerp_color(start: Color32, end: Color32, progress: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * progress).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(start.r(), end.r()),
        mix(start.g(), end.g()),
        mix(start.b(), end.b()),
        mix(start.a(), end.a()),
    )
}

fn library_search_field(ui: &mut egui::Ui, value: &mut String, width: f32) -> egui::Response {
    search_field(ui, value, width, "Search your library…", Some("Ctrl K"))
}

pub(crate) fn search_field(
    ui: &mut egui::Ui,
    value: &mut String,
    width: f32,
    hint: &str,
    shortcut: Option<&str>,
) -> egui::Response {
    let mut search_response = None;
    egui::Frame::default()
        .fill(Color32::from_rgb(20, 17, 27))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(12, 4))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 24.0;
            ui.set_min_width((width - 26.0).max(194.0));
            ui.set_min_height(30.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon_rect, _) =
                    ui.allocate_exact_size(Vec2::new(17.0, 20.0), egui::Sense::hover());
                let icon_center = egui::pos2(icon_rect.left() + 7.0, icon_rect.center().y - 1.0);
                ui.painter()
                    .circle_stroke(icon_center, 5.0, Stroke::new(1.6_f32, MUTED));
                ui.painter().line_segment(
                    [
                        icon_center + Vec2::new(3.6, 3.6),
                        icon_rect.right_bottom() - Vec2::new(1.0, 1.0),
                    ],
                    Stroke::new(1.6_f32, MUTED),
                );
                let reserved_width = if shortcut.is_some() { 107.0 } else { 51.0 };
                let field_width = (width - reserved_width).max(80.0);
                search_response = Some(
                    ui.add_sized(
                        [field_width, 24.0],
                        egui::TextEdit::singleline(value)
                            .margin(egui::Margin::ZERO)
                            .vertical_align(egui::Align::Center)
                            .hint_text(hint)
                            .font(egui::FontId::proportional(12.5))
                            .frame(false),
                    ),
                );
                if let Some(shortcut) = shortcut {
                    let (shortcut_rect, _) =
                        ui.allocate_exact_size(Vec2::new(48.0, 20.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(shortcut_rect, 6.0, Color32::from_rgb(27, 23, 36));
                    ui.painter().rect_stroke(
                        shortcut_rect,
                        6.0,
                        Stroke::new(1.0_f32, Color32::from_rgb(48, 42, 60)),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        shortcut_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        shortcut,
                        egui::FontId::proportional(9.0),
                        MUTED,
                    );
                }
            });
        });
    search_response.expect("library search field was not allocated")
}

fn library_filter_segments(ui: &mut egui::Ui, filter: &mut Filter, favorites_only: &mut bool) {
    egui::Frame::default()
        .fill(Color32::from_rgb(20, 17, 27))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(5, 3))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = 32.0;
            ui.spacing_mut().item_spacing.x = 4.0;
            for (filter_option, label, width) in [
                (Filter::All, "All", 52.0),
                (Filter::Wallpapers, "Wallpapers", 92.0),
                (Filter::Animations, "Animations", 94.0),
                (Filter::Favorites, "Favorites", 86.0),
            ] {
                let selected = match filter_option {
                    Filter::All => *filter == Filter::All && !*favorites_only,
                    Filter::Wallpapers => *filter == Filter::Wallpapers,
                    Filter::Animations => *filter == Filter::Animations,
                    Filter::Favorites => *filter == Filter::Favorites || *favorites_only,
                };
                let label = if selected {
                    RichText::new(label).size(11.5).strong().color(TEXT)
                } else {
                    RichText::new(label).size(11.5).color(MUTED)
                };
                let button = egui::Button::new(label)
                    .fill(if selected {
                        Color32::from_rgb(42, 36, 64)
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .corner_radius(8);
                if ui.add_sized([width, 32.0], button).clicked() {
                    *filter = filter_option;
                    *favorites_only = false;
                }
            }
        });
}

fn library_filter_options_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(40.0), egui::Sense::click());
    let fill = if response.hovered() {
        Color32::from_rgb(31, 27, 40)
    } else {
        Color32::from_rgb(20, 17, 27)
    };
    ui.painter().rect_filled(rect, 10.0, fill);
    ui.painter().rect_stroke(
        rect,
        10.0,
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
    icons::paint_node(
        ui.painter(),
        rect.center(),
        92,
        SECONDARY,
        Some(Vec2::splat(14.0)),
    );
    response.on_hover_text("Advanced filters")
}

fn toggle_switch(ui: &mut egui::Ui, value: &mut bool, enabled: bool) -> egui::Response {
    toggle_switch_sized(ui, value, enabled, Vec2::new(36.0, 20.0))
}

fn toggle_switch_sized(
    ui: &mut egui::Ui,
    value: &mut bool,
    enabled: bool,
    size: Vec2,
) -> egui::Response {
    let enabled = enabled && ui.is_enabled();
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    if enabled && response.clicked() {
        *value = !*value;
        response.mark_changed();
    }

    let track = match (enabled, *value) {
        (true, true) => ACCENT,
        (true, false) => Color32::from_rgb(46, 41, 56),
        (false, true) => Color32::from_rgb(76, 70, 104),
        (false, false) => Color32::from_rgb(52, 48, 61),
    };
    let knob = match (enabled, *value) {
        (true, true) => SURFACE,
        (true, false) => SECONDARY,
        (false, _) => MUTED,
    };
    let radius = size.y * 0.5;
    ui.painter().rect_filled(rect, radius, track);
    let knob_x = if *value {
        rect.right() - radius
    } else {
        rect.left() + radius
    };
    ui.painter()
        .circle_filled(egui::pos2(knob_x, rect.center().y), size.y * 0.4, knob);
    response
}

pub(crate) fn paint_speaker_icon(painter: &egui::Painter, rect: egui::Rect, enabled: bool) {
    icons::paint_node(
        painter,
        rect.center(),
        if enabled { 64 } else { 638 },
        if enabled { TEXT } else { MUTED },
        None,
    );
}

fn paint_image_icon(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    icons::paint_node(painter, center, 1216, color, Some(Vec2::splat(20.0)));
}

pub(crate) fn icon_link_with_metrics(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    font_size: f32,
    gap: f32,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let font = egui::FontId::new(font_size, egui::FontFamily::Name("Inter Medium".into()));
    let color = if enabled { ACCENT } else { MUTED };
    let text_width = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font.clone(), color)
        .size()
        .x;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(text_width + gap + 11.0, 20.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let color = if response.hovered() && enabled {
        Color32::from_rgb(174, 164, 255)
    } else {
        color
    };
    ui.painter().text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    let icon_center = egui::pos2(rect.left() + text_width + gap + 5.5, rect.center().y);
    if icon == "arrow-right" {
        icons::paint_node(
            ui.painter(),
            icon_center,
            968,
            color,
            Some(Vec2::splat(11.0)),
        );
    } else {
        paint_action_icon(ui.painter(), icon_center, icon, color);
    }
    response.on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    })
}

pub(crate) fn status_dot(ui: &mut egui::Ui, color: Color32, diameter: f32) {
    let size = diameter + 4.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), diameter * 0.5, color);
}

fn context_menu_item(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    enabled: bool,
    color: Color32,
) -> bool {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 32.0), sense);
    if response.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, 7.0, Color32::from_white_alpha(12));
    }
    let text_color = if enabled { color } else { MUTED };
    let icon_center_x = rect.left()
        + match icon {
            "play" => 24.0,
            "music" | "trash" => 22.0,
            "heart" | "folder" => 21.0,
            "library" => 23.0,
            _ => 20.0,
        };
    paint_action_icon(
        ui.painter(),
        egui::pos2(icon_center_x, rect.center().y),
        icon,
        text_color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(10.5, egui::FontFamily::Name("Inter Medium".into())),
        text_color,
    );
    if icon == "play" {
        ui.painter().text(
            egui::pos2(rect.left() + 196.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "↵",
            egui::FontId::new(8.5, egui::FontFamily::Name("Inter Medium".into())),
            MUTED,
        );
    }
    response.clicked()
}

pub(crate) fn paint_play_outline(
    painter: &egui::Painter,
    center: egui::Pos2,
    color: Color32,
    size: Vec2,
    _stroke_width: f32,
) {
    let node = if size.x <= 8.5 {
        137
    } else if size.x >= 19.5 {
        286
    } else {
        974
    };
    icons::paint_node(painter, center, node, color, Some(size));
}

pub(crate) fn paint_action_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    icon: &str,
    color: Color32,
) {
    if icons::paint(painter, center, icon, color) {
        return;
    }
    match icon {
        "chevron-up" => {
            painter.add(egui::Shape::line(
                vec![
                    center + Vec2::new(-6.0, 3.0),
                    center + Vec2::new(0.0, -3.0),
                    center + Vec2::new(6.0, 3.0),
                ],
                Stroke::new(1.8_f32, color),
            ));
        }
        "restore" => {
            for offset in [Vec2::new(-1.5, 1.5), Vec2::new(1.5, -1.5)] {
                painter.rect_stroke(
                    egui::Rect::from_center_size(center + offset, Vec2::splat(8.0)),
                    1.0,
                    Stroke::new(1.8_f32, color),
                    egui::StrokeKind::Inside,
                );
            }
        }
        "grip" => {
            for x in [-3.0, 3.0] {
                for y in [-4.0, 0.0, 4.0] {
                    painter.circle_filled(center + Vec2::new(x, y), 1.2, color);
                }
            }
        }
        _ => {
            icons::paint_node(painter, center, 786, color, None);
        }
    }
}

fn handle_borderless_resize(ctx: &egui::Context) {
    let screen = ctx.screen_rect();
    let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    let pressed = ctx.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary));
    let edge = 6.5;
    let west = pointer.x <= screen.left() + edge;
    let east = pointer.x >= screen.right() - edge;
    let north = pointer.y <= screen.top() + edge;
    let south = pointer.y >= screen.bottom() - edge;
    let direction = match (north, south, west, east) {
        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
        (true, _, _, true) => Some(egui::ResizeDirection::NorthEast),
        (_, true, true, _) => Some(egui::ResizeDirection::SouthWest),
        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
        (true, _, _, _) => Some(egui::ResizeDirection::North),
        (_, true, _, _) => Some(egui::ResizeDirection::South),
        (_, _, true, _) => Some(egui::ResizeDirection::West),
        (_, _, _, true) => Some(egui::ResizeDirection::East),
        _ => None,
    };
    if let Some(direction) = direction {
        ctx.set_cursor_icon(match direction {
            egui::ResizeDirection::North | egui::ResizeDirection::South => {
                egui::CursorIcon::ResizeVertical
            }
            egui::ResizeDirection::East | egui::ResizeDirection::West => {
                egui::CursorIcon::ResizeHorizontal
            }
            egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
                egui::CursorIcon::ResizeNwSe
            }
            egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
                egui::CursorIcon::ResizeNeSw
            }
        });
        if !pressed {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
    }
}

fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--verify-backend") {
        let result = args
            .get(1)
            .ok_or_else(|| "Missing isolated test directory".to_owned())
            .and_then(|root| backend_check::run(std::path::Path::new(root)));
        if let Err(error) = result {
            eprintln!("Backend verification failed: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if let Some(config) = preview::PreviewConfig::parse(&args) {
        return preview::run(config);
    }
    if args.first().is_some_and(|arg| arg == "--overlay-worker") {
        if let Err(error) = waniengine_wayland::run_cli(args.into_iter().skip(1)) {
            eprintln!("waniengine overlay worker: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            concat!(
                "WaniEngine {} — Rust/egui desktop manager\n\n",
                "Usage: waniengine-egui [OPTIONS]\n\n",
                "Options:\n",
                "  -h, --help             Show this help\n",
                "  -V, --version          Show version\n",
                "      --data-dir DIR     Use DIR for the WaniEngine library\n",
                "      --resume-session   Mark launch from the saved session\n",
                "      --resume-wallpaper Restore the saved wallpaper\n",
                "      --background       Start hidden in the system tray"
            ),
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("WaniEngine {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let location = LibraryLocation::from_environment(args).unwrap_or_else(|error| {
        eprintln!("waniengine-egui: {error}");
        std::process::exit(2);
    });
    let control = match control::start() {
        Ok(control) => control,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            if location.start_in_tray {
                return Ok(());
            }
            if let Err(error) = control::request_show() {
                eprintln!("waniengine-egui: {error}");
                std::process::exit(2);
            }
            return Ok(());
        }
        Err(error) => {
            eprintln!("waniengine-egui: could not start control socket: {error}");
            std::process::exit(2);
        }
    };
    let tray = match tray::TrayController::start() {
        Ok(tray) => Some(tray),
        Err(error) => {
            eprintln!("waniengine-egui: tray integration unavailable: {error}");
            None
        }
    };
    let start_in_tray =
        location.start_in_tray && tray.as_ref().is_some_and(tray::TrayController::is_running);
    if location.start_in_tray && !start_in_tray {
        eprintln!("waniengine-egui: no tray host is available; opening the window normally");
    }
    let msaa_samples = location
        .store()
        .load_scene()
        .ok()
        .and_then(|scene| scene["settings"]["msaaSamples"].as_u64())
        .filter(|samples| matches!(samples, 0 | 2 | 4 | 8))
        .unwrap_or(4) as u16;
    let options = || eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("org.waniengine.WaniEngine")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([960.0, 720.0])
            .with_transparent(true)
            .with_decorations(false),
        multisampling: msaa_samples,
        ..Default::default()
    };

    // Workers keep this context for wakeups; it outlives every window and
    // forwards their repaint requests to whichever window is open.
    let service = egui::Context::default();
    let window = Arc::new(Mutex::new(None::<egui::Context>));
    let (wake_tx, wake_rx) = mpsc::sync_channel(1);
    {
        let window = Arc::clone(&window);
        service.set_request_repaint_callback(move |info| {
            match &*window.lock().unwrap_or_else(|e| e.into_inner()) {
                Some(ctx) => ctx.request_repaint_after(info.delay),
                None if info.delay < Duration::from_millis(10) => {
                    let _ = wake_tx.try_send(());
                }
                None => {}
            }
        });
    }
    let slot = Rc::new(RefCell::new(None::<App>));
    let mut services = Some((location, Some(control), tray));
    if start_in_tray {
        let (location, control, tray) = services.take().expect("services start once");
        *slot.borrow_mut() = Some(App::new(
            &service,
            service.clone(),
            location,
            control,
            tray,
            true,
            None,
        ));
    }
    loop {
        let hidden = slot.borrow_mut().take();
        if let Some(mut app) = hidden {
            app.run_in_tray(&service, &wake_rx);
            let exit = app.force_exit;
            *slot.borrow_mut() = Some(app);
            if exit {
                break;
            }
        }
        let creator_slot = Rc::clone(&slot);
        let host_slot = Rc::clone(&slot);
        let surface = Rc::new(std::cell::Cell::new(None));
        let creator_surface = Rc::clone(&surface);
        let creator_window = Arc::clone(&window);
        let creator_service = service.clone();
        let mut first_services = services.take();
        eframe::run_native(
            "WaniEngine",
            options(),
            Box::new(move |cc| {
                *creator_window.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(cc.egui_ctx.clone());
                creator_surface.set(WaylandSurface::of(cc));
                let app = match creator_slot.borrow_mut().take() {
                    Some(mut app) => {
                        app.attach_window(&cc.egui_ctx);
                        app
                    }
                    None => {
                        let (location, control, tray) =
                            first_services.take().expect("services start once");
                        App::new(
                            &cc.egui_ctx,
                            creator_service.clone(),
                            location,
                            control,
                            tray,
                            false,
                            None,
                        )
                    }
                };
                Ok(Box::new(WindowHost {
                    app: Some(app),
                    slot: host_slot,
                    service: creator_service,
                }))
            }),
        )?;
        *window.lock().unwrap_or_else(|e| e.into_inner()) = None;
        match slot.borrow_mut().as_mut() {
            Some(app) if app.window_hidden && !app.force_exit => {
                if let Some(surface) = surface.get() {
                    surface.unmap();
                }
                app.detach_window();
            }
            _ => break,
        }
    }
    // Stopping the services restores the desktop before the process exits.
    drop(slot.borrow_mut().take());
    Ok(())
}

/// winit destroys a closed Wayland window only on its event loop's next
/// iteration, which never comes while WaniEngine runs in the tray, so the
/// closed window would stay on screen. Unmapping its surface hides it now.
#[derive(Clone, Copy)]
struct WaylandSurface {
    display: *mut wayland_sys::client::wl_display,
    surface: *mut wayland_sys::client::wl_proxy,
}

impl WaylandSurface {
    fn of(
        handles: &(impl raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle),
    ) -> Option<Self> {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
        let RawWindowHandle::Wayland(window) = handles.window_handle().ok()?.as_raw() else {
            return None;
        };
        let RawDisplayHandle::Wayland(display) = handles.display_handle().ok()?.as_raw() else {
            return None;
        };
        Some(Self {
            display: display.display.as_ptr().cast(),
            surface: window.surface.as_ptr().cast(),
        })
    }

    fn unmap(self) {
        use wayland_sys::{client::wayland_client_option, common::wl_argument};
        const ATTACH: u32 = 1;
        const COMMIT: u32 = 6;
        let Some(client) = wayland_client_option() else {
            return;
        };
        let mut attach = [
            wl_argument { o: std::ptr::null() },
            wl_argument { i: 0 },
            wl_argument { i: 0 },
        ];
        let mut commit = [wl_argument { i: 0 }];
        // SAFETY: winit keeps the surface proxy and its connection alive until
        // its event loop runs again, and nothing else uses them meanwhile.
        // A null buffer followed by a commit is how xdg-shell unmaps a toplevel.
        unsafe {
            (client.wl_proxy_marshal_array)(self.surface, ATTACH, attach.as_mut_ptr());
            (client.wl_proxy_marshal_array)(self.surface, COMMIT, commit.as_mut_ptr());
            (client.wl_display_flush)(self.display);
        }
    }
}

/// The window's view of the manager. Closing the window hands the manager
/// back to `main` instead of dropping it, so it can keep running in the tray.
struct WindowHost {
    app: Option<App>,
    slot: Rc<RefCell<Option<App>>>,
    service: egui::Context,
}

impl eframe::App for WindowHost {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // An empty pass re-arms the service context's repaint callback.
        let _ = self.service.run(egui::RawInput::default(), |_| {});
        if let Some(app) = &mut self.app {
            app.run_frame(ctx, true);
        }
    }
}

impl Drop for WindowHost {
    fn drop(&mut self) {
        if let Some(app) = self.app.take() {
            *self.slot.borrow_mut() = Some(app);
        }
    }
}

#[cfg(test)]
mod library_filter_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn positioned_child_does_not_rewind_the_parent_layout() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(300.0, 244.0), egui::Sense::hover());
                let before = ui.cursor();
                child_ui(
                    ui,
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        rect.min + Vec2::new(20.0, 160.0),
                        Vec2::new(120.0, 40.0),
                    )),
                    |ui| {
                        let _ = ui.button("Explore");
                    },
                );
                assert_eq!(ui.cursor(), before);
            });
        });
    }

    #[test]
    fn volume_drag_keeps_its_value_until_release_and_does_not_leak_to_another_item() {
        let ctx = egui::Context::default();
        let mut draft = None;
        let mut frame = |events: Vec<egui::Event>| {
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
                    let mut value = volume_draft_value(&mut draft, "first", 50.0);
                    let response = crate::widgets::slider(ui, &mut value, 0.0..=100.0, 200.0);
                    let commit = record_volume_response(&mut draft, "first", value, &response);
                    result = Some((response.rect, value, commit));
                });
            });
            result.unwrap()
        };
        let (rect, _, _) = frame(vec![]);
        let start = egui::pos2(rect.left() + rect.width() * 0.25, rect.center().y);
        let end = egui::pos2(rect.left() + rect.width() * 0.8, rect.center().y);
        let (_, _, committed_on_press) = frame(vec![
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        assert!(!committed_on_press);
        let (_, dragged, committed_while_dragging) = frame(vec![egui::Event::PointerMoved(end)]);
        assert!(dragged > 70.0);
        assert!(!committed_while_dragging);
        let (_, held, _) = frame(vec![]);
        assert_eq!(held, dragged);
        let (_, released, committed_on_release) = frame(vec![egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(committed_on_release);
        assert_eq!(released, dragged);
        let (_, keyboard, committed_on_key) = frame(vec![egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(keyboard, (released + 1.0).min(100.0));
        assert!(committed_on_key);
        drop(frame);
        assert_eq!(volume_draft_value(&mut draft, "second", 30.0), 30.0);
        assert!(draft.is_none());
    }

    #[test]
    fn library_filters_combine_content_type_favorites_and_search() {
        let favorite_wallpaper = json!({
            "name": "Aurora Veil",
            "kind": "wallpaper",
            "favorite": true
        });
        let regular_wallpaper = json!({
            "name": "Moonlit Pines",
            "kind": "wallpaper",
            "favorite": false
        });
        let favorite_character = json!({
            "name": "Mochi",
            "kind": "character",
            "favorite": true
        });

        assert!(library_asset_matches(
            &favorite_wallpaper,
            Filter::Wallpapers,
            true,
            "aurora"
        ));
        assert!(!library_asset_matches(
            &regular_wallpaper,
            Filter::Wallpapers,
            true,
            ""
        ));
        assert!(!library_asset_matches(
            &favorite_character,
            Filter::Wallpapers,
            false,
            ""
        ));
        assert!(library_asset_matches(
            &favorite_character,
            Filter::Animations,
            false,
            "MOCHI"
        ));
        assert!(!library_asset_matches(
            &favorite_wallpaper,
            Filter::All,
            false,
            "moon"
        ));
    }
}
