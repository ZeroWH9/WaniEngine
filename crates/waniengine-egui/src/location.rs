//! Match the existing manager's library location and legacy scene fallback.
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};
use waniengine_core::library::{LibraryStore, library_directory};

#[derive(Clone, Debug)]
pub struct LibraryLocation {
    pub root: PathBuf,
    pub legacy: PathBuf,
    pub installed_scene: PathBuf,
    pub migrate: bool,
    pub start_in_tray: bool,
}

impl LibraryLocation {
    pub fn from_environment(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let state = nonempty(std::env::var_os("XDG_STATE_HOME"))
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .ok_or("HOME and XDG_STATE_HOME are not set")?;
        Self::resolve(
            state,
            library_directory(),
            std::env::var_os("WANIENGINE_DATA_ROOT"),
            std::env::var_os("WANIENGINE_INSTALLED_SCENE"),
            std::env::var_os("WANIENGINE_SKIP_MIGRATION").is_some(),
            args,
        )
    }

    fn resolve(
        state: PathBuf,
        default_root: PathBuf,
        data_root: Option<OsString>,
        installed: Option<OsString>,
        skip_migration: bool,
        args: impl IntoIterator<Item = OsString>,
    ) -> Result<Self, String> {
        let legacy = state.join("WaniEngine/waniengine/library");
        let mut root = nonempty(data_root)
            .map(PathBuf::from)
            .unwrap_or(default_root);
        let mut start_in_tray = false;
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == OsStr::new("--data-dir") {
                let path = args
                    .next()
                    .filter(|value| !value.is_empty() && !value.to_string_lossy().starts_with("--"))
                    .ok_or("--data-dir requires a directory")?;
                root = PathBuf::from(path);
            } else if arg == OsStr::new("--resume-session") {
                // Kept as the XDG autostart entry's explicit session-start marker.
            } else if arg == OsStr::new("--resume-wallpaper") {
                // Restoration is unconditional now; the legacy flag is only accepted.
            } else if arg == OsStr::new("--background") {
                start_in_tray = true;
            } else {
                return Err(format!("Unknown option: {}", arg.to_string_lossy()));
            }
        }
        let installed_scene = nonempty(installed)
            .map(PathBuf::from)
            .unwrap_or_else(|| legacy.join("scene.json"));
        let migrate = !skip_migration && root != legacy;
        Ok(Self {
            root,
            legacy,
            installed_scene,
            migrate,
            start_in_tray,
        })
    }

    pub fn store(&self) -> LibraryStore {
        LibraryStore::new(&self.root, &self.installed_scene)
    }
}

fn nonempty(value: Option<OsString>) -> Option<OsString> {
    value.filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_line_overrides_environment_and_uses_legacy_scene() {
        let location = LibraryLocation::resolve(
            "/state".into(),
            "/default".into(),
            Some("/env".into()),
            None,
            false,
            ["--data-dir".into(), "/explicit".into()],
        )
        .unwrap();
        assert_eq!(location.root, PathBuf::from("/explicit"));
        assert_eq!(
            location.legacy,
            PathBuf::from("/state/WaniEngine/waniengine/library")
        );
        assert_eq!(location.installed_scene, location.legacy.join("scene.json"));
        assert!(location.migrate);
    }
    #[test]
    fn respects_overrides_and_skips_self_migration() {
        let legacy = PathBuf::from("/state/WaniEngine/waniengine/library");
        let location = LibraryLocation::resolve(
            "/state".into(),
            legacy.clone(),
            None,
            Some("/installed.json".into()),
            false,
            [],
        )
        .unwrap();
        assert_eq!(location.installed_scene, PathBuf::from("/installed.json"));
        assert!(!location.migrate);
        let skipped =
            LibraryLocation::resolve("/state".into(), "/default".into(), None, None, true, [])
                .unwrap();
        assert!(!skipped.migrate);
        assert!(
            LibraryLocation::resolve(
                "/state".into(),
                "/default".into(),
                None,
                None,
                false,
                ["--data-dir".into()]
            )
            .is_err()
        );
        assert!(
            LibraryLocation::resolve(
                "/state".into(),
                "/default".into(),
                None,
                None,
                false,
                ["--bogus".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_session_restore_arguments_for_xdg_autostart() {
        let location = LibraryLocation::resolve(
            "/state".into(),
            "/default".into(),
            None,
            None,
            false,
            ["--resume-session".into()],
        )
        .unwrap();
        assert!(!location.start_in_tray);
        let legacy = LibraryLocation::resolve(
            "/state".into(),
            "/default".into(),
            None,
            None,
            false,
            ["--resume-wallpaper".into()],
        )
        .unwrap();
        assert!(!legacy.start_in_tray);
    }
}
