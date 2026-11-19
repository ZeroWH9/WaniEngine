//! Bounded SVG rasterization for the character renderer.
use image::DynamicImage;
use std::{
    io::{Cursor, Read},
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(1);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

pub fn renderer_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let Ok(mut child) = Command::new("rsvg-convert")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = Instant::now() + AVAILABILITY_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) | Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    })
}

pub fn rasterize(path: &Path) -> Result<DynamicImage, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("Could not inspect SVG: {error}"))?;
    if !metadata.is_file() {
        return Err("SVG file is not a regular file".into());
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err("SVG file exceeds the 16 MiB size limit".into());
    }

    let mut child = Command::new("rsvg-convert")
        .args([
            "--format",
            "png",
            "--width",
            "2048",
            "--height",
            "2048",
            "--keep-aspect-ratio",
            "--output",
            "-",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "SVG characters require rsvg-convert (librsvg)".to_owned()
            } else {
                format!("Could not start SVG renderer: {error}")
            }
        })?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or("SVG renderer output is unavailable")?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or("SVG renderer diagnostics are unavailable")?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 16 * 1024];
        let mut oversized = false;
        loop {
            let read = stdout.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            let available = MAX_OUTPUT_BYTES.saturating_sub(bytes.len() as u64) as usize;
            let keep = available.min(read);
            bytes.extend_from_slice(&chunk[..keep]);
            oversized |= keep < read;
        }
        (!oversized).then_some(bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8 * 1024];
        loop {
            let read = stderr.read(&mut chunk).ok()?;
            if read == 0 {
                break;
            }
            let available = (64_u64 * 1024).saturating_sub(bytes.len() as u64) as usize;
            let keep = available.min(read);
            bytes.extend_from_slice(&chunk[..keep]);
        }
        Some(String::from_utf8_lossy(&bytes).into_owned())
    });

    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err("SVG rendering exceeded its 5-second time limit".into());
            }
        }
    };
    let png = stdout_reader
        .join()
        .map_err(|_| "SVG renderer output worker stopped unexpectedly")?
        .ok_or("SVG renderer output exceeded the 64 MiB limit or could not be read")?;
    let diagnostics = stderr_reader
        .join()
        .map_err(|_| "SVG renderer diagnostics worker stopped unexpectedly")?
        .unwrap_or_default();
    if !status.success() {
        return Err(if diagnostics.trim().is_empty() {
            "rsvg-convert could not render this SVG".into()
        } else {
            format!("rsvg-convert: {}", diagnostics.trim())
        });
    }
    let mut reader = image::ImageReader::new(Cursor::new(png))
        .with_guessed_format()
        .map_err(|error| format!("Could not inspect the rendered SVG: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| format!("Could not decode the rendered SVG: {error}"))
}
