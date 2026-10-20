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
    unsafe {
        builder
            .cc_builder(|cc| {
                cc.include(&manifest)
                    .include(manifest.join("platform/qt"))
                    .include("/usr/include/LayerShellQt")
                    .include("/usr/include/KF6/KConfigGui")
                    .include("/usr/include/KF6/KConfigCore")
                    .include("/usr/include/KF6/KConfigQml")
                    .include("/usr/include/KF6/KConfig");
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
    for library in [
        "LayerShellQtInterface",
        "KF6ConfigGui",
        "KF6ConfigQml",
        "KF6ConfigCore",
    ] {
        println!("cargo:rustc-link-lib={library}");
    }
    println!("cargo:rerun-if-changed=platform");
    println!("cargo:rerun-if-changed=qml");
}
