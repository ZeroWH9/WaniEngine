//! Linux StatusNotifierItem integration over the session D-Bus.
//! The application remains usable when the desktop has no StatusNotifier host.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use zbus::{
    blocking::Connection,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/Menu";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Activate,
    ContextMenu,
    Exit,
    Unavailable,
}

pub struct TrayController {
    receiver: Receiver<TrayAction>,
    server: TrayServer,
}

impl TrayController {
    pub fn start() -> Result<Self, String> {
        let (event_tx, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("wani-status-notifier".into())
            .spawn(move || run_status_notifier(event_tx, ready_tx, worker_stop))
            .map_err(|error| error.to_string())?;
        let server = TrayServer {
            stop,
            worker: Some(worker),
        };
        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(())) => Ok(Self { receiver, server }),
            Ok(Err(error)) => Err(error),
            Err(error) => {
                drop(server);
                Err(format!("StatusNotifierItem did not become ready: {error}"))
            }
        }
    }

    pub fn take_actions(&self) -> impl Iterator<Item = TrayAction> + '_ {
        self.receiver.try_iter()
    }

    pub fn is_running(&self) -> bool {
        !self.server.stop.load(Ordering::Acquire)
    }
}

struct TrayServer {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for TrayServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct StatusNotifierItem {
    events: Sender<TrayAction>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl StatusNotifierItem {
    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".into()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "waniengine".into()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "WaniEngine".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "Active".into()
    }

    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        "waniengine".into()
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    fn attention_icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    fn overlay_icon_name(&self) -> String {
        String::new()
    }

    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        Vec::new()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        (
            "waniengine".into(),
            Vec::new(),
            "WaniEngine".into(),
            "Wallpaper and character manager".into(),
        )
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(MENU_PATH).expect("menu is a valid D-Bus object path")
    }

    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayAction::Activate);
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayAction::Activate);
    }

    fn context_menu(&self, _x: i32, _y: i32) {
        let _ = self.events.send(TrayAction::ContextMenu);
    }

    fn scroll(&self, _delta: i32, _orientation: String) {}
}

struct TrayMenu {
    events: Sender<TrayAction>,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl TrayMenu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".into()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".into()
    }

    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)) {
        (
            1,
            (
                0,
                HashMap::new(),
                vec![menu_node(1, "Open WaniEngine"), menu_node(2, "Exit")],
            ),
        )
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        ids.into_iter()
            .filter(|id| matches!(id, 1 | 2))
            .map(|id| {
                let label = if id == 1 { "Open WaniEngine" } else { "Exit" };
                let properties = menu_properties(label)
                    .into_iter()
                    .filter(|(name, _)| property_names.is_empty() || property_names.contains(name))
                    .collect();
                (id, properties)
            })
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> Result<OwnedValue, zbus::fdo::Error> {
        if !matches!(id, 1 | 2) {
            return Err(zbus::fdo::Error::InvalidArgs(format!(
                "unknown WaniEngine tray menu item {id}"
            )));
        }
        let label = if id == 1 { "Open WaniEngine" } else { "Exit" };
        menu_properties(label)
            .remove(&name)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("unknown menu property {name}")))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            1 => {
                let _ = self.events.send(TrayAction::Activate);
            }
            2 => {
                let _ = self.events.send(TrayAction::Exit);
            }
            _ => {}
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, data, timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }
}

fn menu_node(id: i32, label: &str) -> OwnedValue {
    OwnedValue::try_from(Value::new((
        id,
        menu_properties(label),
        Vec::<OwnedValue>::new(),
    )))
    .expect("tray menu node uses supported D-Bus values")
}

fn menu_properties(label: &str) -> HashMap<String, OwnedValue> {
    HashMap::from([
        ("type".into(), string_value("standard")),
        ("label".into(), string_value(label)),
        ("enabled".into(), OwnedValue::from(true)),
        ("visible".into(), OwnedValue::from(true)),
    ])
}

fn string_value(value: &str) -> OwnedValue {
    OwnedValue::try_from(Value::new(value)).expect("string is a supported D-Bus value")
}

fn run_status_notifier(
    events: Sender<TrayAction>,
    ready: mpsc::SyncSender<Result<(), String>>,
    stop: Arc<AtomicBool>,
) {
    let result = register_status_notifier(events.clone(), &stop);
    match result {
        Ok(connection) => {
            let _ = ready.send(Ok(()));
            while !stop.load(Ordering::Acquire) {
                if connection.is_closed() {
                    let _ = events.send(TrayAction::Unavailable);
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
}

fn register_status_notifier(
    events: Sender<TrayAction>,
    stop: &AtomicBool,
) -> Result<Connection, String> {
    if stop.load(Ordering::Acquire) {
        return Err("StatusNotifierItem startup was cancelled".into());
    }
    let connection = Connection::session().map_err(|error| format!("session D-Bus: {error}"))?;
    let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    connection
        .request_name(service.as_str())
        .map_err(|error| format!("claiming the tray service name: {error}"))?;
    connection
        .object_server()
        .at(
            ITEM_PATH,
            StatusNotifierItem {
                events: events.clone(),
            },
        )
        .map_err(|error| format!("exporting StatusNotifierItem: {error}"))?;
    connection
        .object_server()
        .at(MENU_PATH, TrayMenu { events })
        .map_err(|error| format!("exporting the tray menu: {error}"))?;

    let mut last_error = None;
    for (destination, interface) in [
        (
            "org.kde.StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        ),
        (
            "org.freedesktop.StatusNotifierWatcher",
            "org.freedesktop.StatusNotifierWatcher",
        ),
    ] {
        // A watcher without a registered host accepts items nobody displays.
        let host_registered = connection
            .call_method(
                Some(destination),
                WATCHER_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(interface, "IsStatusNotifierHostRegistered"),
            )
            .ok()
            .and_then(|reply| {
                reply
                    .body()
                    .deserialize::<zbus::zvariant::OwnedValue>()
                    .ok()
            })
            .and_then(|value| bool::try_from(value).ok());
        if host_registered == Some(false) {
            last_error = Some("no tray host is registered".into());
            continue;
        }
        match connection.call_method(
            Some(destination),
            WATCHER_PATH,
            Some(interface),
            "RegisterStatusNotifierItem",
            &service,
        ) {
            Ok(_) => return Ok(connection),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    Err(format!(
        "no StatusNotifierWatcher accepted WaniEngine: {}",
        last_error.unwrap_or_else(|| "the watcher is unavailable".into())
    ))
}
