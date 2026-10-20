mod bridge;
use cxx_qt_lib as _;

fn main() {
    cxx_qt::init_crate!(waniengine);
    cxx_qt::init_qml_module!("WaniEngine");
    let arguments =
        serde_json::to_string(&std::env::args().collect::<Vec<_>>()).expect("serialize arguments");
    let installed = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|folder| folder.join("../lib/qml")));
    let runtime = std::env::var("WANIENGINE_RUNTIME_ROOT")
        .unwrap_or_else(|_| match installed {
            Some(path) if path.join("WaniEngine/Runtime/qmldir").is_file() => {
                path.to_string_lossy().into_owned()
            }
            _ => env!("WANIENGINE_BUNDLED_RUNTIME_ROOT").to_owned(),
        });
    std::process::exit(bridge::ffi::run_manager(&arguments, &runtime));
}
