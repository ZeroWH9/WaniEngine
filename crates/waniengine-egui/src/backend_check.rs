//! Exercises production services only inside an explicitly private test session.
use serde_json::json;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use waniengine_egui::{
    desktop::{GNOME_OUTPUT, WallpaperController},
    native_wallpaper::VideoSettings,
};

pub fn run(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    if !root.starts_with("/tmp")
        || !root
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("wani-backend-check."))
    {
        return Err("Verification requires a private /tmp/wani-backend-check.* directory".into());
    }
    for key in [
        "HOME",
        "XDG_RUNTIME_DIR",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ] {
        if !std::env::var_os(key).is_some_and(|v| Path::new(&v).starts_with(&root)) {
            return Err(format!("{key} must be private to this test"));
        }
    }
    if std::env::var("WANIENGINE_ISOLATED_TEST_ROOT")
        .ok()
        .as_deref()
        != root.to_str()
    {
        return Err("Missing isolated verification guard".into());
    }
    let controller = WallpaperController::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !controller.capabilities_ready() {
        if Instant::now() > deadline {
            return Err("Capabilities probe timed out".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    if std::env::var_os("WANIENGINE_EXPECT_UNSUPPORTED").is_some() {
        if controller.supports_images() || controller.supports_video() {
            return Err("Unsupported desktop advertised a wallpaper capability".into());
        }
        if controller
            .apply(
                "test",
                (0, 0),
                (800, 600),
                &root.join("still.png"),
                "cover",
                false,
            )
            .is_ok()
        {
            return Err("Unsupported desktop accepted Apply".into());
        }
        std::fs::write(root.join("report.json"), b"{\"unsupported_refused\":true}")
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    if !controller.supports_images() || !controller.supports_video() {
        return Err("Desktop capabilities unavailable in private session".into());
    }
    let outputs = if controller.is_gnome() {
        waniengine_wayland::x11_wallpaper_outputs()?
    } else {
        let result =
            std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                .args(["--overlay-worker", "--list-outputs"])
                .output()
                .map_err(|e| e.to_string())?;
        serde_json::from_slice(&result.stdout).map_err(|e| e.to_string())?
    };
    let first = outputs.first().ok_or("No private outputs")?;
    let pair = |key: &str| {
        let v = &first[key];
        (
            v[0].as_i64().unwrap_or(0) as i32,
            v[1].as_i64().unwrap_or(0) as i32,
        )
    };
    let output = if controller.is_gnome() {
        GNOME_OUTPUT
    } else {
        first["name"].as_str().ok_or("Unnamed output")?
    };
    let position = pair("logicalPosition");
    let size = pair("logicalSize");
    let source = root.join("video.mp4");
    let image = root.join("still.png");
    controller.apply(output, position, size, &image, "cover", false)?;
    let visual = std::env::var_os("WANIENGINE_VERIFY_COMPOSITION").is_some();
    let before = if visual {
        thread::sleep(Duration::from_millis(800));
        Some(private_snapshot(&root, "before-video")?)
    } else {
        None
    };
    let settings = VideoSettings {
        duration: Some(3.0),
        speed: 100.0,
        fps: 15,
        looping: true,
        playing: true,
    };
    let mut timings = Vec::new();
    let mut first_visible_upper_bound_ms = None;
    let swaps = std::env::var("WANIENGINE_TEST_SWAPS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(3, 1000);
    for index in 0..swaps {
        eprintln!("Adapter apply {index}");
        let now = Instant::now();
        controller.apply_video(output, position, size, &source, "cover", settings)?;
        timings.push(now.elapsed().as_secs_f64() * 1000.0);
        if index == 0 {
            if let Some(before) = &before {
                loop {
                    let visible = private_snapshot(&root, "first-visible")?;
                    match check_composition(before, &visible, size, controller.is_gnome()) {
                        Ok(()) => {
                            first_visible_upper_bound_ms =
                                Some(now.elapsed().as_secs_f64() * 1000.0);
                            break;
                        }
                        Err(error) if now.elapsed() >= Duration::from_secs(2) => return Err(error),
                        Err(_) => thread::sleep(Duration::from_millis(10)),
                    }
                }
            }
        }
    }
    if let Some(before) = &before {
        thread::sleep(Duration::from_millis(700));
        let after = private_snapshot(&root, "with-video")?;
        check_composition(before, &after, size, controller.is_gnome())?;
    }
    controller.set_animation_enabled(output, position, size, false)?;
    let paused = controller
        .video_position(output)
        .ok_or("Missing video clock")?;
    thread::sleep(Duration::from_millis(120));
    if (controller.video_position(output).unwrap_or(0.0) - paused).abs() > 0.02 {
        return Err("Video clock advances while paused".into());
    }
    controller.set_fps(10)?;
    controller.set_animation_enabled(output, position, size, true)?;
    let bad = controller.apply_video(
        output,
        position,
        size,
        &root.join("invalid.mp4"),
        "cover",
        settings,
    );
    if bad.is_ok() || controller.video_position(output).is_none() {
        return Err("Failed replacement discarded active wallpaper".into());
    }
    crate::video_wallpaper::verify_transport(&source)?;
    let finite_video = std::env::var_os("WANIENGINE_TEST_FINITE_VIDEO").is_some();
    if finite_video {
        let finite = VideoSettings {
            looping: false,
            ..settings
        };
        controller.apply_video(output, position, size, &source, "cover", finite)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while !controller.video_finished(output) {
            if let Some((_, error)) = controller.take_video_error() {
                return Err(error);
            }
            if Instant::now() > deadline {
                return Err("Non-looping video did not reach EOF".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let end = controller
            .video_position(output)
            .ok_or("Missing EOF clock")?;
        thread::sleep(Duration::from_millis(200));
        if (controller.video_position(output).unwrap_or(0.0) - end).abs() > 0.02 {
            return Err("Media clock advances after EOF".into());
        }
        controller.set_animation_enabled(output, position, size, false)?;
        controller.set_animation_enabled(output, position, size, true)?;
        if !controller.video_finished(output) {
            return Err("Automatic resume restarted a non-looping video after EOF".into());
        }
        controller.set_fps(60)?;
        if controller.take_video_error().is_some() {
            return Err("FPS update discarded a completed video".into());
        }
        controller.apply_video(output, position, size, &source, "cover", settings)?;
        if controller.video_finished(output) {
            return Err("New Apply did not restart the source".into());
        }
    }
    // Hold the replacement shell until the controller has restored its state.
    let mut restarted_shell = None;
    let shell_restart =
        std::env::var_os("WANIENGINE_TEST_SHELL_RESTART").is_some() && !controller.is_gnome();
    if shell_restart {
        restarted_shell = Some(restart_private_shell(&root)?);
        controller.reconcile_outputs()?;
        if let Some((_, error)) = controller.take_video_error() {
            return Err(error);
        }
        if controller.video_position(output).is_none() {
            return Err("Shell restart discarded the video worker".into());
        }
        if let Some(before) = &before {
            let after = private_snapshot(&root, "after-shell-restart")?;
            check_composition(before, &after, size, false)?;
        }
    }
    let external_change =
        std::env::var_os("WANIENGINE_TEST_EXTERNAL_CHANGE").is_some() && !controller.is_gnome();
    if external_change {
        let external = root.join("external.png");
        std::fs::copy(&image, &external).map_err(|e| e.to_string())?;
        let script = format!(
            "var d=desktopForScreen(screenForConnector({}));d.wallpaperPlugin='org.kde.image';d.currentConfigGroup=['Wallpaper','org.kde.image','General'];d.writeConfig('Image',{});",
            json!(output),
            json!(waniengine_core::library::file_url(&external))
        );
        private_plasma_script(&script)?;
        controller.reconcile_outputs()?;
        if controller.video_position(output).is_some()
            || !controller
                .take_video_error()
                .is_some_and(|(_, e)| e.contains("outside"))
        {
            return Err("External wallpaper change did not retire its video worker".into());
        }
        let state = private_plasma_script(&format!(
            "var d=desktopForScreen(screenForConnector({}));d.currentConfigGroup=['Wallpaper','org.kde.image','General'];print(String(d.wallpaperPlugin)+' '+String(d.readConfig('Image','')));",
            json!(output)
        ))?;
        if !state.contains(&waniengine_core::library::file_url(&external)) {
            return Err("Retiring a worker overwrote the external wallpaper".into());
        }
        controller.apply_video(output, position, size, &source, "cover", settings)?;
    }
    let hotplug = std::env::var_os("WANIENGINE_TEST_HOTPLUG").is_some();
    if hotplug && !controller.is_gnome() {
        let second = outputs
            .get(1)
            .ok_or("Hotplug test requires two private outputs")?;
        let name = second["name"].as_str().ok_or("Unnamed hotplug output")?;
        let geometry = |value: &serde_json::Value, key: &str| {
            (
                value[key][0].as_i64().unwrap() as i32,
                value[key][1].as_i64().unwrap() as i32,
            )
        };
        eprintln!("Hotplug: apply second output {name}");
        controller.apply_video(
            name,
            geometry(second, "logicalPosition"),
            geometry(second, "logicalSize"),
            &source,
            "contain",
            settings,
        )?;
        eprintln!("Hotplug: disable {name}");
        configure_output(&format!("output.{name}.disable"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((failed, _)) = controller.take_video_error() {
                if failed != name {
                    return Err(format!("Hotplug stopped unrelated output {failed}"));
                }
                break;
            }
            if Instant::now() > deadline {
                return Err("Disconnected video worker did not exit".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        if controller.video_position(output).is_none() {
            return Err("Hotplug discarded the other display".into());
        }
        eprintln!("Hotplug: reconnect {name}");
        configure_output(&format!("output.{name}.enable"))?;
        thread::sleep(Duration::from_millis(300));
        let refreshed = private_wayland_outputs()?;
        let reconnected = refreshed
            .iter()
            .find(|value| value["name"] == name)
            .ok_or("Reconnected output is missing")?;
        eprintln!("Hotplug: reconnected geometry {reconnected}");
        controller.apply_video(
            name,
            geometry(reconnected, "logicalPosition"),
            geometry(reconnected, "logicalSize"),
            &source,
            "cover",
            settings,
        )?;
        eprintln!("Hotplug: scale {name}");
        configure_output(&format!("output.{name}.scale.1.25"))?;
        thread::sleep(Duration::from_millis(500));
        if let Some((_, error)) = controller.take_video_error() {
            return Err(format!("Output scale change failed: {error}"));
        }
        if visual {
            let after = private_snapshot(&root, "after-hotplug")?;
            check_hotplug_composition(&after, &private_wayland_outputs()?)?;
        }
        let refreshed = private_wayland_outputs()?;
        let scaled = refreshed
            .iter()
            .find(|value| value["name"] == name)
            .ok_or("Scaled output is missing")?;
        controller.apply(
            name,
            geometry(scaled, "logicalPosition"),
            geometry(scaled, "logicalSize"),
            &image,
            "cover",
            false,
        )?;
        configure_output(&format!("output.{name}.scale.1"))?;
    }
    if std::env::var_os("WANIENGINE_CRASH_FIXTURE").is_some() {
        let mut audio = crate::video_wallpaper::WallpaperAudio::default();
        let asset = json!({"url":waniengine_core::library::file_url(&source),"video":true,"hasAudio":true,"duration":3.0,"volume":30,"speed":100,"loop":true});
        audio.sync(
            Some(&asset),
            100.0,
            false,
            controller.video_position(output),
        );
        thread::sleep(Duration::from_millis(200));
        std::fs::write(root.join("crash-ready"), std::process::id().to_string())
            .map_err(|e| e.to_string())?;
        // The outer private runner deliberately SIGKILLs this fixture.
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    let soak = std::env::var("WANIENGINE_SOAK_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
        .min(30000);
    if soak > 0 {
        controller.set_fps(1)?;
        let mut collector = std::process::Command::new("python3")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../scripts/trace-wallpaper-memory.py"),
            )
            .args([
                "--pid",
                &std::process::id().to_string(),
                "--duration",
                &soak.to_string(),
                "--interval",
                "5",
                "--output",
            ])
            .arg(root.join("memory.jsonl"))
            .spawn()
            .map_err(|e| e.to_string())?;
        let began = Instant::now();
        let mut audio = crate::video_wallpaper::WallpaperAudio::default();
        let mut asset = json!({"url":waniengine_core::library::file_url(&source),"video":true,"hasAudio":true,"duration":3.0,"volume":30,"speed":100,"loop":true});
        while began.elapsed() < Duration::from_secs(soak) {
            asset["volume"] = json!(20 + (began.elapsed().as_secs() / 10) % 70);
            if let Some(error) = audio.sync(
                Some(&asset),
                100.0,
                false,
                controller.video_position(output),
            ) {
                return Err(error);
            }
            if let Some((_, error)) = controller.take_video_error() {
                return Err(error);
            }
            thread::sleep(Duration::from_millis(100));
        }
        audio.stop();
        let _ = collector.wait();
    }
    controller.apply(output, position, size, &image, "contain", false)?;
    if controller.video_position(output).is_some() {
        return Err("Video worker survived image replacement".into());
    }
    drop(controller);
    if let Some(child) = &mut restarted_shell {
        let _ = child.0.kill();
        let _ = child.0.wait();
    }
    std::fs::write(root.join("report.json"),serde_json::to_vec_pretty(&json!({"production_backend":true,"composition_adapter_exercised":true,"visual_icons_verified":visual,"hotplug_verified":hotplug,"finite_video_verified":finite_video,"shell_restart_verified":shell_restart,"external_change_verified":external_change,"apply_ms":timings,"first_visible_upper_bound_ms":first_visible_upper_bound_ms,"soak_seconds":soak,"checks":["still image","real desktop video adapter","pause clock","live FPS","failed replacement retains video","PCM volume retains transport","video to image","normal cleanup"]})).unwrap()).map_err(|e|e.to_string())?;
    Ok(())
}

struct PrivateShell(std::process::Child);
impl Drop for PrivateShell {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn private_plasma_script(script: &str) -> Result<String, String> {
    let output = waniengine_core::process::capture(
        std::process::Command::new("qdbus6").args([
            "org.kde.plasmashell",
            "/PlasmaShell",
            "org.kde.PlasmaShell.evaluateScript",
            script,
        ]),
        Duration::from_secs(5),
    )
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn restart_private_shell(root: &Path) -> Result<PrivateShell, String> {
    let pid = std::env::var("WANIENGINE_TEST_SHELL_PID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or("Missing private shell PID")?;
    let environment = std::fs::read(format!("/proc/{pid}/environ")).map_err(|e| e.to_string())?;
    let expected = format!("WANIENGINE_ISOLATED_TEST_ROOT={}", root.display());
    if !environment
        .split(|v| *v == 0)
        .any(|v| v == expected.as_bytes())
    {
        return Err("Refusing to restart a shell outside this private test".into());
    }
    let command = std::fs::read(format!("/proc/{pid}/cmdline")).map_err(|e| e.to_string())?;
    if !command
        .split(|v| *v == 0)
        .next()
        .is_some_and(|v| v.ends_with(b"plasmashell"))
    {
        return Err("Private shell PID no longer names plasmashell".into());
    }
    let result = std::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .map_err(|e| e.to_string())?;
    if !result.success() {
        return Err("Could not stop the private shell".into());
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while private_plasma_script("print('ready')").is_ok() {
        if Instant::now() > deadline {
            return Err("Private shell did not stop".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let log =
        std::fs::File::create(root.join("plasma-restarted.log")).map_err(|e| e.to_string())?;
    let child = PrivateShell(
        std::process::Command::new("plasmashell")
            .arg("--no-respawn")
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| e.to_string())?,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !private_plasma_script("print(desktops().length)")
        .is_ok_and(|v| v.trim().parse::<usize>().is_ok_and(|n| n > 0))
    {
        if Instant::now() > deadline {
            return Err("Private shell did not restart".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    thread::sleep(Duration::from_secs(3));
    Ok(child)
}

fn configure_output(option: &str) -> Result<(), String> {
    let result = waniengine_core::process::capture(
        std::process::Command::new("kscreen-doctor").arg(option),
        Duration::from_secs(10),
    )
    .map_err(|e| e.to_string())?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Private output configuration failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ))
    }
}

fn private_wayland_outputs() -> Result<Vec<serde_json::Value>, String> {
    let result = waniengine_core::process::capture(
        std::process::Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(["--overlay-worker", "--list-outputs"]),
        Duration::from_secs(3),
    )
    .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into());
    }
    serde_json::from_slice(&result.stdout).map_err(|e| e.to_string())
}

fn private_snapshot(root: &Path, name: &str) -> Result<image::RgbaImage, String> {
    let path = root.join(format!("{name}.png"));
    let mut command = if let Some(helper) = std::env::var_os("WANIENGINE_TEST_SCREENSHOT") {
        let helper = std::path::PathBuf::from(helper)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !helper.starts_with(root) {
            return Err("Capture helper must belong to the private test root".into());
        }
        std::process::Command::new(helper)
    } else {
        let mut command = std::process::Command::new("spectacle");
        command.args(["-f", "--scaled", "-b", "-n", "-o"]);
        command
    };
    let result = waniengine_core::process::capture(command.arg(&path), Duration::from_secs(20))
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "Private compositor capture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    image::open(path)
        .map(|i| i.to_rgba8())
        .map_err(|e| e.to_string())
}

fn check_hotplug_composition(
    after: &image::RgbaImage,
    outputs: &[serde_json::Value],
) -> Result<(), String> {
    let (mut icons, mut panels) = (0, 0);
    for output in outputs {
        let x = output["logicalPosition"][0].as_i64().unwrap() as u32;
        let y = output["logicalPosition"][1].as_i64().unwrap() as u32;
        let w = output["logicalSize"][0].as_i64().unwrap() as u32;
        let h = output["logicalSize"][1].as_i64().unwrap() as u32;
        if x + w > after.width() || y + h > after.height() {
            return Err("Scaled capture does not match logical outputs".into());
        }
        let (mut visible, mut total) = (0, 0);
        for py in (y + h / 3..y + h * 2 / 3).step_by(16) {
            for px in (x + w / 3..x + w * 2 / 3).step_by(16) {
                let pixel = after.get_pixel(px, py);
                total += 1;
                if pixel[3] > 250 && pixel.0[..3].iter().map(|v| *v as u32).sum::<u32>() > 80 {
                    visible += 1;
                }
            }
        }
        if visible * 2 < total {
            return Err(format!("Video hidden after hotplug on {}", output["name"]));
        }
        // Plasma legitimately moves the containment (icons and panel) to the
        // newly primary output. Look for the fixtures on either output.
        for py in y..(y + 180).min(y + h) {
            for px in x..(x + 180).min(x + w) {
                if after.get_pixel(px, py).0[..3].iter().all(|v| *v > 210) {
                    icons += 1;
                }
            }
        }
        for py in y + h.saturating_sub(50)..y + h {
            for px in x..x + w {
                if after.get_pixel(px, py).0[..3].iter().all(|v| *v > 210) {
                    panels += 1;
                }
            }
        }
    }
    if icons < 30 || panels < 30 {
        return Err(format!(
            "Desktop fixtures missing after hotplug: icons={icons}, panel={panels}"
        ));
    }
    Ok(())
}
fn check_composition(
    before: &image::RgbaImage,
    after: &image::RgbaImage,
    size: (i32, i32),
    gnome: bool,
) -> Result<(), String> {
    let (w, h) = (size.0 as u32, size.1 as u32);
    if before.dimensions() != after.dimensions() || w > before.width() || h > before.height() {
        return Err("Private capture geometry differs".into());
    }
    let mut changed = 0;
    let mut visible_video = 0;
    let mut total = 0;
    for y in (h / 3..h * 2 / 3).step_by(8) {
        for x in (w / 3..w * 2 / 3).step_by(8) {
            total += 1;
            let a = before.get_pixel(x, y);
            let b = after.get_pixel(x, y);
            if b[3] > 250 && b.0[..3].iter().map(|c| *c as u32).sum::<u32>() > 80 {
                visible_video += 1;
            }
            if a.0[..3]
                .iter()
                .zip(&b.0[..3])
                .any(|(a, b)| a.abs_diff(*b) > 40)
            {
                changed += 1;
            }
        }
    }
    if changed * 4 < total || visible_video * 2 < total {
        return Err("Video fixture is hidden or the desktop has only become transparent".into());
    }
    let regions = if gnome {
        [
            (0, 32, 200, 220, "desktop icon"),
            (0, 0, w, 32, "panel widgets"),
        ]
    } else {
        [
            (0, 0, 180, 180, "desktop icon"),
            (0, h.saturating_sub(50), w, h, "panel widgets"),
        ]
    };
    for (left, top, right, bottom, label) in regions {
        let mut bright = 0;
        let mut preserved = 0;
        for y in top..bottom.min(h) {
            for x in left..right.min(w) {
                let a = before.get_pixel(x, y);
                if a.0[..3].iter().all(|v| *v > 210) {
                    bright += 1;
                    if after.get_pixel(x, y).0[..3].iter().all(|v| *v > 180) {
                        preserved += 1;
                    }
                }
            }
        }
        if bright < 30 || preserved * 100 < bright * 70 {
            return Err(format!(
                "The {label} fixture was absent or covered ({preserved}/{bright} bright pixels preserved)"
            ));
        }
    }
    Ok(())
}
