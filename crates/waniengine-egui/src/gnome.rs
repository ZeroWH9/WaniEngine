//! GNOME static wallpaper control through the desktop's GSettings schema.
//!
//! GNOME exposes one background image for the whole desktop. This backend
//! handles still images only. Video is routed by desktop.rs to the Rust port
//! of the repository's GNOME/XWayland desktop-window adapter.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{self, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use waniengine_core::lock::SessionLock;

const SCHEMA: &str = "org.gnome.desktop.background";
const KEYS: [&str; 3] = ["picture-uri", "picture-uri-dark", "picture-options"];
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    original: BTreeMap<String, String>,
    applied: BTreeMap<String, String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Recovery {
    owner_pid: u32,
    owner_identity: String,
    snapshot: Option<Snapshot>,
}

struct Inner {
    path: PathBuf,
    program: PathBuf,
    recovery: Mutex<Recovery>,
    owns_recovery: AtomicBool,
    _session_lock: Option<SessionLock>,
}

#[derive(Clone)]
pub struct WallpaperController(Arc<Inner>);

impl WallpaperController {
    pub fn owns_session(&self) -> bool {
        self.0._session_lock.is_some()
    }
    pub fn new() -> Self {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("waniengine-state-{}", process::id()))
            })
            .join("WaniEngine/gnome-wallpaper-session.json");
        Self::with_paths(state, PathBuf::from("gsettings"))
    }

    fn with_paths(path: PathBuf, program: PathBuf) -> Self {
        let session_lock = lock_session(&path.with_extension("lock"));
        let mut recovery = read_recovery(&path).unwrap_or_default();
        let owns_recovery = session_lock.is_some();
        if owns_recovery && recovery.snapshot.is_some() && !owner_alive(&recovery) {
            if let Err(error) = restore_recovery(&program, &mut recovery) {
                eprintln!("waniengine: GNOME wallpaper recovery will be retried: {error}");
            }
            if recovery.snapshot.is_none() {
                let _ = fs::remove_file(&path);
            } else {
                let _ = write_recovery(&path, &recovery);
            }
        }
        Self(Arc::new(Inner {
            path,
            program,
            recovery: Mutex::new(recovery),
            owns_recovery: AtomicBool::new(owns_recovery),
            _session_lock: session_lock,
        }))
    }

    /// Applies a still image across all GNOME displays.
    pub fn apply(&self, image: &Path, fit: &str, animated: bool) -> Result<(), String> {
        if self.0._session_lock.is_none() {
            return Err(
                "Another WaniEngine instance currently controls the GNOME wallpaper".into(),
            );
        }
        if animated {
            return Err("GNOME's GSettings wallpaper supports still images only".into());
        }
        if image
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"
                )
            })
        {
            return Err("GNOME's GSettings wallpaper supports still images only".into());
        }
        if !matches!(fit, "cover" | "contain" | "stretch" | "center") {
            return Err("Unsupported wallpaper fit mode".into());
        }
        if !image.is_file() {
            return Err("Wallpaper file is unavailable".into());
        }
        let image = image.canonicalize().map_err(|error| error.to_string())?;
        let uri = waniengine_core::library::file_url(&image);
        let options = match fit {
            "cover" => "zoom",
            "contain" => "scaled",
            "stretch" => "stretched",
            "center" => "centered",
            _ => unreachable!("fit mode was validated"),
        };
        // File URLs are percent encoded, so they cannot inject GVariant syntax.
        let applied = BTreeMap::from([
            ("picture-uri".to_owned(), format!("'{uri}'")),
            ("picture-uri-dark".to_owned(), format!("'{uri}'")),
            ("picture-options".to_owned(), format!("'{options}'")),
        ]);

        let mut recovery = self
            .0
            .recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if recovery.snapshot.is_some() && !owner_alive(&recovery) {
            restore_recovery(&self.0.program, &mut recovery)?;
            if recovery.snapshot.is_some() {
                let _ = write_recovery(&self.0.path, &recovery);
                return Err("A previous GNOME wallpaper session still needs restoration".into());
            }
        }

        let current = read_values(&self.0.program)?;
        let mut original = BTreeMap::new();
        if let Some(previous) = recovery.snapshot.take() {
            for key in KEYS {
                let current_value = current.get(key).cloned().unwrap_or_default();
                let still_owned = previous.applied.get(key) == Some(&current_value);
                let value = if still_owned {
                    previous.original.get(key).cloned().unwrap_or(current_value)
                } else {
                    current_value
                };
                original.insert(key.to_owned(), value);
            }
        } else {
            original = current;
        }

        recovery.snapshot = Some(Snapshot {
            original,
            applied: applied.clone(),
        });
        recovery.owner_pid = process::id();
        recovery.owner_identity = process_identity(process::id()).unwrap_or_default();
        write_recovery(&self.0.path, &recovery)?;
        self.0.owns_recovery.store(true, Ordering::Release);

        let result = (|| {
            for (key, value) in &applied {
                set_value(&self.0.program, key, value)?;
            }
            let confirmed = read_values(&self.0.program)?;
            if applied
                .iter()
                .all(|(key, value)| confirmed.get(key) == Some(value))
            {
                Ok(())
            } else {
                Err("GNOME did not confirm the wallpaper change".to_owned())
            }
        })();
        if let Err(error) = result {
            if let Err(restore_error) = restore_recovery(&self.0.program, &mut recovery) {
                let _ = write_recovery(&self.0.path, &recovery);
                return Err(format!(
                    "{error}; wallpaper restoration is pending: {restore_error}"
                ));
            }
            let _ = fs::remove_file(&self.0.path);
            return Err(error);
        }
        Ok(())
    }

    pub fn restore(&self) -> Result<(), String> {
        if self.0._session_lock.is_none() {
            return Err(
                "Another WaniEngine instance currently controls the GNOME wallpaper".into(),
            );
        }
        let mut recovery = self
            .0
            .recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        restore_recovery(&self.0.program, &mut recovery)?;
        recovery.owner_pid = 0;
        recovery.owner_identity.clear();
        let _ = fs::remove_file(&self.0.path);
        Ok(())
    }
}

