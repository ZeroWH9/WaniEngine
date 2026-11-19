//! Manages isolated Rust overlay workers. Workers terminate when their control
//! pipe closes, and all process I/O is handled outside the UI thread.
use eframe::egui;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

const MAX_ACTORS: usize = 16;
const MAX_OUTPUT_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Monitor {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "logicalPosition")]
    pub logical_position: Option<(i32, i32)>,
    #[serde(rename = "logicalSize")]
    pub logical_size: Option<(i32, i32)>,
    pub scale: i32,
}

#[derive(Clone, Debug)]
pub struct ActorClickBounds {
    pub actor_url: String,
    pub center_x: f64,
    pub center_y: f64,
    pub half_width: f64,
    pub half_height: f64,
    pub rotation: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ActorSettings {
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub opacity: f32,
    pub rotation: f32,
    #[serde(rename = "flipX")]
    pub flip_x: bool,
    pub smoothing: bool,
    pub speed: f32,
    pub playing: bool,
    #[serde(rename = "loop")]
    pub looping: bool,
}
impl Default for ActorSettings {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.96,
            scale: 100.0,
            opacity: 100.0,
            rotation: 0.0,
            flip_x: false,
            smoothing: true,
            speed: 100.0,
            playing: true,
            looping: true,
        }
    }
}

impl ActorSettings {
    pub fn validate(self) -> Result<(), &'static str> {
        for (value, min, max) in [
            (self.x, 0.0, 1.0),
            (self.y, 0.0, 1.0),
            (self.scale, 10.0, 300.0),
            (self.opacity, 0.0, 100.0),
            (self.rotation, -180.0, 180.0),
            (self.speed, 10.0, 300.0),
        ] {
            if !value.is_finite() || value < min || value > max {
                return Err("Invalid character settings");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerStatus {
    Starting,
    Running,
    Stopping,
}

pub struct Worker {
    pub status: WorkerStatus,
    pub settings: ActorSettings,
    pub confirmed: ActorSettings,
    pub monitor: String,
    pub pid: u32,
    token: u64,
    next_id: u64,
    pending: HashMap<u64, ActorSettings>,
    commands: Sender<Vec<u8>>,
}

enum Event {
    Monitors(Result<Vec<Monitor>, String>),
    Message {
        key: String,
        token: u64,
        message: Value,
    },
    Exited {
        key: String,
        token: u64,
        success: bool,
        detail: String,
    },
    WriteFailed {
        key: String,
        token: u64,
        detail: String,
    },
}

pub struct OverlayManager {
    executable: PathBuf,
    ctx: egui::Context,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    workers: HashMap<String, Worker>,
    next_token: u64,
    pub monitors: Vec<Monitor>,
    pub loading_monitors: bool,
    pub notice: Option<String>,
    exited: Vec<String>,
}

impl OverlayManager {
    pub fn new(executable: PathBuf, ctx: egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            executable,
            ctx,
            sender,
            receiver,
            workers: HashMap::new(),
            next_token: 0,
            monitors: Vec::new(),
            loading_monitors: false,
            notice: None,
            exited: Vec::new(),
        }
    }

    pub fn refresh_monitors(&mut self) {
        if self.loading_monitors {
            return;
        }
        self.loading_monitors = true;
        let executable = self.executable.clone();
        let sender = self.sender.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let result = list_monitors(&executable);
            let _ = sender.send(Event::Monitors(result));
            ctx.request_repaint();
        });
    }

    pub fn worker(&self, key: &str) -> Option<&Worker> {
        self.workers.get(key)
    }
    pub fn active_count(&self) -> usize {
        self.workers.len()
    }

    /// Report the visible actor rectangles in X11 root-window coordinates.
    /// The coordinates match the placement used by the transparent overlay.
    pub fn click_bounds(&self, assets: &[Value]) -> Vec<ActorClickBounds> {
        self.workers
            .iter()
            .filter_map(|(url, worker)| {
                if worker.status != WorkerStatus::Running {
                    return None;
                }
                let monitor = self
                    .monitors
                    .iter()
                    .find(|monitor| monitor.name.as_deref() == Some(worker.monitor.as_str()))?;
                let (position, size) = (monitor.logical_position?, monitor.logical_size?);
                let asset = assets
                    .iter()
                    .find(|asset| asset["url"].as_str() == Some(url.as_str()))?;
                let source_width = asset["width"].as_f64()?;
                let source_height = asset["height"].as_f64()?;
                if !source_width.is_finite()
                    || !source_height.is_finite()
                    || source_width <= 0.0
                    || source_height <= 0.0
                    || size.0 <= 0
                    || size.1 <= 0
                {
                    return None;
                }
                let width = (source_width * f64::from(worker.settings.scale) / 100.0)
                    .min(f64::from(size.0));
                let height = width * source_height / source_width;
                Some(ActorClickBounds {
                    actor_url: url.clone(),
                    center_x: f64::from(position.0)
                        + f64::from(size.0) * f64::from(worker.settings.x),
                    center_y: f64::from(position.1)
                        + f64::from(size.1) * f64::from(worker.settings.y)
                        - height / 2.0,
                    half_width: width / 2.0,
                    half_height: height / 2.0,
                    rotation: f64::from(worker.settings.rotation),
                })
            })
            .collect()
    }

