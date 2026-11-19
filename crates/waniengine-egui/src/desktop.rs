//! Selects the wallpaper integration available in the current desktop session.
use crate::native_wallpaper::{Sessions, VideoSettings};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub const GNOME_OUTPUT: &str = "__waniengine_gnome_all_displays__";

#[derive(Clone)]
enum Backend {
    Gnome(crate::gnome::WallpaperController),
    Plasma(crate::plasma::WallpaperController),
    Unsupported,
}

#[derive(Clone)]
pub struct WallpaperController {
    // Drop video surfaces/bridge before the image controller restores its snapshot.
    sessions: Arc<Mutex<Sessions>>,
    backend: Backend,
    capabilities: Arc<std::sync::RwLock<(bool, bool, bool)>>,
}

impl WallpaperController {
    pub fn new() -> Self {
        if is_plasma_session() {
            crate::plasma_bridge::recover_stale();
        }
        let backend = if is_gnome_session() {
            Backend::Gnome(crate::gnome::WallpaperController::new())
        } else if is_plasma_session() {
            Backend::Plasma(crate::plasma::WallpaperController::new())
        } else {
            Backend::Unsupported
        };
        let capabilities = Arc::new(std::sync::RwLock::new((false, false, false)));
        let state = Arc::downgrade(&capabilities);
        let plasma = matches!(&backend, Backend::Plasma(_));
        let gnome = matches!(&backend, Backend::Gnome(_));
        std::thread::spawn(move || {
            loop {
                let Some(state) = state.upgrade() else {
                    break;
                };
                let value = probe_capabilities(plasma, gnome);
                *state.write().unwrap_or_else(|e| e.into_inner()) = (value.0, value.1, true);
                drop(state);
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });
        Self {
            sessions: Arc::default(),
            backend,
            capabilities,
        }
    }

    pub fn is_gnome(&self) -> bool {
        matches!(&self.backend, Backend::Gnome(_))
    }

    pub fn capabilities_ready(&self) -> bool {
        self.capabilities
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .2
    }
    pub fn supports_images(&self) -> bool {
        self.capabilities
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .0
    }
    pub fn supports_video(&self) -> bool {
        self.capabilities
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .1
    }
    pub fn supports_animated(&self) -> bool {
        self.supports_images() && matches!(&self.backend, Backend::Plasma(_))
    }
    pub fn reconcile_outputs(&self) -> Result<(), String> {
        if self.is_gnome() {
            self.sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .reconcile_gnome()
        } else {
            self.sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .reconcile_plasma()
        }
    }
    pub fn set_fps(&self, fps: u32) -> Result<(), String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_fps(fps)
    }
    pub fn video_position(&self, output: &str) -> Option<f64> {
        self.sessions.try_lock().ok()?.position(output)
    }
    pub fn video_finished(&self, output: &str) -> bool {
        self.sessions
            .try_lock()
            .is_ok_and(|sessions| sessions.finished(output))
    }
    pub fn apply(
        &self,
        output_name: &str,
        position: (i32, i32),
        size: (i32, i32),
        image: &Path,
        fit: &str,
        animated: bool,
    ) -> Result<(), String> {
        if !self.supports_images() {
            return Err("Wallpaper images are unavailable in this desktop session".into());
        }
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        match &self.backend {
            Backend::Gnome(controller) => {
                if output_name != GNOME_OUTPUT {
                    return Err("GNOME applies one wallpaper across all displays".into());
                }
                sessions.replace_gnome_with_image(|| controller.apply(image, fit, animated))
            }
            Backend::Unsupported => Err("This desktop has no supported wallpaper adapter".into()),
            Backend::Plasma(controller) => sessions.replace_with_image(output_name, || {
                controller.apply(output_name, position, size, image, fit, animated)
            }),
        }
    }

    pub fn apply_video(
        &self,
        output_name: &str,
        position: (i32, i32),
        size: (i32, i32),
        source: &Path,
        fit: &str,
        settings: VideoSettings,
    ) -> Result<(), String> {
        if !self.supports_video() {
            return Err("Video wallpaper requires a working GNOME/XWayland adapter or Plasma Wayland with KWin 6.5+ and layer-shell".into());
        }
        if self.is_gnome() {
            if let Backend::Gnome(controller) = &self.backend {
                if !controller.owns_session() {
                    return Err("Another WaniEngine instance owns this desktop".into());
                }
            }
            if output_name != GNOME_OUTPUT {
                return Err("Select all GNOME displays for video wallpaper".into());
            }
            if !self.supports_video() {
                return Err(
                    "The GNOME video adapter requires XWayland (DISPLAY is unavailable)".into(),
                );
            }
            return self
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .apply_gnome(source, fit, settings);
        }
        if std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("x11"))
            || std::env::var_os("WAYLAND_DISPLAY").is_none()
        {
            return Err("Direct video wallpapers currently require Wayland with layer-shell; X11 video is unavailable".into());
        }
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .apply(
                output_name,
                source,
                fit,
                settings,
                is_plasma_session().then_some((position, size)),
            )
    }

    pub fn take_video_error(&self) -> Option<(String, String)> {
        let mut sessions = self.sessions.try_lock().ok()?;
        let (output, error) = sessions.take_error()?;
        if self.is_gnome() {
            sessions.stop_all();
            Some((GNOME_OUTPUT.into(), error))
        } else {
            Some((output, error))
        }
    }

    pub fn set_animation_enabled(
        &self,
        output_name: &str,
        position: (i32, i32),
        size: (i32, i32),
        enabled: bool,
    ) -> Result<(), String> {
        if self.is_gnome() {
            return self
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_all_playing(enabled)
                .unwrap_or_else(|| Err("No GNOME video wallpaper is active".into()));
        }
        if let Some(result) = self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .set_playing(output_name, enabled)
        {
            return result;
        }
        match &self.backend {
            Backend::Unsupported => Err("This desktop has no animation adapter".into()),
            Backend::Gnome(_) => Err("GNOME wallpaper animation control is unavailable".into()),
            Backend::Plasma(controller) => {
                controller.set_animation_enabled(output_name, position, size, enabled)
            }
        }
    }
}