impl Default for WallpaperController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.owns_recovery.load(Ordering::Acquire) {
            return;
        }
        let recovery = self
            .recovery
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if let Err(error) = restore_recovery(&self.program, recovery) {
            let _ = write_recovery(&self.path, recovery);
            eprintln!(
                "waniengine: GNOME wallpaper restoration will be retried next launch: {error}"
            );
        } else {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn read_values(program: &Path) -> Result<BTreeMap<String, String>, String> {
    KEYS.into_iter()
        .map(|key| get_value(program, key).map(|value| (key.to_owned(), value)))
        .collect()
}

fn get_value(program: &Path, key: &str) -> Result<String, String> {
    let output = run_gsettings(program, &["get", SCHEMA, key])?;
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("gsettings returned invalid output for {key}: {error}"))
}

fn set_value(program: &Path, key: &str, value: &str) -> Result<(), String> {
    run_gsettings(program, &["set", SCHEMA, key, value]).map(|_| ())
}

fn run_gsettings(program: &Path, args: &[&str]) -> Result<std::process::Output, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not run gsettings: {error}"))?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child
            .try_wait()
            .map_err(|error| format!("Could not wait for gsettings: {error}"))?
        {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("gsettings did not finish within 5 seconds".into());
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Could not collect gsettings output: {error}"))?;
    if output.status.success() {
        Ok(output)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            "gsettings rejected the GNOME wallpaper operation".into()
        } else {
            format!("gsettings failed: {stderr}")
        })
    }
}

