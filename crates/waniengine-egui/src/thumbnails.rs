//! Bounded background thumbnail decoding. No library files are written.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use waniengine_core::library::file_path;

const MAX_TEXTURES: usize = 128;
const MAX_FAILED_TEXTURES: usize = 256;
const MAX_INPUT_BYTES: u64 = 128 * 1024 * 1024;

type Decoded = (String, usize, Option<ColorImage>);
type AnimatedDecoded = (String, usize, Option<(Vec<ColorImage>, Vec<u64>)>);
type ActorDecoded = (String, usize, Option<ColorImage>);

struct AnimatedPreview {
    url: String,
    quality: usize,
    images: Vec<ColorImage>,
    texture: TextureHandle,
    current_frame: usize,
    smoothing: bool,
    delays_ms: Vec<u64>,
    elapsed: Duration,
    last_tick: Instant,
    was_playing: bool,
}

struct StaticActorPreview {
    url: String,
    quality: usize,
    image: ColorImage,
    texture: TextureHandle,
    smoothing: bool,
}

impl AnimatedPreview {
    fn frame(&mut self, speed: f32, playing: bool, looping: bool, ctx: &egui::Context) -> usize {
        let now = Instant::now();
        if self.was_playing {
            self.elapsed += now.saturating_duration_since(self.last_tick);
        }
        self.was_playing = playing;
        self.last_tick = now;

        let duration_ms = self.delays_ms.iter().sum::<u64>().max(1);
        let elapsed_ms = (self.elapsed.as_secs_f64() * f64::from(speed.clamp(10.0, 300.0)) / 100.0
            * 1000.0) as u64;
        let ended = !looping && elapsed_ms >= duration_ms;
        let mut position = if looping {
            elapsed_ms % duration_ms
        } else {
            elapsed_ms.min(duration_ms.saturating_sub(1))
        };
        for (index, delay) in self.delays_ms.iter().enumerate() {
            if position < *delay {
                if playing && !ended {
                    ctx.request_repaint_after(Duration::from_millis(
                        ((*delay - position) as f32 / speed.clamp(10.0, 300.0) * 100.0).ceil()
                            as u64,
                    ));
                }
                return index.min(self.images.len().saturating_sub(1));
            }
            position -= delay;
        }
        self.images.len().saturating_sub(1)
    }
}

pub struct Thumbnails {
    requests: mpsc::SyncSender<String>,
    results: mpsc::Receiver<Decoded>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    textures: HashMap<String, (TextureHandle, u64)>,
    clock: u64,
    max_dimension: Arc<AtomicUsize>,
    animation_requests: mpsc::SyncSender<(String, usize)>,
    animation_results: mpsc::Receiver<AnimatedDecoded>,
    animation_pending: HashSet<(String, usize)>,
    animation_failed: HashSet<(String, usize)>,
    animation_preview: Option<AnimatedPreview>,
    animation_target: Option<(String, usize)>,
    actor_preview_requests: mpsc::SyncSender<(String, usize)>,
    actor_preview_results: mpsc::Receiver<ActorDecoded>,
    actor_preview_pending: HashSet<(String, usize)>,
    actor_preview_failed: HashSet<(String, usize)>,
    actor_preview: Option<StaticActorPreview>,
    actor_preview_target: Option<(String, usize)>,
}

