# WaniEngine

[![BSD 3-Clause License](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](LICENSE)

WaniEngine is an open-source desktop customization app for animated wallpapers and interactive desktop characters. It brings a local media library, playback controls, and native desktop integration together in one app.

![WaniEngine project artwork](assets/waniengine-cover.png)

If WaniEngine is useful to you, consider starring the repository.

> **Project status:** WaniEngine **0.2.0** is under active development. Features and behavior may change before 1.0.

## Features

- Import and organize local image, animated image, and video wallpapers, plus transparent desktop characters.
- Preview and adjust playback, fit, position, scale, opacity, rotation, audio, and display settings.
- Save playlists and restore wallpapers independently for each connected monitor.
- Render click-through Wayland character overlays above normal and full-screen windows.
- Reduce oversized videos to the physical pixel dimensions of the selected display before playback, while keeping the original media unchanged.
- Install, update, and uninstall per user while preserving the local library and backups.
- Browse wallpapers from the local library in **Discover**.

## Run WaniEngine

WaniEngine supports **KDE Plasma 6 (Wayland)** and **GNOME** sessions. On GNOME the wallpaper is drawn through XWayland, so a GNOME session needs XWayland enabled. Hardware, graphics drivers, and available video codecs can affect playback.

## Install

Clone the repository and run the installer from its root:

```bash
./install.sh
```

WaniEngine installs for the current user and preserves your library when updating.

Update with `./update.sh`. Uninstall with `./install.sh --uninstall`; your library is kept.

## Requirements

- Rust/Cargo 1.88 or newer
- FFmpeg (`ffmpeg`, `ffprobe` and `ffplay`)
- SDL2
- Python 3
- A C compiler, `pkg-config` and `strip`

Install Rust with [rustup](https://rustup.rs/) if `cargo --version` is not
available. When using VS Code as a Flatpak, build from a host terminal or make
sure `~/.cargo/bin` is present in the terminal's `PATH`.

## Build from source

Clone the repository and compile the application:

```bash
git clone https://github.com/ZeroWH9/WaniEngine.git
cd WaniEngine
cargo build --release -p waniengine-egui
```

The executable is written to `target/release/waniengine-egui`.

## Development and validation

Run these checks from the repository root:

```bash
cargo test --workspace
python3 tests/test-installer.py
scripts/test-egui-desktop.sh
```

The desktop script starts its own isolated D-Bus, Wayland, KDE Plasma, and synthetic audio sessions for verification; it does not connect to your running desktop session.

## Roadmap

### Completed
- [x] Add desktop support beyond KDE Plasma, starting with GNOME.

### Planned
- [ ] Turn Discover into a wallpaper catalog that searches built-in sources automatically and supports user-configured sources.
- [ ] Let users preview catalog wallpapers, review their source and license, and import selected wallpapers into their local library.

## Contributing

Contributions are welcome. Report bugs and propose features through the repository's GitHub issue templates. For larger changes, open an issue first and include clear build or reproduction steps in your pull request.

## License

WaniEngine code is released under the [BSD 3-Clause License](LICENSE). Redistributions must retain the copyright notice, license conditions, and disclaimer. Protocol files and other third-party components retain the licenses included with their source. User-imported wallpapers and characters stay in the local library and are not included in the app package. Report security vulnerabilities privately to the repository maintainer through GitHub.

## Star history

<a href="https://www.star-history.com/?repos=ZeroWH9%2FWaniEngine&type=date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=ZeroWH9/WaniEngine&type=date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=ZeroWH9/WaniEngine&type=date" />
    <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=ZeroWH9/WaniEngine&type=date" />
  </picture>
</a>
