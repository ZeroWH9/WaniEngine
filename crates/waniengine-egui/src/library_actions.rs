//! Library operations shared by the egui controls, using the existing on-disk format.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use waniengine_core::library::{self, LibraryStore};
use waniengine_egui::overlay::ActorSettings;

#[derive(Clone, Copy, PartialEq)]
pub enum ImportKind {
    Wallpaper,
    Character,
}

pub struct ImportResult {
    pub added: usize,
    pub skipped: usize,
    pub errors: Vec<String>,
    pub first_url: Option<String>,
    pub imported_urls: Vec<String>,
}

pub struct LoadedPlaylist {
    pub name: String,
    pub urls: Vec<String>,
    pub unavailable: usize,
}

pub fn import(
    store: &LibraryStore,
    paths: &[PathBuf],
    kind: ImportKind,
) -> library::Result<ImportResult> {
    // Load immediately before the operation so unrelated settings and fields are retained.
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let mut result = ImportResult {
        added: 0,
        skipped: 0,
        errors: Vec::new(),
        first_url: None,
        imported_urls: Vec::new(),
    };
    for path in paths {
        let operation = || -> library::Result<Option<Value>> {
            let path = path.canonicalize()?;
            if path.to_str().is_none() {
                return Err("File path is not valid UTF-8".into());
            }
            let extension = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let video = matches!(
                extension.as_str(),
                "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"
            );
            if !video
                && !matches!(
                    extension.as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "apng" | "bmp" | "svg"
                )
            {
                return Err("Unsupported media format".into());
            }
            let url = library::file_url(&path);
            if scene["assets"].as_array().unwrap().iter().any(|asset| {
                asset["url"].as_str() == Some(&url)
                    || asset["originalSource"].as_str() == Some(&url)
            }) {
                return Ok(None);
            }
            let name = path
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or("Invalid file name")?;
            let added = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u64::MAX as u128) as u64;
            let mut asset = json!({"url":url,"name":name,"extension":extension.to_ascii_uppercase(),"kind":if kind == ImportKind::Character {"character"} else {"wallpaper"},"video":video,"animated":matches!(extension.as_str(), "gif" | "apng"),"favorite":false,"addedAt":added,"speed":100,"posX":50,"posY":96,"opacity":100,"flipX":false,"scale":100,"volume":50,"muted":true,"fit":"cover"});
            if let Some(metadata) = crate::metadata::describe(&path, video).as_object() {
                asset.as_object_mut().unwrap().extend(metadata.clone());
            }
            store.organize(asset).map(Some)
        };
        match operation() {
            Ok(Some(asset)) => {
                let url = asset["url"].as_str().map(str::to_owned);
                scene["assets"].as_array_mut().unwrap().push(asset);
                // Save each successful import so a later failed file does not lose it.
                if let Err(error) = store.save_scene(scene.clone()) {
                    result.errors.push(format!("{}: could not save library: {error}; the created project remains in the library folder", path.display()));
                    break;
                }
                result.added += 1;
                if let Some(url) = url.as_ref() {
                    result.imported_urls.push(url.clone());
                }
                if result.first_url.is_none() {
                    result.first_url = url;
                }
            }
            Ok(None) => result.skipped += 1,
            Err(error) => result.errors.push(format!("{}: {error}", path.display())),
        }
    }
    Ok(result)
}

pub fn set_favorite(store: &LibraryStore, url: &str, favorite: bool) -> library::Result<()> {
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This item is not in the managed library")?;
    if asset["missing"].as_bool() == Some(true) {
        return Err("This item is unavailable".into());
    }
    asset["favorite"] = json!(favorite);
    store.save_scene(scene)
}

