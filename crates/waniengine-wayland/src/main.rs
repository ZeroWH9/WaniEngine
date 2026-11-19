fn main() {
    if let Err(error) = waniengine_wayland::run_cli(std::env::args_os().skip(1)) {
        eprintln!("waniengine-wayland: {error}");
        std::process::exit(1);
    }
}
