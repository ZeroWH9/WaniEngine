//! Rust-owned lifecycle for character audio processes.
//!
//! FFplay is used as the media decoder/output backend because FFmpeg is already
//! a runtime dependency for metadata and preview work. This keeps Qt Multimedia
//! out of the new application while preserving the audio formats users already
//! have in their character projects.

use crate::x11_click::{Click, ClickObserver};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
use waniengine_core::library::file_path;
use waniengine_egui::overlay::ActorClickBounds;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Once,
    Loop,
    Interval,
    Click,
}

#[derive(Clone, Debug, PartialEq)]
struct TrackConfig {
    actor_url: String,
    track_url: String,
    track_name: String,
    path: PathBuf,
    mode: Mode,
    volume: u8,
    output: Option<String>,
    interval: Duration,
    start_immediately: bool,
    click_radius: f64,
    click_cooldown: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputDevice {
    pub name: String,
    pub description: String,
}

/// Return PulseAudio-compatible output sinks. PipeWire systems usually expose
/// the same interface through pipewire-pulse, which SDL/FFplay can target.
pub fn discover_outputs() -> Result<Vec<OutputDevice>, String> {
    let output = Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
        .map_err(|error| format!("Could not list audio outputs with pactl: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let sinks: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not read audio outputs: {error}"))?;
    let mut outputs = sinks
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|sink| {
            let name = sink["name"].as_str()?.trim();
            if name.is_empty() || name.ends_with(".monitor") {
                return None;
            }
            Some(OutputDevice {
                name: name.to_owned(),
                description: sink["description"]
                    .as_str()
                    .filter(|description| !description.trim().is_empty())
                    .unwrap_or(name)
                    .to_owned(),
            })
        })
        .collect::<Vec<_>>();
    outputs.sort_by(|left, right| {
        left.description
            .to_lowercase()
            .cmp(&right.description.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(outputs)
}

struct TrackState {
    config: TrackConfig,
    player: Option<Child>,
    next_at: Option<Instant>,
    last_click: Option<Instant>,
    failures: u8,
}

pub struct AudioManager {
    tracks: HashMap<String, TrackState>,
    reported: HashSet<String>,
    test_player: Option<Child>,
    errors_tx: Sender<(String, String)>,
    errors_rx: Receiver<(String, String)>,
    asynchronous_errors: Vec<(String, String)>,
    click_observer: ClickObserver,
}

impl Default for AudioManager {
    fn default() -> Self {
        let (errors_tx, errors_rx) = mpsc::channel();
        Self {
            tracks: HashMap::new(),
            reported: HashSet::new(),
            test_player: None,
            errors_tx,
            errors_rx,
            asynchronous_errors: Vec::new(),
            click_observer: ClickObserver::default(),
        }
    }
}

impl AudioManager {
    /// Synchronize players with the currently active character assets.
    /// Returns new, user-facing errors once; unchanged failures are not repeated
    /// every egui frame.
    pub fn sync(
        &mut self,
        assets: &[Value],
        actor_urls: &[String],
        global_volume: f64,
        output: Option<&str>,
        click_bounds: &[ActorClickBounds],
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let mut desired = HashMap::new();
        let global_volume = finite_percent(global_volume, 100.0);

        for actor_url in actor_urls {
            let Some(actor) = assets
                .iter()
                .find(|asset| asset["url"].as_str() == Some(actor_url.as_str()))
            else {
                continue;
            };
            let audio = &actor["audio"];
            if audio["enabled"].as_bool() != Some(true) {
                continue;
            }
            let actor_volume = finite_percent(actor["volume"].as_f64().unwrap_or(50.0), 50.0);
            for (index, track) in audio["files"].as_array().into_iter().flatten().enumerate() {
                let playback = &track["playback"];
                if playback["enabled"].as_bool().unwrap_or(true) == false {
                    continue;
                }
                let Some(track_url) = track["url"].as_str() else {
                    let key = format!("{actor_url}|track-{index}");
                    Self::report_once(
                        &mut self.reported,
                        &mut errors,
                        &key,
                        "This character has an audio track without a file path.",
                    );
                    continue;
                };
                let path = match file_path(track_url) {
                    Ok(path) if path.is_file() => path,
                    Ok(_) => {
                        let key = format!("{actor_url}|{track_url}");
                        Self::report_once(
                            &mut self.reported,
                            &mut errors,
                            &key,
                            "A character audio file is missing.",
                        );
                        continue;
                    }
                    Err(error) => {
                        let key = format!("{actor_url}|{track_url}");
                        Self::report_once(
                            &mut self.reported,
                            &mut errors,
                            &key,
                            &format!("Could not open a character audio file: {error}"),
                        );
                        continue;
                    }
                };
                let track_volume =
                    finite_percent(playback["volume"].as_f64().unwrap_or(100.0), 100.0);
                let effective_volume = (track_volume * actor_volume * global_volume / 10_000.0)
                    .round()
                    .clamp(0.0, 100.0) as u8;
                let mode = match playback["mode"].as_str().unwrap_or("once") {
                    "loop" => Mode::Loop,
                    "interval" => Mode::Interval,
                    "click" => Mode::Click,
                    _ => Mode::Once,
                };
                let interval_seconds = playback["intervalSeconds"]
                    .as_f64()
                    .unwrap_or(30.0)
                    .clamp(1.0, 86_400.0);
                let config = TrackConfig {
                    actor_url: actor_url.clone(),
                    track_url: track_url.to_owned(),
                    track_name: track["name"]
                        .as_str()
                        .unwrap_or("character audio")
                        .to_owned(),
                    path,
                    mode,
                    volume: effective_volume,
                    output: output.map(str::to_owned),
                    interval: Duration::from_secs_f64(interval_seconds),
                    start_immediately: playback["startImmediately"].as_bool().unwrap_or(true),
                    click_radius: playback["radiusPixels"]
                        .as_f64()
                        .unwrap_or(40.0)
                        .clamp(0.0, 500.0),
                    click_cooldown: Duration::from_secs_f64(
                        playback["cooldownSeconds"]
                            .as_f64()
                            .unwrap_or(2.0)
                            .clamp(0.0, 3600.0),
                    ),
                };
                desired.insert(track_key(actor_url, track_url), config);
            }
        }

        self.receive_process_errors(&mut errors);
        match self.test_player.as_mut().map(Child::try_wait) {
            Some(Ok(Some(status))) => {
                self.test_player = None;
                if !status.success() {
                    Self::report_once(
                        &mut self.reported,
                        &mut errors,
                        "audio-test",
                        "The test audio could not be played.",
                    );
                }
            }
            Some(Err(error)) => {
                stop_player(&mut self.test_player);
                Self::report_once(
                    &mut self.reported,
                    &mut errors,
                    "audio-test",
                    &format!("Could not inspect test audio playback: {error}"),
                );
            }
            Some(Ok(None)) | None => {}
        }

        let obsolete = self
            .tracks
            .iter()
            .filter(|(key, state)| desired.get(*key) != Some(&state.config))
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in obsolete {
            if let Some(mut state) = self.tracks.remove(&key) {
                stop_player(&mut state.player);
            }
            self.reported
                .retain(|entry| !entry.starts_with(&format!("{key}|")));
        }

        for (key, config) in desired {
            if self.tracks.contains_key(&key) {
                continue;
            }
            let now = Instant::now();
            let next_at = match config.mode {
                Mode::Once | Mode::Loop => Some(now),
                Mode::Interval if config.start_immediately => Some(now),
                Mode::Interval => Some(now + config.interval),
                Mode::Click => None,
            };
            self.tracks.insert(
                key,
                TrackState {
                    config,
                    player: None,
                    next_at,
                    last_click: None,
                    failures: 0,
                },
            );
        }

        let click_enabled = self
            .tracks
            .values()
            .any(|state| state.config.mode == Mode::Click);
        if let Err(error) = self.click_observer.set_enabled(click_enabled) {
            Self::report_once(&mut self.reported, &mut errors, "click-observer", &error);
        }
        let (clicks, observer_errors) = self.click_observer.drain();
        for error in observer_errors {
            Self::report_once(&mut self.reported, &mut errors, "click-observer", &error);
        }
        let click_now = Instant::now();
        for click in clicks {
            for state in self.tracks.values_mut() {
                if state.config.mode != Mode::Click || state.player.is_some() {
                    continue;
                }
                let Some(bounds) = click_bounds
                    .iter()
                    .find(|bounds| bounds.actor_url == state.config.actor_url)
                else {
                    continue;
                };
                if state.last_click.is_some_and(|last| {
                    click_now.saturating_duration_since(last) < state.config.click_cooldown
                }) {
                    continue;
                }
                if click_hits_actor(bounds, click, state.config.click_radius) {
                    state.next_at = Some(click_now);
                    state.last_click = Some(click_now);
                }
            }
        }

        let now = Instant::now();
        for (key, state) in &mut self.tracks {
            let mut finished = None;
            let mut player_uninspectable = false;
            if let Some(player) = state.player.as_mut() {
                match player.try_wait() {
                    Ok(Some(status)) => finished = Some(status.success()),
                    Ok(None) => {}
                    Err(error) => {
                        finished = Some(false);
                        player_uninspectable = true;
                        Self::report_once(
                            &mut self.reported,
                            &mut errors,
                            key,
                            &format!("Could not inspect character audio playback: {error}"),
                        );
                    }
                }
            }
            if let Some(success) = finished {
                if player_uninspectable {
                    stop_player(&mut state.player);
                } else {
                    state.player = None;
                }
                if success {
                    state.failures = 0;
                    if state.config.mode == Mode::Once {
                        state.next_at = None;
                    } else if state.config.mode == Mode::Loop {
                        state.next_at = Some(now + Duration::from_millis(120));
                    } else if state.config.mode == Mode::Interval {
                        let mut due = state.next_at.unwrap_or(now);
                        while due <= now {
                            due += state.config.interval;
                        }
                        state.next_at = Some(due);
                    }
                } else {
                    state.failures = state.failures.saturating_add(1);
                    Self::report_once(
                        &mut self.reported,
                        &mut errors,
                        key,
                        &format!(
                            "Could not play ‘{}’. Check that FFplay supports this audio file.",
                            state.config.track_name
                        ),
                    );
                    if state.config.mode == Mode::Loop && state.failures < 3 {
                        state.next_at = Some(now + Duration::from_secs(2));
                    } else if state.config.mode == Mode::Interval {
                        state.next_at = Some(now + state.config.interval);
                    } else {
                        state.next_at = None;
                    }
                }
            }

            if state.config.mode == Mode::Interval
                && state.player.is_some()
                && state.next_at.is_some_and(|due| due <= now)
            {
                if let Some(mut due) = state.next_at {
                    while due <= now {
                        due += state.config.interval;
                    }
                    state.next_at = Some(due);
                }
            }

            if state.player.is_none() && state.next_at.is_some_and(|due| due <= now) {
                match spawn_player(&state.config, &self.errors_tx, key.clone()) {
                    Ok(player) => {
                        state.player = Some(player);
                        if state.config.mode == Mode::Interval {
                            state.next_at = Some(now + state.config.interval);
                        } else if state.config.mode == Mode::Once {
                            state.next_at = None;
                        }
                    }
                    Err(error) => {
                        state.failures = state.failures.saturating_add(1);
                        Self::report_once(&mut self.reported, &mut errors, key, &error);
                        state.next_at = if state.config.mode == Mode::Loop && state.failures < 3 {
                            Some(now + Duration::from_secs(2))
                        } else if state.config.mode == Mode::Interval {
                            Some(now + state.config.interval)
                        } else {
                            None
                        };
                    }
                }
            }
        }
        errors
    }

    fn receive_process_errors(&mut self, errors: &mut Vec<String>) {
        while let Ok((key, detail)) = self.errors_rx.try_recv() {
            self.asynchronous_errors.push((key, detail));
        }
        for (key, detail) in self.asynchronous_errors.drain(..) {
            Self::report_once(
                &mut self.reported,
                errors,
                &key,
                &format!("Character audio player: {detail}"),
            );
        }
    }

    pub fn test_track(
        &mut self,
        actor: &Value,
        track_index: usize,
        global_volume: f64,
        output: Option<&str>,
    ) -> Result<(), String> {
        if actor["audio"]["enabled"].as_bool() != Some(true) {
            return Err("Enable character sounds before testing a track.".into());
        }
        let track = actor["audio"]["files"]
            .as_array()
            .and_then(|tracks| tracks.get(track_index))
            .ok_or("Select a character audio track first")?;
        let playback = &track["playback"];
        if playback["enabled"].as_bool().unwrap_or(true) == false {
            return Err("Enable this audio track before testing it.".into());
        }
        let track_url = track["url"]
            .as_str()
            .ok_or("Invalid character audio path")?;
        let path = file_path(track_url).map_err(|error| error.to_string())?;
        if !path.is_file() {
            return Err("The selected character audio file is missing.".into());
        }
        let track_volume = finite_percent(playback["volume"].as_f64().unwrap_or(100.0), 100.0);
        let actor_volume = finite_percent(actor["volume"].as_f64().unwrap_or(50.0), 50.0);
        let global_volume = finite_percent(global_volume, 100.0);
        let config = TrackConfig {
            actor_url: actor["url"].as_str().unwrap_or_default().to_owned(),
            track_url: track_url.to_owned(),
            track_name: track["name"]
                .as_str()
                .unwrap_or("character audio")
                .to_owned(),
            path,
            mode: Mode::Once,
            volume: (track_volume * actor_volume * global_volume / 10_000.0)
                .round()
                .clamp(0.0, 100.0) as u8,
            output: output.map(str::to_owned),
            interval: Duration::from_secs(1),
            start_immediately: true,
            click_radius: 40.0,
            click_cooldown: Duration::from_secs(2),
        };
        stop_player(&mut self.test_player);
        self.test_player = Some(spawn_player(
            &config,
            &self.errors_tx,
            format!("audio-test|{}", track_url),
        )?);
        Ok(())
    }

    fn report_once(
        reported: &mut HashSet<String>,
        errors: &mut Vec<String>,
        key: &str,
        detail: &str,
    ) {
        let report_key = format!("{key}|{detail}");
        if reported.insert(report_key) {
            errors.push(detail.to_owned());
        }
    }
}

fn click_hits_actor(bounds: &ActorClickBounds, click: Click, radius: f64) -> bool {
    let dx = f64::from(click.x) - bounds.center_x;
    let dy = f64::from(click.y) - bounds.center_y;
    let angle = bounds.rotation.to_radians();
    let (sin, cos) = angle.sin_cos();
    // Inverse-rotate the pointer so the hit test follows the actor transform.
    let local_x = cos * dx + sin * dy;
    let local_y = -sin * dx + cos * dy;
    let dx = (local_x.abs() - bounds.half_width).max(0.0);
    let dy = (local_y.abs() - bounds.half_height).max(0.0);
    dx.mul_add(dx, dy * dy) <= radius * radius
}

impl Drop for AudioManager {
    fn drop(&mut self) {
        stop_player(&mut self.test_player);
        for state in self.tracks.values_mut() {
            stop_player(&mut state.player);
        }
    }
}

fn finite_percent(value: f64, default: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        default
    }
}

fn track_key(actor_url: &str, track_url: &str) -> String {
    format!("{actor_url}|{track_url}")
}

fn spawn_player(
    config: &TrackConfig,
    errors: &Sender<(String, String)>,
    key: String,
) -> Result<Child, String> {
    let mut command = Command::new("ffplay");
    command.args([
        "-nodisp",
        "-autoexit",
        "-hide_banner",
        "-nostats",
        "-loglevel",
        "error",
        "-volume",
        &config.volume.to_string(),
    ]);
    if let Some(output) = &config.output {
        command.env("SDL_AUDIODRIVER", "pulseaudio");
        command.env("PULSE_SINK", output);
    }
    waniengine_core::process::die_with_parent(&mut command);
    let mut child = command
        .arg(&config.path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start FFplay for character audio: {error}"))?;
    if let Some(stderr) = child.stderr.take() {
        let sender = errors.clone();
        thread::spawn(move || {
            use std::io::Read;
            let mut stderr = stderr;
            let mut message = String::new();
            let mut buffer = [0_u8; 512];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) if message.len() < 4096 => {
                        let remaining = 4096 - message.len();
                        message.push_str(&String::from_utf8_lossy(&buffer[..count.min(remaining)]));
                    }
                    Ok(_) => {}
                }
            }
            let message = message.trim().to_owned();
            if !message.is_empty() {
                let _ = sender.send((key, message));
            }
        });
    }
    Ok(child)
}

fn stop_player(player: &mut Option<Child>) {
    if let Some(mut child) = player.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}
