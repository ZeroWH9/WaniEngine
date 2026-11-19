//! Owns direct video surfaces. Replace an old surface only after the new worker
//! confirms its first committed frame. Closing the control pipe stops workers.
use crate::plasma_bridge::Bridge;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
pub struct VideoSettings {
    pub duration: Option<f64>,
    pub speed: f64,
    pub fps: u32,
    pub looping: bool,
    pub playing: bool,
}

#[derive(Default)]
pub(crate) struct Sessions {
    workers: HashMap<String, Worker>,
    gnome_source: Option<(PathBuf, String, VideoSettings)>,
    gnome_outputs: Vec<Value>,
    ordering: Option<Arc<crate::plasma_bridge::OrderingHub>>,
    retiring: HashMap<String, thread::JoinHandle<()>>,
    pending_errors: std::collections::VecDeque<(String, String)>,
}

struct Worker {
    child: Child,
    input: Option<ChildStdin>,
    events: Receiver<Value>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    next_id: u64,
    bridge: Option<Bridge>,
    elapsed: std::time::Duration,
    changed_at: Instant,
    playing: bool,
    speed: f64,
    telemetry: Arc<Mutex<Option<(f64, Instant)>>>,
    plasma: bool,
    ended: Arc<AtomicBool>,
}

impl Sessions {
    pub fn apply(
        &mut self,
        output: &str,
        source: &Path,
        fit: &str,
        settings: VideoSettings,
        plasma_geometry: Option<((i32, i32), (i32, i32))>,
    ) -> Result<(), String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        self.apply_internal(
            &executable,
            output,
            source,
            fit,
            settings,
            plasma_geometry,
            None,
        )
    }

    #[cfg(test)]
    pub fn apply_with_executable(
        &mut self,
        executable: &Path,
        output: &str,
        source: &Path,
        fit: &str,
        settings: VideoSettings,
        _plasma: bool,
    ) -> Result<(), String> {
        self.apply_internal(executable, output, source, fit, settings, None, None)
    }

    fn apply_internal(
        &mut self,
        executable: &Path,
        output: &str,
        source: &Path,
        fit: &str,
        settings: VideoSettings,
        plasma_geometry: Option<((i32, i32), (i32, i32))>,
        x11_geometry: Option<((i32, i32), (i32, i32))>,
    ) -> Result<(), String> {
        // Failure polling retires workers off egui's paint thread. A reconnect
        // must wait for that worker's bridge/lock restoration before reusing
        // the same desktop, rather than failing with a spurious second owner.
        if let Some(cleanup) = self.retiring.remove(output) {
            cleanup
                .join()
                .map_err(|_| "Previous wallpaper cleanup failed")?;
        }
        if plasma_geometry.is_some() {
            self.reconcile_plasma()?;
        }
        self.pending_errors.retain(|(name, _)| name != output);
        if !source.is_file() {
            return Err("The video wallpaper file is unavailable".into());
        }
        if output.is_empty() {
            return Err("Select a connected display".into());
        }
        if !settings.speed.is_finite() || !(10.0..=300.0).contains(&settings.speed) {
            return Err("Invalid video playback speed".into());
        }
        if !(1..=240).contains(&settings.fps)
            || !["cover", "contain", "stretch", "center"].contains(&fit)
        {
            return Err("Invalid video wallpaper rendering settings".into());
        }
        let mut command = Command::new(executable);
        command
            .args(["--overlay-worker", "--video"])
            .arg(source)
            .args([
                "--output",
                output,
                "--control-stdin",
                "--fit",
                fit,
                "--fps",
                &settings.fps.to_string(),
                "--speed",
                &settings.speed.to_string(),
                "--layer",
                "background",
            ]);
        if let Some(duration) = settings
            .duration
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 604800.0)
        {
            command.args(["--duration", &duration.to_string()]);
        }
        if plasma_geometry.is_some() {
            command.arg("--defer-present");
        }
        if let Some((position, size)) = x11_geometry {
            command.arg("--x11-wallpaper").args([
                "--output-geometry".to_owned(),
                position.0.to_string(),
                position.1.to_string(),
                size.0.to_string(),
                size.1.to_string(),
            ]);
        }
        if !settings.looping {
            command.arg("--no-loop");
        }
        if !settings.playing {
            command.arg("--paused");
        }
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start the video wallpaper renderer: {e}"))?;
        let mut worker = Worker::new(child);
        worker.plasma = plasma_geometry.is_some();
        if let Some((position, size)) = plasma_geometry {
            worker.wait_for("ready", None)?;
            worker.bridge = Some(Bridge::prepare(
                output,
                position,
                size,
                worker.child.id(),
                self.workers.get(output).and_then(|old| old.bridge.as_ref()),
                &mut self.ordering,
            )?);
            let input = worker
                .input
                .as_mut()
                .ok_or("Video control pipe is closed")?;
            input
                .write_all(b"{\"action\":\"present\",\"id\":0}\n")
                .and_then(|_| input.flush())
                .map_err(|e| e.to_string())?;
            worker.wait_for("configured", None)?;
            worker.bridge.as_mut().unwrap().activate()?;
        } else {
            worker.wait_for("configured", None)?;
        }
        // HashMap::insert drops the previous worker only after the replacement
        // has a real frame on the compositor. Failed replacements preserve it.
        worker.changed_at = Instant::now();
        worker.speed = settings.speed;
        worker.playing = settings.playing;
        self.workers.insert(output.to_owned(), worker);
        Ok(())
    }

    pub fn position(&self, output: &str) -> Option<f64> {
        let worker = if output == crate::desktop::GNOME_OUTPUT {
            self.workers.values().next()?
        } else {
            self.workers.get(output)?
        };
        if !worker.playing {
            return Some(worker.elapsed.as_secs_f64());
        }
        if let Some((seconds, at)) = *worker.telemetry.lock().unwrap_or_else(|e| e.into_inner()) {
            return Some(
                seconds
                    + if worker.playing && !worker.ended.load(Ordering::Acquire) {
                        at.elapsed().as_secs_f64() * worker.speed / 100.0
                    } else {
                        0.0
                    },
            );
        }
        Some(
            worker.elapsed.as_secs_f64()
                + if worker.playing {
                    worker.changed_at.elapsed().as_secs_f64() * worker.speed / 100.0
                } else {
                    0.0
                },
        )
    }

    pub fn finished(&self, output: &str) -> bool {
        if output == crate::desktop::GNOME_OUTPUT {
            !self.workers.is_empty()
                && self
                    .workers
                    .values()
                    .all(|worker| worker.ended.load(Ordering::Acquire))
        } else {
            self.workers
                .get(output)
                .is_some_and(|worker| worker.ended.load(Ordering::Acquire))
        }
    }

    pub fn apply_gnome(
        &mut self,
        source: &Path,
        fit: &str,
        settings: VideoSettings,
    ) -> Result<(), String> {
        let outputs = waniengine_wayland::x11_wallpaper_outputs()?;
        if outputs.is_empty() {
            return Err("No GNOME/XWayland displays are available".into());
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut staged = Self::default();
        staged.gnome_outputs = outputs.clone();
        staged.gnome_source = Some((source.to_owned(), fit.to_owned(), settings));
        for output in outputs {
            let name = output["name"]
                .as_str()
                .ok_or("XWayland returned no display name")?;
            let pair = |key: &str| -> Result<(i32, i32), String> {
                let values = output[key]
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .ok_or("XWayland returned incomplete display geometry")?;
                let x = values[0]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("Invalid display geometry")?;
                let y = values[1]
                    .as_i64()
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or("Invalid display geometry")?;
                Ok((x, y))
            };
            let position = pair("logicalPosition")?;
            let size = pair("logicalSize")?;
            if size.0 <= 0 || size.1 <= 0 {
                return Err("Invalid display dimensions".into());
            }
            staged.apply_internal(
                &executable,
                name,
                source,
                fit,
                settings,
                None,
                Some((position, size)),
            )?;
        }
        // Keep the complete previous set until every replacement display is ready.
        *self = staged;
        Ok(())
    }

    pub fn reconcile_gnome(&mut self) -> Result<(), String> {
        let Some((path, fit, settings)) = self.gnome_source.clone() else {
            return Ok(());
        };
        let outputs = waniengine_wayland::x11_wallpaper_outputs()?;
        if outputs != self.gnome_outputs {
            self.apply_gnome(&path, &fit, settings)?;
        }
        Ok(())
    }

    pub fn reconcile_plasma(&mut self) -> Result<(), String> {
        if !self.workers.values().any(|worker| worker.plasma) {
            return Ok(());
        }
        for (_, cleanup) in std::mem::take(&mut self.retiring) {
            cleanup.join().map_err(|_| "Wallpaper cleanup failed")?;
        }
        let result = waniengine_core::process::capture(
            Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .args(["--overlay-worker", "--list-outputs"]),
            Duration::from_secs(3),
        )
        .map_err(|e| e.to_string())?;
        let outputs: Vec<Value> =
            serde_json::from_slice(&result.stdout).map_err(|e| e.to_string())?;
        let states = Bridge::output_states(&outputs)?;
        let external: Vec<_> = self
            .workers
            .iter()
            .filter_map(|(name, worker)| {
                let bridge = worker.bridge.as_ref()?;
                let state = states
                    .iter()
                    .find(|state| state["id"].as_i64() == Some(bridge.desktop_id() as i64))?;
                (!bridge.owns_state(state)).then_some(name.clone())
            })
            .collect();
        for name in external {
            self.workers.remove(&name);
            self.pending_errors
                .push_back((name, "Wallpaper changed outside WaniEngine".into()));
        }
        let changed = self.workers.iter().any(|(name, worker)| {
            worker.plasma
                && states
                    .iter()
                    .find(|state| state["name"] == *name)
                    .is_some_and(|state| {
                        worker.bridge.as_ref().is_none_or(|bridge| {
                            Some(bridge.desktop_id() as i64) != state["id"].as_i64()
                        })
                    })
        });
        if !changed {
            return Ok(());
        }
        // A different wallpaper is an external change, not permission to
        // overwrite it while reconciling another monitor.
        if self
            .workers
            .values()
            .filter_map(|worker| worker.bridge.as_ref())
            .any(|bridge| !states.iter().any(|state| bridge.owns_state(state)))
        {
            return Err(
                "A wallpaper changed outside WaniEngine while displays were reconnecting".into(),
            );
        }
        // Release all old desktop locks before rebinding: two containments may
        // have exchanged outputs. Keep renderers and media clocks alive.
        for worker in self.workers.values_mut().filter(|worker| worker.plasma) {
            if let Some(mut bridge) = worker.bridge.take() {
                bridge.suspend()?;
                drop(bridge);
            }
        }
        for (name, worker) in self.workers.iter_mut().filter(|(_, worker)| worker.plasma) {
            let Some(output) = outputs.iter().find(|value| value["name"] == *name) else {
                continue;
            };
            let pair = |key: &str| -> Result<(i32, i32), String> {
                Ok((
                    output[key][0]
                        .as_i64()
                        .and_then(|v| i32::try_from(v).ok())
                        .ok_or("Invalid output geometry")?,
                    output[key][1]
                        .as_i64()
                        .and_then(|v| i32::try_from(v).ok())
                        .ok_or("Invalid output geometry")?,
                ))
            };
            let mut bridge = Bridge::prepare(
                name,
                pair("logicalPosition")?,
                pair("logicalSize")?,
                worker.child.id(),
                None,
                &mut self.ordering,
            )?;
            bridge.activate()?;
            worker.bridge = Some(bridge);
        }
        Ok(())
    }
    pub fn replace_gnome_with_image(
        &mut self,
        apply: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        apply()?;
        self.workers.clear();
        self.gnome_source = None;
        self.gnome_outputs.clear();
        Ok(())
    }

    pub fn set_fps(&mut self, fps: u32) -> Result<(), String> {
        if let Some((_, _, settings)) = &mut self.gnome_source {
            settings.fps = fps;
        }
        if !(1..=240).contains(&fps) {
            return Err("Invalid video frame rate".into());
        }
        for worker in self.workers.values_mut() {
            worker.next_id += 1;
            let id = worker.next_id;
            let input = worker
                .input
                .as_mut()
                .ok_or("Video control pipe is closed")?;
            writeln!(
                input,
                "{}",
                json!({"action":"video_settings","id":id,"fps":fps})
            )
            .map_err(|e| e.to_string())?;
            input.flush().map_err(|e| e.to_string())?;
            worker.wait_for("applied", Some(id))?;
        }
        Ok(())
    }

    pub fn set_all_playing(&mut self, playing: bool) -> Option<Result<(), String>> {
        if let Some((_, _, settings)) = &mut self.gnome_source {
            settings.playing = playing;
        }
        if self.workers.is_empty() {
            return None;
        }
        let outputs = self.workers.keys().cloned().collect::<Vec<_>>();
        for output in outputs {
            if let Some(Err(error)) = self.set_playing(&output, playing) {
                return Some(Err(error));
            }
        }
        Some(Ok(()))
    }

    pub fn stop_all(&mut self) {
        self.gnome_source = None;
        self.gnome_outputs.clear();
        let workers = std::mem::take(&mut self.workers);
        thread::spawn(move || drop(workers));
    }

    pub fn stop(&mut self, output: &str) {
        self.workers.remove(output);
    }

    pub fn replace_with_image(
        &mut self,
        output: &str,
        apply: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let bridged = self
            .workers
            .get_mut(output)
            .and_then(|worker| worker.bridge.as_mut());
        if let Some(bridge) = bridged {
            bridge.suspend()?;
        }
        match apply() {
            Ok(()) => {
                self.stop(output);
                Ok(())
            }
            Err(error) => {
                if let Some(bridge) = self
                    .workers
                    .get_mut(output)
                    .and_then(|worker| worker.bridge.as_mut())
                {
                    if let Err(resume_error) = bridge.resume() {
                        return Err(format!("{error}; video restoration failed: {resume_error}"));
                    }
                }
                Err(error)
            }
        }
    }

    pub fn set_playing(&mut self, output: &str, playing: bool) -> Option<Result<(), String>> {
        self.workers.get_mut(output).map(|worker| {
            worker.next_id += 1;
            let id = worker.next_id;
            let input = worker
                .input
                .as_mut()
                .ok_or("Video control pipe is closed")?;
            serde_json::to_writer(
                &mut *input,
                &json!({"action":"update","id":id,"playing":playing}),
            )
            .map_err(|e| e.to_string())?;
            input
                .write_all(b"\n")
                .and_then(|_| input.flush())
                .map_err(|e| e.to_string())?;
            worker.wait_for("applied", Some(id))?;
            let now = Instant::now();
            if let Some((seconds, at)) =
                &mut *worker.telemetry.lock().unwrap_or_else(|e| e.into_inner())
            {
                worker.elapsed = Duration::from_secs_f64(*seconds);
                *at = now;
            }
            worker.changed_at = now;
            worker.playing = playing;
            Ok(())
        })
    }

    pub fn take_error(&mut self) -> Option<(String, String)> {
        if let Some(error) = self.pending_errors.pop_front() {
            return Some(error);
        }
        let failed =
            self.workers
                .iter_mut()
                .find_map(|(output, worker)| match worker.child.try_wait() {
                    Ok(None) => None,
                    Ok(Some(_)) | Err(_) => Some((
                        output.clone(),
                        worker.error("The video wallpaper renderer stopped"),
                    )),
                });
        if let Some((output, _)) = &failed {
            // Failure polling runs on egui's thread; D-Bus restoration belongs
            // to the worker cleanup thread, never to the paint loop.
            let worker = self.workers.remove(output);
            self.retiring
                .insert(output.clone(), thread::spawn(move || drop(worker)));
        }
        failed
    }
}

