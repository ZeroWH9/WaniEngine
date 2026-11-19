//! Bounded, line-delimited control messages from the owning manager.
use crate::ActorOptions;
use serde::Deserialize;
use std::{
    io::{self, BufRead, Read},
    time::{Duration, Instant},
};

pub const MAX_MESSAGE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Update {
        id: u64,
        x: Option<f32>,
        y: Option<f32>,
        scale: Option<f32>,
        opacity: Option<f32>,
        rotation: Option<f32>,
        #[serde(rename = "flipX")]
        flip_x: Option<bool>,
        smoothing: Option<bool>,
        speed: Option<f32>,
        playing: Option<bool>,
        #[serde(rename = "loop")]
        looping: Option<bool>,
    },
    VideoSettings {
        id: u64,
        fps: u32,
    },
    Stop {
        id: u64,
    },
    Present {
        id: u64,
    },
}

impl Command {
    pub fn update(&self, previous: ActorOptions) -> Result<Option<(u64, ActorOptions)>, String> {
        let Self::Update {
            id,
            x,
            y,
            scale,
            opacity,
            rotation,
            flip_x,
            smoothing,
            speed,
            playing,
            looping,
        } = self
        else {
            return Ok(None);
        };
        let mut result = previous;
        for (value, name, min, max, target) in [
            (*x, "x", 0.0, 1.0, &mut result.x),
            (*y, "y", 0.0, 1.0, &mut result.y),
            (*scale, "scale", 10.0, 300.0, &mut result.scale),
            (*opacity, "opacity", 0.0, 100.0, &mut result.opacity),
            (*rotation, "rotation", -180.0, 180.0, &mut result.rotation),
            (*speed, "speed", 10.0, 300.0, &mut result.speed),
        ] {
            if let Some(value) = value {
                if !value.is_finite() || value < min || value > max {
                    return Err(format!("{name} must be between {min} and {max}"));
                }
                *target = value;
            }
        }
        if let Some(value) = flip_x {
            result.flip_x = *value;
        }
        if let Some(value) = smoothing {
            result.smoothing = *value;
        }
        if let Some(value) = playing {
            result.playing = *value;
        }
        if let Some(value) = looping {
            result.looping = *value;
        }
        Ok(Some((*id, result)))
    }
    pub fn id(&self) -> u64 {
        match self {
            Self::Update { id, .. }
            | Self::Stop { id }
            | Self::Present { id }
            | Self::VideoSettings { id, .. } => *id,
        }
    }
}

/// Bound reads even when a peer never sends a newline.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_MESSAGE_BYTES + 1)
        .read_until(b'\n', &mut bytes)?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Control message exceeds 64 KiB",
        ));
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Incomplete control message",
        ));
    }
    Ok(Some(bytes))
}

#[derive(Debug)]
pub struct PlaybackClock {
    elapsed: Duration,
    changed_at: Instant,
    speed: f32,
    playing: bool,
}
impl PlaybackClock {
    pub fn new(actor: ActorOptions, now: Instant) -> Self {
        Self {
            elapsed: Duration::ZERO,
            changed_at: now,
            speed: actor.speed,
            playing: actor.playing,
        }
    }
    pub fn position(&self, now: Instant) -> Duration {
        if !self.playing {
            return self.elapsed;
        }
        let advance = now
            .saturating_duration_since(self.changed_at)
            .mul_f64(f64::from(self.speed) / 100.0);
        self.elapsed.saturating_add(advance)
    }
    pub fn update(&mut self, actor: ActorOptions, now: Instant) {
        self.elapsed = self.position(now);
        self.changed_at = now;
        self.speed = actor.speed;
        self.playing = actor.playing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patches_are_atomic_and_validated() {
        let old = ActorOptions::default();
        let command: Command = serde_json::from_str(
            r#"{"action":"update","id":7,"x":0.8,"opacity":20,"playing":false}"#,
        )
        .unwrap();
        let (_, changed) = command.update(old).unwrap().unwrap();
        assert_eq!(changed.x, 0.8);
        assert_eq!(changed.opacity, 20.0);
        assert!(!changed.playing);
        let bad: Command =
            serde_json::from_str(r#"{"action":"update","id":8,"x":2,"opacity":20}"#).unwrap();
        assert!(bad.update(old).is_err());
        assert_eq!(old.opacity, 100.0);
        assert!(
            serde_json::from_str::<Command>(r#"{"action":"update","id":1,"unknown":true}"#)
                .is_err()
        );
    }
    #[test]
    fn clock_preserves_position_across_pause_and_speed_changes() {
        let start = Instant::now();
        let actor = ActorOptions::default();
        let mut clock = PlaybackClock::new(actor, start);
        assert_eq!(
            clock.position(start + Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        let paused = ActorOptions {
            playing: false,
            ..actor
        };
        clock.update(paused, start + Duration::from_secs(1));
        assert_eq!(
            clock.position(start + Duration::from_secs(5)),
            Duration::from_secs(1)
        );
        let resumed = ActorOptions {
            playing: true,
            speed: 200.0,
            ..paused
        };
        clock.update(resumed, start + Duration::from_secs(5));
        assert_eq!(
            clock.position(start + Duration::from_secs(6)),
            Duration::from_secs(3)
        );
    }
    #[test]
    fn input_is_bounded_and_requires_newline() {
        assert_eq!(read_message(&mut &b"{}\n"[..]).unwrap().unwrap(), b"{}\n");
        assert!(read_message(&mut &b"{}"[..]).is_err());
        assert!(read_message(&mut &vec![b'a'; MAX_MESSAGE_BYTES as usize + 1][..]).is_err());
        assert!(read_message(&mut &b""[..]).unwrap().is_none());
    }
}
