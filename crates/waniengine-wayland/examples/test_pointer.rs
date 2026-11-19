//! Inject one pointer click into an isolated test compositor only.
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_protocols_plasma::fake_input::client::org_kde_kwin_fake_input::OrgKdeKwinFakeInput;

struct State;
wayland_client::delegate_noop!(State: ignore OrgKdeKwinFakeInput);
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Never connect this test injector to a normal desktop session.
    if std::env::var("WAYLAND_DISPLAY").as_deref() != Ok("wani-runtime")
        || !std::env::var("XDG_RUNTIME_DIR")
            .is_ok_and(|path| path.starts_with("/tmp/waniengine-wayland-test."))
    {
        return Err("test_pointer requires the isolated wani-runtime socket".into());
    }
    let mut args = std::env::args().skip(1);
    let x: f64 = args.next().ok_or("missing x")?.parse()?;
    let y: f64 = args.next().ok_or("missing y")?.parse()?;
    if !x.is_finite()
        || !y.is_finite()
        || !(0.0..=1600.0).contains(&x)
        || !(0.0..=600.0).contains(&y)
    {
        return Err("invalid test coordinate".into());
    }
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let device: OrgKdeKwinFakeInput = globals.bind(&qh, 3..=5, ())?;
    device.authenticate(
        "WaniEngine isolated test".into(),
        "Verify overlay click-through".into(),
    );
    device.pointer_motion_absolute(x, y);
    queue.roundtrip(&mut State)?;
    // Allow the test client to bind the newly advertised pointer capability.
    std::thread::sleep(std::time::Duration::from_millis(250));
    device.button(0x110, 1);
    queue.roundtrip(&mut State)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    device.button(0x110, 0);
    queue.roundtrip(&mut State)?;
    Ok(())
}
