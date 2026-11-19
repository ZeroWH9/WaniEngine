//! Read the active X11/XWayland window state through the EWMH root properties.
//! Native Wayland toplevel state is intentionally not inferred from geometry.

use std::{
    collections::HashMap,
    os::fd::{AsFd, AsRawFd},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};
use wayland_protocols_plasma::plasma_window_management::client::{
    org_kde_plasma_stacking_order::OrgKdePlasmaStackingOrder,
    org_kde_plasma_window::OrgKdePlasmaWindow,
    org_kde_plasma_window_management::OrgKdePlasmaWindowManagement,
};
use x11rb::{
    connection::Connection as X11Connection,
    protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window},
    rust_connection::RustConnection,
};

const POLL_INTERVAL: Duration = Duration::from_millis(250);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActiveWindowState {
    pub fullscreen: bool,
    pub maximized: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    X11,
    Wayland,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MonitorBackend {
    #[default]
    Unavailable,
    X11,
    KWinWayland,
}

impl SessionMode {
    fn detect() -> Self {
        if std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE")
                .is_ok_and(|value| value.eq_ignore_ascii_case("wayland"))
        {
            Self::Wayland
        } else if std::env::var_os("DISPLAY").is_some() {
            Self::X11
        } else {
            Self::Unknown
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: SessionMode,
    pub backend: MonitorBackend,
    pub connected: bool,
    pub active: ActiveWindowState,
    pub error: Option<String>,
}

pub struct WindowStateMonitor {
    snapshot: Arc<RwLock<Snapshot>>,
    locked: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    lock_worker: Option<JoinHandle<()>>,
}

impl WindowStateMonitor {
    pub fn new(ctx: eframe::egui::Context) -> Self {
        let mode = SessionMode::detect();
        let snapshot = Arc::new(RwLock::new(Snapshot {
            mode,
            backend: MonitorBackend::Unavailable,
            connected: false,
            active: ActiveWindowState::default(),
            error: None,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let locked = Arc::new(AtomicBool::new(false));
        let worker_snapshot = Arc::clone(&snapshot);
        let worker_stop = Arc::clone(&stop);
        let lock_ctx = ctx.clone();
        let worker = thread::Builder::new()
            .name("wani-window-state".into())
            .spawn(move || monitor_windows(worker_snapshot, worker_stop, ctx))
            .ok();
        let (lock_state, lock_stop) = (Arc::clone(&locked), Arc::clone(&stop));
        let lock_worker = thread::Builder::new()
            .name("wani-session-lock".into())
            .spawn(move || monitor_session_lock(lock_state, lock_stop, lock_ctx))
            .ok();
        Self {
            snapshot,
            locked,
            stop,
            worker,
            lock_worker,
        }
    }

    /// True while the screen is locked or a screensaver is active; nothing is visible then.
    pub fn session_locked(&self) -> bool {
        self.locked.load(Ordering::Acquire)
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for WindowStateMonitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        for worker in [self.worker.take(), self.lock_worker.take()]
            .into_iter()
            .flatten()
        {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

struct Atoms {
    active_window: Atom,
    window_state: Atom,
    fullscreen: Atom,
    maximized_horizontal: Atom,
    maximized_vertical: Atom,
    window_pid: Atom,
}

impl Atoms {
    fn new(connection: &RustConnection) -> Result<Self, String> {
        Ok(Self {
            active_window: intern(connection, b"_NET_ACTIVE_WINDOW")?,
            window_state: intern(connection, b"_NET_WM_STATE")?,
            fullscreen: intern(connection, b"_NET_WM_STATE_FULLSCREEN")?,
            maximized_horizontal: intern(connection, b"_NET_WM_STATE_MAXIMIZED_HORZ")?,
            maximized_vertical: intern(connection, b"_NET_WM_STATE_MAXIMIZED_VERT")?,
            window_pid: intern(connection, b"_NET_WM_PID")?,
        })
    }
}

fn monitor_windows(
    shared: Arc<RwLock<Snapshot>>,
    stop: Arc<AtomicBool>,
    ctx: eframe::egui::Context,
) {
    while !stop.load(Ordering::Acquire) {
        if SessionMode::detect() == SessionMode::Wayland {
            if let Err(error) = monitor_kwin_wayland(&shared, &stop, &ctx) {
                set_snapshot(
                    &shared,
                    Snapshot {
                        mode: SessionMode::Wayland,
                        backend: MonitorBackend::Unavailable,
                        connected: false,
                        active: ActiveWindowState::default(),
                        error: Some(error),
                    },
                    &ctx,
                );
            }
            if stop.load(Ordering::Acquire) {
                return;
            }
        }
        match x11rb::connect(None) {
            Ok((connection, screen_index)) => {
                let Some(screen) = connection.setup().roots.get(screen_index) else {
                    set_snapshot(
                        &shared,
                        Snapshot {
                            mode: SessionMode::detect(),
                            backend: MonitorBackend::Unavailable,
                            connected: false,
                            active: ActiveWindowState::default(),
                            error: Some(
                                "The X11 server did not provide the selected screen.".into(),
                            ),
                        },
                        &ctx,
                    );
                    wait_or_stop(&stop, RETRY_INTERVAL);
                    continue;
                };
                let root = screen.root;
                let atoms = match Atoms::new(&connection) {
                    Ok(atoms) => atoms,
                    Err(error) => {
                        set_snapshot(
                            &shared,
                            Snapshot {
                                mode: SessionMode::detect(),
                                backend: MonitorBackend::Unavailable,
                                connected: false,
                                active: ActiveWindowState::default(),
                                error: Some(error),
                            },
                            &ctx,
                        );
                        wait_or_stop(&stop, RETRY_INTERVAL);
                        continue;
                    }
                };

                while !stop.load(Ordering::Acquire) {
                    match query_active_window(&connection, root, &atoms) {
                        Ok(active) => set_snapshot(
                            &shared,
                            Snapshot {
                                mode: SessionMode::detect(),
                                backend: MonitorBackend::X11,
                                connected: true,
                                active,
                                error: None,
                            },
                            &ctx,
                        ),
                        Err(error) => {
                            set_snapshot(
                                &shared,
                                Snapshot {
                                    mode: SessionMode::detect(),
                                    backend: MonitorBackend::Unavailable,
                                    connected: false,
                                    active: ActiveWindowState::default(),
                                    error: Some(error),
                                },
                                &ctx,
                            );
                            break;
                        }
                    }
                    wait_or_stop(&stop, POLL_INTERVAL);
                }
            }
            Err(error) => {
                set_snapshot(
                    &shared,
                    Snapshot {
                        mode: SessionMode::detect(),
                        backend: MonitorBackend::Unavailable,
                        connected: false,
                        active: ActiveWindowState::default(),
                        error: Some(error.to_string()),
                    },
                    &ctx,
                );
                wait_or_stop(&stop, RETRY_INTERVAL);
            }
        }
    }
}

const KWIN_ACTIVE: u32 = 0x1;
const KWIN_MAXIMIZED: u32 = 0x4;
const KWIN_FULLSCREEN: u32 = 0x8;
const WANIENGINE_APP_ID: &str = "org.waniengine.WaniEngine";
const KWIN_MONITOR_INTERVAL_MS: i32 = 250;

#[derive(Default)]
struct KWinWindow {
    state: u32,
    pid: Option<u32>,
    app_id: Option<String>,
}

struct KWinMonitor {
    windows: HashMap<String, KWinWindow>,
    manager: Option<OrgKdePlasmaWindowManagement>,
    stacking_orders: Vec<OrgKdePlasmaStackingOrder>,
    shared: Arc<RwLock<Snapshot>>,
    ctx: eframe::egui::Context,
}

impl KWinMonitor {
    fn track_window(&mut self, uuid: String, qh: &QueueHandle<Self>) {
        if uuid.is_empty() || self.windows.contains_key(&uuid) {
            return;
        }
        self.windows.insert(uuid.clone(), KWinWindow::default());
        if let Some(manager) = &self.manager {
            manager.get_window_by_uuid(uuid.clone(), qh, uuid);
        }
    }

    fn request_stacking_order(&mut self, qh: &QueueHandle<Self>) {
        if let Some(manager) = &self.manager {
            self.stacking_orders
                .push(manager.get_stacking_order(qh, ()));
        }
    }

    fn publish(&self) {
        let own_pid = std::process::id();
        let active = self
            .windows
            .values()
            .find(|window| window.state & KWIN_ACTIVE != 0)
            .filter(|window| {
                window.pid != Some(own_pid)
                    && window.app_id.as_deref() != Some(WANIENGINE_APP_ID)
                    && window.app_id.as_deref() != Some("waniengine-egui")
            })
            .map(|window| ActiveWindowState {
                fullscreen: window.state & KWIN_FULLSCREEN != 0,
                maximized: window.state & KWIN_MAXIMIZED != 0,
            })
            .unwrap_or_default();
        set_snapshot(
            &self.shared,
            Snapshot {
                mode: SessionMode::Wayland,
                backend: MonitorBackend::KWinWayland,
                connected: true,
                active,
                error: None,
            },
            &self.ctx,
        );
    }
}

fn monitor_kwin_wayland(
    shared: &Arc<RwLock<Snapshot>>,
    stop: &AtomicBool,
    ctx: &eframe::egui::Context,
) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|error| error.to_string())?;
    let (globals, mut queue) =
        registry_queue_init::<KWinMonitor>(&connection).map_err(|error| error.to_string())?;
    let qh = queue.handle();
    // KWin filters this internal protocol unless the app's desktop entry requests it.
    // Version 17 introduced the stacking-order object and is required by current KWin.
    let _manager = globals
        .bind::<OrgKdePlasmaWindowManagement, _, _>(&qh, 17..=18, ())
        .map_err(|error| format!("KWin window tracking is unavailable ({error:?}). Start WaniEngine from its application menu entry: its desktop file is what lets KWin share window state."))?;
    let mut state = KWinMonitor {
        windows: HashMap::new(),
        manager: Some(_manager.clone()),
        stacking_orders: Vec::new(),
        shared: Arc::clone(shared),
        ctx: ctx.clone(),
    };
    state.request_stacking_order(&qh);
    queue
        .roundtrip(&mut state)
        .map_err(|error| format!("KWin window tracking failed: {error}"))?;
    state.publish();

    while !stop.load(Ordering::Acquire) {
        queue
            .dispatch_pending(&mut state)
            .map_err(|error| format!("KWin window event failed: {error}"))?;
        connection
            .flush()
            .map_err(|error| format!("KWin connection failed: {error}"))?;
        let guard = loop {
            if let Some(guard) = connection.prepare_read() {
                break guard;
            }
            queue
                .dispatch_pending(&mut state)
                .map_err(|error| format!("KWin window event failed: {error}"))?;
        };
        let mut descriptor = libc::pollfd {
            fd: connection.as_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // A short poll interval keeps application shutdown responsive without a helper thread.
        let ready = unsafe { libc::poll(&mut descriptor, 1, KWIN_MONITOR_INTERVAL_MS) };
        if ready < 0 {
            drop(guard);
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.to_string());
        }
        if ready == 0 {
            drop(guard);
            continue;
        }
        guard
            .read()
            .map_err(|error| format!("KWin event read failed: {error}"))?;
        queue
            .dispatch_pending(&mut state)
            .map_err(|error| format!("KWin window event failed: {error}"))?;
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for KWinMonitor {
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

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for KWinMonitor {
    fn event(
        state: &mut Self,
        _manager: &OrgKdePlasmaWindowManagement,
        event: wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window_management::Event;
        match event {
            Event::StackingOrderChanged2 => state.request_stacking_order(qh),
            Event::WindowWithUuid { uuid, .. } => state.track_window(uuid, qh),
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaStackingOrder, ()> for KWinMonitor {
    fn event(
        state: &mut Self,
        proxy: &OrgKdePlasmaStackingOrder,
        event: wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_stacking_order::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_stacking_order::Event;
        match event {
            Event::Window { uuid } => state.track_window(uuid, qh),
            Event::Done => state
                .stacking_orders
                .retain(|order| order.id() != proxy.id()),
            _ => {}
        }
    }
}

impl Dispatch<OrgKdePlasmaWindow, String> for KWinMonitor {
    fn event(
        state: &mut Self,
        _: &OrgKdePlasmaWindow,
        event: wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window::Event,
        uuid: &String,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wayland_protocols_plasma::plasma_window_management::client::org_kde_plasma_window::Event;
        match event {
            Event::StateChanged { flags } => {
                if let Some(window) = state.windows.get_mut(uuid) {
                    window.state = flags;
                }
            }
            Event::PidChanged { pid } => {
                if let Some(window) = state.windows.get_mut(uuid) {
                    window.pid = Some(pid);
                }
            }
            Event::AppIdChanged { app_id } => {
                if let Some(window) = state.windows.get_mut(uuid) {
                    window.app_id = Some(app_id);
                }
            }
            Event::Unmapped => {
                state.windows.remove(uuid);
            }
            _ => {}
        }
        state.publish();
    }
}

const LOCK_POLL_INTERVAL: Duration = Duration::from_secs(2);

fn dbus_bool(
    connection: &zbus::blocking::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    property: &str,
) -> Option<bool> {
    // Uncached read: a cached proxy would keep the value from its first query.
    let proxy = zbus::blocking::Proxy::new(
        connection,
        destination.to_owned(),
        path.to_owned(),
        "org.freedesktop.DBus.Properties",
    )
    .ok()?;
    let value: zbus::zvariant::OwnedValue = proxy.call("Get", &(interface, property)).ok()?;
    bool::try_from(value).ok()
}

fn screensaver_active(connection: &zbus::blocking::Connection) -> Option<bool> {
    zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.ScreenSaver",
        "/org/freedesktop/ScreenSaver",
        "org.freedesktop.ScreenSaver",
    )
    .ok()?
    .call("GetActive", &())
    .ok()
}

// Pauses wallpapers and characters while the session is locked (logind LockedHint) or the
// screensaver is active, so nothing keeps decoding behind a lock screen. Missing services
// simply leave the flag false.
fn monitor_session_lock(
    locked: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    ctx: eframe::egui::Context,
) {
    let system = zbus::blocking::Connection::system().ok();
    let session = zbus::blocking::Connection::session().ok();
    while !stop.load(Ordering::Acquire) {
        let logind = system.as_ref().and_then(|connection| {
            dbus_bool(
                connection,
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "LockedHint",
            )
        });
        let saver = session.as_ref().and_then(screensaver_active);
        let now = logind.unwrap_or(false) || saver.unwrap_or(false);
        if locked.swap(now, Ordering::AcqRel) != now {
            ctx.request_repaint();
        }
        wait_or_stop(&stop, LOCK_POLL_INTERVAL);
    }
}

fn wait_or_stop(stop: &AtomicBool, duration: Duration) {
    if !stop.load(Ordering::Acquire) {
        thread::park_timeout(duration);
    }
}

fn set_snapshot(shared: &RwLock<Snapshot>, next: Snapshot, ctx: &eframe::egui::Context) {
    let changed = {
        let mut current = shared
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current == next {
            false
        } else {
            *current = next;
            true
        }
    };
    if changed {
        ctx.request_repaint();
    }
}

fn intern(connection: &RustConnection, name: &[u8]) -> Result<Atom, String> {
    connection
        .intern_atom(false, name)
        .map_err(|error| error.to_string())?
        .reply()
        .map(|reply| reply.atom)
        .map_err(|error| error.to_string())
}

fn query_active_window(
    connection: &RustConnection,
    root: Window,
    atoms: &Atoms,
) -> Result<ActiveWindowState, String> {
    let active_window = connection
        .get_property(false, root, atoms.active_window, AtomEnum::WINDOW, 0, 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .value32()
        .and_then(|mut values| values.next())
        .unwrap_or(0);
    if active_window == 0 || is_our_window(connection, active_window, atoms.window_pid)? {
        return Ok(ActiveWindowState::default());
    }
    let states = connection
        .get_property(
            false,
            active_window,
            atoms.window_state,
            AtomEnum::ATOM,
            0,
            64,
        )
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .value32()
        .map(|values| values.collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(state_from_atoms(
        &states,
        atoms.fullscreen,
        atoms.maximized_horizontal,
        atoms.maximized_vertical,
    ))
}

fn is_our_window(
    connection: &RustConnection,
    window: Window,
    pid_atom: Atom,
) -> Result<bool, String> {
    let pid = connection
        .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
        .map_err(|error| error.to_string())?
        .reply()
        .map_err(|error| error.to_string())?
        .value32()
        .and_then(|mut values| values.next());
    Ok(pid == Some(std::process::id()))
}

fn state_from_atoms(
    states: &[Atom],
    fullscreen: Atom,
    maximized_horizontal: Atom,
    maximized_vertical: Atom,
) -> ActiveWindowState {
    ActiveWindowState {
        fullscreen: states.contains(&fullscreen),
        maximized: states.contains(&maximized_horizontal) && states.contains(&maximized_vertical),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_fullscreen_and_requires_both_axes_for_maximized() {
        let state = state_from_atoms(&[10, 11, 12], 10, 11, 12);
        assert_eq!(
            state,
            ActiveWindowState {
                fullscreen: true,
                maximized: true,
            }
        );

        let half_maximized = state_from_atoms(&[11], 10, 11, 12);
        assert_eq!(
            half_maximized,
            ActiveWindowState {
                fullscreen: false,
                maximized: false,
            }
        );
    }
}
