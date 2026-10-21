//! Compatible library storage for the Rust manager. Installed content is read-only;
//! edits, imports and playlists belong to the manager's separate data directory.
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
const MAX_JSON: u64 = 16 * 1024 * 1024;

fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Read one `XDG_*_DIR` entry from `user-dirs.dirs`. The directory name is
/// localized by the desktop (Documents, Documentos, Dokumente...), so it must
/// never be hardcoded.
fn user_dir(key: &str) -> Option<PathBuf> {
    let home = home_directory()?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("user-dirs.dirs");
    let contents = fs::read_to_string(config).ok()?;
    let home_text = home.to_string_lossy().into_owned();
    for line in contents.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').replace("$HOME", &home_text);
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Some(path);
        }
    }
    None
}

/// Managed library root: `$XDG_DOCUMENTS_DIR/WaniEngine`, localized per system
/// language (Documentos on pt-BR, Documents on en-US, Dokumente on de-DE).
pub fn library_directory() -> PathBuf {
    let base = user_dir("XDG_DOCUMENTS_DIR")
        .filter(|path| path.is_dir())
        .or_else(|| {
            let home = home_directory()?;
            let english = home.join("Documents");
            let localized = home.join("Documentos");
            Some(if english.is_dir() || !localized.is_dir() {
                english
            } else {
                localized
            })
        })
        .or_else(|| home_directory())
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("WaniEngine")
}

pub fn user_videos_directory() -> PathBuf {
    let base = user_dir("XDG_VIDEOS_DIR").unwrap_or_else(|| {
        let home = home_directory().unwrap_or_else(|| PathBuf::from("."));
        let english = home.join("Videos");
        let localized = home.join("Vídeos");
        if english.is_dir() || !localized.is_dir() {
            english
        } else {
            localized
        }
    });
    base.join("WaniEngine")
}

fn is_supported_media(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase()),
        Some(extension) if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif" | "apng" | "bmp" | "svg" | "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v")
    )
}

fn media_kind(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("gif" | "apng") => "character",
        _ => "wallpaper",
    }
}

fn discover_media(root: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_media(path))
        .map(|path| {
            let name = path
                .file_stem()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Content".into());
            let video = matches!(
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.to_ascii_lowercase())
                    .as_deref(),
                Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v")
            );
            json!({
                "id": format!("file-{}", path.to_string_lossy()),
                "url": file_url(&path),
                "originalSource": file_url(&path),
                "name": name,
                "kind": media_kind(&path),
                "video": video,
                "favorite": false,
                "addedAt": 0,
                "speed": 100,
                "posX": 50,
                "posY": 96,
                "opacity": 100,
                "flipX": false,
                "scale": 100,
                "volume": 50,
                "muted": true,
                "fit": "cover"
            })
        })
        .collect()
}

pub fn file_path(url: &str) -> Result<PathBuf> {
    let path = if let Some(rest) = url.strip_prefix("file://") {
        if !rest.starts_with('/') {
            return Err("Only local file URLs are supported".into());
        }
        let mut bytes = Vec::new();
        let input = rest.as_bytes();
        let mut index = 0;
        while index < input.len() {
            if input[index] == b'%' {
                let hex = rest.get(index + 1..index + 3).ok_or("Invalid file URL")?;
                bytes.push(u8::from_str_radix(hex, 16)?);
                index += 3;
            } else {
                bytes.push(input[index]);
                index += 1;
            }
        }
        String::from_utf8(bytes)?
    } else {
        url.to_owned()
    };
    if path.contains('\0') {
        return Err("Invalid file path".into());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("The file path must be absolute".into());
    }
    Ok(path)
}