fn restore_recovery(program: &Path, recovery: &mut Recovery) -> Result<(), String> {
    let Some(snapshot) = recovery.snapshot.as_mut() else {
        return Ok(());
    };
    let mut errors = Vec::new();
    for key in KEYS {
        let Some(expected) = snapshot.applied.get(key).cloned() else {
            continue;
        };
        let original = snapshot.original.get(key).cloned().unwrap_or_default();
        match get_value(program, key) {
            Ok(current) if current == expected => {
                match set_value(program, key, &original).and_then(|()| get_value(program, key)) {
                    Ok(restored) if restored == original => {
                        snapshot.applied.remove(key);
                    }
                    Ok(_) => errors.push(format!("GNOME did not confirm restoration of {key}")),
                    Err(error) => errors.push(error),
                }
            }
            // A user or another tool changed this key after WaniEngine applied it.
            Ok(_) => {
                snapshot.applied.remove(key);
            }
            Err(error) => errors.push(error),
        }
    }
    if snapshot.applied.is_empty() {
        recovery.snapshot = None;
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(unix)]
fn lock_session(path: &Path) -> Option<SessionLock> {
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .ok()?;
    SessionLock::try_lock(file)
}

#[cfg(not(unix))]
fn lock_session(_path: &Path) -> Option<SessionLock> {
    None
}

fn read_recovery(path: &Path) -> Option<Recovery> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_recovery(path: &Path, recovery: &Recovery) -> Result<(), String> {
    waniengine_core::library::write_json(
        path,
        &serde_json::to_value(recovery).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

fn process_identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.get(stat.rfind(')')? + 1..)?;
    after.split_ascii_whitespace().nth(19).map(str::to_owned)
}

fn owner_alive(recovery: &Recovery) -> bool {
    recovery.owner_pid > 0
        && process_identity(recovery.owner_pid)
            .is_some_and(|identity| identity == recovery.owner_identity)
}

#[cfg(test)]
mod tests {
    use super::WallpaperController;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!(
                "wani-gnome-wallpaper-{}-{}",
                std::process::id(),
                NEXT_TEST.fetch_add(1, Ordering::Relaxed)
            )))
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fake_gsettings(directory: &Path) -> PathBuf {
        fs::create_dir_all(directory).unwrap();
        for (key, value) in [
            ("picture-uri", "'file:///original-light.jpg'"),
            ("picture-uri-dark", "'file:///original-dark.jpg'"),
            ("picture-options", "'zoom'"),
        ] {
            fs::write(directory.join(key), value).unwrap();
        }
        let binary = directory.join("gsettings-fake");
        let script = format!(
            "#!/bin/sh\nSTATE_DIR='{}'\nif [ \"$1\" = get ]; then cat \"$STATE_DIR/$3\"; exit 0; fi\nif [ \"$1\" = set ]; then printf '%s' \"$4\" > \"$STATE_DIR/$3\"; exit 0; fi\nexit 2\n",
            directory.display()
        );
        fs::write(&binary, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        }
        binary
    }

    #[test]
    fn applies_and_restores_static_wallpaper_settings() {
        let directory = TestDirectory::new();
        let fake = fake_gsettings(directory.path());
        let state = directory.path().join("state/session.json");
        let image = directory.path().join("wallpaper image.jpg");
        fs::write(&image, b"fixture").unwrap();
        {
            let controller = WallpaperController::with_paths(state, fake);
            controller.apply(&image, "cover", false).unwrap();
            assert_eq!(
                fs::read_to_string(directory.path().join("picture-uri")).unwrap(),
                format!(
                    "'{}'",
                    waniengine_core::library::file_url(&image.canonicalize().unwrap())
                )
            );
            assert_eq!(
                fs::read_to_string(directory.path().join("picture-options")).unwrap(),
                "'zoom'"
            );
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("picture-uri")).unwrap(),
            "'file:///original-light.jpg'"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("picture-uri-dark")).unwrap(),
            "'file:///original-dark.jpg'"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("picture-options")).unwrap(),
            "'zoom'"
        );
    }

    #[test]
    fn restoration_preserves_settings_changed_by_the_user() {
        let directory = TestDirectory::new();
        let fake = fake_gsettings(directory.path());
        let state = directory.path().join("state/session.json");
        let image = directory.path().join("wallpaper.jpg");
        fs::write(&image, b"fixture").unwrap();
        {
            let controller = WallpaperController::with_paths(state, fake);
            controller.apply(&image, "stretch", false).unwrap();
            fs::write(
                directory.path().join("picture-uri"),
                "'file:///changed-by-user.jpg'",
            )
            .unwrap();
        }
        assert_eq!(
            fs::read_to_string(directory.path().join("picture-uri")).unwrap(),
            "'file:///changed-by-user.jpg'"
        );
        assert_eq!(
            fs::read_to_string(directory.path().join("picture-options")).unwrap(),
            "'zoom'"
        );
    }
}
