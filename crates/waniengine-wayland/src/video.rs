//! Streaming video decoder. Only a bounded number of raw frames are in memory;
//! no intermediate movie, animated image, or duration probe is needed.
use smithay_client_toolkit::reexports::calloop::channel::SyncSender;
use std::{
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub path: PathBuf,
    pub offset: f64,
    pub duration: Option<f64>,
    pub fit: String,
    pub speed: f32,
    pub fps: f32,
    pub looping: bool,
    pub playing: bool,
}

pub(crate) enum Event {
    Frame {
        generation: u64,
        pixels: Vec<u8>,
        position: f64,
    },
    End {
        generation: u64,
        result: Result<(), String>,
    },
}

pub(crate) struct Decoder {
    child: Child,
    pub size: (u32, u32),
    paused: bool,
    // Start paused wallpapers with one frame, then suspend decoding.
    pause_after_first: bool,
}

pub(crate) fn filter(
    fit: &str,
    width: u32,
    height: u32,
    speed: f32,
    fps: f32,
) -> Result<String, String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > super::MAX_PIXELS {
        return Err("Video wallpaper dimensions exceed the renderer limit".into());
    }
    let geometry = match fit {
        "cover" => format!(
            "scale={width}:{height}:force_original_aspect_ratio=increase:flags=fast_bilinear,crop={width}:{height}"
        ),
        "contain" => format!(
            "scale={width}:{height}:force_original_aspect_ratio=decrease:flags=fast_bilinear,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
        "stretch" => format!("scale={width}:{height}:flags=fast_bilinear"),
        "center" => format!(
            "crop=w='min(iw,{width})':h='min(ih,{height})',pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black"
        ),
        _ => return Err("Unsupported wallpaper fit mode".into()),
    };
    Ok(format!(
        "settb=AVTB,setpts=(PTS-STARTPTS)/{:.5},select='isnan(prev_selected_t)+gt(floor(t*{fps:.3}+0.001),floor(prev_selected_t*{fps:.3}+0.001))',{geometry},showinfo=checksum=0",
        speed / 100.0
    ))
}