impl Default for WallpaperController {
    fn default() -> Self {
        Self::new()
    }
}

fn session_name() -> String {
    first_session_name(
        [
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "DESKTOP_SESSION",
        ]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok()),
    )
}
fn first_session_name(values: impl IntoIterator<Item = String>) -> String {
    values
        .into_iter()
        .find(|v| !v.trim().is_empty())
        .unwrap_or_default()
}
pub fn is_plasma_session() -> bool {
    is_plasma_name(&session_name())
}
fn is_plasma_name(value: &str) -> bool {
    value
        .to_ascii_lowercase()
        .split([':', ';', ','])
        .any(|v| matches!(v.trim(), "kde" | "plasma" | "plasmawayland" | "plasmax11"))
}
pub fn is_gnome_session() -> bool {
    is_gnome_name(&session_name())
}
fn kwin_video_api(info: &str) -> bool {
    info.lines()
        .find_map(|line| line.trim().strip_prefix("KWin version:"))
        .is_some_and(|version| {
            let mut parts = version
                .trim()
                .split('.')
                .filter_map(|v| v.parse::<u32>().ok());
            let major = parts.next().unwrap_or(0);
            let minor = parts.next().unwrap_or(0);
            major > 6 || (major == 6 && minor >= 5)
        })
}
fn probe_capabilities(plasma: bool, gnome: bool) -> (bool, bool) {
    if gnome {
        let images = waniengine_core::process::capture(
            std::process::Command::new("gsettings")
                .args(["list-keys", "org.gnome.desktop.background"]),
            std::time::Duration::from_secs(3),
        )
        .is_ok_and(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|s| s == "picture-uri")
        });
        let video = waniengine_wayland::x11_wallpaper_outputs()
            .is_ok_and(|outputs| !outputs.is_empty())
            && waniengine_core::process::direct_video_decoder_available();
        return (images, video);
    }
    if !plasma {
        return (false, false);
    }
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return (false, false);
    };
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&connection) else {
        return (false, false);
    };
    let images = bus
        .name_has_owner(zbus::names::BusName::try_from("org.kde.plasmashell").unwrap())
        .unwrap_or(false);
    let video = images
        && waniengine_core::process::direct_video_decoder_available()
        && waniengine_wayland::layer_shell_available()
        && zbus::blocking::Proxy::new(&connection, "org.kde.KWin", "/KWin", "org.kde.KWin")
            .ok()
            .and_then(|proxy| proxy.call::<_, _, String>("supportInformation", &()).ok())
            .is_some_and(|info| kwin_video_api(&info));
    (images, video)
}

fn is_gnome_name(value: &str) -> bool {
    value
        .to_ascii_lowercase()
        .split([':', ';', ','])
        .map(str::trim)
        .any(|desktop| desktop == "gnome" || desktop.starts_with("gnome-") || desktop == "ubuntu")
}

#[cfg(test)]
mod tests {
    #[test]
    fn capabilities_do_not_assume_every_desktop_is_plasma() {
        for name in ["XFCE", "X-Cinnamon", "sway", "MATE", ""] {
            assert!(!super::is_plasma_name(name));
            assert!(!super::is_gnome_name(name));
        }
        assert!(super::is_plasma_name("KDE"));
        assert!(super::kwin_video_api("KWin version: 6.5.1\n"));
        assert!(!super::kwin_video_api("KWin version: 6.4.5\n"));
    }
    #[test]
    fn session_matrix_uses_the_first_non_empty_variable_and_never_guesses() {
        let pick =
            |values: &[&str]| super::first_session_name(values.iter().map(|v| v.to_string()));
        // (variables in priority order, plasma, gnome)
        let table: [(&[&str], bool, bool); 9] = [
            (&["KDE", "plasma"], true, false),
            (&["ubuntu:GNOME"], false, true),
            (&["", "  ", "X-Cinnamon"], false, false),
            (&["sway", "KDE"], false, false),
            (&["", "GNOME"], false, true),
            (&["XFCE", "gnome"], false, false),
            (&["", "", ""], false, false),
            (&[], false, false),
            (&["KDE:GNOME"], true, true),
        ];
        for (values, plasma, gnome) in table {
            let name = pick(values);
            assert_eq!(super::is_plasma_name(&name), plasma, "{values:?}");
            assert_eq!(super::is_gnome_name(&name), gnome, "{values:?}");
        }
    }
    #[test]
    fn identifies_common_gnome_session_names() {
        assert!(super::is_gnome_name("gnome"));
        assert!(super::is_gnome_name("ubuntu:GNOME"));
        assert!(super::is_gnome_name("gnome-classic"));
        assert!(!super::is_gnome_name("KDE"));
        assert!(!super::is_gnome_name("sway"));
    }
}
