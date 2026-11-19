//! Composition boundary for live services and the isolated UI preview.
use crate::window_state;
use eframe::egui;
use std::path::Path;
use waniengine_egui::desktop::WallpaperController;

#[derive(Clone)]
pub struct WallpaperUi {
    live: Option<WallpaperController>,
}
impl WallpaperUi {
    pub fn new(preview: bool) -> Self {
        Self {
            live: (!preview).then(WallpaperController::new),
        }
    }
    pub fn is_gnome(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(WallpaperController::is_gnome)
    }
    pub fn capabilities_ready(&self) -> bool {
        self.live
            .as_ref()
            .is_none_or(WallpaperController::capabilities_ready)
    }
    pub fn supports_images(&self) -> bool {
        self.live
            .as_ref()
            .is_none_or(WallpaperController::supports_images)
    }
    pub fn reconcile_outputs(&self) -> Result<(), String> {
        self.live
            .as_ref()
            .map_or(Ok(()), |live| live.reconcile_outputs())
    }
    pub fn video_position(&self, output: &str) -> Option<f64> {
        self.live.as_ref()?.video_position(output)
    }
    pub fn video_finished(&self, output: &str) -> bool {
        self.live
            .as_ref()
            .is_some_and(|live| live.video_finished(output))
    }
    pub fn supports_video(&self) -> bool {
        self.live
            .as_ref()
            .is_none_or(WallpaperController::supports_video)
    }
    pub fn supports_animated(&self) -> bool {
        self.live
            .as_ref()
            .is_none_or(WallpaperController::supports_animated)
    }
    pub fn apply(
        &self,
        output: &str,
        position: (i32, i32),
        size: (i32, i32),
        image: &Path,
        fit: &str,
        animated: bool,
    ) -> Result<(), String> {
        self.live
            .as_ref()
            .ok_or("UI preview does not invoke desktop services")?
            .apply(output, position, size, image, fit, animated)
    }
    pub fn apply_video(
        &self,
        output: &str,
        position: (i32, i32),
        size: (i32, i32),
        source: &Path,
        fit: &str,
        settings: waniengine_egui::native_wallpaper::VideoSettings,
    ) -> Result<(), String> {
        self.live
            .as_ref()
            .ok_or("UI preview does not invoke desktop services")?
            .apply_video(output, position, size, source, fit, settings)
    }
    pub fn take_video_error(&self) -> Option<(String, String)> {
        self.live.as_ref()?.take_video_error()
    }
    pub fn set_animation_enabled(
        &self,
        output: &str,
        position: (i32, i32),
        size: (i32, i32),
        enabled: bool,
    ) -> Result<(), String> {
        self.live
            .as_ref()
            .ok_or("UI preview does not invoke desktop services")?
            .set_animation_enabled(output, position, size, enabled)
    }
}

pub struct WindowMonitorUi {
    live: Option<window_state::WindowStateMonitor>,
}
impl WindowMonitorUi {
    pub fn new(ctx: egui::Context, preview: bool) -> Self {
        Self {
            live: (!preview).then(|| window_state::WindowStateMonitor::new(ctx)),
        }
    }
    pub fn session_locked(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(window_state::WindowStateMonitor::session_locked)
    }
    pub fn snapshot(&self) -> window_state::Snapshot {
        self.live.as_ref().map_or(
            window_state::Snapshot {
                mode: window_state::SessionMode::Wayland,
                backend: window_state::MonitorBackend::KWinWayland,
                connected: true,
                active: window_state::ActiveWindowState::default(),
                error: None,
            },
            window_state::WindowStateMonitor::snapshot,
        )
    }
}
