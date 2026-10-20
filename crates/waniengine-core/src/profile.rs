use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

pub const MAX_ACTORS: usize = 64;
pub const MAX_ID_BYTES: usize = 128;
pub const MAX_TITLE_CHARS: usize = 120;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub schema_version: u32,
    pub revision: u64,
    pub title: String,
    pub paused: bool,
    pub wallpaper: Option<PathBuf>,
    pub actors: Vec<ActorInstance>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            title: "My desktop".to_owned(),
            paused: false,
            wallpaper: None,
            actors: Vec::new(),
        }
    }
}

impl Profile {
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema_version != 1 {
            return Err(ProfileError::UnsupportedSchema(self.schema_version));
        }
        let title_len = self.title.chars().count();
        if title_len == 0 || title_len > MAX_TITLE_CHARS {
            return Err(ProfileError::InvalidTitle);
        }
        if self.actors.len() > MAX_ACTORS {
            return Err(ProfileError::TooManyActors(self.actors.len()));
        }
        if let Some(path) = &self.wallpaper {
            let path_text = path.to_string_lossy();
            if !path.is_absolute() || path_text.contains('\0') {
                return Err(ProfileError::InvalidWallpaperPath);
            }
        }

        let mut instance_ids = HashSet::with_capacity(self.actors.len());
        for actor in &self.actors {
            validate_id("instance_id", &actor.instance_id)?;
            validate_id("asset_id", &actor.asset_id)?;
            if !instance_ids.insert(actor.instance_id.as_str()) {
                return Err(ProfileError::DuplicateInstanceId(actor.instance_id.clone()));
            }
            if !actor.x.is_finite()
                || !actor.y.is_finite()
                || !(0.0..=1.0).contains(&actor.x)
                || !(0.0..=1.0).contains(&actor.y)
            {
                return Err(ProfileError::InvalidPosition(actor.instance_id.clone()));
            }
            if !actor.scale.is_finite() || !(0.1..=4.0).contains(&actor.scale) {
                return Err(ProfileError::InvalidScale(actor.instance_id.clone()));
            }
        }
        Ok(())
    }

    pub fn bump_revision(&mut self) -> Result<(), ProfileError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(ProfileError::RevisionExhausted)?;
        Ok(())
    }
}

fn validate_id(field: &'static str, value: &str) -> Result<(), ProfileError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ProfileError::InvalidId(field))
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActorInstance {
    pub instance_id: String,
    pub asset_id: String,
    /// Horizontal position, normalized to the output (0 is left, 1 is right).
    pub x: f64,
    /// Vertical position, normalized to the output (0 is top, 1 is bottom).
    pub y: f64,
    pub scale: f64,
    pub state: AnimationState,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationState {
    Idle,
    Dance,
    Walk,
    Sleep,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProfileError {
    UnsupportedSchema(u32),
    InvalidTitle,
    TooManyActors(usize),
    InvalidWallpaperPath,
    InvalidId(&'static str),
    DuplicateInstanceId(String),
    InvalidPosition(String),
    InvalidScale(String),
    RevisionExhausted,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported profile version: {version}")
            }
            Self::InvalidTitle => write!(
                f,
                "title must contain between 1 and {MAX_TITLE_CHARS} characters"
            ),
            Self::TooManyActors(count) => write!(
                f,
                "profile contains {count} characters; the limit is {MAX_ACTORS}"
            ),
            Self::InvalidWallpaperPath => {
                write!(f, "wallpaper must use a valid absolute path")
            }
            Self::InvalidId(field) => {
                write!(
                    f,
                    "{field} may contain only letters, numbers, periods, hyphens, and underscores"
                )
            }
            Self::DuplicateInstanceId(id) => {
                write!(f, "profile contains more than one character with id {id}")
            }
            Self::InvalidPosition(id) => {
                write!(f, "character {id} position must be between 0 and 1")
            }
            Self::InvalidScale(id) => {
                write!(f, "character {id} scale must be between 0.1 and 4")
            }
            Self::RevisionExhausted => write!(f, "profile revision limit reached"),
        }
    }
}

impl std::error::Error for ProfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> ActorInstance {
        ActorInstance {
            instance_id: "actor-1".to_owned(),
            asset_id: "local.chibi-dance".to_owned(),
            x: 0.9,
            y: 0.8,
            scale: 1.0,
            state: AnimationState::Dance,
        }
    }

    #[test]
    fn accepts_normalized_actor_placement() {
        let mut profile = Profile::default();
        profile.actors.push(actor());
        assert_eq!(profile.validate(), Ok(()));
    }

    #[test]
    fn rejects_out_of_range_positions_and_non_finite_values() {
        let mut profile = Profile::default();
        let mut misplaced = actor();
        misplaced.x = 1.01;
        profile.actors.push(misplaced);
        assert!(matches!(
            profile.validate(),
            Err(ProfileError::InvalidPosition(_))
        ));

        profile.actors[0].x = f64::NAN;
        assert!(matches!(
            profile.validate(),
            Err(ProfileError::InvalidPosition(_))
        ));
    }

    #[test]
    fn rejects_invalid_ids_and_oversized_actor_lists() {
        let mut profile = Profile::default();
        let mut invalid = actor();
        invalid.asset_id = "../../script.exe".to_owned();
        profile.actors.push(invalid);
        assert_eq!(profile.validate(), Err(ProfileError::InvalidId("asset_id")));

        profile.actors = (0..=MAX_ACTORS)
            .map(|index| {
                let mut item = actor();
                item.instance_id = format!("actor-{index}");
                item
            })
            .collect();
        assert!(matches!(
            profile.validate(),
            Err(ProfileError::TooManyActors(_))
        ));
    }

    #[test]
    fn rejects_a_wallpaper_path_that_is_not_absolute() {
        let profile = Profile {
            wallpaper: Some(PathBuf::from("wallpapers/night.mp4")),
            ..Profile::default()
        };
        assert_eq!(profile.validate(), Err(ProfileError::InvalidWallpaperPath));
    }

    #[test]
    fn rejects_duplicate_actor_instance_ids() {
        let profile = Profile {
            actors: vec![actor(), actor()],
            ..Profile::default()
        };
        assert_eq!(
            profile.validate(),
            Err(ProfileError::DuplicateInstanceId("actor-1".to_owned()))
        );
    }
}
