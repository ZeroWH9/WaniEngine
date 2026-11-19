#!/usr/bin/env python3
"""Exercise real video surfaces in a private KWin virtual compositor.

No connection to the user's display or D-Bus session is inherited. The report
measures first buffer commits and compositor frame callbacks, not UI fixtures.
"""
import argparse
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def read_event(worker, name, timeout=8, event_id=None):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ready, _, _ = select.select([worker.stdout], [], [], max(0, deadline - time.monotonic()))
        if not ready:
            break
        line = worker.stdout.readline()
        if not line:
            raise RuntimeError(f"Worker exited: {worker.stderr.read().decode()}")
        event = json.loads(line)
        if event.get("event") == "error":
            raise RuntimeError(event)
        if event.get("event") == name and (event_id is None or event.get("id") == event_id):
            return event
    raise RuntimeError(f"No {name} event within {timeout}s")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / "target/video-wallpaper-check")
    args = parser.parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--offline", "-p", "waniengine-egui"], cwd=ROOT, check=True)
    args.output.mkdir(parents=True, exist_ok=True)
    checks = []
    timings = []
    workers = []
    with tempfile.TemporaryDirectory(prefix="wani-video-test-") as folder:
        runtime = Path(folder)
        runtime.chmod(0o700)
        env = dict(os.environ)
        for key in ["DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS", "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE"]:
            env.pop(key, None)
        env.update(XDG_RUNTIME_DIR=folder, XDG_CONFIG_HOME=str(runtime / "config"), XDG_DATA_HOME=str(runtime / "data"),
                   XDG_STATE_HOME=str(runtime / "state"), XDG_CACHE_HOME=str(runtime / "cache"),
                   XDG_SESSION_TYPE="wayland", XDG_CURRENT_DESKTOP="KDE", DESKTOP_SESSION="plasma",
                   KWIN_COMPOSE="O2", QT_QPA_PLATFORM="offscreen", LIBGL_ALWAYS_SOFTWARE="1")
        long_video = runtime / "long.mp4"
        short_video = runtime / "short.mp4"
        for path, duration, size in [(long_video, "60", "1920x1080"), (short_video, "0.5", "640x360")]:
            subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i", f"testsrc2=size={size}:rate=30",
                            "-t", duration, "-c:v", "libx264", "-preset", "ultrafast", "-threads", "2", "-y", str(path)], check=True, env=env)
        invalid = runtime / "invalid.mp4"
        invalid.write_bytes(b"This is not a video file")
        # A successful run with this guard proves Apply never consults ffprobe.
        tools = runtime / "tools"
        tools.mkdir()
        probe = tools / "ffprobe"
        probe.write_text(f"#!/bin/sh\nprintf called > '{runtime / 'ffprobe-called'}'\nexit 99\n")
        probe.chmod(0o700)
        env["PATH"] = str(tools) + os.pathsep + env["PATH"]
        log = open(args.output / "kwin.log", "wb")
        compositor = subprocess.Popen(["dbus-run-session", "--", "kwin_wayland", "--virtual", "--width", "1920", "--height", "1080",
                                       "--socket", "wani-video", "--no-lockscreen", "--no-global-shortcuts", "--no-kactivities"],
                                      env=env, stdout=log, stderr=log, start_new_session=True)
        try:
            deadline = time.monotonic() + 12
            while not (runtime / "wani-video").exists():
                if compositor.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError((args.output / "kwin.log").read_text())
                time.sleep(0.02)
            env["WAYLAND_DISPLAY"] = "wani-video"
            binary = ROOT / "target/debug/waniengine-egui"
            result = subprocess.run([str(binary), "--overlay-worker", "--list-outputs"], env=env, capture_output=True, check=True)
            output = json.loads(result.stdout)[0]["name"]

            def start(path, *options):
                worker = subprocess.Popen([str(binary), "--overlay-worker", "--video", str(path), "--output", output,
                                           "--control-stdin", "--layer", "background", *options], env=env,
                                          stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0)
                workers.append(worker)
                return worker

            for label, path, options in [
                ("1080p60s-first-apply", long_video, ["--fit", "cover", "--fps", "30"]),
                ("1080p60s-switch", long_video, ["--fit", "contain", "--fps", "30", "--speed", "125"]),
                ("center", short_video, ["--fit", "center"]),
                ("stretch-paused", short_video, ["--fit", "stretch", "--paused"]),
            ]:
                before = time.monotonic()
                worker = start(path, *options)
                configured = read_event(worker, "configured")
                commit_ms = (time.monotonic() - before) * 1000
                read_event(worker, "presented")
                callback_ms = (time.monotonic() - before) * 1000
                assert configured["width"] == 1920 and configured["height"] == 1080, configured
                assert commit_ms < 1500, f"First frame too slow: {commit_ms:.1f}ms"
                timings.append(dict(scenario=label, first_commit_ms=round(commit_ms, 2), frame_callback_ms=round(callback_ms, 2)))
                print(json.dumps(timings[-1]), flush=True)
                for event_id, playing in [(1, False), (2, True)]:
                    worker.stdin.write((json.dumps(dict(action="update", id=event_id, playing=playing)) + "\n").encode())
                    worker.stdin.flush()
                    read_event(worker, "applied", event_id=event_id)
                checks.append(label + ": first frame, compositor callback, pause/resume")
                worker.stdin.close()
                assert worker.wait(timeout=2) == 0, worker.stderr.read().decode()
            worker = start(short_video, "--no-loop")
            read_event(worker, "configured")
            read_event(worker, "ended")
            assert worker.poll() is None, "Last frame should remain visible at EOF"
            worker.stdin.close()
            assert worker.wait(timeout=2) == 0
            checks.append("non-looping video holds its final frame")
            worker = start(short_video)
            read_event(worker, "configured")
            time.sleep(1.6)
            assert worker.poll() is None
            worker.stdin.write(b'{"action":"update","id":9,"playing":false}\n')
            worker.stdin.flush()
            read_event(worker, "applied", event_id=9)
            worker.stdin.close()
            assert worker.wait(timeout=2) == 0
            checks.append("loop remains running beyond three source durations")
            worker = start(invalid)
            assert worker.wait(timeout=8) != 0
            worker.stdin.close()
            detail = worker.stderr.read().decode()
            assert "FFmpeg" in detail and ("no video frames" in detail or "could not play" in detail), detail
            checks.append("invalid video reports decoder failure")
            subprocess.run(["cargo", "test", "--offline", "-p", "waniengine-egui", "--lib",
                            "native_wallpaper::tests::isolated_replacement_and_lifecycle", "--", "--ignored", "--nocapture"],
                           cwd=ROOT, env={**env, "WANIENGINE_VIDEO_TEST_EXE": str(binary), "WANIENGINE_VIDEO_TEST_SOURCE": str(long_video),
                                         "WANIENGINE_VIDEO_TEST_OUTPUT": output, "WANIENGINE_VIDEO_TEST_INVALID": str(invalid)}, check=True, timeout=90)
            checks.append("real manager: replacement rollback, pause/resume, worker and decoder cleanup")
            assert not (runtime / "ffprobe-called").exists()
            assert not list(runtime.rglob("*.webp"))
            checks.append("no ffprobe calls or converted WebP files")
        finally:
            for worker in workers:
                if worker.poll() is None:
                    worker.kill()
                    worker.wait()
            os.killpg(compositor.pid, signal.SIGTERM)
            compositor.wait(timeout=5)
            log.close()
    report = dict(compositor="private KWin virtual, 1920x1080", source="synthetic H.264, 60s 1080p and 0.5s 360p",
                  desktop_session_accessed=False, composition_adapter_validated=False, checks=checks, timings=timings)
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
