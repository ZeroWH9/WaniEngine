use cxx_qt_build::{CxxQtBuilder, QmlModule};
use std::{env, fs, path::PathBuf, process::Command};

fn run(command: &mut Command) {
    let status = command.status().expect("run Qt runtime build");
    assert!(status.success(), "Qt runtime build failed");
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let generated = PathBuf::from("qml/generated");
    fs::create_dir_all(manifest.join(&generated)).unwrap();
    let files = [
        "Main_redesign.qml",
        "PropertySlider_redesign.qml",
        "AnimaLibrary_redesign.qml",
        "ActorScene_redesign.qml",
        "Toggle_redesign.qml",
        "Button_redesign.qml",
        "Dialog_redesign.qml",
        "ComboBox_redesign.qml",
        "SpinBox_redesign.qml",
    ];
    for file in files {
        fs::copy(
            manifest.join("qml").join(file),
            manifest.join(&generated).join(file),
        )
        .unwrap();
    }

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let runtime = out.join("qt-runtime");
    run(Command::new("cmake")
        .arg("-S")
        .arg(manifest.join("platform/runtime"))
        .arg("-B")
        .arg(&runtime)
        .arg(format!(
            "-DWANIENGINE_PLATFORM_ROOT={}",
            manifest.join("platform").display()
        ))
        .arg("-DCMAKE_BUILD_TYPE=RelWithDebInfo"));
    run(Command::new("cmake")
        .arg("--build")
        .arg(&runtime)
        .arg("--parallel")
        .arg("2"));
    let runtime_root = runtime.join("qml");
    println!(
        "cargo:rustc-env=WANIENGINE_BUNDLED_RUNTIME_ROOT={}",
        runtime_root.display()
    );
    let services = [
        "actor_audio_controller",
        "application_controller",
        "desktop_integration",
        "gnome_integration",
        "media_optimizer",
        "overlay_controller",
        "plasma_integration",
        "plasma_scripts",
        "package_validator",
        "media_metadata",
    ];
    let mut cpp = Vec::new();
    for service in services {
        cpp.push(manifest.join("platform/qt").join(format!("{service}.cpp")));
        cpp.push(manifest.join("platform/qt").join(format!("{service}.h")));
    }
    let mut module = QmlModule::new("WaniEngine");
    for file in files {
        module = module.qml_file(generated.join(file));
    }
    let builder = CxxQtBuilder::new_qml_module(module)
        .file("src/bridge.rs")
        .cpp_files(cpp)
        .cpp_files([
            "platform/qt_host.cpp",
            "platform/services.cpp",
            "platform/services.hpp",
            "platform/verification.cpp",
        ])
        .qrc("platform/resources.qrc")
        .qt_module("Multimedia")
        .qt_module("DBus")
        .qt_module("Widgets")
        .qt_module("Network")
        .qt_module("OpenGL")
        .qt_module("Quick")
        .qt_module("Concurrent");
    let runtime_definition = format!("\"{}\"", runtime_root.display());
    let wallpaper_definition = format!("\"{}\"", runtime.join("wallpaper").display());
    let version_definition = format!("\"{}\"", env!("CARGO_PKG_VERSION"));

    let layer_shell = pkg_config(&["layershellqtinterface", "LayerShellQtInterface"]);
    let kf6_config = pkg_config(&["KF6ConfigGui", "KF6ConfigCore", "KF6ConfigQml"]);
    let x11 = pkg_config(&["x11"]);

    unsafe {
        builder
            .cc_builder(|cc| {
                cc.include(&manifest)
                    .include(manifest.join("platform/qt"));
                if let Some(flags) = &layer_shell {
                    for path in &flags.includes {
                        cc.include(path);
                    }
                    cc.define("WANIENGINE_HAS_LAYER_SHELL_QT", "1");
                }
                if let Some(flags) = &kf6_config {
                    for path in &flags.includes {
                        cc.include(path);
                    }
                    cc.define("WANIENGINE_HAS_KF6", "1");
                }
                if x11.is_some() {
                    cc.define("WANIENGINE_HAS_X11", "1");
                }
                cc.flag_if_supported("-std=c++20");
                cc.define("WANIENGINE_RUNTIME_ROOT", runtime_definition.as_str())
                    .define(
                        "WANIENGINE_PLASMA_PACKAGE_SOURCE",
                        wallpaper_definition.as_str(),
                    )
                    .define("WANIENGINE_VERSION", version_definition.as_str());
            })
            .build();
    }
    for flags in [&layer_shell, &kf6_config, &x11] {
        if let Some(flags) = flags {
            for directory in &flags.lib_dirs {
                println!("cargo:rustc-link-search=native={}", directory.display());
            }
            for library in &flags.libs {
                println!("cargo:rustc-link-lib={library}");
            }
        }
    }
    for key in ["PATH", "PKG_CONFIG_PATH", "PKG_CONFIG_LIBDIR"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!("cargo:rerun-if-changed=platform");
    println!("cargo:rerun-if-changed=qml");
}

struct PkgConfigFlags {
    includes: Vec<PathBuf>,
    lib_dirs: Vec<PathBuf>,
    libs: Vec<String>,
}

/// Ask pkg-config for a module's compile and link flags. Returns `None` when
/// pkg-config is unavailable or the module is not installed, so optional
/// dependencies degrade instead of breaking the build.
fn pkg_config(candidates: &[&str]) -> Option<PkgConfigFlags> {
    let name = candidates
        .iter()
        .find(|name| Command::new("pkg-config").args(["--exists", name]).status().map(|s| s.success()).unwrap_or(false))?;
    let output = Command::new("pkg-config")
        .args(["--cflags", "--libs", "--libs-only-l", "--libs-only-L", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut flags = PkgConfigFlags {
        includes: Vec::new(),
        lib_dirs: Vec::new(),
        libs: Vec::new(),
    };
    for token in text.split_whitespace() {
        let token = token.trim();
        if let Some(path) = token.strip_prefix("-I") {
            flags.includes.push(PathBuf::from(path));
        } else if let Some(path) = token.strip_prefix("-L") {
            flags.lib_dirs.push(PathBuf::from(path));
        } else if let Some(library) = token.strip_prefix("-l") {
            if !flags.libs.iter().any(|existing| existing == library) {
                flags.libs.push(library.to_owned());
            }
        }
    }
    Some(flags)
}
