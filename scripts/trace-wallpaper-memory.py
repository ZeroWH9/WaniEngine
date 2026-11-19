#!/usr/bin/env python3
"""Read-only, bounded memory trace for an already running WaniEngine process.

Starts no renderer, decoder or GPU workload. RAM and GPU counters are distinct;
unavailable GPU counters are recorded as unavailable, never as zero usage.
"""
import argparse
import json
import os
from pathlib import Path
import time

PROC = Path("/proc")
DESKTOPS = {"plasmashell", "kwin_wayland", "kwin_x11", "gnome-shell", "Xwayland"}


def identity(pid):
    try:
        folder = PROC / str(pid)
        if folder.stat().st_uid != os.getuid():
            return None
        stat = (folder / "stat").read_text()
        end = stat.rfind(")")
        fields = stat[end + 2:].split()
        return dict(pid=pid, comm=stat[stat.find("(") + 1:end],
                    state=fields[0], ppid=int(fields[1]), start=fields[19])
    except (OSError, ValueError, IndexError):
        return None


def descendants(root):
    pending, seen = [root], set()
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        try:
            # A worker may be spawned by any manager thread, not just main.
            for thread in (PROC / str(pid) / "task").iterdir():
                try:
                    pending.extend(int(p) for p in (thread / "children").read_text().split())
                except (OSError, ValueError):
                    pass
        except OSError:
            pass
    return seen


def desktop_pids():
    result = set()
    for folder in PROC.iterdir():
        if folder.name.isdecimal():
            item = identity(int(folder.name))
            if item and item["comm"] in DESKTOPS:
                result.add(item["pid"])
    return result


def byte_value(value):
    fields = value.split()
    try:
        amount = int(fields[0])
        unit = fields[1] if len(fields) > 1 else "B"
        factor = {"B": 1, "KiB": 1024, "MiB": 1048576}.get(unit)
        return amount * factor if factor is not None else None
    except (IndexError, ValueError):
        return None


def resources(pid):
    folder = PROC / str(pid)
    ram, shared, gpu = {}, {}, {}
    denied = False
    try:
        for line in (folder / "status").read_text().splitlines():
            key, _, value = line.partition(":")
            if key in {"VmRSS", "VmHWM", "RssAnon", "RssFile", "RssShmem"}:
                ram[key + "_kib"] = int(value.split()[0])
        fds = list((folder / "fd").iterdir())
    except (OSError, ValueError, IndexError):
        return dict(ram_kib=ram, unavailable=True)
    for fd in fds:
        try:
            target = os.readlink(fd)
            if "memfd:" in target or target.startswith("/dev/shm/"):
                stat = fd.stat()
                shared[(stat.st_dev, stat.st_ino)] = dict(name=target, size_bytes=stat.st_size)
            if not target.startswith(("/dev/dri/", "/dev/nvidia")):
                continue
            fields = {}
            for line in (folder / "fdinfo" / fd.name).read_text().splitlines():
                key, _, value = line.partition(":")
                if key.startswith("drm-"):
                    fields[key] = value.strip()
            if not fields:
                continue
            client = (fields.get("drm-driver"), fields.get("drm-pdev"), fields.get("drm-client-id"))
            # Without a client id, do not invent a total by adding aliased FDs.
            key = client if client[2] is not None else ("fd", fd.name)
            memory = {name: byte_value(value) for name, value in fields.items()
                      if name.startswith(("drm-memory-", "drm-total-", "drm-shared-", "drm-resident-", "drm-purgeable-", "drm-active-"))
                      and not name.startswith("drm-total-cycles-")
                      and byte_value(value) is not None}
            gpu[key] = dict(device=target, client=fields.get("drm-client-id"),
                            counters=fields, memory_bytes=memory)
        except OSError:
            denied = True
    return dict(ram_kib=ram, fd_count=len(fds), shared_files=list(shared.values()),
                gpu_clients=list(gpu.values()), gpu_memory_available=any(g["memory_bytes"] for g in gpu.values()),
                some_fd_data_unavailable=denied)


def command(pid):
    try:
        return [v.decode(errors="replace") for v in (PROC / str(pid) / "cmdline").read_bytes().split(b"\0") if v]
    except OSError:
        return []