/// Update a supported per-asset playback option while retaining unknown fields.
pub fn update_asset_property(
    store: &LibraryStore,
    url: &str,
    key: &str,
    value: Value,
) -> library::Result<()> {
    let valid = match key {
        "fit" => matches!(
            value.as_str(),
            Some("cover" | "contain" | "stretch" | "center")
        ),
        "loop" | "audioEnabled" => value.is_boolean(),
        "volume" => value
            .as_f64()
            .is_some_and(|number| (0.0..=100.0).contains(&number)),
        "speed" => value
            .as_f64()
            .is_some_and(|number| (25.0..=200.0).contains(&number)),
        _ => false,
    };
    if !valid {
        return Err("Unsupported or invalid content setting".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This item is not in the managed library")?;
    if asset["missing"].as_bool() == Some(true) {
        return Err("This item is unavailable".into());
    }
    asset[key] = value;
    store.save_scene(scene)
}

/// Copy audio files into a character project and register them in scene.json.
/// The originals remain at their source paths.
pub fn add_character_audio(
    store: &LibraryStore,
    url: &str,
    paths: &[PathBuf],
) -> library::Result<usize> {
    if paths.is_empty() {
        return Ok(0);
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This character is not in the managed library")?;
    if asset["kind"].as_str() != Some("character") || asset["missing"].as_bool() == Some(true) {
        return Err("This character is unavailable".into());
    }
    let sources = paths
        .iter()
        .map(|path| {
            let path = path.canonicalize()?;
            if !path.is_file() || path.to_str().is_none() {
                return Err("Audio path is not a regular UTF-8 file".into());
            }
            Ok(json!(library::file_url(&path)))
        })
        .collect::<library::Result<Vec<_>>>()?;
    let tracks = store.add_audio(asset, &sources)?;
    if !asset["audio"].is_object() {
        asset["audio"] = json!({"enabled":false,"files":[]});
    }
    asset["audio"]["files"] = json!(tracks);
    store.save_scene(scene)?;
    Ok(paths.len())
}

/// Persist a character's audio controls while retaining unknown scene fields.
pub fn save_character_audio(
    store: &LibraryStore,
    url: &str,
    audio: Value,
    volume: f64,
) -> library::Result<()> {
    validate_character_audio(&audio)?;
    if !volume.is_finite() || !(0.0..=100.0).contains(&volume) {
        return Err("Invalid character volume".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This character is not in the managed library")?;
    if asset["kind"].as_str() != Some("character") || asset["missing"].as_bool() == Some(true) {
        return Err("This character is unavailable".into());
    }
    asset["audio"] = audio;
    asset["volume"] = json!(volume);
    store.save_scene(scene)
}

fn validate_character_audio(audio: &Value) -> library::Result<()> {
    if !audio.is_object() || audio["enabled"].as_bool().is_none() || !audio["files"].is_array() {
        return Err("Invalid character audio settings".into());
    }
    for track in audio["files"].as_array().unwrap() {
        if track["url"].as_str().is_none() || !track["playback"].is_object() {
            return Err("Invalid character audio track".into());
        }
        let playback = &track["playback"];
        if !playback["enabled"].is_null() && playback["enabled"].as_bool().is_none() {
            return Err("Invalid character audio track".into());
        }
        if playback["mode"]
            .as_str()
            .is_some_and(|mode| !matches!(mode, "once" | "loop" | "interval" | "click"))
        {
            return Err("Invalid character audio trigger".into());
        }
        for (key, min, max) in [
            ("volume", 0.0, 100.0),
            ("intervalSeconds", 1.0, 86_400.0),
            ("radiusPixels", 0.0, 500.0),
            ("cooldownSeconds", 0.0, 3_600.0),
        ] {
            if !playback[key].is_null() {
                let Some(value) = playback[key].as_f64() else {
                    return Err("Invalid character audio track".into());
                };
                if !value.is_finite() || !(min..=max).contains(&value) {
                    return Err("Invalid character audio track".into());
                }
            }
        }
        if !playback["startImmediately"].is_null()
            && playback["startImmediately"].as_bool().is_none()
        {
            return Err("Invalid character audio track".into());
        }
    }
    Ok(())
}

/// Persist character controls using the existing per-asset properties format.
pub fn save_character_settings(
    store: &LibraryStore,
    url: &str,
    settings: ActorSettings,
) -> library::Result<()> {
    settings.validate()?;
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This character is not in the managed library")?;
    if asset["kind"].as_str() != Some("character") || asset["missing"].as_bool() == Some(true) {
        return Err("This character is unavailable".into());
    }
    let fields = [
        ("posX", json!(settings.x * 100.0)),
        ("posY", json!(settings.y * 100.0)),
        ("scale", json!(settings.scale)),
        ("opacity", json!(settings.opacity)),
        ("rotation", json!(settings.rotation)),
        ("flipX", json!(settings.flip_x)),
        ("smoothing", json!(settings.smoothing)),
        ("speed", json!(settings.speed)),
        ("playing", json!(settings.playing)),
        ("loop", json!(settings.looping)),
    ];
    for (key, value) in fields {
        asset[key] = value;
    }
    store.save_scene(scene)
}

#[cfg(test)]
pub fn save_app_setting(store: &LibraryStore, key: &str, value: Value) -> library::Result<()> {
    if !matches!(
        key,
        "previewQuality"
            | "globalVolume"
            | "audioOutput"
            | "frameRateLimit"
            | "playlistInterval"
            | "playlistRandom"
            | "restoreOnLaunch"
            | "startWithSession"
            | "closeToTray"
            | "msaaSamples"
            | "pauseFullscreen"
            | "pauseMaximized"
    ) {
        return Err("Unsupported application setting".into());
    }
    let mut settings = serde_json::Map::new();
    settings.insert(key.to_owned(), value);
    save_app_settings(store, &Value::Object(settings))
}

/// Persist supported app settings as one change while retaining unknown fields.
pub fn save_app_settings(store: &LibraryStore, settings: &Value) -> library::Result<()> {
    let values = settings
        .as_object()
        .ok_or("Application settings must be an object")?;
    for (key, value) in values {
        let valid = match key.as_str() {
            "previewQuality" => matches!(value.as_str(), Some("Low" | "Medium" | "High")),
            "globalVolume" => value
                .as_f64()
                .is_some_and(|volume| volume.is_finite() && (0.0..=100.0).contains(&volume)),
            "audioOutput" => value
                .as_str()
                .is_some_and(|output| output.len() <= 256 && !output.chars().any(char::is_control)),
            "frameRateLimit" => value
                .as_u64()
                .is_some_and(|rate| (15..=240).contains(&rate)),
            "msaaSamples" => value
                .as_u64()
                .is_some_and(|samples| matches!(samples, 0 | 2 | 4 | 8)),
            "playlistInterval" => value
                .as_u64()
                .is_some_and(|minutes| (1..=1440).contains(&minutes)),
            "playlistRandom" | "restoreOnLaunch" | "startWithSession" | "closeToTray"
            | "pauseFullscreen" | "pauseMaximized" => value.is_boolean(),
            _ => true,
        };
        if !valid {
            return Err(format!("Invalid value for application setting '{key}'").into());
        }
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    for key in [
        "previewQuality",
        "globalVolume",
        "audioOutput",
        "frameRateLimit",
        "msaaSamples",
        "playlistInterval",
        "playlistRandom",
        "restoreOnLaunch",
        "startWithSession",
        "closeToTray",
        "pauseFullscreen",
        "pauseMaximized",
    ] {
        if let Some(value) = values.get(key) {
            scene["settings"][key] = value.clone();
        }
    }
    store.save_scene(scene)
}

/// Persist the characters that should be restored with the desktop session.
pub fn save_active_actors(store: &LibraryStore, urls: &[String]) -> library::Result<()> {
    if urls.len() > 16 {
        return Err("At most 16 characters can be active".into());
    }
    let mut unique = std::collections::HashSet::new();
    if urls.iter().any(|url| !unique.insert(url)) {
        return Err("A character cannot be active more than once".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let assets = scene["assets"].as_array().unwrap();
    for url in urls {
        let asset = assets
            .iter()
            .find(|asset| asset["url"].as_str() == Some(url.as_str()))
            .ok_or("Every active character must be in the managed library")?;
        if asset["kind"].as_str() != Some("character") || asset["missing"].as_bool() == Some(true) {
            return Err("Every active character must be available".into());
        }
    }
    scene["actorUrls"] = json!(urls);
    store.save_scene(scene)
}

/// Remember which managed wallpaper is assigned to a connected display so the
/// manager can restore it after the desktop session starts again.
pub fn save_wallpaper_assignment(
    store: &LibraryStore,
    monitor: &str,
    url: &str,
    select_monitor: bool,
) -> library::Result<()> {
    if monitor.trim().is_empty() || monitor.len() > 256 {
        return Err("A valid display name is required".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let asset = scene["assets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|asset| asset["url"].as_str() == Some(url))
        .ok_or("This wallpaper is not in the managed library")?;
    if asset["kind"].as_str() == Some("character") || asset["missing"].as_bool() == Some(true) {
        return Err("This wallpaper is unavailable".into());
    }
    if !scene["wallpapersByMonitor"].is_object() {
        scene["wallpapersByMonitor"] = json!({});
    }
    let assignments = scene["wallpapersByMonitor"].as_object_mut().unwrap();
    assignments.insert(monitor.to_owned(), json!({"url":url}));
    if select_monitor {
        scene["selectedMonitor"] = json!(monitor);
    }
    store.save_scene(scene)
}

pub fn save_playlist(store: &LibraryStore, playlist: &[String]) -> library::Result<()> {
    if playlist.len() > 1000 {
        return Err("A playlist can contain at most 1000 items".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let assets = scene["assets"].as_array().unwrap();
    let mut unique = std::collections::HashSet::new();
    for url in playlist {
        if !unique.insert(url) {
            return Err("A playlist cannot contain duplicate items".into());
        }
        let asset = assets
            .iter()
            .find(|asset| asset["url"].as_str() == Some(url.as_str()))
            .ok_or("Every playlist item must be in the managed library")?;
        if asset["kind"].as_str() == Some("character") || asset["missing"].as_bool() == Some(true) {
            return Err("Unavailable characters cannot be added to a playlist".into());
        }
    }
    scene["playlist"] = json!(playlist);
    scene["playlistDirty"] = json!(true);
    store.save_scene(scene)
}

pub fn save_playlist_configuration(
    store: &LibraryStore,
    playlist: &[String],
    interval_minutes: u64,
    random: bool,
) -> library::Result<()> {
    if playlist.len() > 1000 || !(1..=1440).contains(&interval_minutes) {
        return Err("Playlist length or rotation interval is outside the supported range".into());
    }
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let assets = scene["assets"].as_array().unwrap();
    let mut unique = std::collections::HashSet::new();
    for url in playlist {
        if !unique.insert(url) {
            return Err("A playlist cannot contain duplicate items".into());
        }
        let asset = assets
            .iter()
            .find(|asset| asset["url"].as_str() == Some(url.as_str()))
            .ok_or("Every playlist item must be in the managed library")?;
        if asset["kind"].as_str() == Some("character") || asset["missing"].as_bool() == Some(true) {
            return Err("Unavailable characters cannot be added to a playlist".into());
        }
    }
    scene["playlist"] = json!(playlist);
    scene["playlistDirty"] = json!(true);
    scene["settings"]["playlistInterval"] = json!(interval_minutes);
    scene["settings"]["playlistRandom"] = json!(random);
    store.save_scene(scene)
}

pub fn save_named_playlist(
    store: &LibraryStore,
    path: &Path,
    name: &str,
    playlist: &[String],
    interval_minutes: u64,
    random: bool,
) -> library::Result<()> {
    if playlist.is_empty() || playlist.len() > 1000 {
        return Err("A saved playlist must contain between 1 and 1000 wallpapers".into());
    }
    if !(1..=1440).contains(&interval_minutes) {
        return Err("Playlist interval must be between 1 and 1440 minutes".into());
    }
    let _transaction = store.lock()?;
    let scene = store.load_scene()?;
    let assets = scene["assets"].as_array().unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(playlist.len());
    for url in playlist {
        if !seen.insert(url) {
            return Err("A playlist cannot contain duplicate items".into());
        }
        let asset = assets
            .iter()
            .find(|asset| asset["url"].as_str() == Some(url.as_str()))
            .ok_or("Every playlist item must be in the managed library")?;
        if asset["kind"].as_str() == Some("character") || asset["missing"].as_bool() == Some(true) {
            return Err("Unavailable characters cannot be saved in a playlist".into());
        }
        let source = library::file_path(url)?;
        if !source.is_file() {
            return Err("A playlist wallpaper file is unavailable".into());
        }
        items.push(asset.clone());
    }
    let name = name.trim();
    if name.is_empty() || name.len() > 120 {
        return Err("Playlist name must contain between 1 and 120 characters".into());
    }
    let document = json!({
        "format":"waniengine-playlist",
        "version":1,
        "name":name,
        "assets":items,
        "intervalMinutes":interval_minutes,
        "random":random
    });
    library::save_playlist(&library::file_url(path), &document)?;
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    scene["playlistName"] = json!(name);
    scene["playlistDirty"] = json!(false);
    scene["settings"]["playlistInterval"] = json!(interval_minutes);
    scene["settings"]["playlistRandom"] = json!(random);
    store.save_scene(scene)
}

pub fn load_named_playlist(store: &LibraryStore, path: &Path) -> library::Result<LoadedPlaylist> {
    let document = library::load_playlist(&library::file_url(path))?;
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    let assets = scene["assets"].as_array_mut().unwrap();
    let mut urls = Vec::new();
    let mut unavailable = 0;
    for item in document["assets"].as_array().unwrap() {
        let source_url = item["originalSource"]
            .as_str()
            .filter(|url| library::file_path(url).is_ok_and(|path| path.is_file()))
            .or_else(|| {
                item["url"]
                    .as_str()
                    .filter(|url| library::file_path(url).is_ok_and(|path| path.is_file()))
            });
        let Some(source_url) = source_url else {
            unavailable += 1;
            continue;
        };
        let existing = assets.iter().find(|asset| {
            asset["url"].as_str() == Some(source_url)
                || asset["originalSource"].as_str() == Some(source_url)
                || (item["id"].is_string() && asset["id"] == item["id"])
        });
        let url = if let Some(asset) = existing {
            asset["url"].as_str().unwrap_or_default().to_owned()
        } else {
            let mut candidate = item.clone();
            candidate["url"] = json!(source_url);
            for key in ["configUrl", "folderUrl", "missing"] {
                candidate.as_object_mut().unwrap().remove(key);
            }
            if candidate["thumbnailUrl"]
                .as_str()
                .is_some_and(|url| library::file_path(url).map_or(true, |path| !path.is_file()))
            {
                candidate.as_object_mut().unwrap().remove("thumbnailUrl");
            }
            match store.organize(candidate) {
                Ok(asset) => {
                    let url = asset["url"].as_str().unwrap_or_default().to_owned();
                    assets.push(asset);
                    url
                }
                Err(_) => {
                    unavailable += 1;
                    continue;
                }
            }
        };
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    let name = document["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "Playlist".into());
    let interval_minutes = document["intervalMinutes"]
        .as_u64()
        .unwrap_or(30)
        .clamp(1, 1440);
    let random = document["random"].as_bool().unwrap_or(false);
    scene["playlist"] = json!(urls.clone());
    scene["playlistName"] = json!(name);
    scene["playlistDirty"] = json!(false);
    scene["settings"]["playlistInterval"] = json!(interval_minutes);
    scene["settings"]["playlistRandom"] = json!(random);
    store.save_scene(scene)?;
    Ok(LoadedPlaylist {
        name,
        urls,
        unavailable,
    })
}

/// Remove a managed entry without deleting the project or original media.
/// Saved active references must be stopped by the desktop service first.
pub fn remove(store: &LibraryStore, url: &str) -> library::Result<()> {
    let _transaction = store.lock()?;
    let mut scene = store.load_scene()?;
    if scene["actorUrls"]
        .as_array()
        .is_some_and(|actors| actors.iter().any(|item| item.as_str() == Some(url)))
        || scene["wallpapersByMonitor"]
            .as_object()
            .is_some_and(|outputs| {
                outputs
                    .values()
                    .any(|entry| entry["url"].as_str() == Some(url))
            })
    {
        return Err(
            "This content is assigned to a display or active character; disable it before removal"
                .into(),
        );
    }
    let assets = scene["assets"].as_array_mut().unwrap();
    let previous = assets.len();
    assets.retain(|item| item["url"].as_str() != Some(url));
    if assets.len() == previous {
        return Err("This item is not in the managed library".into());
    }
    let replacement = assets
        .iter()
        .find(|item| item["missing"].as_bool() != Some(true))
        .and_then(|item| item["url"].as_str())
        .map(str::to_owned);
    if scene["selectedUrl"].as_str() == Some(url) {
        scene["selectedUrl"] = json!(replacement.unwrap_or_default());
    }
    if let Some(playlist) = scene["playlist"].as_array_mut() {
        let previous = playlist.len();
        playlist.retain(|item| item.as_str() != Some(url));
        if playlist.len() != previous {
            scene["playlistDirty"] = json!(true);
        }
    }
    store.save_scene(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "wani-egui-actions-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self, scene: Value) -> LibraryStore {
            let store = LibraryStore::new(self.0.join("library"), "");
            store.save_scene(scene).unwrap();
            store
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn imports_without_touching_original_and_preserves_state_and_favorite() {
        let tmp = Temp::new();
        let path = tmp.0.join("image.png");
        image::RgbaImage::from_pixel(24, 16, image::Rgba([100, 20, 40, 255]))
            .save(&path)
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        store
            .save_scene(json!({"assets":[],"settings":{"custom":true},"extra":"keep"}))
            .unwrap();
        let result = import(&store, std::slice::from_ref(&path), ImportKind::Character).unwrap();
        assert_eq!(result.added, 1);
        let url = result.first_url.clone().unwrap();
        assert_eq!(result.imported_urls, [url.clone()]);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let duplicate = import(&store, &[path], ImportKind::Character).unwrap();
        assert_eq!(duplicate.skipped, 1);
        assert!(duplicate.imported_urls.is_empty());
        set_favorite(&store, &url, true).unwrap();
        let scene = store.load_scene().unwrap();
        assert_eq!(scene["assets"][0]["favorite"], true);
        assert_eq!(scene["assets"][0]["kind"], "character");
        assert_eq!(scene["assets"][0]["width"], 24);
        assert_eq!(scene["assets"][0]["height"], 16);
        assert_eq!(scene["settings"]["custom"], true);
        assert_eq!(scene["extra"], "keep");
    }

    #[test]
    fn updates_wallpaper_properties_and_rejects_invalid_values() {
        let tmp = Temp::new();
        let path = tmp.0.join("wallpaper.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let url = import(&store, &[path], ImportKind::Wallpaper)
            .unwrap()
            .first_url
            .unwrap();
        update_asset_property(&store, &url, "fit", json!("contain")).unwrap();
        update_asset_property(&store, &url, "loop", json!(false)).unwrap();
        update_asset_property(&store, &url, "volume", json!(33)).unwrap();
        assert!(update_asset_property(&store, &url, "fit", json!("unknown")).is_err());
        assert!(update_asset_property(&store, &url, "volume", json!(101)).is_err());
        assert!(update_asset_property(&store, &url, "selectedUrl", json!("other")).is_err());
        let asset = &store.load_scene().unwrap()["assets"][0];
        assert_eq!(asset["fit"], "contain");
        assert_eq!(asset["loop"], false);
        assert_eq!(asset["volume"], 33);
    }

    #[test]
    fn saves_rotation_interval_and_shuffle_with_validation() {
        let tmp = Temp::new();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        store
            .save_scene(json!({"assets":[],"settings":{"custom":"keep"}}))
            .unwrap();
        save_app_setting(&store, "playlistInterval", json!(45)).unwrap();
        save_app_setting(&store, "playlistRandom", json!(true)).unwrap();
        save_app_setting(&store, "restoreOnLaunch", json!(false)).unwrap();
        save_app_setting(&store, "startWithSession", json!(true)).unwrap();
        assert!(save_app_setting(&store, "playlistInterval", json!(0)).is_err());
        assert!(save_app_setting(&store, "playlistRandom", json!("true")).is_err());
        assert!(save_app_setting(&store, "restoreOnLaunch", json!("yes")).is_err());
        assert!(save_app_setting(&store, "unknown", json!(true)).is_err());
        let saved = store.load_scene().unwrap();
        assert_eq!(saved["settings"]["playlistInterval"], 45);
        assert_eq!(saved["settings"]["playlistRandom"], true);
        assert_eq!(saved["settings"]["custom"], "keep");
    }

    #[test]
    fn saves_window_pause_preferences_as_booleans() {
        let tmp = Temp::new();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        store
            .save_scene(json!({"assets":[],"settings":{"custom":"keep"}}))
            .unwrap();

        save_app_setting(&store, "pauseFullscreen", json!(false)).unwrap();
        save_app_setting(&store, "pauseMaximized", json!(true)).unwrap();
        assert!(save_app_setting(&store, "pauseFullscreen", json!("false")).is_err());

        let saved = store.load_scene().unwrap();
        assert_eq!(saved["settings"]["pauseFullscreen"], false);
        assert_eq!(saved["settings"]["pauseMaximized"], true);
        assert_eq!(saved["settings"]["custom"], "keep");
    }

    #[test]
    fn saves_active_actor_urls_while_preserving_other_scene_data() {
        let tmp = Temp::new();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let actor = "file:///tmp/character.png".to_owned();
        store
            .save_scene(json!({
                "assets": [{"url":actor,"kind":"character"}],
                "settings": {"restoreOnLaunch":true},
                "otherField": "preserved"
            }))
            .unwrap();
        save_active_actors(&store, std::slice::from_ref(&actor)).unwrap();
        let saved = store.load_scene().unwrap();
        assert_eq!(saved["actorUrls"], json!([actor]));
        assert_eq!(saved["settings"]["restoreOnLaunch"], true);
        assert_eq!(saved["otherField"], "preserved");
        assert!(save_active_actors(&store, &[actor.clone(), actor]).is_err());
    }

    #[test]
    fn saves_wallpaper_assignment_and_preserves_other_scene_data() {
        let temp = Temp::new();
        let wallpaper = "file:///tmp/wallpaper.png";
        let store = temp.store(json!({
            "schema": 2,
            "assets": [{"url": wallpaper, "kind": "wallpaper"}],
            "settings": {"restoreOnLaunch": true},
            "customField": "keep"
        }));

        save_wallpaper_assignment(&store, "DP-1", wallpaper, true).unwrap();
        let saved = store.load_scene().unwrap();
        assert_eq!(saved["wallpapersByMonitor"]["DP-1"]["url"], wallpaper);
        assert_eq!(saved["selectedMonitor"], "DP-1");
        assert_eq!(saved["settings"]["restoreOnLaunch"], true);
        assert_eq!(saved["customField"], "keep");

        save_wallpaper_assignment(&store, "HDMI-A-1", wallpaper, false).unwrap();
        let saved = store.load_scene().unwrap();
        assert_eq!(saved["wallpapersByMonitor"]["HDMI-A-1"]["url"], wallpaper);
        assert_eq!(saved["selectedMonitor"], "DP-1");
    }

    #[test]
    fn concurrent_real_actions_lose_no_favorite_setting_playlist_or_assignment() {
        let temp = Temp::new();
        let urls: Vec<String> = ["a b.png", "ção ü.png", "100%.png", "日本.png"]
            .iter()
            .map(|name| library::file_url(&temp.0.join(name)))
            .collect();
        let assets: Vec<Value> = urls
            .iter()
            .map(|url| json!({"url": url, "kind": "wallpaper"}))
            .collect();
        let store = temp.store(json!({"schema": 2, "assets": assets, "settings": {}}));
        let threads: Vec<_> = (0..4)
            .map(|index| {
                let store = store.clone();
                let urls = urls.clone();
                std::thread::spawn(move || {
                    for round in 0..10 {
                        set_favorite(&store, &urls[index], round % 2 == 0).unwrap();
                        save_app_setting(&store, "frameRateLimit", json!(30 + index)).unwrap();
                        save_wallpaper_assignment(
                            &store,
                            &format!("Monitor {index} ü"),
                            &urls[index],
                            false,
                        )
                        .unwrap();
                        save_playlist(&store, &urls[..=index]).unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let scene = store.load_scene().unwrap();
        for (index, url) in urls.iter().enumerate() {
            assert_eq!(scene["assets"][index]["favorite"], false, "{url}");
            assert_eq!(
                scene["wallpapersByMonitor"][format!("Monitor {index} ü")]["url"],
                *url
            );
        }
        assert!(scene["settings"]["frameRateLimit"].is_number());
        assert!(scene["playlist"].as_array().is_some_and(|v| !v.is_empty()));
    }

    #[test]
    fn refuses_invalid_wallpaper_assignment() {
        let temp = Temp::new();
        let store = temp.store(json!({
            "assets": [
                {"url":"file:///tmp/actor.png", "kind":"character"},
                {"url":"file:///tmp/missing.png", "kind":"wallpaper", "missing":true}
            ]
        }));
        assert!(
            save_wallpaper_assignment(&store, "DP-1", "file:///tmp/unknown.png", false).is_err()
        );
        assert!(save_wallpaper_assignment(&store, "DP-1", "file:///tmp/actor.png", false).is_err());
        assert!(
            save_wallpaper_assignment(&store, "DP-1", "file:///tmp/missing.png", false).is_err()
        );
        assert!(save_wallpaper_assignment(&store, "", "file:///tmp/missing.png", false).is_err());
    }

    #[test]
    fn refuses_to_restore_removed_or_non_character_assets() {
        let tmp = Temp::new();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        store
            .save_scene(json!({
                "assets": [
                    {"url":"file:///tmp/wallpaper.png","kind":"wallpaper"},
                    {"url":"file:///tmp/missing.png","kind":"character","missing":true}
                ]
            }))
            .unwrap();
        assert!(save_active_actors(&store, &["file:///tmp/wallpaper.png".into()]).is_err());
        assert!(save_active_actors(&store, &["file:///tmp/missing.png".into()]).is_err());
        assert_eq!(store.load_scene().unwrap()["actorUrls"], Value::Null);
    }

    #[test]
    fn named_playlist_round_trips_into_a_different_library() {
        let tmp = Temp::new();
        let source_path = tmp.0.join("aurora.png");
        image::RgbaImage::from_pixel(32, 18, image::Rgba([40, 20, 90, 255]))
            .save(&source_path)
            .unwrap();
        let first_store = LibraryStore::new(tmp.0.join("library-a"), "");
        let url = import(&first_store, &[source_path], ImportKind::Wallpaper)
            .unwrap()
            .first_url
            .unwrap();
        let playlist_path = tmp.0.join("Playlists/evening.json");
        save_named_playlist(&first_store, &playlist_path, "Evening", &[url], 45, true).unwrap();
        let second_store = LibraryStore::new(tmp.0.join("library-b"), "");
        let loaded = load_named_playlist(&second_store, &playlist_path).unwrap();
        assert_eq!(loaded.name, "Evening");
        assert_eq!(loaded.unavailable, 0);
        assert_eq!(loaded.urls.len(), 1);
        let scene = second_store.load_scene().unwrap();
        assert_eq!(scene["playlist"], serde_json::json!(loaded.urls));
        assert_eq!(scene["playlistName"], "Evening");
        assert_eq!(scene["settings"]["playlistInterval"], 45);
        assert_eq!(scene["settings"]["playlistRandom"], true);
        assert_eq!(scene["assets"].as_array().unwrap().len(), 1);
        assert!(
            library::file_path(scene["assets"][0]["url"].as_str().unwrap())
                .unwrap()
                .is_file()
        );
    }

    #[test]
    fn character_settings_round_trip_without_overwriting_other_properties() {
        let tmp = Temp::new();
        let path = tmp.0.join("character.png");
        image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .unwrap();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let url = import(&store, &[path], ImportKind::Character)
            .unwrap()
            .first_url
            .unwrap();
        set_favorite(&store, &url, true).unwrap();
        let changed = ActorSettings {
            x: 0.25,
            y: 0.75,
            scale: 140.0,
            opacity: 65.0,
            rotation: -20.0,
            flip_x: true,
            smoothing: true,
            speed: 120.0,
            playing: false,
            looping: false,
        };
        save_character_settings(&store, &url, changed).unwrap();
        let saved = store.load_scene().unwrap();
        let asset = &saved["assets"][0];
        assert_eq!(asset["posX"], 25.0);
        assert_eq!(asset["posY"], 75.0);
        assert_eq!(asset["scale"], 140.0);
        assert_eq!(asset["opacity"], 65.0);
        assert_eq!(asset["rotation"], -20.0);
        assert_eq!(asset["flipX"], true);
        assert_eq!(asset["playing"], false);
        assert_eq!(asset["loop"], false);
        assert_eq!(asset["favorite"], true);
        assert!(
            save_character_settings(
                &store,
                &url,
                ActorSettings {
                    x: f32::NAN,
                    ..changed
                }
            )
            .is_err()
        );
        assert_eq!(store.load_scene().unwrap()["assets"][0]["posX"], 25.0);
    }

    #[test]
    fn removal_preserves_files_and_cleans_playlist_and_selection() {
        let tmp = Temp::new();
        let source = tmp.0.join("image.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([100, 20, 40, 255]))
            .save(&source)
            .unwrap();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let url = import(&store, std::slice::from_ref(&source), ImportKind::Wallpaper)
            .unwrap()
            .first_url
            .unwrap();
        let mut scene = store.load_scene().unwrap();
        let project_file =
            library::file_path(scene["assets"][0]["configUrl"].as_str().unwrap()).unwrap();
        scene["selectedUrl"] = json!(url);
        scene["playlist"] = json!([url, "file:///other"]);
        store.save_scene(scene).unwrap();
        remove(&store, &url).unwrap();
        let saved = store.load_scene().unwrap();
        assert!(saved["assets"].as_array().unwrap().is_empty());
        assert_eq!(saved["playlist"], json!(["file:///other"]));
        assert_eq!(saved["playlistDirty"], true);
        assert_eq!(saved["selectedUrl"], "");
        assert!(source.is_file() && project_file.is_file());
    }

    #[test]
    fn removal_refuses_assigned_wallpapers_and_actors() {
        let tmp = Temp::new();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let url = "file:///tmp/wallpaper.png";
        let state = json!({"assets":[{"url":url}], "wallpapersByMonitor":{"one":{"url":url}}});
        store.save_scene(state).unwrap();
        assert!(remove(&store, url).is_err());
        let mut state = store.load_scene().unwrap();
        state["wallpapersByMonitor"] = json!({});
        state["actorUrls"] = json!([url]);
        store.save_scene(state).unwrap();
        assert!(remove(&store, url).is_err());
        assert_eq!(
            store.load_scene().unwrap()["assets"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn rejects_unsupported_files_and_unknown_favorites() {
        let tmp = Temp::new();
        let path = tmp.0.join("notes.txt");
        std::fs::write(&path, b"text").unwrap();
        let store = LibraryStore::new(tmp.0.join("library"), "");
        let result = import(&store, &[path], ImportKind::Wallpaper).unwrap();
        assert_eq!(result.added, 0);
        assert_eq!(result.errors.len(), 1);
        assert!(set_favorite(&store, "file:///unknown", true).is_err());
    }
}
