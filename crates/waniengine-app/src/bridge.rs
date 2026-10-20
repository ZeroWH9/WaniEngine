use waniengine_core::library::{self, LibraryStore};

#[cxx::bridge(namespace = "waniengine")]
pub mod ffi {
    unsafe extern "C++" {
        include!("platform/qt_host.hpp");
        fn run_manager(arguments: &str, runtime_root: &str) -> i32;
    }
    extern "Rust" {
        fn load_scene(root: &str, source: &str) -> Result<String>;
        fn save_scene(root: &str, json: &str) -> Result<()>;
        fn organize_asset(root: &str, json: &str) -> Result<String>;
        fn add_audio(root: &str, asset: &str, sources: &str) -> Result<String>;
        fn load_playlist(url: &str) -> Result<String>;
        fn save_playlist(url: &str, json: &str) -> Result<()>;
        fn set_autostart(enabled: bool, executable: &str) -> Result<()>;
    }
}

fn load_scene(root: &str, source: &str) -> library::Result<String> {
    Ok(LibraryStore::new(root, source).load_scene()?.to_string())
}
fn save_scene(root: &str, json: &str) -> library::Result<()> {
    LibraryStore::new(root, "").save_scene(serde_json::from_str(json)?)
}
fn organize_asset(root: &str, json: &str) -> library::Result<String> {
    Ok(LibraryStore::new(root, "")
        .organize(serde_json::from_str(json)?)?
        .to_string())
}
fn add_audio(root: &str, asset: &str, sources: &str) -> library::Result<String> {
    Ok(LibraryStore::new(root, "")
        .add_audio(
            &serde_json::from_str(asset)?,
            &serde_json::from_str::<Vec<serde_json::Value>>(sources)?,
        )?
        .to_string())
}
fn load_playlist(url: &str) -> library::Result<String> {
    Ok(library::load_playlist(url)?.to_string())
}
fn save_playlist(url: &str, json: &str) -> library::Result<()> {
    library::save_playlist(url, &serde_json::from_str(json)?)
}

fn set_autostart(enabled: bool, executable: &str) -> library::Result<()> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    let path = config.join("autostart/org.waniengine.WaniEngine.desktop");
    if !enabled {
        if path.is_file() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    if !std::path::Path::new(executable).is_absolute() {
        return Err("Invalid autostart executable".into());
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    let stable_launcher = std::env::var("WANIENGINE_LAUNCHER").ok().or_else(|| {
        if !std::path::Path::new(executable).ancestors().any(|path| path.join(".waniengine-installation").is_file()) {
            return None;
        }
        std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/bin/waniengine").to_string_lossy().into_owned())
    }).filter(|path| {
        std::path::Path::new(path).is_absolute() && std::path::Path::new(path).is_file()
    });
    let executable = stable_launcher.as_deref().unwrap_or(executable);
    let escaped = executable
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    std::fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName=WaniEngine\nComment=Restore wallpapers and characters\nExec=\"{escaped}\" --redesign --background --resume-wallpaper\nIcon=waniengine\nTerminal=false\nX-GNOME-Autostart-enabled=true\nX-WaniEngine-Managed=true\nX-KDE-autostart-phase=2\nX-KDE-Wayland-Interfaces=org_kde_plasma_window_management\n"
        ),
    )?;
    Ok(())
}