def append(path, item, maximum):
    if path.exists() and path.stat().st_size >= maximum:
        path.replace(path.with_suffix(path.suffix + ".1"))
    data = (json.dumps(item, ensure_ascii=False) + "\n").encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    with os.fdopen(fd, "ab") as output:
        output.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, help="manager PID; auto-detects a single Rust manager when omitted")
    parser.add_argument("--duration", type=float, default=7200, help="seconds; default two hours")
    parser.add_argument("--interval", type=float, default=5, help="seconds, minimum one")
    parser.add_argument("--desktop", action="store_true", help="also observe this user's desktop compositor/shell processes")
    parser.add_argument("--desktop-pid", type=int, action="append", default=[], help="observe only these explicit compositor/shell PIDs (repeatable; useful for private tests)")
    parser.add_argument("--output", type=Path, default=Path("target/memory-trace.jsonl"))
    parser.add_argument("--max-mib", type=int, default=16, help="per-file size, with one rotated backup")
    args = parser.parse_args()
    selected_desktops = set(args.desktop_pid)
    for pid in selected_desktops:
        info = identity(pid)
        if not info or info['comm'] not in DESKTOPS:
            parser.error(f"Desktop PID {pid} is not an accessible compositor or shell")
    if args.pid is None:
        candidates = []
        for folder in PROC.iterdir():
            if not folder.name.isdecimal():
                continue
            pid = int(folder.name)
            info = identity(pid)
            argv = command(pid) if info else []
            if argv and Path(argv[0]).name == "waniengine-egui" and not any(flag in argv for flag in ["--overlay-worker", "--ui-preview", "--help", "--version"]):
                candidates.append(pid)
        if len(candidates) != 1:
            parser.error(f"Expected one running Rust manager; found {candidates}. Use --pid to choose.")
        args.pid = candidates[0]
    root = identity(args.pid)
    if not root:
        parser.error("The manager PID is unavailable or belongs to another user")
    root_command = command(args.pid)
    if not root_command or "waniengine" not in Path(root_command[0]).name.lower() or "--overlay-worker" in root_command:
        parser.error("Choose the WaniEngine manager PID")
    if args.duration <= 0 or args.max_mib < 1 or args.interval < 1:
        parser.error("duration/max-mib must be positive; interval must be at least one second")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    maximum = args.max_mib * 1024 * 1024
    known, baseline, last = {}, {}, {}
    desktop = selected_desktops.copy()
    started, refreshed = time.monotonic(), 0.0
    root_gone = None
    append(args.output, dict(event="trace-start", manager_pid=args.pid, started_unix=time.time(),
                            ram_unit="KiB", gpu_unit="bytes", gpu_counters_not_additive_across_processes=True,
                            note="Desktop GPU totals include other applications; missing driver counters are not zero usage."), maximum)
    try:
        while time.monotonic() - started < args.duration:
            now = time.monotonic()
            current_root = identity(args.pid)
            root_alive = current_root and current_root["start"] == root["start"] and current_root["state"] not in {"Z", "X"}
            tree = descendants(args.pid) if root_alive else set()
            if not root_alive and root_gone is None:
                root_gone = now
            if args.desktop and now - refreshed > 30:
                desktop, refreshed = desktop_pids() | selected_desktops, now
            # Keep following former descendants, including reparented orphans.
            tracked = tree | set(known) | desktop
            current, renderers = {}, {}
            for pid in sorted(tracked):
                info = identity(pid)
                if not info or (pid not in tree and pid in known and info["start"] != known[pid]):
                    continue
                if pid not in tree and pid not in known and pid not in desktop:
                    continue
                argv = command(pid)
                if pid in tree:
                    known[pid] = info["start"]
                info["command"] = argv
                info["role"] = "desktop" if pid in desktop else ("manager" if pid == args.pid else ("renderer" if "--video" in argv else "decoder" if info["comm"] == "ffmpeg" else "child"))
                info["detached_from_manager"] = pid in known and pid not in tree
                info.update(resources(pid))
                rss = info["ram_kib"].get("VmRSS_kib")
                key = (pid, info["start"])
                if rss is not None:
                    baseline.setdefault(key, rss)
                    info["rss_growth_kib"] = rss - baseline[key]
                if info["role"] == "renderer" and "--output" in argv:
                    index = argv.index("--output") + 1
                    if index < len(argv):
                        renderers.setdefault(argv[index], []).append(pid)
                current[key] = info
            born = [current[key] for key in current.keys() - last.keys()]
            gone = [dict(pid=key[0], start=key[1], role=last[key]["role"]) for key in last.keys() - current.keys()]
            append(args.output, dict(event="sample", elapsed_s=round(now-started, 3), unix_time=time.time(),
                                    started=[dict(pid=p["pid"], role=p["role"], command=p["command"]) for p in born],
                                    exited=gone, video_pids_by_output=renderers, processes=list(current.values())), maximum)
            last = current
            if root_gone is not None and now - root_gone >= 15:
                break
            time.sleep(min(args.interval, max(0, args.duration - (time.monotonic()-started))))
    except KeyboardInterrupt:
        pass
    finally:
        append(args.output, dict(event="trace-end", elapsed_s=round(time.monotonic()-started, 3),
                                remaining_detached=[p["pid"] for p in last.values() if p["detached_from_manager"]]), maximum)
        print(f"Memory trace saved: {args.output} (and at most one rotated .1 file)")


if __name__ == "__main__":
    main()
