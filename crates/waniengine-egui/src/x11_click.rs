//! Passive X11 mouse-button observation for character audio triggers.
//!
//! This is intentionally unavailable in Wayland sessions: XWayland does not
//! receive native Wayland pointer events, and the character overlay must stay
//! click-through. The RECORD range contains only core button-press events.

use std::{
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread,
    time::Duration,
};
use x11rb::{
    connection::{Connection, RequestConnection},
    protocol::{record, record::ConnectionExt as _, xproto},
    wrapper::ConnectionExt as _,
    x11_utils::TryParse,
};

#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub x: i32,
    pub y: i32,
}

enum Event {
    Click(Click),
    Error(String),
    Stopped,
}

pub struct ClickObserver {
    requested: bool,
    stop: Option<Sender<()>>,
    events: Option<Receiver<Event>>,
}

impl Default for ClickObserver {
    fn default() -> Self {
        Self {
            requested: false,
            stop: None,
            events: None,
        }
    }
}

impl ClickObserver {
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if !enabled {
            self.requested = false;
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            return Ok(());
        }
        if self.requested {
            return Ok(());
        }
        self.requested = true;
        if !available_in_session() {
            return Err("Character click audio is available only in an X11 session.".into());
        }

        let (event_tx, event_rx) = mpsc::sync_channel(128);
        let (stop_tx, stop_rx) = mpsc::channel();
        thread::Builder::new()
            .name("waniengine-x11-click-observer".into())
            .spawn(move || {
                if let Err(error) = observe(event_tx.clone(), stop_rx) {
                    let _ = event_tx.try_send(Event::Error(error));
                }
                let _ = event_tx.try_send(Event::Stopped);
            })
            .map_err(|error| {
                self.requested = false;
                format!("Could not start the X11 click observer: {error}")
            })?;
        self.stop = Some(stop_tx);
        self.events = Some(event_rx);
        Ok(())
    }

    pub fn drain(&mut self) -> (Vec<Click>, Vec<String>) {
        let mut clicks = Vec::new();
        let mut errors = Vec::new();
        let Some(events) = &self.events else {
            return (clicks, errors);
        };
        loop {
            match events.try_recv() {
                Ok(Event::Click(click)) => clicks.push(click),
                Ok(Event::Error(error)) => errors.push(error),
                Ok(Event::Stopped) => self.stop = None,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        (clicks, errors)
    }
}

impl Drop for ClickObserver {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// True only for an X11 desktop session, never for XWayland in a Wayland session.
pub fn available_in_session() -> bool {
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    match std::env::var("XDG_SESSION_TYPE") {
        Ok(session_type) => session_type.eq_ignore_ascii_case("x11"),
        Err(_) => std::env::var_os("WAYLAND_DISPLAY").is_none(),
    }
}

fn observe(events: SyncSender<Event>, stop: Receiver<()>) -> Result<(), String> {
    let (control, _) = x11rb::connect(None)
        .map_err(|error| format!("Could not connect to the X11 server: {error}"))?;
    let (data, _) = x11rb::connect(None)
        .map_err(|error| format!("Could not open the X11 event stream: {error}"))?;
    if control
        .extension_information(record::X11_EXTENSION_NAME)
        .map_err(|error| format!("Could not query X11 RECORD: {error}"))?
        .is_none()
    {
        return Err("The X11 server does not provide the RECORD extension.".into());
    }
    let major_version = u16::try_from(record::X11_XML_VERSION.0)
        .map_err(|error| format!("Invalid X11 RECORD major version: {error}"))?;
    let minor_version = u16::try_from(record::X11_XML_VERSION.1)
        .map_err(|error| format!("Invalid X11 RECORD minor version: {error}"))?;
    let version = control
        .record_query_version(major_version, minor_version)
        .map_err(|error| format!("Could not initialize X11 RECORD: {error}"))?;
    let version = version
        .reply()
        .map_err(|error| format!("Could not read X11 RECORD version: {error}"))?;
    if (version.major_version, version.minor_version) < (major_version, minor_version) {
        return Err("The X11 RECORD extension version is too old.".into());
    }

    let context = control
        .generate_id()
        .map_err(|error| format!("Could not allocate an X11 RECORD context: {error}"))?;
    let empty = record::Range8 { first: 0, last: 0 };
    let empty_ext = record::ExtRange {
        major: empty,
        minor: record::Range16 { first: 0, last: 0 },
    };
    let ranges = [record::Range {
        core_requests: empty,
        core_replies: empty,
        ext_requests: empty_ext,
        ext_replies: empty_ext,
        delivered_events: empty,
        device_events: record::Range8 {
            first: xproto::BUTTON_PRESS_EVENT,
            last: xproto::BUTTON_PRESS_EVENT,
        },
        errors: empty,
        client_started: false,
        client_died: false,
    }];
    let create_context = control
        .record_create_context(context, 0, &[record::CS::ALL_CLIENTS.into()], &ranges)
        .map_err(|error| format!("Could not create an X11 RECORD context: {error}"))?;
    create_context
        .check()
        .map_err(|error| format!("Could not confirm the X11 RECORD context: {error}"))?;

    let (finished_tx, finished_rx) = mpsc::channel();
    let shutdown = thread::Builder::new()
        .name("waniengine-x11-click-shutdown".into())
        .spawn(move || {
            loop {
                match stop.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if finished_rx.try_recv().is_ok() {
                            break;
                        }
                    }
                }
            }
            if let Ok(cookie) = control.record_disable_context(context) {
                let _ = cookie.check();
            }
            let _ = control.sync();
            if let Ok(cookie) = control.record_free_context(context) {
                let _ = cookie.check();
            }
        })
        .map_err(|error| format!("Could not start X11 RECORD shutdown: {error}"))?;

    let enabled = match data.record_enable_context(context) {
        Ok(enabled) => enabled,
        Err(error) => {
            let _ = finished_tx.send(());
            let _ = shutdown.join();
            return Err(format!("Could not observe X11 pointer events: {error}"));
        }
    };
    let mut capture_error = None;
    for reply in enabled {
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                capture_error = Some(format!("X11 click observation stopped: {error}"));
                break;
            }
        };
        if reply.category == 0 && !reply.client_swapped {
            parse_button_events(&reply.data, &events);
        }
        if reply.category == 5 {
            break;
        }
    }
    let _ = finished_tx.send(());
    let _ = shutdown.join();
    if let Some(error) = capture_error {
        Err(error)
    } else {
        Ok(())
    }
}

fn parse_button_events(mut data: &[u8], events: &SyncSender<Event>) {
    while data.len() >= 32 {
        if data[0] & 0x7f == xproto::BUTTON_PRESS_EVENT {
            match xproto::ButtonPressEvent::try_parse(data) {
                Ok((event, remaining)) => {
                    if event.detail == 1 {
                        let _ = events.try_send(Event::Click(Click {
                            x: i32::from(event.root_x),
                            y: i32::from(event.root_y),
                        }));
                    }
                    data = remaining;
                }
                Err(_) => break,
            }
        } else {
            data = &data[32..];
        }
    }
}