impl Thumbnails {
    pub fn has_cached(&self, url: &str) -> bool {
        self.textures.contains_key(url)
    }
    pub fn insert_preview(&mut self, ctx: &egui::Context, url: &str, image: &image::RgbaImage) {
        let texture = ctx.load_texture(
            url,
            ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image,
            ),
            TextureOptions::LINEAR,
        );
        self.textures.insert(url.to_owned(), (texture, 0));
    }

    pub fn new(ctx: egui::Context) -> Self {
        let (requests, queue) = mpsc::sync_channel::<String>(16);
        let (results, receiver) = mpsc::channel();
        let max_dimension = Arc::new(AtomicUsize::new(480));
        let decode_size = Arc::clone(&max_dimension);
        let thumbnail_ctx = ctx.clone();
        thread::Builder::new()
            .name("thumbnail-decoder".into())
            .spawn(move || {
                while let Ok(url) = queue.recv() {
                    let quality = decode_size.load(Ordering::Relaxed);
                    let image = file_path(&url).ok().and_then(|path| decode(&path, quality));
                    if results.send((url, quality, image)).is_err() {
                        break;
                    }
                    thumbnail_ctx.request_repaint();
                }
            })
            .expect("start thumbnail decoder");
        let (animation_requests, animation_queue) = mpsc::sync_channel::<(String, usize)>(2);
        let (animation_results, animation_receiver) = mpsc::channel();
        let animation_decode_size = Arc::clone(&max_dimension);
        let animation_ctx = ctx.clone();
        thread::Builder::new()
            .name("animation-preview-decoder".into())
            .spawn(move || {
                while let Ok((url, quality)) = animation_queue.recv() {
                    let frames = file_path(&url).ok().and_then(|path| {
                        if !path.is_file()
                            || path.metadata().ok()?.len() > MAX_INPUT_BYTES
                            || quality != animation_decode_size.load(Ordering::Relaxed)
                        {
                            return None;
                        }
                        let (images, delays) =
                            waniengine_wayland::load_animation_preview(&path, quality as u32)
                                .ok()??;
                        let frames = images
                            .into_iter()
                            .map(|image| {
                                let image = image.to_rgba8();
                                ColorImage::from_rgba_unmultiplied(
                                    [image.width() as usize, image.height() as usize],
                                    &image,
                                )
                            })
                            .collect();
                        Some((frames, delays))
                    });
                    if animation_results.send((url, quality, frames)).is_err() {
                        break;
                    }
                    animation_ctx.request_repaint();
                }
            })
            .expect("start animation preview decoder");
        let (actor_preview_requests, actor_preview_queue) =
            mpsc::sync_channel::<(String, usize)>(1);
        let (actor_preview_results, actor_preview_receiver) = mpsc::channel();
        let actor_preview_size = Arc::clone(&max_dimension);
        let actor_preview_ctx = ctx;
        thread::Builder::new()
            .name("actor-preview-decoder".into())
            .spawn(move || {
                while let Ok((url, quality)) = actor_preview_queue.recv() {
                    let image = file_path(&url).ok().and_then(|path| {
                        if !path.is_file()
                            || path.metadata().ok()?.len() > MAX_INPUT_BYTES
                            || quality != actor_preview_size.load(Ordering::Relaxed)
                        {
                            return None;
                        }
                        decode(&path, quality)
                    });
                    if actor_preview_results.send((url, quality, image)).is_err() {
                        break;
                    }
                    actor_preview_ctx.request_repaint();
                }
            })
            .expect("start actor preview decoder");
        Self {
            requests,
            results: receiver,
            pending: HashSet::new(),
            failed: HashSet::new(),
            textures: HashMap::new(),
            clock: 0,
            max_dimension,
            animation_requests,
            animation_results: animation_receiver,
            animation_pending: HashSet::new(),
            animation_failed: HashSet::new(),
            animation_preview: None,
            animation_target: None,
            actor_preview_requests,
            actor_preview_results: actor_preview_receiver,
            actor_preview_pending: HashSet::new(),
            actor_preview_failed: HashSet::new(),
            actor_preview: None,
            actor_preview_target: None,
        }
    }

    pub fn set_quality(&mut self, max_dimension: usize) {
        let max_dimension = max_dimension.clamp(120, 960);
        if self.max_dimension.swap(max_dimension, Ordering::Relaxed) != max_dimension {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        self.textures.clear();
        self.failed.clear();
        self.animation_failed.clear();
        self.animation_pending.clear();
        self.animation_preview = None;
        self.animation_target = None;
        self.actor_preview_pending.clear();
        self.actor_preview_failed.clear();
        self.actor_preview = None;
        self.actor_preview_target = None;
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((url, quality, image)) = self.results.try_recv() {
            self.pending.remove(&url);
            if quality != self.max_dimension.load(Ordering::Relaxed) {
                continue;
            }
            if let Some(image) = image {
                if self.textures.len() >= MAX_TEXTURES {
                    if let Some(oldest) = self
                        .textures
                        .iter()
                        .min_by_key(|(_, (_, used))| *used)
                        .map(|(key, _)| key.clone())
                    {
                        self.textures.remove(&oldest);
                    }
                }
                let texture = ctx.load_texture(&url, image, TextureOptions::LINEAR);
                self.textures.insert(url, (texture, self.clock));
            } else {
                remember_failure(&mut self.failed, url);
            }
        }
        while let Ok((url, quality, decoded)) = self.animation_results.try_recv() {
            self.animation_pending.remove(&(url.clone(), quality));
            if quality != self.max_dimension.load(Ordering::Relaxed)
                || self.animation_target.as_ref() != Some(&(url.clone(), quality))
            {
                continue;
            }
            match decoded {
                Some((images, delays_ms))
                    if !images.is_empty() && images.len() == delays_ms.len() =>
                {
                    let texture = ctx.load_texture(
                        format!("{url}|animation-preview|{quality}"),
                        images[0].clone(),
                        TextureOptions::LINEAR,
                    );
                    self.animation_preview = Some(AnimatedPreview {
                        url,
                        quality,
                        images,
                        texture,
                        current_frame: 0,
                        smoothing: true,
                        delays_ms,
                        elapsed: Duration::ZERO,
                        last_tick: Instant::now(),
                        was_playing: true,
                    });
                }
                _ => {
                    remember_failure(&mut self.animation_failed, (url, quality));
                }
            }
        }
        while let Ok((url, quality, decoded)) = self.actor_preview_results.try_recv() {
            let key = (url.clone(), quality);
            self.actor_preview_pending.remove(&key);
            if quality != self.max_dimension.load(Ordering::Relaxed)
                || self.actor_preview_target.as_ref() != Some(&key)
            {
                continue;
            }
            match decoded {
                Some(image) => {
                    let smoothing = self
                        .actor_preview
                        .as_ref()
                        .filter(|preview| preview.url == url && preview.quality == quality)
                        .is_none_or(|preview| preview.smoothing);
                    let options = if smoothing {
                        TextureOptions::LINEAR
                    } else {
                        TextureOptions::NEAREST
                    };
                    let texture = ctx.load_texture(
                        format!("{url}|actor-preview|{quality}"),
                        image.clone(),
                        options,
                    );
                    self.actor_preview = Some(StaticActorPreview {
                        url,
                        quality,
                        image,
                        texture,
                        smoothing,
                    });
                }
                None => {
                    remember_failure(&mut self.actor_preview_failed, key);
                }
            }
        }
    }

    pub fn get(&mut self, url: &str) -> Option<&TextureHandle> {
        self.clock = self.clock.wrapping_add(1);
        if let Some((texture, used)) = self.textures.get_mut(url) {
            *used = self.clock;
            return Some(texture);
        }
        if !self.pending.contains(url)
            && !self.failed.contains(url)
            && self.requests.try_send(url.to_owned()).is_ok()
        {
            self.pending.insert(url.to_owned());
        }
        None
    }

    pub fn animation_frame(
        &mut self,
        url: &str,
        speed: f32,
        playing: bool,
        looping: bool,
        smoothing: bool,
        ctx: &egui::Context,
    ) -> Option<&TextureHandle> {
        let quality = self.max_dimension.load(Ordering::Relaxed);
        let key = (url.to_owned(), quality);
        self.animation_target = Some(key.clone());
        if self
            .animation_preview
            .as_ref()
            .is_some_and(|preview| preview.url != url || preview.quality != quality)
        {
            self.animation_preview = None;
        }
        if !self
            .animation_preview
            .as_ref()
            .is_some_and(|preview| preview.url == url && preview.quality == quality)
            && !self.animation_pending.contains(&key)
            && !self.animation_failed.contains(&key)
            && self.animation_requests.try_send(key.clone()).is_ok()
        {
            self.animation_pending.insert(key);
        }
        let preview = self
            .animation_preview
            .as_mut()
            .filter(|preview| preview.url == url && preview.quality == quality)?;
        let index = preview.frame(speed, playing, looping, ctx);
        let image = preview.images.get(index)?;
        if preview.current_frame != index || preview.smoothing != smoothing {
            let options = if smoothing {
                TextureOptions::LINEAR
            } else {
                TextureOptions::NEAREST
            };
            preview.texture.set(image.clone(), options);
            preview.current_frame = index;
            preview.smoothing = smoothing;
        }
        Some(&preview.texture)
    }

    pub fn static_actor_preview(&mut self, url: &str, smoothing: bool) -> Option<&TextureHandle> {
        let quality = self.max_dimension.load(Ordering::Relaxed);
        let key = (url.to_owned(), quality);
        if self.actor_preview_target.as_ref() != Some(&key) {
            self.actor_preview_target = Some(key.clone());
            if self
                .actor_preview
                .as_ref()
                .is_some_and(|preview| preview.url != url || preview.quality != quality)
            {
                self.actor_preview = None;
            }
        }
        if let Some(preview) = self
            .actor_preview
            .as_mut()
            .filter(|preview| preview.url == url && preview.quality == quality)
        {
            if preview.smoothing != smoothing {
                let options = if smoothing {
                    TextureOptions::LINEAR
                } else {
                    TextureOptions::NEAREST
                };
                preview.texture.set(preview.image.clone(), options);
                preview.smoothing = smoothing;
            }
            return Some(&preview.texture);
        }
        if !self.actor_preview_pending.contains(&key)
            && !self.actor_preview_failed.contains(&key)
            && self.actor_preview_requests.try_send(key.clone()).is_ok()
        {
            self.actor_preview_pending.insert(key);
        }
        None
    }
}