fn frame_time(line: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(line).ok()?;
    if !text.starts_with("[Parsed_showinfo_") {
        return None;
    }
    text.split_once("pts_time:")?
        .1
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn is_error(line: &[u8]) -> bool {
    let text = String::from_utf8_lossy(line);
    let critical = |s: &str| s.starts_with("[error]") || s.starts_with("[fatal]");
    critical(&text)
        || text
            .split_once("] ")
            .is_some_and(|(_, rest)| critical(rest))
}

impl Decoder {
    pub fn start(
        options: &Options,
        size: (u32, u32),
        generation: u64,
        sender: SyncSender<Event>,
    ) -> Result<Self, String> {
        if !options.path.is_file() {
            return Err("The video wallpaper file is unavailable".into());
        }
        let filter = filter(&options.fit, size.0, size.1, options.speed, options.fps)?;
        let mut command = Command::new("ffmpeg");
        command.args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "level+info",
            "-nostats",
            "-threads",
            "2",
        ]);
        if options.looping {
            command.args(["-stream_loop", "-1"]);
        }
        let offset = if options.looping {
            options
                .duration
                .filter(|v| v.is_finite() && *v > 0.0)
                .map_or(options.offset, |duration| {
                    options.offset.rem_euclid(duration)
                })
        } else {
            options.offset
        };
        // Decode in real time, but permit the first frame immediately. Input
        // pacing bounds CPU even when the compositor hides this surface.
        command
            .args([
                "-readrate",
                &(options.speed / 100.0).to_string(),
                "-readrate_catchup",
                &(options.speed / 100.0).to_string(),
                "-ss",
                &format!("{:.6}", offset.max(0.0)),
                "-fflags",
                "+genpts",
                "-i",
            ])
            .arg(&options.path)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-sn",
                "-dn",
                "-filter_threads",
                "1",
                "-vf",
                &filter,
                "-threads",
                "2",
                "-f",
                "rawvideo",
                "-fps_mode",
                "passthrough",
                "-enc_time_base",
                "1:1000000",
                "-pix_fmt",
                "bgr0",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // If the renderer is terminated abruptly, its decoder must not survive
        // as an orphan. Also closes the race where the parent died pre-exec.
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::process::CommandExt;
            let parent = unsafe { libc::getpid() };
            unsafe {
                command.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::getppid() != parent {
                        libc::_exit(1);
                    }
                    Ok(())
                });
            }
        }
        let mut child = command.spawn().map_err(|error| {
            format!("Could not start the video decoder: {error}. FFmpeg is required.")
        })?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let errors = diagnostic.clone();
        let (times, positions) = std::sync::mpsc::sync_channel(2);
        let error_reader = thread::spawn(move || {
            let mut buffer = [0; 4096];
            let mut line = Vec::new();
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                for &byte in &buffer[..count] {
                    if byte == b'\n' {
                        if let Some(position) = frame_time(&line) {
                            if times.send(position).is_err() {
                                return;
                            }
                        } else if is_error(&line) {
                            let mut kept = errors.lock().unwrap_or_else(|e| e.into_inner());
                            let count = line.len().min(4096usize.saturating_sub(kept.len()));
                            kept.extend_from_slice(&line[..count]);
                        }
                        line.clear();
                    } else if line.len() < 4096 {
                        line.push(byte);
                    }
                }
            }
        });
        let bytes = size.0 as usize * size.1 as usize * 4;
        let offset = options.offset;
        let rate = f64::from(options.speed) / 100.0;
        thread::spawn(move || {
            let mut count = 0;
            loop {
                let mut pixels = vec![0; bytes];
                if stdout.read_exact(&mut pixels).is_err() {
                    break;
                }
                let position = match positions.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(position) => offset + position * rate,
                    Err(_) => {
                        let _ = sender.send(Event::End {
                            generation,
                            result: Err("FFmpeg did not publish a frame timestamp".into()),
                        });
                        return;
                    }
                };
                count += 1;
                if sender
                    .send(Event::Frame {
                        generation,
                        pixels,
                        position,
                    })
                    .is_err()
                {
                    return;
                }
            }
            drop(positions);
            let _ = error_reader.join();
            let diagnostic = diagnostic.lock().unwrap_or_else(|e| e.into_inner());
            let detail = String::from_utf8_lossy(&diagnostic).trim().to_owned();
            let result = if !detail.is_empty() {
                Err(format!("FFmpeg could not play this video: {detail}"))
            } else if count == 0 {
                Err("FFmpeg returned no video frames".into())
            } else {
                Ok(())
            };
            let _ = sender.send(Event::End { generation, result });
        });
        Ok(Self {
            child,
            size,
            paused: false,
            pause_after_first: !options.playing,
        })
    }

    pub fn first_frame(&mut self) -> Result<(), String> {
        if self.pause_after_first {
            self.pause_after_first = false;
            self.set_playing(false)?;
        }
        Ok(())
    }

    pub fn set_playing(&mut self, playing: bool) -> Result<(), String> {
        if self.paused == !playing {
            return Ok(());
        }
        #[cfg(unix)]
        {
            if self.child.try_wait().map_err(|e| e.to_string())?.is_some() {
                return Ok(());
            }
            let signal = if playing {
                libc::SIGCONT
            } else {
                libc::SIGSTOP
            };
            if unsafe { libc::kill(self.child.id() as i32, signal) } != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            self.paused = !playing;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err("Video wallpaper playback requires a Unix session".into())
        }
    }

    pub fn finish(&mut self) -> Result<(), String> {
        let status = self.child.wait().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("The video decoder stopped unexpectedly ({status})"))
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg; run cargo test -p waniengine-wayland -- --ignored"]
    fn fps_is_a_cap_and_does_not_duplicate_source_frames() {
        for (rate, cap, speed, variable, expected) in [
            (2, 60.0, 100.0, false, 4),
            (2, 1.0, 100.0, false, 2),
            (2, 60.0, 200.0, false, 4),
            (60, 59.0, 100.0, false, 118),
            (60, 60.0, 100.0, false, 120),
            (30, 60.0, 100.0, true, 48),
        ] {
            let input = format!("testsrc2=size=16x16:rate={rate}:duration=2");
            let graph = format!(
                "{}{}",
                if variable {
                    "select='not(eq(mod(n,5),0))',"
                } else {
                    ""
                },
                filter("cover", 16, 16, speed, cap).unwrap()
            );
            let result = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    &input,
                    "-vf",
                    &graph,
                    "-threads",
                    "1",
                    "-fps_mode",
                    "passthrough",
                    "-enc_time_base",
                    "1:1000000",
                    "-f",
                    "framemd5",
                    "-",
                ])
                .output()
                .expect("FFmpeg test dependency");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let frames = String::from_utf8_lossy(&result.stdout)
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .count();
            assert_eq!(frames, expected, "cap={cap}, speed={speed}");
        }
    }
    #[test]
    fn filters_preserve_fit_and_reject_unbounded_allocations() {
        for fit in ["cover", "contain", "center", "stretch"] {
            assert!(
                filter(fit, 1920, 1080, 125.0, 60.0)
                    .unwrap()
                    .starts_with("settb=AVTB,setpts=(PTS-STARTPTS)/1.25000")
            );
        }
        assert!(filter("tile", 1920, 1080, 100.0, 30.0).is_err());
        assert!(filter("cover", u32::MAX, u32::MAX, 100.0, 30.0).is_err());
        assert!(filter("cover", 0, 1080, 100.0, 30.0).is_err());
    }
}
