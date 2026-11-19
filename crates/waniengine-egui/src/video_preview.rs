//! Bounded, asynchronous video frames for the selected Library hero.
//!
//! FFmpeg decodes into a one-frame channel so decoding never blocks egui and
//! a slow window cannot accumulate an unbounded queue of stale frames.

use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    thread,
};

const MAX_EDGE: u32 = 640;
const PREVIEW_FPS: &str = "15";

pub struct Frame {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

pub struct VideoPreview {
    source: Option<String>,
    path: Option<PathBuf>,
    source_size: Option<(u32, u32)>,
    context: Option<eframe::egui::Context>,
    child: Option<Child>,
    frames: Option<Receiver<Vec<u8>>>,
    size: Option<[usize; 2]>,
    error: Option<String>,
    paused: bool,
}

impl Default for VideoPreview {
    fn default() -> Self {
        Self {
            source: None,
            path: None,
            source_size: None,
            context: None,
            child: None,
            frames: None,
            size: None,
            error: None,
            paused: false,
        }
    }
}

impl VideoPreview {
    /// Select a source without starting playback. Returns true when the source
    /// changes and the caller should discard its old texture.
    pub fn set_source(
        &mut self,
        source: &str,
        path: &Path,
        source_size: (u32, u32),
        ctx: eframe::egui::Context,
    ) -> bool {
        if self.source.as_deref() == Some(source) {
            return false;
        }
        self.stop();
        self.source = Some(source.to_owned());
        self.error = None;
        if !path.is_file() {
            self.error = Some("The video preview file is unavailable.".into());
            return true;
        }
        if output_size(source_size).is_none() {
            self.error = Some("The video has invalid dimensions.".into());
            return true;
        }
        self.path = Some(path.to_owned());
        self.source_size = Some(source_size);
        self.context = Some(ctx);
        true
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.frames = None;
        self.size = None;
        self.source = None;
        self.path = None;
        self.source_size = None;
        self.context = None;
        self.error = None;
        self.paused = false;
    }

    pub fn is_available(&self) -> bool {
        self.source.is_some() && self.error.is_none()
    }

    pub fn is_playing(&self) -> bool {
        self.child.is_some() && !self.paused
    }

    pub fn toggle_playback(&mut self) -> Result<bool, String> {
        if self.child.is_none() {
            self.start()?;
            return Ok(true);
        }
        let child = self.child.as_ref().expect("checked above");
        #[cfg(unix)]
        {
            let signal = if self.paused {
                libc::SIGCONT
            } else {
                libc::SIGSTOP
            };
            // The preview is a dedicated FFmpeg child process on Unix.
            if unsafe { libc::kill(child.id() as libc::pid_t, signal) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            self.paused = !self.paused;
            Ok(!self.paused)
        }
        #[cfg(not(unix))]
        {
            let _ = child;
            Err("Pausing video preview is unavailable on this platform.".into())
        }
    }

    fn start(&mut self) -> Result<(), String> {
        let path = self
            .path
            .as_deref()
            .ok_or_else(|| "The video preview file is unavailable.".to_owned())?;
        if !path.is_file() {
            let error = "The video preview file is unavailable.".to_owned();
            self.error = Some(error.clone());
            return Err(error);
        }
        let source_size = self
            .source_size
            .ok_or_else(|| "The video has invalid dimensions.".to_owned())?;
        let size = output_size(source_size)
            .ok_or_else(|| "The video has invalid dimensions.".to_owned())?;
        let frame_bytes = size[0]
            .checked_mul(size[1])
            .and_then(|pixels| pixels.checked_mul(4));
        let Some(frame_bytes) = frame_bytes.filter(|bytes| *bytes <= 2 * 1024 * 1024) else {
            let error = "The video preview exceeds its memory limit.".to_owned();
            self.error = Some(error.clone());
            return Err(error);
        };
        let filter = format!(
            "fps={PREVIEW_FPS},scale={}:{}:flags=fast_bilinear",
            size[0], size[1]
        );
        let context = self
            .context
            .clone()
            .ok_or_else(|| "The video preview window is unavailable.".to_owned())?;
        let child = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-stream_loop",
                "-1",
                "-re",
                "-i",
            ])
            .arg(path)
            .args([
                "-map", "0:v:0", "-an", "-sn", "-dn", "-vf", &filter, "-f", "rawvideo", "-pix_fmt",
                "rgba", "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(error) => {
                let message = format!("Could not start FFmpeg video preview: {error}");
                self.error = Some(message.clone());
                return Err(message);
            }
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            let message = "FFmpeg did not open its video frame output.".to_owned();
            self.error = Some(message.clone());
            return Err(message);
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        spawn_frame_reader(stdout, sender, frame_bytes, context);
        self.child = Some(child);
        self.frames = Some(receiver);
        self.size = Some(size);
        self.paused = false;
        self.error = None;
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Frame> {
        let receiver = self.frames.as_ref()?;
        let mut newest = None;
        while let Ok(frame) = receiver.try_recv() {
            newest = Some(frame);
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() && self.error.is_none() {
                        self.error = Some("FFmpeg could not decode the selected video.".into());
                    }
                    self.child = None;
                    self.frames = None;
                    self.paused = false;
                }
                Err(error) => {
                    self.error = Some(format!("Could not inspect the video preview: {error}"));
                    self.child = None;
                    self.frames = None;
                    self.paused = false;
                }
                Ok(None) => {}
            }
        }
        newest.and_then(|rgba| self.size.map(|size| Frame { size, rgba }))
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.error.take()
    }
}

impl Drop for VideoPreview {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_frame_reader(
    mut stdout: impl Read + Send + 'static,
    sender: SyncSender<Vec<u8>>,
    frame_bytes: usize,
    ctx: eframe::egui::Context,
) {
    thread::spawn(move || {
        let mut frame = vec![0; frame_bytes];
        loop {
            if stdout.read_exact(&mut frame).is_err() {
                break;
            }
            match sender.try_send(frame) {
                Ok(()) => {
                    ctx.request_repaint();
                    frame = vec![0; frame_bytes];
                }
                Err(TrySendError::Full(returned)) => frame = returned,
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
    });
}

fn output_size((width, height): (u32, u32)) -> Option<[usize; 2]> {
    if width == 0 || height == 0 {
        return None;
    }
    let factor = (MAX_EDGE as f64 / f64::from(width.max(height))).min(1.0);
    let even = |value: u32| (value.max(2) & !1) as usize;
    Some([
        even((f64::from(width) * factor).round() as u32),
        even((f64::from(height) * factor).round() as u32),
    ])
}
