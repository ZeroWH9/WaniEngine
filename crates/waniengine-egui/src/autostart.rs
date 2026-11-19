//! Per-user session autostart using the XDG Desktop Entry format.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const FILE_NAME: &str = "org.waniengine.WaniEngine.desktop";
const MARKER: &str = "X-WaniEngine-Managed=true";

fn autostart_path() -> Result<PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or("HOME and XDG_CONFIG_HOME are not set")?;
    Ok(config.join("autostart").join(FILE_NAME))
}

pub fn enabled() -> bool {
    autostart_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .is_some_and(|entry| entry.lines().any(|line| line.trim() == MARKER))
}

pub fn set_enabled(enabled: bool, executable: &Path) -> Result<(), String> {
    let path = autostart_path()?;
    if !enabled {
        return remove_entry(&path);
    }
    let executable = launch_executable(executable)?;
    write_entry(&path, &desktop_entry(&executable))
}

fn launch_executable(executable: &Path) -> Result<PathBuf, String> {
    if !executable.is_absolute() || !executable.is_file() {
        return Err("WaniEngine needs an existing absolute executable path.".into());
    }
    let launcher = std::env::var_os("WANIENGINE_LAUNCHER")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .or_else(|| {
            let installed = executable
                .ancestors()
                .any(|path| path.join(".waniengine-installation").is_file());
            if !installed {
                return None;
            }
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".local/bin/waniengine"))
                .filter(|path| path.is_absolute() && path.is_file())
        });
    Ok(launcher.unwrap_or_else(|| executable.to_owned()))
}

fn desktop_entry(executable: &Path) -> String {
    let escaped = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    format!(
        "[Desktop Entry]\nType=Application\nName=WaniEngine\nComment=Restore saved characters\nExec=\"{escaped}\" --resume-session --background\nIcon=waniengine\nTerminal=false\nCategories=Graphics;\nX-WaniEngine-Managed=true\nX-KDE-autostart-phase=2\nX-KDE-Wayland-Interfaces=org_kde_plasma_window_management\n"
    )
}

fn write_entry(path: &Path, contents: &str) -> Result<(), String> {
    if path.exists() {
        let existing = fs::read_to_string(path).map_err(|error| error.to_string())?;
        if !existing.lines().any(|line| line.trim() == MARKER) {
            return Err(format!(
                "Refusing to replace an unmanaged autostart file: {}",
                path.display()
            ));
        }
    }
    let parent = path
        .parent()
        .ok_or_else(|| "Autostart path has no parent directory".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{FILE_NAME}.{}.{}.tmp", std::process::id(), nonce));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(contents.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644))
                .map_err(|error| error.to_string())?;
        }
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn remove_entry(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let existing = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if !existing.lines().any(|line| line.trim() == MARKER) {
        return Err(format!(
            "Refusing to remove an unmanaged autostart file: {}",
            path.display()
        ));
    }
    fs::remove_file(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_quotes_exec_and_requests_session_restore() {
        let entry = desktop_entry(Path::new("/tmp/Wani $Engine%/waniengine"));
        assert!(entry.contains(r#"Exec="/tmp/Wani \$Engine%%/waniengine" --resume-session"#));
        assert!(entry.lines().any(|line| line == MARKER));
    }

    #[test]
    fn refuses_to_replace_or_remove_an_unmanaged_entry() {
        let directory = std::env::temp_dir().join(format!(
            "waniengine-autostart-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(FILE_NAME);
        fs::write(&path, "[Desktop Entry]\nName=Other\n").unwrap();
        assert!(write_entry(&path, &desktop_entry(Path::new("/tmp/waniengine"))).is_err());
        assert!(remove_entry(&path).is_err());
        assert!(path.is_file());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn installs_and_removes_its_managed_entry_atomically() {
        let directory = std::env::temp_dir().join(format!(
            "waniengine-autostart-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join(FILE_NAME);
        let entry = desktop_entry(Path::new("/tmp/waniengine"));
        write_entry(&path, &entry).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), entry);
        write_entry(&path, &desktop_entry(Path::new("/tmp/new-waniengine"))).unwrap();
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("/tmp/new-waniengine")
        );
        remove_entry(&path).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
    }
}
