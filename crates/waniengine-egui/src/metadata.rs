//! Read media dimensions and stream metadata without blocking the UI thread.
use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn describe(path: &Path, video: bool) -> Value {
    if video {
        return probe_video(path).unwrap_or_else(|| json!({}));
    }
    let Some((width, height, animated)) = image_info(path) else {
        return json!({});
    };
    json!({"width":width,"height":height,"animated":animated,"hasAudio":false})
}

fn image_info(path: &Path) -> Option<(u32, u32, bool)> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        let image = waniengine_wayland::rasterize_svg(path).ok()?;
        return Some((image.width(), image.height(), false));
    }
    use image::ImageDecoder;
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let format = reader.format()?;
    let dimensions = reader.into_dimensions().ok()?;
    let animated = match format {
        image::ImageFormat::Gif => true,
        image::ImageFormat::Png => {
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(16384);
            limits.max_image_height = Some(16384);
            limits.max_alloc = Some(64 * 1024 * 1024);
            image::codecs::png::PngDecoder::with_limits(
                BufReader::new(File::open(path).ok()?),
                limits,
            )
            .ok()?
            .is_apng()
            .ok()?
        }
        image::ImageFormat::WebP => {
            let mut decoder =
                image::codecs::webp::WebPDecoder::new(BufReader::new(File::open(path).ok()?))
                    .ok()?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(16384);
            limits.max_image_height = Some(16384);
            limits.max_alloc = Some(64 * 1024 * 1024);
            decoder.set_limits(limits).ok()?;
            decoder.has_animation()
        }
        _ => false,
    };
    Some((dimensions.0, dimensions.1, animated))
}

fn probe_video(path: &Path) -> Option<Value> {
    const MAX_OUTPUT: u64 = 1024 * 1024;
    let mut child = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,width,height,avg_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_OUTPUT).then_some(bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
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
    parse_probe(&serde_json::from_slice::<Value>(&bytes).ok()?)
}

fn parse_probe(probe: &Value) -> Option<Value> {
    let streams = probe["streams"].as_array()?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")?;
    let width = video["width"].as_u64().filter(|&value| value > 0)?;
    let height = video["height"].as_u64().filter(|&value| value > 0)?;
    let has_audio = streams.iter().any(|stream| stream["codec_type"] == "audio");
    let mut result = json!({"width":width,"height":height,"animated":true,"hasAudio":has_audio});
    if let Some(codec) = video["codec_name"].as_str() {
        result["codec"] = json!(codec);
    }
    if let Some(rate) = video["avg_frame_rate"].as_str().and_then(parse_rate) {
        result["nativeFps"] = json!(rate);
    }
    if let Some(duration) = probe["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v >= 0.0)
    {
        result["duration"] = json!(duration);
    }
    Some(result)
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (numerator, denominator) = rate.split_once('/')?;
    let numerator: f64 = numerator.parse().ok()?;
    let denominator: f64 = denominator.parse().ok()?;
    let rate = numerator / denominator;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_animated_png_and_webp() {
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
            let path =
                std::env::temp_dir().join(format!("wani-info-{}.{extension}", std::process::id()));
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
            assert!(command.arg(&path).status().unwrap().success());
            let metadata = describe(&path, false);
            std::fs::remove_file(path).unwrap();
            assert_eq!(metadata["width"], 64, "{codec}");
            assert_eq!(metadata["height"], 64, "{codec}");
            assert_eq!(metadata["animated"], true, "{codec}");
        }
    }

    #[test]
    fn probes_real_video_stream() {
        if Command::new("ffprobe")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_err()
            || Command::new("ffmpeg")
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_err()
        {
            return;
        }
        let path =
            std::env::temp_dir().join(format!("waniengine-metadata-{}.mp4", std::process::id()));
        let status = Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x36:r=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=8000",
                "-t",
                "1",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-y",
            ])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        let metadata = describe(&path, true);
        std::fs::remove_file(path).unwrap();
        assert_eq!(metadata["width"], 64);
        assert_eq!(metadata["height"], 36);
        assert_eq!(metadata["hasAudio"], true);
        assert_eq!(metadata["codec"], "mpeg4");
        assert_eq!(metadata["nativeFps"], 2.0);
    }

    #[test]
    fn parses_video_metadata_and_ignores_invalid_rates() {
        let info = json!({"streams":[{"codec_type":"audio"},{"codec_type":"video","codec_name":"h264","width":1920,"height":1080,"avg_frame_rate":"30000/1001"}],"format":{"duration":"12.5"}});
        let metadata = parse_probe(&info).unwrap();
        assert_eq!(metadata["width"], 1920);
        assert_eq!(metadata["hasAudio"], true);
        assert_eq!(metadata["duration"], 12.5);
        assert!((metadata["nativeFps"].as_f64().unwrap() - 29.970029).abs() < 0.001);
        assert_eq!(parse_rate("0/0"), None);
        assert_eq!(parse_rate("nan/1"), None);
        assert!(parse_probe(&json!({"streams":[{"codec_type":"audio"}]})).is_none());
    }
}
