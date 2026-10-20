use crate::{Profile, ProfileError};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ProfileStore {
    path: PathBuf,
}

impl ProfileStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Profile, StoreError> {
        if !self.path.exists() {
            return Ok(Profile::default());
        }

        let bytes = fs::read(&self.path)?;
        let profile: Profile = serde_json::from_slice(&bytes)?;
        profile.validate()?;
        Ok(profile)
    }

    /// Writes a complete new snapshot and replaces the previous profile atomically.
    pub fn save(&self, profile: &Profile) -> Result<(), StoreError> {
        profile.validate()?;
        let parent = self.path.parent().ok_or(StoreError::PathHasNoParent)?;
        fs::create_dir_all(parent)?;

        let file_name = self
            .path
            .file_name()
            .ok_or(StoreError::PathHasNoFileName)?
            .to_string_lossy();
        let temporary_path = parent.join(format!("{file_name}.{}.tmp", std::process::id()));
        let bytes = serde_json::to_vec_pretty(profile)?;

        let write_result = (|| -> Result<(), StoreError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);

            fs::rename(&temporary_path, &self.path)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Profile(ProfileError),
    PathHasNoParent,
    PathHasNoFileName,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "erro de arquivo: {error}"),
            Self::Json(error) => write!(f, "invalid profile JSON: {error}"),
            Self::Profile(error) => write!(f, "invalid profile: {error}"),
            Self::PathHasNoParent => write!(f, "profile path must have a parent directory"),
            Self::PathHasNoFileName => write!(f, "profile path must include a file name"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Profile(error) => Some(error),
            Self::PathHasNoParent | Self::PathHasNoFileName => None,
        }
    }
}

impl From<std::io::Error> for StoreError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<ProfileError> for StoreError {
    fn from(error: ProfileError) -> Self {
        Self::Profile(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActorInstance, AnimationState};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempProfile(PathBuf);

    impl TempProfile {
        fn new() -> Self {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let directory =
                std::env::temp_dir().join(format!("waniengine-{}-{id}", std::process::id()));
            Self(directory.join("state/profile.json"))
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent().and_then(Path::parent) {
                let _ = fs::remove_dir_all(parent);
            }
        }
    }

    #[test]
    fn a_missing_profile_starts_from_safe_defaults() {
        let path = TempProfile::new();
        let profile = ProfileStore::new(&path.0)
            .load()
            .expect("load default profile");

        assert_eq!(profile, Profile::default());
        assert!(!path.0.exists());
    }

    #[test]
    fn saves_and_loads_the_entire_profile() {
        let path = TempProfile::new();
        let store = ProfileStore::new(&path.0);
        let mut profile = Profile {
            title: "Noite de dança".to_owned(),
            ..Profile::default()
        };
        profile.actors.push(ActorInstance {
            instance_id: "actor-miku".to_owned(),
            asset_id: "local.miku-dance".to_owned(),
            x: 0.78,
            y: 0.91,
            scale: 0.85,
            state: AnimationState::Dance,
        });

        store.save(&profile).expect("save profile");
        assert_eq!(store.load().expect("reload profile"), profile);
        assert!(path.0.is_file());
    }

    #[test]
    fn refuses_to_write_invalid_data() {
        let path = TempProfile::new();
        let store = ProfileStore::new(&path.0);
        let mut profile = Profile::default();
        profile.title.clear();

        assert!(matches!(
            store.save(&profile),
            Err(StoreError::Profile(ProfileError::InvalidTitle))
        ));
        assert!(!path.0.exists());
    }
}
