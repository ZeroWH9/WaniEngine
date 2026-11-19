//! Bounded PCM wallpaper audio. Volume changes affect samples, never restart media.
use serde_json::Value;
use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use waniengine_core::library::file_path;

#[derive(Clone, Debug, PartialEq)]
struct AudioConfig {
    path: PathBuf,
    speed: f64,
    looping: bool,
}
struct ActiveAudio {
    config: AudioConfig,
    child: Arc<Mutex<Option<Child>>>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    position: Arc<AtomicU64>,
    errors: mpsc::Receiver<String>,
    thread: Option<JoinHandle<()>>,
}
impl Drop for ActiveAudio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Default)]
pub struct WallpaperAudio {
    active: Option<ActiveAudio>,
    failed: Option<AudioConfig>,
    last_correction: Option<Instant>,
}
impl WallpaperAudio {
    /// Renderer position is the shared media timeline, including previous pauses.
    pub fn sync(
        &mut self,
        asset: Option<&Value>,
        global_volume: f64,
        paused: bool,
        renderer_position: Option<f64>,
    ) -> Option<String> {
        let desired = asset.and_then(config_for);
        let Some(config) = desired else {
            self.stop();
            return None;
        };
        let volume = (percent(asset.unwrap()["volume"].as_f64().unwrap_or(50.0))
            * percent(global_volume)
            / 100.0)
            .round() as u32;
        let duration = asset
            .and_then(|a| a["duration"].as_f64())
            .filter(|s| s.is_finite() && *s > 0.0);
        let mut target = renderer_position.unwrap_or(0.0).max(0.0);
        if config.looping {
            if let Some(duration) = duration {
                target %= duration;
            }
        }
        let restart = self.active.as_ref().is_some_and(|a| {
            let mut position = a.position.load(Ordering::Relaxed) as f64 / 1e6;
            if config.looping {
                if let Some(duration) = duration {
                    position %= duration;
                }
            }
            let delta = (position - target).abs();
            let delta = duration
                .filter(|_| config.looping)
                .map_or(delta, |d| delta.min((d - delta).abs()));
            delta > 0.25
                && renderer_position.is_some()
                && self
                    .last_correction
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(3))
        });
        if self.active.as_ref().is_some_and(|a| a.config != config) || restart {
            self.active = None;
            self.failed = None;
        }
        if let Some(active) = &self.active {
            active.volume.store(volume, Ordering::Relaxed);
            active.paused.store(paused, Ordering::Release);
            if let Ok(error) = active.errors.try_recv() {
                self.failed = Some(config);
                self.active = None;
                return Some(error);
            }
            return None;
        }
        if self.failed.as_ref() == Some(&config) {
            return None;
        }
        match start(config.clone(), volume, paused, target) {
            Ok(active) => {
                self.active = Some(active);
                self.last_correction = Some(Instant::now());
                None
            }
            Err(error) => {
                self.failed = Some(config);
                Some(error)
            }
        }
    }
    pub(crate) fn stop(&mut self) {
        self.active = None;
        self.failed = None;
        self.last_correction = None;
    }
}
fn percent(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}
fn config_for(asset: &Value) -> Option<AudioConfig> {
    if asset["video"] != true
        || asset["hasAudio"] != true
        || asset["audioEnabled"].as_bool() == Some(false)
    {
        return None;
    }
    let path = file_path(asset["url"].as_str()?).ok()?;
    if !path.is_file() {
        return None;
    }
    let speed = asset["speed"]
        .as_f64()
        .filter(|v| v.is_finite())
        .unwrap_or(100.0)
        .clamp(25.0, 200.0);
    Some(AudioConfig {
        path,
        speed,
        looping: asset["loop"].as_bool().unwrap_or(true),
    })
}
fn apply_gain(bytes: &mut [u8], gain: f32) {
    for sample in bytes.chunks_exact_mut(4) {
        let value = f32::from_le_bytes(sample.try_into().unwrap());
        sample.copy_from_slice(&(value * gain).clamp(-1.0, 1.0).to_le_bytes());
    }
}
fn start(
    config: AudioConfig,
    volume: u32,
    paused: bool,
    offset: f64,
) -> Result<ActiveAudio, String> {
    let child = Arc::new(Mutex::new(None));
    let stop = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(paused));
    let volume = Arc::new(AtomicU32::new(volume));
    let position = Arc::new(AtomicU64::new((offset * 1e6) as u64));
    let (errors_tx, errors) = mpsc::channel();
    let (owner, ending, pausing, gain, clock) = (
        child.clone(),
        stop.clone(),
        paused.clone(),
        volume.clone(),
        position.clone(),
    );
    let options = config.clone();
    let thread = thread::Builder::new()
        .name("wallpaper-pcm".into())
        .spawn(move || {
            let result = (|| -> Result<(), String> {
                let sink = crate::audio_output::Sink::open()?;
                let rate = options.speed / 100.0;
                let filter = if rate < 0.5 {
                    format!("atempo=0.5,atempo={}", rate / 0.5)
                } else {
                    format!("atempo={rate}")
                };
                let mut command = Command::new("ffmpeg");
                command.args([
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-threads",
                    "1",
                ]);
                if options.looping {
                    command.args(["-stream_loop", "-1"]);
                }
                command
                    .args(["-ss", &format!("{offset:.6}"), "-i"])
                    .arg(&options.path)
                    .args([
                        "-map",
                        "0:a:0",
                        "-vn",
                        "-sn",
                        "-dn",
                        "-af",
                        &filter,
                        "-filter_threads",
                        "1",
                        "-ac",
                        "2",
                        "-ar",
                        "48000",
                        "-f",
                        "f32le",
                        "pipe:1",
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                waniengine_core::process::die_with_parent(&mut command);
                let mut decoder = command
                    .spawn()
                    .map_err(|e| format!("Could not decode wallpaper audio: {e}"))?;
                let mut input = decoder.stdout.take().ok_or("No PCM pipe")?;
                {
                    let mut slot = owner.lock().unwrap_or_else(|e| e.into_inner());
                    if ending.load(Ordering::Acquire) {
                        let _ = decoder.kill();
                        let _ = decoder.wait();
                        return Ok(());
                    }
                    *slot = Some(decoder);
                }
                let mut sent = 0u64;
                let mut bytes = [0u8; 4096];
                let mut pending = 0usize;
                loop {
                    if ending.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let paused = pausing.load(Ordering::Acquire);
                    sink.set_paused(paused);
                    let queued = sink.queued_bytes();
                    clock.store(
                        ((offset
                            + (sent.saturating_sub(queued as u64)) as f64 / (48000.0 * 8.0) * rate)
                            * 1e6) as u64,
                        Ordering::Relaxed,
                    );
                    if paused || queued >= 48000 * 8 / 20 {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    let read = input
                        .read(&mut bytes[pending..])
                        .map_err(|e| e.to_string())?;
                    let count = pending + read;
                    if read == 0 {
                        if pending != 0 {
                            return Err("Incomplete PCM audio frame".into());
                        }
                        if queued > 0 {
                            thread::sleep(Duration::from_millis(5));
                            continue;
                        }
                        let mut child = owner.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(mut decoder) = child.take() {
                            let status = decoder.wait().map_err(|e| e.to_string())?;
                            if !status.success() {
                                return Err(format!("Wallpaper audio decoder failed ({status})"));
                            }
                        }
                        return Ok(());
                    }
                    let usable = count / 8 * 8;
                    pending = count - usable;
                    apply_gain(
                        &mut bytes[..usable],
                        gain.load(Ordering::Relaxed) as f32 / 100.0,
                    );
                    if usable > 0 {
                        sink.write(&bytes[..usable])?;
                        sent += usable as u64;
                    }
                    bytes.copy_within(usable..count, 0);
                }
            })();
            if let Err(error) = result {
                if !ending.load(Ordering::Acquire) {
                    let _ = errors_tx.send(error);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(ActiveAudio {
        config,
        child,
        stop,
        paused,
        volume,
        position,
        errors,
        thread: Some(thread),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gain_changes_samples_without_a_transport_restart() {
        let mut bytes = [0.8f32.to_le_bytes(), (-0.4f32).to_le_bytes()].concat();
        apply_gain(&mut bytes, 0.5);
        assert!((f32::from_le_bytes(bytes[..4].try_into().unwrap()) - 0.4).abs() < 0.0001);
        assert!((f32::from_le_bytes(bytes[4..].try_into().unwrap()) + 0.2).abs() < 0.0001);
    }
}

/// Integration harness uses the production PCM path with a private dummy sink.
pub(crate) fn verify_transport(source: &std::path::Path) -> Result<(), String> {
    if std::env::var("SDL_AUDIODRIVER").as_deref() != Ok("dummy") {
        return Err("PCM verification requires SDL dummy audio".into());
    }
    let active = start(
        AudioConfig {
            path: source.to_owned(),
            speed: 100.0,
            looping: true,
        },
        30,
        false,
        0.0,
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(error) = active.errors.try_recv() {
            return Err(error);
        }
        if let Some(child) = active
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            break child.id();
        }
        if Instant::now() > deadline {
            return Err("PCM decoder did not start".into());
        }
        thread::sleep(Duration::from_millis(10));
    };
    thread::sleep(Duration::from_millis(150));
    let before = active.position.load(Ordering::Relaxed);
    for volume in [70, 0, 100, 20] {
        active.volume.store(volume, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(80));
        if active
            .child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(Child::id)
            != Some(pid)
        {
            return Err("Volume restarted PCM transport".into());
        }
    }
    if active.position.load(Ordering::Relaxed) <= before {
        return Err("PCM clock did not advance".into());
    }
    active.paused.store(true, Ordering::Release);
    thread::sleep(Duration::from_millis(60));
    let held = active.position.load(Ordering::Relaxed);
    thread::sleep(Duration::from_millis(100));
    if active.position.load(Ordering::Relaxed).abs_diff(held) > 20000 {
        return Err("Paused PCM clock advanced".into());
    }
    if let Ok(error) = active.errors.try_recv() {
        return Err(error);
    }
    drop(active);
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return Err("PCM decoder survived cleanup".into());
    }
    Ok(())
}