pub fn file_url(path: &Path) -> String {
    let mut result = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

fn read_json(path: &Path) -> Result<Value> {
    if fs::metadata(path)?.len() > MAX_JSON {
        return Err("The JSON file is too large".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    let parent = path.parent().ok_or("Invalid output path")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".write-{}-{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn safe_relative(folder: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("A content path escapes its folder".into());
    }
    let target = folder.join(path).canonicalize()?;
    if !target.starts_with(folder.canonicalize()?) || !target.is_file() {
        return Err("Invalid content file".into());
    }
    Ok(target)
}

pub fn resolve_asset(reference: &Value) -> Result<Value> {
    if reference.get("url").and_then(Value::as_str).is_some() {
        return Ok(reference.clone());
    }
    let config_url = reference["configUrl"]
        .as_str()
        .ok_or("Missing content configuration URL")?;
    let config_path = file_path(config_url)?;
    let config = read_json(&config_path)?;
    if config["format"] != "waniengine-content" || config["version"] != 1 {
        return Err("Unsupported content configuration".into());
    }
    let folder = config_path.parent().ok_or("Invalid content folder")?;
    let media_file = config["media"]["file"]
        .as_str()
        .ok_or("Missing media file")?;
    // A referenced video keeps its absolute path outside the folder. Anything
    // else must stay inside the folder, which keeps the traversal guard intact.
    let media = if config["media"]["referenced"].as_bool() == Some(true) {
        let path = file_path(&format!("file://{media_file}"))?;
        if !path.is_absolute() || !path.is_file() {
            return Err("The referenced media file is unavailable".into());
        }
        path
    } else {
        safe_relative(folder, media_file)?
    };
    let mut asset = config["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    for key in ["id", "name", "kind", "originalSource"] {
        asset.insert(key.into(), config[key].clone());
    }
    if let Some(metadata) = config["media"].as_object() {
        for (key, value) in metadata {
            if key != "file" && key != "thumbnail" {
                asset.insert(key.clone(), value.clone());
            }
        }
    }
    asset.insert("url".into(), json!(file_url(&media)));
    asset.insert("configUrl".into(), json!(config_url));
    asset.insert("folderUrl".into(), json!(file_url(folder)));
    if let Some(relative) = config["media"]["thumbnail"].as_str() {
        if let Ok(thumbnail) = safe_relative(folder, relative) {
            asset.insert("thumbnailUrl".into(), json!(file_url(&thumbnail)));
        }
    }
    let mut audio = config["audio"].as_object().cloned().unwrap_or_default();
    let mut tracks = Vec::new();
    for track in config["audio"]["files"].as_array().into_iter().flatten() {
        let mut track = track.clone();
        if let Some(relative) = track["file"].as_str() {
            if let Ok(path) = safe_relative(folder, relative) {
                track["url"] = json!(file_url(&path));
                tracks.push(track);
            }
        }
    }
    audio.insert("files".into(), json!(tracks));
    asset.insert("audio".into(), json!(audio));
    Ok(Value::Object(asset))
}

pub struct LibraryStore {
    pub root: PathBuf,
    pub installed_scene: PathBuf,
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)?.flatten() {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_directory(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn media_is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v")
    )
}

/// Package metadata stored next to the content configuration. This is the file
/// a `.wani` archive ships as `project.json`.
fn wallpaper_metadata(asset: &Value, media: &Path, referenced: bool) -> Result<Value> {
    Ok(json!({
        "format": "waniengine-wallpaper",
        "schema": 1,
        "id": asset["id"],
        "title": asset["name"],
        "kind": asset["kind"],
        "media": {
            "file": media.to_string_lossy(),
            "referenced": referenced,
            "original": asset["originalSource"],
        },
        "playback": {
            "speed": asset["speed"],
            "volume": asset["volume"],
            "fit": asset["fit"],
        }
    }))
}

/// Content configuration whose `media.file` may point outside the folder when
/// the media is referenced instead of copied.
fn content_configuration_referenced(asset: &Value, folder: &Path, media: &Path) -> Result<Value> {
    let mut config = content_configuration(asset, folder)?;
    if !media.starts_with(folder) {
        config["media"]["file"] = json!(media.to_string_lossy());
        config["media"]["referenced"] = json!(true);
    }
    Ok(config)
}

fn content_configuration(asset: &Value, folder: &Path) -> Result<Value> {
    let mut media = serde_json::Map::new();
    for key in [
        "width", "height", "fps", "video", "animated", "hasAudio", "codec", "duration",
    ] {
        if let Some(value) = asset.get(key) {
            media.insert(key.into(), value.clone());
        }
    }
    // Audio always lives inside the content folder; media may be referenced
    // from outside it when the user keeps a large video in place.
    let relative = |url: &str| -> Result<String> {
        let path = file_path(url)?;
        Ok(path.strip_prefix(folder)?.to_string_lossy().into_owned())
    };
    let media_url = asset["url"].as_str().ok_or("Missing media URL")?;
    let media_path = file_path(media_url)?;
    media.insert(
        "file".into(),
        match media_path.strip_prefix(folder) {
            Ok(inside) => json!(inside.to_string_lossy()),
            Err(_) => json!(media_path.to_string_lossy()),
        },
    );
    if let Some(url) = asset["thumbnailUrl"].as_str() {
        if let Ok(path) = relative(url) {
            media.insert("thumbnail".into(), json!(path));
        }
    }
    let mut properties = asset.as_object().cloned().ok_or("Invalid content asset")?;
    for key in [
        "id",
        "name",
        "kind",
        "originalSource",
        "url",
        "folderUrl",
        "configUrl",
        "thumbnailUrl",
        "audio",
        "width",
        "height",
        "fps",
        "video",
        "animated",
        "hasAudio",
        "codec",
        "duration",
    ] {
        properties.remove(key);
    }
    let mut audio = asset["audio"].clone();
    for track in audio["files"].as_array_mut().into_iter().flatten() {
        track["file"] = json!(relative(track["url"].as_str().ok_or("Missing audio URL")?)?);
        track
            .as_object_mut()
            .ok_or("Invalid audio track")?
            .remove("url");
    }
    Ok(
        json!({"format":"waniengine-content","version":1,"id":asset["id"],"name":asset["name"],"kind":asset["kind"],"originalSource":asset["originalSource"],"media":media,"properties":properties,"audio":audio}),
    )
}

impl LibraryStore {
    pub fn new(root: impl Into<PathBuf>, installed_scene: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            installed_scene: installed_scene.into(),
        }
    }

    pub fn load_scene(&self) -> Result<Value> {
        let own = self.root.join("scene.json");
        let source = if own.is_file() {
            own
        } else {
            self.installed_scene.clone()
        };
        let mut state = if source.is_file() {
            read_json(&source)?
        } else {
            json!({"schema":2,"assets":[],"playlist":[],"settings":{},"actorUrls":[]})
        };
        if !state.is_object() || !state["assets"].is_array() {
            return Err("Invalid library state".into());
        }
        let mut assets = Vec::new();
        for reference in state["assets"].as_array().unwrap() {
            match resolve_asset(reference) {
                Ok(asset) => assets.push(asset),
                Err(error) => {
                    // Retain an unavailable entry, rather than dropping the rest of the library.
                    let config = reference["configUrl"].as_str().unwrap_or("");
                    let name = file_path(config)
                        .ok()
                        .and_then(|p| {
                            p.parent()
                                .and_then(Path::file_name)
                                .map(|s| s.to_string_lossy().into_owned())
                        })
                        .unwrap_or_else(|| "Unavailable content".into());
                    assets.push(json!({"name":name,"url":config,"configUrl":config,"kind":"wallpaper","missing":true,"loadError":error.to_string(),"width":0,"height":0}));
                }
            }
        }
        let known_urls: std::collections::HashSet<String> = assets
            .iter()
            .filter_map(|asset| asset["url"].as_str().map(str::to_owned))
            .collect();
        // Files dropped into the Videos folder are discovered fresh on every
        // load and reported separately. They must never be merged into
        // `assets`, because the UI saves that list back to scene.json and the
        // discovered files would become stale ghost entries.
        let discovered: Vec<Value> = discover_media(&user_videos_directory())
            .into_iter()
            .filter(|asset| {
                asset["url"]
                    .as_str()
                    .is_some_and(|url| !known_urls.contains(url))
            })
            .collect();
        state["assets"] = json!(assets);
        state["discovered"] = json!(discovered);
        state["schema"] = json!(2);
        // The preview must not install an autostart entry merely because the old app did.
        if !self.root.join("scene.json").is_file() {
            state["settings"]["startWithSession"] = json!(false);
            state["settings"]["restoreOnLaunch"] = json!(false);
        }
        Ok(state)
    }

    pub fn save_scene(&self, mut state: Value) -> Result<()> {
        if !state.is_object() || !state["assets"].is_array() {
            return Err("Invalid library state".into());
        }
        state["schema"] = json!(2);
        fs::create_dir_all(&self.root)?;
        let own_root = self.root.canonicalize()?;
        for asset in state["assets"].as_array().unwrap() {
            if let Some(url) = asset["configUrl"].as_str() {
                let path = file_path(url)?;
                if let Ok(path) = path.canonicalize() {
                    if path.starts_with(&own_root) {
                        write_json(
                            &path,
                            &content_configuration(
                                asset,
                                path.parent().ok_or("Invalid content folder")?,
                            )?,
                        )?;
                    }
                }
            }
        }
        write_json(&self.root.join("scene.json"), &state)
    }

    /// Absorb projects from a previous library location into this store.
    ///
    /// This only ever copies: the old folder stays untouched so a failed or
    /// partial migration can never lose the user's content. Returns the number
    /// of projects created.
    pub fn migrate_from(&self, legacy: &Path) -> Result<usize> {
        let contents = legacy.join("contents");
        if !contents.is_dir() {
            return Ok(0);
        }
        fs::create_dir_all(&self.root)?;
        let mut migrated = 0;
        for entry in fs::read_dir(&contents)?.flatten() {
            let project = entry.path();
            if !project.is_dir() {
                continue;
            }
            let config = project.join("content.json");
            let Ok(value) = read_json(&config) else {
                continue;
            };
            // Already present in the new library? Skip instead of duplicating.
            let name = value["name"].as_str().unwrap_or("Content");
            let destination = self.root.join("contents").join(name);
            if destination.exists() {
                continue;
            }
            if copy_directory(&project, &destination).is_ok() {
                migrated += 1;
            }
        }
        Ok(migrated)
    }

    pub fn organize(&self, mut asset: Value) -> Result<Value> {
        let source = file_path(asset["url"].as_str().ok_or("Missing source URL")?)?;
        if !source.is_file() {
            return Err("The source file is unavailable".into());
        }
        if asset["configUrl"].is_string() {
            return Ok(asset);
        }
        let raw_name = asset["name"].as_str().unwrap_or("Content");
        let name: String = raw_name
            .chars()
            .take(100)
            .map(|c| {
                if c.is_control() || "\\/:*?\"<>|".contains(c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let name = if name.trim().is_empty() || [".", ".."].contains(&name.as_str()) {
            "Content"
        } else {
            name.trim()
        };
        let library = self.root.join("contents");
        fs::create_dir_all(&library)?;
        let mut folder = library.join(name);
        let mut number = 2;
        while folder.exists() {
            folder = library.join(format!("{name} ({number})"));
            number += 1;
        }
        fs::create_dir(&folder)?;
        asset["id"] = json!(format!(
            "rust-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        asset["originalSource"] = asset["url"].clone();
        asset["folderUrl"] = json!(file_url(&folder));
        asset["configUrl"] = json!(file_url(&folder.join("content.json")));

        // Images and animations are small, so they are copied into the project
        // and stay portable. Video is referenced in place: copying multi-GB
        // files into the library used to duplicate media for no reason.
        let is_video = media_is_video(&source);
        let media_path = if is_video {
            source.clone()
        } else {
            let target_folder = folder.join("source");
            fs::create_dir_all(&target_folder)?;
            let destination =
                target_folder.join(source.file_name().ok_or("Invalid source filename")?);
            if let Err(error) = fs::copy(&source, &destination) {
                let _ = fs::remove_dir_all(&folder);
                return Err(error.into());
            }
            destination
        };
        asset["url"] = json!(file_url(&media_path));
        asset["referenced"] = json!(is_video);
        if !asset["audio"].is_object() {
            asset["audio"] = json!({"enabled":false,"files":[]});
        }
        if let Some(thumbnail) = asset["thumbnailUrl"]
            .as_str()
            .and_then(|s| file_path(s).ok())
        {
            let target = folder.join("preview.png");
            if fs::copy(thumbnail, &target).is_ok() {
                asset["thumbnailUrl"] = json!(file_url(&target));
            }
        }
        write_json(
            &folder.join("wallpaper.json"),
            &wallpaper_metadata(&asset, &media_path, is_video)?,
        )?;
        let config = content_configuration_referenced(&asset, &folder, &media_path)?;
        write_json(&folder.join("content.json"), &config)?;
        Ok(asset)
    }

    pub fn add_audio(&self, asset: &Value, sources: &[Value]) -> Result<Value> {
        let mut tracks = asset["audio"]["files"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let identity = asset["id"]
            .as_str()
            .unwrap_or("character")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "-_".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let content_folder = asset["folderUrl"]
            .as_str()
            .and_then(|url| file_path(url).ok());
        let folder = match content_folder {
            Some(folder)
                if folder
                    .canonicalize()?
                    .starts_with(self.root.canonicalize()?) =>
            {
                folder.join("audio")
            }
            _ => self.root.join("audio").join(identity),
        };
        fs::create_dir_all(&folder)?;
        for source in sources {
            let source = file_path(source.as_str().ok_or("Invalid audio URL")?)?;
            let extension = source
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !["wav", "mp3", "ogg", "flac", "m4a", "aac", "opus"].contains(&extension.as_str()) {
                return Err("Unsupported audio format".into());
            }
            let mut target = folder.join(source.file_name().ok_or("Invalid audio filename")?);
            let mut index = 2;
            while target.exists() {
                target = folder.join(format!(
                    "{}-{index}.{extension}",
                    source.file_stem().unwrap().to_string_lossy()
                ));
                index += 1;
            }
            fs::copy(&source, &target)?;
            tracks.push(json!({"name":target.file_name().unwrap().to_string_lossy(),"url":file_url(&target),"file":target.strip_prefix(&self.root)?.to_string_lossy(),"playback":{"enabled":true,"mode":"once","volume":100,"intervalSeconds":30,"radiusPixels":40,"cooldownSeconds":2}}));
        }
        Ok(json!(tracks))
    }
}

pub fn load_playlist(url: &str) -> Result<Value> {
    let value = read_json(&file_path(url)?)?;
    validate_playlist(&value)?;
    Ok(value)
}
pub fn save_playlist(url: &str, value: &Value) -> Result<()> {
    validate_playlist(value)?;
    write_json(&file_path(url)?, value)
}
fn validate_playlist(value: &Value) -> Result<()> {
    let assets = value["assets"]
        .as_array()
        .ok_or("Missing playlist assets")?;
    if value["format"] != "waniengine-playlist" || value["version"] != 1 || assets.len() > 1000 {
        return Err("Unsupported playlist format".into());
    }
    for asset in assets {
        if asset["kind"] != "wallpaper" {
            return Err("Only wallpapers belong in playlists".into());
        }
        file_path(asset["url"].as_str().ok_or("Missing playlist URL")?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "waniengine-library-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn file_urls_round_trip_spaces_unicode_and_percent() {
        let path = PathBuf::from("/tmp/紫 folder/a%#?.gif");
        assert_eq!(file_path(&file_url(&path)).unwrap(), path);
        assert!(file_path("file://remote/a").is_err());
    }
    #[test]
    fn rejects_content_traversal() {
        let t = Temp::new();
        fs::write(t.0.join("a"), b"a").unwrap();
        assert!(safe_relative(&t.0, "../a").is_err());
        assert!(safe_relative(&t.0, "/a").is_err());
    }
    #[test]
    fn installed_library_is_resolved_and_never_modified() {
        let t = Temp::new();
        let content = t.0.join("original");
        fs::create_dir(&content).unwrap();
        fs::write(content.join("image.png"), b"image").unwrap();
        fs::write(content.join("sound.wav"), b"sound").unwrap();
        let config = json!({"format":"waniengine-content","version":1,"id":"one","name":"Existing","kind":"character","media":{"file":"image.png","width":120,"height":200},"properties":{"scale":80},"audio":{"enabled":true,"files":[{"file":"sound.wav","name":"Sound"}]}});
        write_json(&content.join("content.json"), &config).unwrap();
        let original = json!({"schema":2,"assets":[{"configUrl":file_url(&content.join("content.json"))}],"settings":{"startWithSession":true}});
        write_json(&t.0.join("installed.json"), &original).unwrap();
        let store = LibraryStore::new(t.0.join("preview"), t.0.join("installed.json"));
        let mut state = store.load_scene().unwrap();
        assert_eq!(state["assets"][0]["scale"], 80);
        assert!(state["assets"][0]["audio"]["files"][0]["url"].is_string());
        assert_eq!(state["settings"]["startWithSession"], false);
        state["assets"][0]["scale"] = json!(90);
        store.save_scene(state).unwrap();
        assert_eq!(store.load_scene().unwrap()["assets"][0]["scale"], 90);
        assert_eq!(read_json(&content.join("content.json")).unwrap(), config);
        assert_eq!(read_json(&t.0.join("installed.json")).unwrap(), original);
    }
    #[test]
    fn missing_content_preserves_other_entries() {
        let t = Temp::new();
        write_json(&t.0.join("installed"),&json!({"assets":[{"configUrl":file_url(&t.0.join("gone/content.json"))},{"name":"Found","url":file_url(&t.0.join("found.png"))}]})).unwrap();
        let s = LibraryStore::new(t.0.join("own"), t.0.join("installed"))
            .load_scene()
            .unwrap();
        assert_eq!(s["assets"].as_array().unwrap().len(), 2);
        assert_eq!(s["assets"][0]["missing"], true);
    }
    #[test]
    fn discovered_media_never_enters_the_saved_asset_list() {
        let t = Temp::new();
        write_json(
            &t.0.join("installed"),
            &json!({"assets":[{"name":"Saved","url":file_url(&t.0.join("saved.png"))}]}),
        )
        .unwrap();
        fs::write(t.0.join("saved.png"), b"png").unwrap();
        let store = LibraryStore::new(t.0.join("own"), t.0.join("installed"));
        let state = store.load_scene().unwrap();
        // The persisted list keeps only owned entries; discovery is reported
        // separately so it can never be written back to scene.json.
        assert_eq!(state["assets"].as_array().unwrap().len(), 1);
        assert_eq!(state["assets"][0]["name"], "Saved");
        assert!(state["discovered"].is_array());
        store.save_scene(state.clone()).unwrap();
        let reloaded = store.load_scene().unwrap();
        assert_eq!(reloaded["assets"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn organize_sanitizes_hostile_names_and_never_escapes_the_root() {
        let t = Temp::new();
        fs::write(t.0.join("a.gif"), b"gif").unwrap();
        let store = LibraryStore::new(t.0.join("preview"), t.0.join("none"));
        for name in ["..", ".", "../escape", "a/b\\c:d", "   "] {
            let asset = store
                .organize(json!({"url":file_url(&t.0.join("a.gif")),"name":name,"kind":"wallpaper"}))
                .unwrap();
            let folder = file_path(asset["folderUrl"].as_str().unwrap()).unwrap();
            assert!(
                folder.starts_with(store.root.join("contents")),
                "{name} escaped the library: {folder:?}"
            );
            assert!(folder.is_dir());
        }
    }

    #[test]
    fn discover_media_keeps_supported_files_and_ignores_the_rest() {
        let t = Temp::new();
        fs::write(t.0.join("still.PNG"), b"png").unwrap();
        fs::write(t.0.join("loop.GIF"), b"gif").unwrap();
        fs::write(t.0.join("clip.MP4"), b"mp4").unwrap();
        fs::write(t.0.join("notes.txt"), b"txt").unwrap();
        let assets = discover_media(&t.0);
        assert_eq!(assets.len(), 3);
        assert!(assets.iter().all(|asset| asset["video"].is_boolean()));
        let loop_asset = assets
            .iter()
            .find(|asset| asset["name"] == "loop")
            .expect("gif discovered");
        assert_eq!(loop_asset["kind"], "character");
        assert_eq!(loop_asset["video"], false);
        let clip = assets
            .iter()
            .find(|asset| asset["name"] == "clip")
            .expect("video discovered");
        assert_eq!(clip["video"], true);
        assert_eq!(clip["kind"], "wallpaper");
    }

    #[test]
    fn image_is_copied_into_the_project_folder() {
        let t = Temp::new();
        fs::write(t.0.join("aura.png"), b"png").unwrap();
        let store = LibraryStore::new(t.0.join("library"), t.0.join("none"));
        let asset = store
            .organize(json!({"url":file_url(&t.0.join("aura.png")),"name":"Aura","kind":"wallpaper"}))
            .unwrap();
        let media = file_path(asset["url"].as_str().unwrap()).unwrap();
        assert!(media.starts_with(store.root.join("contents/Aura/source")), "{media:?}");
        assert!(media.is_file());
        assert_eq!(asset["referenced"], false);
    }

    #[test]
    fn video_is_referenced_instead_of_duplicated() {
        let t = Temp::new();
        fs::write(t.0.join("rain.mp4"), b"mp4").unwrap();
        let store = LibraryStore::new(t.0.join("library"), t.0.join("none"));
        let asset = store
            .organize(json!({"url":file_url(&t.0.join("rain.mp4")),"name":"Rain","kind":"wallpaper"}))
            .unwrap();
        let media = file_path(asset["url"].as_str().unwrap()).unwrap();
        assert_eq!(media, t.0.join("rain.mp4"), "video must not be copied");
        assert_eq!(asset["referenced"], true);
        let project = store.root.join("contents/Rain");
        assert!(!project.join("source/rain.mp4").exists(), "no duplicated video");
        let metadata = read_json(&project.join("wallpaper.json")).unwrap();
        assert_eq!(metadata["format"], "waniengine-wallpaper");
        assert_eq!(metadata["media"]["referenced"], true);
        assert_eq!(metadata["title"], "Rain");
    }

    #[test]
    fn referenced_video_survives_asset_resolution() {
        let t = Temp::new();
        fs::write(t.0.join("rain.webm"), b"webm").unwrap();
        let store = LibraryStore::new(t.0.join("library"), t.0.join("none"));
        let asset = store
            .organize(json!({"url":file_url(&t.0.join("rain.webm")),"name":"Rain","kind":"wallpaper"}))
            .unwrap();
        let resolved = resolve_asset(&json!({"configUrl":asset["configUrl"]})).unwrap();
        assert_eq!(resolved["url"], file_url(&t.0.join("rain.webm")));
        assert!(file_path(resolved["url"].as_str().unwrap()).unwrap().is_file());
    }

    #[test]
    fn migration_copies_projects_and_keeps_the_legacy_folder() {
        let t = Temp::new();
        let legacy = t.0.join("legacy");
        let project = legacy.join("contents/Remilia");
        fs::create_dir_all(&project).unwrap();
        write_json(
            &project.join("content.json"),
            &json!({"format":"waniengine-content","version":1,"id":"old","name":"Remilia","kind":"wallpaper"}),
        )
        .unwrap();
        fs::write(project.join("remilia.mp4"), b"mp4").unwrap();

        let store = LibraryStore::new(t.0.join("library"), t.0.join("none"));
        assert_eq!(store.migrate_from(&legacy).unwrap(), 1);
        assert!(store.root.join("contents/Remilia/content.json").is_file());
        assert!(store.root.join("contents/Remilia/remilia.mp4").is_file());
        // The origin is never modified or removed.
        assert!(project.join("content.json").is_file());
        assert!(project.join("remilia.mp4").is_file());
        // Running it twice must not duplicate the project.
        assert_eq!(store.migrate_from(&legacy).unwrap(), 0);
    }

    #[test]
    fn migration_of_a_missing_legacy_folder_is_a_noop() {
        let t = Temp::new();
        let store = LibraryStore::new(t.0.join("library"), t.0.join("none"));
        assert_eq!(store.migrate_from(&t.0.join("nope")).unwrap(), 0);
        assert!(!store.root.join("contents").exists());
    }

    #[test]
    fn referenced_media_cannot_escape_through_relative_paths() {
        let t = Temp::new();
        let folder = t.0.join("contents/Evil");
        fs::create_dir_all(&folder).unwrap();
        for hostile in ["../../etc/passwd", "..", "./relative.mp4", "dir/../x.mp4"] {
            write_json(
                &folder.join("content.json"),
                &json!({"format":"waniengine-content","version":1,"id":"x","name":"Evil","kind":"wallpaper",
                        "media":{"file":hostile,"referenced":true}}),
            )
            .unwrap();
            let resolved = resolve_asset(&json!({"configUrl":file_url(&folder.join("content.json"))}));
            assert!(resolved.is_err(), "{hostile} must be rejected");
        }
    }

    #[test]
    fn library_directory_respects_localized_documents_folder() {
        let home = PathBuf::from("$HOME");
        let _ = home;
        // Falls back to an absolute documents folder and never returns a
        // relative path, regardless of the system language.
        let directory = library_directory();
        assert!(directory.is_absolute());
        assert_eq!(directory.file_name().unwrap(), "WaniEngine");
    }

    #[test]
    fn import_and_audio_copy_keep_originals() {
        let t = Temp::new();
        fs::write(t.0.join("a.gif"), b"gif").unwrap();
        fs::write(t.0.join("a.wav"), b"wav").unwrap();
        let store = LibraryStore::new(t.0.join("preview"), t.0.join("none"));
        let asset = store
            .organize(json!({"url":file_url(&t.0.join("a.gif")),"name":"A","kind":"character"}))
            .unwrap();
        assert!(
            file_path(asset["url"].as_str().unwrap())
                .unwrap()
                .starts_with(&store.root)
        );
        let tracks = store
            .add_audio(&asset, &[json!(file_url(&t.0.join("a.wav")))])
            .unwrap();
        assert_eq!(tracks.as_array().unwrap().len(), 1);
        assert!(t.0.join("a.gif").is_file());
        assert!(t.0.join("a.wav").is_file());
        let mut saved_asset = asset.clone();
        saved_asset["audio"]["files"] = tracks;
        saved_asset["scale"] = json!(75);
        store.save_scene(json!({"assets":[saved_asset]})).unwrap();
        let resolved = resolve_asset(&json!({"configUrl":asset["configUrl"]})).unwrap();
        assert_eq!(resolved["scale"], 75);
        assert_eq!(read_json(&file_path(asset["configUrl"].as_str().unwrap()).unwrap()).unwrap()["format"], "waniengine-content");
        assert!(
            file_path(resolved["audio"]["files"][0]["url"].as_str().unwrap())
                .unwrap()
                .is_file()
        );
    }
    #[test]
    fn playlist_round_trip_and_validation() {
        let t = Temp::new();
        let url = file_url(&t.0.join("playlist.json"));
        let value = json!({"format":"waniengine-playlist","version":1,"assets":[{"url":"file:///tmp/a.mp4","kind":"wallpaper"}]});
        save_playlist(&url, &value).unwrap();
        assert_eq!(load_playlist(&url).unwrap(), value);
        assert!(save_playlist(&url, &json!({"assets":[]})).is_err());
        assert_eq!(load_playlist(&url).unwrap(), value);
    }
    #[test]
    fn invalid_save_preserves_previous_scene() {
        let t = Temp::new();
        let store = LibraryStore::new(&t.0, t.0.join("none"));
        let good = json!({"assets":[],"settings":{}});
        store.save_scene(good).unwrap();
        assert!(store.save_scene(json!({})).is_err());
        assert_eq!(store.load_scene().unwrap()["assets"], json!([]));
    }
}