fn remember_failure<K: Eq + std::hash::Hash>(failures: &mut HashSet<K>, key: K) {
    if failures.len() >= MAX_FAILED_TEXTURES {
        failures.clear();
    }
    failures.insert(key);
}

fn decode(path: &Path, max_dimension: usize) -> Option<ColorImage> {
    if !path.is_file() {
        return None;
    }
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"
    ) {
        return decode_video(path, max_dimension);
    }
    if extension == "svg" {
        let pixels = waniengine_wayland::rasterize_svg(path)
            .ok()?
            .thumbnail(max_dimension as u32, max_dimension as u32)
            .to_rgba8();
        return Some(ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            &pixels,
        ));
    }
    if path.metadata().ok()?.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let pixels = reader
        .decode()
        .ok()?
        .thumbnail(max_dimension as u32, max_dimension as u32)
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        &pixels,
    ))
}

// FFmpeg is already a host dependency of WaniEngine. A timeout and bounded
// stdout prevent a damaged video or an absent decoder from blocking the UI.
fn decode_video(path: &Path, max_dimension: usize) -> Option<ColorImage> {
    const MAX_FRAME_BYTES: u64 = 4 * 1024 * 1024;
    let mut child = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "error", "-threads", "1", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={max_dimension}:{max_dimension}:force_original_aspect_ratio=decrease"),
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_FRAME_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() as u64 <= MAX_FRAME_BYTES).then_some(bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(8);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let bytes = reader.join().ok()??;
    if !success {
        return None;
    }
    let pixels = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [pixels.width() as usize, pixels.height() as usize],
        &pixels,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_and_resizes_an_image() {
        let path =
            std::env::temp_dir().join(format!("waniengine-thumbnail-{}.png", std::process::id()));
        let image = image::RgbaImage::from_pixel(960, 540, image::Rgba([120, 40, 180, 255]));
        image.save(&path).unwrap();
        let decoded = decode(&path, 480).expect("decode thumbnail");
        std::fs::remove_file(path).unwrap();
        assert_eq!(decoded.size, [480, 270]);
        assert_eq!(decoded.pixels[0], egui::Color32::from_rgb(120, 40, 180));
    }

    #[test]
    fn decodes_a_video_frame_when_ffmpeg_is_available() {
        if Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
        {
            return;
        }
        let path =
            std::env::temp_dir().join(format!("waniengine-thumbnail-{}.mp4", std::process::id()));
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=purple:s=64x36:r=1",
                "-frames:v",
                "1",
                "-c:v",
                "mpeg4",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let decoded = decode(&path, 480).expect("decode video frame");
        std::fs::remove_file(path).unwrap();
        assert_eq!(decoded.size, [480, 270]);
    }

    #[test]
    fn rejects_missing_and_invalid_images() {
        assert!(decode(Path::new("/does/not/exist.png"), 480).is_none());
        assert!(decode(Path::new("/dev/null"), 480).is_none());
    }
}