    pub fn take_exited(&mut self) -> Vec<String> {
        std::mem::take(&mut self.exited)
    }

    pub fn start(
        &mut self,
        key: &str,
        image: &Path,
        monitor: &str,
        settings: ActorSettings,
    ) -> Result<(), String> {
        settings.validate().map_err(str::to_owned)?;
        if self.workers.contains_key(key) {
            return Err("Character is already active".into());
        }
        if self.workers.len() >= MAX_ACTORS {
            return Err("At most 16 characters can be active".into());
        }
        if !image.is_file() {
            return Err("Character file is unavailable".into());
        }
        if monitor.is_empty()
            || !self
                .monitors
                .iter()
                .any(|item| item.name.as_deref() == Some(monitor))
        {
            return Err("Select a connected display".into());
        }
        let output_geometry = self
            .monitors
            .iter()
            .find(|item| item.name.as_deref() == Some(monitor))
            .and_then(|item| Some((item.logical_position?, item.logical_size?)));
        self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or("Worker identifier exhausted")?;
        let token = self.next_token;
        let mut command = Command::new(&self.executable);
        command
            .arg("--overlay-worker")
            .arg("--actor")
            .arg("--control-stdin")
            .arg("--image")
            .arg(image)
            .arg("--output")
            .arg(monitor)
            .args([
                "--x",
                &settings.x.to_string(),
                "--y",
                &settings.y.to_string(),
                "--scale",
                &settings.scale.to_string(),
                "--opacity",
                &settings.opacity.to_string(),
                "--rotation",
                &settings.rotation.to_string(),
                "--speed",
                &settings.speed.to_string(),
            ]);
        if let Some((position, size)) = output_geometry {
            command.args([
                "--output-geometry".to_owned(),
                position.0.to_string(),
                position.1.to_string(),
                size.0.to_string(),
                size.1.to_string(),
            ]);
        }
        if settings.flip_x {
            command.arg("--flip-x");
        }
        if !settings.smoothing {
            command.arg("--no-smooth-scaling");
        }
        if !settings.playing {
            command.arg("--paused");
        }
        if !settings.looping {
            command.arg("--no-loop");
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start overlay: {error}"))?;
        let pid = child.id();
        let stdin = child
            .stdin
            .take()
            .ok_or("Overlay control pipe is unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Overlay output pipe is unavailable")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Overlay error pipe is unavailable")?;
        let (commands, input) = mpsc::channel::<Vec<u8>>();
        let key_owned = key.to_owned();
        let writer_key = key_owned.clone();
        let writer_sender = self.sender.clone();
        let writer_ctx = self.ctx.clone();
        thread::spawn(move || {
            let mut stdin = stdin;
            while let Ok(bytes) = input.recv() {
                if let Err(error) = stdin.write_all(&bytes).and_then(|_| stdin.flush()) {
                    let _ = writer_sender.send(Event::WriteFailed {
                        key: writer_key,
                        token,
                        detail: error.to_string(),
                    });
                    writer_ctx.request_repaint();
                    break;
                }
            }
        });
        let output_key = key_owned.clone();
        let output_sender = self.sender.clone();
        let output_ctx = self.ctx.clone();
        let output_thread = thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match (&mut stdout)
                    .take(64 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() <= 64 * 1024 && line.last() == Some(&b'\n') => {
                        if let Ok(message) = serde_json::from_slice(&line) {
                            if output_sender
                                .send(Event::Message {
                                    key: output_key.clone(),
                                    token,
                                    message,
                                })
                                .is_err()
                            {
                                break;
                            }
                            output_ctx.request_repaint();
                        }
                    }
                    _ => break,
                }
            }
        });
        let error_thread = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.take(16 * 1024).read_to_string(&mut text);
            text
        });
        let wait_sender = self.sender.clone();
        let wait_ctx = self.ctx.clone();
        let wait_key = key_owned.clone();
        thread::spawn(move || {
            let status = child.wait();
            let _ = output_thread.join();
            let detail = error_thread.join().unwrap_or_default();
            let success = status.as_ref().is_ok_and(|status| status.success());
            let detail = if detail.trim().is_empty() {
                status
                    .err()
                    .map(|error| error.to_string())
                    .unwrap_or_default()
            } else {
                detail.trim().to_owned()
            };
            let _ = wait_sender.send(Event::Exited {
                key: wait_key,
                token,
                success,
                detail,
            });
            wait_ctx.request_repaint();
        });
        self.workers.insert(
            key_owned,
            Worker {
                status: WorkerStatus::Starting,
                settings,
                confirmed: settings,
                monitor: monitor.to_owned(),
                pid,
                token,
                next_id: 0,
                pending: HashMap::new(),
                commands,
            },
        );
        Ok(())
    }

    pub fn update(&mut self, key: &str, settings: ActorSettings) -> Result<(), String> {
        settings.validate().map_err(str::to_owned)?;
        let worker = self.workers.get_mut(key).ok_or("Character is not active")?;
        if worker.status != WorkerStatus::Running {
            return Err("Character is not ready")?;
        }
        worker.next_id = worker
            .next_id
            .checked_add(1)
            .ok_or("Control identifier exhausted")?;
        let id = worker.next_id;
        let mut message = serde_json::to_value(settings).map_err(|error| error.to_string())?;
        message["action"] = json!("update");
        message["id"] = json!(id);
        let mut bytes = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        worker
            .commands
            .send(bytes)
            .map_err(|_| "Overlay control pipe is closed")?;
        worker.pending.insert(id, settings);
        worker.settings = settings;
        Ok(())
    }

    pub fn stop(&mut self, key: &str) -> Result<(), String> {
        let worker = self.workers.get_mut(key).ok_or("Character is not active")?;
        if worker.status == WorkerStatus::Stopping {
            return Ok(());
        }
        worker.next_id = worker
            .next_id
            .checked_add(1)
            .ok_or("Control identifier exhausted")?;
        let mut bytes = serde_json::to_vec(&json!({"action":"stop","id":worker.next_id}))
            .map_err(|error| error.to_string())?;
        bytes.push(b'\n');
        worker
            .commands
            .send(bytes)
            .map_err(|_| "Overlay control pipe is closed")?;
        worker.status = WorkerStatus::Stopping;
        Ok(())
    }

    pub fn poll(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Monitors(result) => {
                    self.loading_monitors = false;
                    match result {
                        Ok(monitors) => {
                            self.monitors = monitors;
                            self.notice = None;
                        }
                        Err(error) => {
                            self.monitors.clear();
                            self.notice = Some(error);
                        }
                    }
                }
                Event::Message {
                    key,
                    token,
                    message,
                } => {
                    let Some(worker) = self
                        .workers
                        .get_mut(&key)
                        .filter(|worker| worker.token == token)
                    else {
                        continue;
                    };
                    match message["event"].as_str() {
                        Some("configured") if worker.status == WorkerStatus::Starting => {
                            worker.status = WorkerStatus::Running
                        }
                        Some("applied") => {
                            if let Some(settings) = message["id"]
                                .as_u64()
                                .and_then(|id| worker.pending.remove(&id))
                            {
                                worker.confirmed = settings;
                            }
                        }
                        Some("superseded") => {
                            if let Some(id) = message["id"].as_u64() {
                                worker.pending.remove(&id);
                            }
                        }
                        Some("error") => {
                            if let Some(id) = message["id"].as_u64() {
                                worker.pending.remove(&id);
                            }
                            self.notice = Some(
                                message["message"]
                                    .as_str()
                                    .unwrap_or("Overlay update failed")
                                    .to_owned(),
                            );
                        }
                        _ => {}
                    }
                }
                Event::WriteFailed { key, token, detail } => {
                    if self
                        .workers
                        .get(&key)
                        .is_some_and(|worker| worker.token == token)
                    {
                        self.notice = Some(format!("Overlay control failed: {detail}"));
                    }
                }
                Event::Exited {
                    key,
                    token,
                    success,
                    detail,
                } => {
                    if self
                        .workers
                        .get(&key)
                        .is_some_and(|worker| worker.token == token)
                    {
                        let worker = self.workers.remove(&key).unwrap();
                        self.exited.push(key);
                        if !success || worker.status != WorkerStatus::Stopping {
                            self.notice = Some(if detail.is_empty() {
                                "Overlay stopped unexpectedly".into()
                            } else {
                                format!("Overlay stopped: {detail}")
                            });
                        }
                    }
                }
            }
        }
    }
}

fn list_monitors(executable: &Path) -> Result<Vec<Monitor>, String> {
    let mut child = Command::new(executable)
        .args(["--overlay-worker", "--list-outputs"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Could not query displays (timeout or process error)".into());
            }
        }
    };
    if !status.success() {
        return Err("Could not query displays; a Wayland session is required".into());
    }
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("No display list")?
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_OUTPUT_BYTES {
        return Err("Display list is too large".into());
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("Invalid display list: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_actor_settings() {
        assert!(ActorSettings::default().validate().is_ok());
        assert!(
            ActorSettings {
                x: f32::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ActorSettings {
                speed: 0.0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ActorSettings {
                opacity: 101.0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
