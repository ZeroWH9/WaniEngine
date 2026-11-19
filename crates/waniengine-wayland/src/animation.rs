//! Bounded GIF, APNG and animated WebP decoding for the experimental overlay.
use image::{AnimationDecoder, DynamicImage, ImageDecoder};
use std::{error::Error, fs::File, io::BufReader, path::Path, time::Duration};

const MAX_FRAMES: usize = 360;
const MAX_FRAME_BYTES: usize = 96 * 1024 * 1024;
const MIN_DELAY_MS: u64 = 16;

pub struct Animation {
    pub frames: Vec<DynamicImage>,
    delays_ms: Vec<u64>,
    duration_ms: u64,
}

impl Animation {
    /// Detect the media format by its contents. Static images return None.
    pub fn load_if_animated(path: &Path) -> Result<Option<Self>, Box<dyn Error>> {
        let format = image::ImageReader::open(path)?
            .with_guessed_format()?
            .format();
        let reader = || File::open(path).map(BufReader::new);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(64 * 1024 * 1024);
        match format {
            Some(image::ImageFormat::Gif) => {
                let mut decoder = image::codecs::gif::GifDecoder::new(reader()?)?;
                decoder.set_limits(limits)?;
                let dimensions = decoder.dimensions();
                Self::from_frames(dimensions, decoder.into_frames()).map(Some)
            }
            Some(image::ImageFormat::Png) => {
                let decoder = image::codecs::png::PngDecoder::with_limits(reader()?, limits)?;
                if !decoder.is_apng()? {
                    return Ok(None);
                }
                let dimensions = decoder.dimensions();
                Self::from_frames(dimensions, decoder.apng()?.into_frames()).map(Some)
            }
            Some(image::ImageFormat::WebP) => {
                let mut decoder = image::codecs::webp::WebPDecoder::new(reader()?)?;
                decoder.set_limits(limits)?;
                if !decoder.has_animation() {
                    return Ok(None);
                }
                let dimensions = decoder.dimensions();
                Self::from_frames(dimensions, decoder.into_frames()).map(Some)
            }
            _ => Ok(None),
        }
    }

    fn from_frames(
        (width, height): (u32, u32),
        decoded: image::Frames<'_>,
    ) -> Result<Self, Box<dyn Error>> {
        let bytes_per_frame = (width as usize)
            .checked_mul(height as usize)
            .and_then(|value| value.checked_mul(4))
            .ok_or("Animation dimensions overflow")?;
        if bytes_per_frame == 0 || bytes_per_frame > MAX_FRAME_BYTES {
            return Err("Animation frame exceeds memory limit".into());
        }
        let mut frames = Vec::new();
        let mut delays_ms = Vec::new();
        for frame in decoded {
            if frames.len() >= MAX_FRAMES
                || (frames.len() + 1)
                    .checked_mul(bytes_per_frame)
                    .is_none_or(|size| size > MAX_FRAME_BYTES)
            {
                return Err("Animation exceeds frame or memory limit".into());
            }
            let frame = frame?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let delay = if denominator == 0 {
                MIN_DELAY_MS
            } else {
                u64::from(numerator)
                    .div_ceil(u64::from(denominator))
                    .max(MIN_DELAY_MS)
            };
            delays_ms.push(delay);
            frames.push(DynamicImage::ImageRgba8(frame.into_buffer()));
        }
        if frames.is_empty() {
            return Err("Animation contains no frames".into());
        }
        let duration_ms = delays_ms.iter().sum();
        Ok(Self {
            frames,
            delays_ms,
            duration_ms,
        })
    }

    /// Returns the frame and whether another compositor callback is needed.
    pub fn frame_at(&self, elapsed: Duration, speed: f32, looping: bool) -> (usize, bool) {
        if self.frames.len() <= 1 {
            return (0, false);
        }
        let elapsed_ms = (elapsed.as_secs_f64() * 1000.0 * f64::from(speed) / 100.0) as u64;
        if !looping && elapsed_ms >= self.duration_ms {
            return (self.frames.len() - 1, false);
        }
        let mut position = elapsed_ms % self.duration_ms;
        for (index, delay) in self.delays_ms.iter().enumerate() {
            if position < *delay {
                return (index, true);
            }
            position -= *delay;
        }
        (self.frames.len() - 1, false)
    }

    /// Reduce decoded frames for an editor preview without enlarging small art.
    pub fn into_preview_frames(self, max_dimension: u32) -> (Vec<DynamicImage>, Vec<u64>) {
        let max_dimension = max_dimension.clamp(1, 2048);
        let frames = self
            .frames
            .into_iter()
            .map(|frame| frame.thumbnail(max_dimension, max_dimension))
            .collect();
        (frames, self.delays_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn respects_duration_speed_and_looping() {
        let animation = Animation {
            frames: vec![DynamicImage::new_rgba8(1, 1), DynamicImage::new_rgba8(1, 1)],
            delays_ms: vec![100, 200],
            duration_ms: 300,
        };
        assert_eq!(
            animation.frame_at(Duration::from_millis(99), 100.0, true),
            (0, true)
        );
        assert_eq!(
            animation.frame_at(Duration::from_millis(100), 100.0, true),
            (1, true)
        );
        assert_eq!(
            animation.frame_at(Duration::from_millis(300), 100.0, true),
            (0, true)
        );
        assert_eq!(
            animation.frame_at(Duration::from_millis(300), 100.0, false),
            (1, false)
        );
        assert_eq!(
            animation.frame_at(Duration::from_millis(50), 200.0, true),
            (1, true)
        );
    }

    #[test]
    fn rejects_animations_with_too_many_frames() {
        let path =
            std::env::temp_dir().join(format!("wani-frame-limit-{}.gif", std::process::id()));
        {
            let file = File::create(&path).unwrap();
            let mut encoder = image::codecs::gif::GifEncoder::new(file);
            for _ in 0..=MAX_FRAMES {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::new(1, 1)))
                    .unwrap();
            }
        }
        let result = Animation::load_if_animated(&path);
        std::fs::remove_file(path).unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn detects_apng_and_animated_webp_by_content() {
        if Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let encoders = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .unwrap();
        let encoders = String::from_utf8_lossy(&encoders.stdout);
        for (extension, codec) in [("png", "apng"), ("webp", "libwebp_anim")] {
            if !encoders.contains(codec) {
                continue;
            }
            let path = std::env::temp_dir()
                .join(format!("wani-animation-{}.{extension}", std::process::id()));
            let mut command = Command::new("ffmpeg");
            command.args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=64x64:r=2:d=1",
                "-c:v",
                codec,
                "-y",
            ]);
            if codec == "apng" {
                command.args(["-f", "apng"]);
            }
            let status = command.arg(&path).status().unwrap();
            assert!(status.success(), "failed to create {codec} test media");
            let animation = Animation::load_if_animated(&path)
                .unwrap()
                .expect("expected animation");
            std::fs::remove_file(path).unwrap();
            assert_eq!(animation.frames.len(), 2, "{codec}");
            assert_eq!(
                animation
                    .frame_at(Duration::from_millis(600), 100.0, true)
                    .0,
                1,
                "{codec}"
            );
        }
    }
}
