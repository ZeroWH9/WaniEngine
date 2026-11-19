use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use waniengine_egui::overlay::{ActorSettings, OverlayManager, WorkerStatus};

fn wait_for(manager: &mut OverlayManager, mut predicate: impl FnMut(&OverlayManager) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        manager.poll();
        if predicate(manager) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("overlay controller timed out: {:?}", manager.notice);
}

#[test]
#[ignore = "requires the private compositor from scripts/test-wayland-runtime.sh"]
fn isolated_controller_starts_updates_and_reaps_multiple_workers() {
    assert_eq!(
        std::env::var("WANIENGINE_OVERLAY_INTEGRATION").as_deref(),
        Ok("1"),
        "Run via scripts/test-wayland-runtime.sh"
    );
    assert_eq!(std::env::var("WAYLAND_DISPLAY").unwrap(), "wani-runtime");
    assert!(
        std::env::var("XDG_RUNTIME_DIR")
            .unwrap()
            .starts_with("/tmp/waniengine-wayland-test.")
    );
    let image = PathBuf::from(std::env::var_os("WANIENGINE_TEST_ACTOR").unwrap());
    let output = std::env::var("WANIENGINE_TEST_OUTPUT").unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_waniengine-egui"));
    let mut manager = OverlayManager::new(executable, eframe::egui::Context::default());
    manager.refresh_monitors();
    wait_for(&mut manager, |manager| !manager.loading_monitors);
    assert!(
        manager
            .monitors
            .iter()
            .any(|monitor| monitor.name.as_deref() == Some(&output)),
        "{:?}",
        manager.notice
    );
    manager
        .start("first", &image, &output, ActorSettings::default())
        .unwrap();
    manager
        .start(
            "second",
            &image,
            &output,
            ActorSettings {
                x: 0.7,
                ..Default::default()
            },
        )
        .unwrap();
    wait_for(&mut manager, |manager| {
        ["first", "second"].iter().all(|key| {
            manager
                .worker(key)
                .is_some_and(|worker| worker.status == WorkerStatus::Running)
        })
    });
    let changed = ActorSettings {
        x: 0.25,
        scale: 150.0,
        opacity: 40.0,
        ..Default::default()
    };
    manager.update("first", changed).unwrap();
    wait_for(&mut manager, |manager| {
        manager
            .worker("first")
            .is_some_and(|worker| worker.confirmed == changed)
    });
    // Dragging a slider sends updates faster than a compositor can configure
    // surfaces; the latest settings must eventually win.
    let intermediate = ActorSettings {
        x: 0.35,
        scale: 175.0,
        ..changed
    };
    let latest = ActorSettings {
        x: 0.45,
        scale: 200.0,
        ..changed
    };
    manager.update("first", intermediate).unwrap();
    manager.update("first", latest).unwrap();
    wait_for(&mut manager, |manager| {
        manager
            .worker("first")
            .is_some_and(|worker| worker.confirmed == latest)
    });
    assert_eq!(manager.worker("second").unwrap().confirmed.x, 0.7);
    manager.stop("first").unwrap();
    wait_for(&mut manager, |manager| manager.worker("first").is_none());
    let second_pid = manager.worker("second").unwrap().pid;
    drop(manager);
    let deadline = Instant::now() + Duration::from_secs(5);
    while PathBuf::from(format!("/proc/{second_pid}")).exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !PathBuf::from(format!("/proc/{second_pid}")).exists(),
        "worker survived its manager"
    );
}