impl Worker {
    fn new(mut child: Child) -> Self {
        let input = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let kept_errors = diagnostic.clone();
        thread::spawn(move || {
            let mut bytes = [0; 1024];
            while let Ok(count) = stderr.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                let mut kept = kept_errors.lock().unwrap_or_else(|e| e.into_inner());
                let count = count.min(4096usize.saturating_sub(kept.len()));
                kept.extend_from_slice(&bytes[..count]);
            }
        });
        let (sender, events) = mpsc::sync_channel(16);
        let telemetry = Arc::new(Mutex::new(None));
        let status = telemetry.clone();
        let ended = Arc::new(AtomicBool::new(false));
        let finished = ended.clone();
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                match (&mut stdout)
                    .take(64 * 1024 + 1)
                    .read_until(b'\n', &mut line)
                {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() <= 64 * 1024 && line.last() == Some(&b'\n') => {
                        if let Ok(event) = serde_json::from_slice::<Value>(&line) {
                            if event["event"] == "position"
                                || event["event"] == "ended"
                                || event["event"] == "applied"
                            {
                                if let Some(seconds) = event["seconds"]
                                    .as_f64()
                                    .filter(|v| v.is_finite() && *v >= 0.0)
                                {
                                    *status.lock().unwrap_or_else(|e| e.into_inner()) =
                                        Some((seconds, Instant::now()));
                                }
                                if event["event"] == "ended" {
                                    finished.store(true, Ordering::Release);
                                }
                                if let Some(value) = event["ended"].as_bool() {
                                    finished.store(value, Ordering::Release);
                                }
                                if event["event"] == "position" {
                                    continue;
                                }
                            }
                            if sender.send(event).is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
            }
        });
        Self {
            child,
            input,
            events,
            diagnostic,
            next_id: 0,
            bridge: None,
            elapsed: Duration::ZERO,
            changed_at: Instant::now(),
            playing: false,
            speed: 100.0,
            telemetry,
            plasma: false,
            ended,
        }
    }

    fn error(&self, fallback: &str) -> String {
        let bytes = self.diagnostic.lock().unwrap_or_else(|e| e.into_inner());
        let detail = String::from_utf8_lossy(&bytes);
        if detail.trim().is_empty() {
            fallback.into()
        } else {
            detail.trim().into()
        }
    }

    fn wait_for(&mut self, expected: &str, id: Option<u64>) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            let event = self.events.recv_timeout(timeout).map_err(|_| {
                self.error("The video renderer did not confirm its first frame or playback update")
            })?;
            if event["event"] == "error" {
                return Err(event["message"]
                    .as_str()
                    .unwrap_or("Video playback failed")
                    .into());
            }
            if event["event"] == expected && (id.is_none() || event["id"].as_u64() == id) {
                return Ok(());
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(bridge) = &self.bridge {
            bridge.stop_health();
        }
        self.input = None;
        let deadline = Instant::now() + Duration::from_millis(300);
        while self.child.try_wait().is_ok_and(|status| status.is_none())
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Keep ordering constraints until the surface has actually disappeared.
        self.bridge = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoder_pid(worker: u32) -> u32 {
        let path = format!("/proc/{worker}/task/{worker}/children");
        std::fs::read_to_string(path)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    fn process_exists(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    #[test]
    #[ignore = "requires the private compositor launched by scripts/check-video-wallpaper.py"]
    fn isolated_replacement_and_lifecycle() {
        assert!(
            std::env::var("XDG_RUNTIME_DIR")
                .unwrap()
                .starts_with("/tmp/wani-video-test-")
        );
        assert_eq!(std::env::var("WAYLAND_DISPLAY").unwrap(), "wani-video");
        let executable = std::env::var_os("WANIENGINE_VIDEO_TEST_EXE").unwrap();
        let source = std::env::var_os("WANIENGINE_VIDEO_TEST_SOURCE").unwrap();
        let invalid = std::env::var_os("WANIENGINE_VIDEO_TEST_INVALID").unwrap();
        let output = std::env::var("WANIENGINE_VIDEO_TEST_OUTPUT").unwrap();
        let settings = VideoSettings {
            duration: None,
            speed: 100.0,
            fps: 30,
            looping: true,
            playing: true,
        };
        let mut sessions = Sessions::default();
        let before = Instant::now();
        sessions
            .apply_with_executable(
                Path::new(&executable),
                &output,
                Path::new(&source),
                "cover",
                settings,
                true,
            )
            .unwrap();
        assert!(before.elapsed() < Duration::from_millis(1500));
        let first = sessions.workers[&output].child.id();
        let decoder = decoder_pid(first);
        sessions.set_playing(&output, false).unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let status = std::fs::read_to_string(format!("/proc/{decoder}/status")).unwrap();
            if status
                .lines()
                .any(|line| line.starts_with("State:") && line.contains("T (stopped)"))
            {
                break;
            }
            assert!(Instant::now() < deadline, "decoder did not pause");
            thread::sleep(Duration::from_millis(5));
        }
        sessions.set_playing(&output, true).unwrap().unwrap();
        assert!(
            sessions
                .apply_with_executable(
                    Path::new(&executable),
                    &output,
                    Path::new(&invalid),
                    "cover",
                    settings,
                    true
                )
                .is_err()
        );
        assert_eq!(
            sessions.workers[&output].child.id(),
            first,
            "failed replacement discarded the working wallpaper"
        );
        assert!(process_exists(first) && process_exists(decoder));
        sessions
            .apply_with_executable(
                Path::new(&executable),
                &output,
                Path::new(&source),
                "contain",
                settings,
                true,
            )
            .unwrap();
        assert!(
            !process_exists(first) && !process_exists(decoder),
            "old renderer/decoder survived replacement"
        );
        let replacement = sessions.workers[&output].child.id();
        let replacement_decoder = decoder_pid(replacement);
        drop(sessions);
        assert!(
            !process_exists(replacement) && !process_exists(replacement_decoder),
            "video processes survived the manager"
        );
    }
}
