//! The Plasma adapter only exposes the Rust surface through the desktop's
//! wallpaper item. Playback never runs in QML or in plasmashell.
use crate::plasma::{eval_plasma, new_token, process_identity, resolve_screen};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use waniengine_core::lock::SessionLock;
use zbus::blocking::{Connection, Proxy};

pub const PLUGIN: &str = "org.waniengine.rustbridge";
const QML: &str = include_str!("../assets/plasma-adapter/contents/ui/main.qml");
const CONFIG: &str = include_str!("../assets/plasma-adapter/contents/config/main.xml");
const METADATA: &str = include_str!("../assets/plasma-adapter/metadata.json");
const SCRIPT: &str = include_str!("../assets/plasma-adapter/kwin-ordering.js");

#[derive(Clone, Serialize, Deserialize)]
struct Snapshot {
    owner_pid: u32,
    owner_identity: String,
    desktop_id: i32,
    token: String,
    original_plugin: String,
    fallback: String,
    image: String,
    image_token: String,
    #[serde(default)]
    ordering_token: String,
}

pub(crate) struct Bridge {
    snapshot: Snapshot,
    state_path: PathBuf,
    hub: Arc<OrderingHub>,
    lock: Arc<SessionLock>,
    health: Health,
    expected_plugin: String,
    expected_bridge_token: String,
    installed: Arc<AtomicBool>,
    ordered: Arc<AtomicBool>,
}
type Members =
    Arc<Mutex<std::collections::HashMap<String, (u32, Arc<AtomicBool>, Arc<AtomicBool>)>>>;
struct OrderingStatus {
    members: Members,
    idle: Arc<AtomicBool>,
}
#[zbus::interface(name = "org.waniengine.Bridge")]
impl OrderingStatus {
    fn idle(&self) {
        self.idle.store(true, Ordering::Release);
    }
    fn renderers(&self) -> String {
        let members: Vec<_> = self
            .members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(token, (pid, _, _))| (*pid, token.clone()))
            .collect();
        serde_json::to_string(&members).unwrap_or_else(|_| "[]".into())
    }
    fn report(&self, token: &str, ready: bool) {
        if let Some((_, installed, ordered)) = self
            .members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(token)
        {
            ordered.store(ready, Ordering::Release);
            installed.store(true, Ordering::Release);
        }
    }
}
pub(crate) struct OrderingHub {
    members: Members,
    token: String,
    path: PathBuf,
    _connection: Connection,
    idle: Arc<AtomicBool>,
}
impl OrderingHub {
    fn new() -> Result<Arc<Self>, String> {
        let token = new_token();
        let service = format!("org.waniengine.Bridge.s{}", token.replace('-', "_"));
        let connection = Connection::session().map_err(|e| e.to_string())?;
        let members = Arc::default();
        let idle = Arc::new(AtomicBool::new(false));
        connection
            .object_server()
            .at(
                "/Bridge",
                OrderingStatus {
                    members: Arc::clone(&members),
                    idle: idle.clone(),
                },
            )
            .map_err(|e| e.to_string())?;
        connection
            .request_name(service.as_str())
            .map_err(|e| e.to_string())?;
        let folder = state_directory().join("scripts");
        private_directory(&folder)?;
        let path = folder.join(format!("{token}.js"));
        let label = format!("waniengine-rustbridge-{token}");
        write_atomic(
            &path,
            SCRIPT
                .replace(
                    "__WANI_PARAMS__",
                    &json!({"service":service,"label":label}).to_string(),
                )
                .as_bytes(),
        )?;
        let hub = Arc::new(Self {
            members,
            token,
            path,
            _connection: connection,
            idle,
        });
        let scripting = Proxy::new(
            &hub._connection,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        )
        .map_err(|e| e.to_string())?;
        let id: i32 = scripting
            .call(
                "loadScript",
                &(hub.path.to_string_lossy().as_ref(), label.as_str()),
            )
            .map_err(|e| e.to_string())?;
        if id < 0 {
            return Err("Could not load the shared desktop ordering adapter".into());
        }
        let script_path = format!("/Scripting/Script{id}");
        let script = Proxy::new(
            &hub._connection,
            "org.kde.KWin",
            script_path.as_str(),
            "org.kde.kwin.Script",
        )
        .map_err(|e| e.to_string())?;
        let _: () = script.call("run", &()).map_err(|e| e.to_string())?;
        drop(script);
        drop(scripting);
        Ok(hub)
    }
}
impl Drop for OrderingHub {
    fn drop(&mut self) {
        self.idle.store(false, Ordering::Release);
        self.members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !self.idle.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if !self.idle.load(Ordering::Acquire) {
            // Keep the observer alive: its owner-loss handler restores opacity
            // and unloads itself after this D-Bus connection disappears.
            let _ = fs::remove_file(&self.path);
            return;
        }
        if let Ok(proxy) = Proxy::new(
            &self._connection,
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
        ) {
            let _: Result<bool, _> = proxy.call(
                "unloadScript",
                &(format!("waniengine-rustbridge-{}", self.token),),
            );
        }
        let _ = fs::remove_file(&self.path);
    }
}

impl Bridge {
    pub(crate) fn desktop_id(&self) -> i32 {
        self.snapshot.desktop_id
    }
    pub(crate) fn owns_state(&self, state: &Value) -> bool {
        state["id"].as_i64() == Some(self.snapshot.desktop_id as i64)
            && state["plugin"] == PLUGIN
            && state["token"] == self.snapshot.token
    }
    /// Plasma can reassign containment IDs when screen priorities change.
    /// Read all assignments in one script before changing any of them.
    pub(crate) fn output_states(outputs: &[Value]) -> Result<Vec<Value>, String> {
        let value = eval_plasma(
            json!({"outputs":outputs}),
            r#"
            var states=[];
            for(var i=0;i<p.outputs.length;i++){
                var o=p.outputs[i],s=Number(screenForConnector(o.name));
                if(s<0)continue;var d=desktopForScreen(s);if(!d)continue;
                d.currentConfigGroup=['Wallpaper','org.waniengine.rustbridge','General'];
                states.push({name:o.name,id:Number(d.id),plugin:String(d.wallpaperPlugin),token:String(d.readConfig('SessionToken',''))});
            }
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,states:states}));
        "#,
        )?;
        value["states"]
            .as_array()
            .cloned()
            .ok_or("Plasma output mapping is unavailable".into())
    }
    /// Stage configuration and install the ordering observer while the worker
    /// has no mapped buffer. The currently playing bridge remains untouched.
    pub fn prepare(
        output: &str,
        position: (i32, i32),
        size: (i32, i32),
        pid: u32,
        previous: Option<&Self>,
        ordering: &mut Option<Arc<OrderingHub>>,
    ) -> Result<Self, String> {
        install_adapter()?;
        let screen = resolve_screen(output, position, size)?;
        let current = eval_plasma(
            json!({"screen":screen}),
            r#"
            var d=desktopForScreen(p.screen);if(!d)throw new Error('Desktop unavailable');
            var plugin=String(d.wallpaperPlugin);
            d.currentConfigGroup=['Wallpaper','org.waniengine.rustbridge','General'];
            var token=String(d.readConfig('SessionToken',''));
            var previous=String(d.readConfig('PreviousPlugin','org.kde.image'));
            d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
            print('WANIENGINE_JSON:'+JSON.stringify({id:Number(d.id),plugin:plugin,token:token,previous:previous,image:String(d.readConfig('Image','')),imageToken:String(d.readConfig('WaniEngineSessionToken',''))}));
        "#,
        )?;
        let desktop_id = current["id"]
            .as_i64()
            .ok_or("Plasma did not identify the desktop")? as i32;
        let reusable = previous.filter(|old| old.snapshot.desktop_id == desktop_id);
        let folder = state_directory();
        private_directory(&folder)?;
        let lock = match reusable {
            Some(old) => old.lock.clone(),
            None => {
                let file = OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(folder.join(format!("desktop-{desktop_id}.lock")))
                    .map_err(|e| e.to_string())?;
                Arc::new(
                    SessionLock::try_lock(file)
                        .ok_or(format!("Another WaniEngine session owns Plasma desktop {desktop_id} on {output} (screen {screen})"))?,
                )
            }
        };
        let inherited = reusable
            .filter(|old| current["plugin"] == PLUGIN && current["token"] == old.snapshot.token);
        let original_plugin = inherited
            .map(|old| old.snapshot.original_plugin.clone())
            .unwrap_or_else(|| {
                if current["plugin"] == PLUGIN {
                    current["previous"]
                        .as_str()
                        .unwrap_or("org.kde.image")
                        .into()
                } else {
                    current["plugin"].as_str().unwrap_or("org.kde.image").into()
                }
            });
        let fallback = inherited
            .map(|old| old.snapshot.fallback.clone())
            .unwrap_or_else(|| fallback_image(current["image"].as_str().unwrap_or(""), size));
        let token = new_token();
        if ordering.is_none() {
            *ordering = Some(OrderingHub::new()?);
        }
        let hub = ordering.as_ref().unwrap().clone();
        let snapshot = Snapshot {
            owner_pid: std::process::id(),
            owner_identity: process_identity(std::process::id()).unwrap_or_default(),
            desktop_id,
            token: token.clone(),
            ordering_token: hub.token.clone(),
            original_plugin,
            fallback,
            image: inherited
                .map(|old| old.snapshot.image.clone())
                .unwrap_or_else(|| current["image"].as_str().unwrap_or("").into()),
            image_token: inherited
                .map(|old| old.snapshot.image_token.clone())
                .unwrap_or_else(|| current["imageToken"].as_str().unwrap_or("").into()),
        };
        let state_path = folder.join(format!("session-{desktop_id}-{token}.json"));
        let installed = Arc::new(AtomicBool::new(false));
        let ordered = Arc::new(AtomicBool::new(false));
        let health = Health::new(pid, &token, ordered.clone())?;
        hub.members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(token.clone(), (pid, installed.clone(), ordered.clone()));
        let bridge = Self {
            snapshot,
            state_path,
            hub,
            lock,
            health,
            expected_plugin: current["plugin"].as_str().unwrap_or("").into(),
            expected_bridge_token: current["token"].as_str().unwrap_or("").into(),
            installed,
            ordered,
        };
        write_atomic(
            &bridge.state_path,
            &serde_json::to_vec(&bridge.snapshot).map_err(|e| e.to_string())?,
        )?;
        bridge.wait_for("PendingInstalled")?;
        Ok(bridge)
    }

    /// Called only after the gated Rust worker commits its first frame.
    pub fn activate(&mut self) -> Result<(), String> {
        self.wait_for("PendingReady")?;
        self.health.ready.store(true, Ordering::Release);
        let result = self.config(r#"
            if(String(d.wallpaperPlugin)!==p.expectedPlugin)throw new Error('Wallpaper changed outside WaniEngine');
            if(p.expectedPlugin==='org.waniengine.rustbridge'&&String(d.readConfig('SessionToken',''))!==p.expectedToken)throw new Error('Another video session replaced this wallpaper');
            if(p.expectedPlugin==='org.kde.image'){
                d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
                if(String(d.readConfig('Image',''))!==p.image||String(d.readConfig('WaniEngineSessionToken',''))!==p.imageToken)throw new Error('Wallpaper changed outside WaniEngine');
                d.currentConfigGroup=['Wallpaper','org.waniengine.rustbridge','General'];
            }
            d.writeConfig('SessionToken',p.token);d.writeConfig('Endpoint',p.endpoint);
            d.writeConfig('FallbackImage',p.fallback);d.writeConfig('PreviousPlugin',p.previous);
            d.writeConfig('BridgeReady',true);d.wallpaperPlugin='org.waniengine.rustbridge';d.reloadConfig();
        "#);
        if result.is_err() {
            self.health.ready.store(false, Ordering::Release);
        }
        result?;
        self.wait_presented()?;
        Ok(())
    }

    /// Make the original desktop opaque before replacing video with an image.
    /// The worker stays alive so a failed image change can reactivate it.
    pub fn suspend(&mut self) -> Result<(), String> {
        self.health.ready.store(false, Ordering::Release);
        restore(&self.snapshot)?;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        self.health.presented.store(false, Ordering::Release);
        self.health.ready.store(true, Ordering::Release);
        self.config(r#"
            if(String(d.wallpaperPlugin)!==p.previous)throw new Error('Wallpaper changed outside WaniEngine');
            if(p.previous==='org.kde.image'){
                d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
                if(String(d.readConfig('Image',''))!==p.image||String(d.readConfig('WaniEngineSessionToken',''))!==p.imageToken)throw new Error('Wallpaper changed outside WaniEngine');
                d.currentConfigGroup=['Wallpaper','org.waniengine.rustbridge','General'];
            }
            d.writeConfig('SessionToken',p.token);d.writeConfig('Endpoint',p.endpoint);
            d.writeConfig('FallbackImage',p.fallback);d.writeConfig('PreviousPlugin',p.previous);
            d.writeConfig('BridgeReady',true);d.wallpaperPlugin='org.waniengine.rustbridge';d.reloadConfig();
        "#)?;
        self.wait_presented()?;
        Ok(())
    }

    fn wait_presented(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.health.presented.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                self.health.ready.store(false, Ordering::Release);
                return Err("Plasma did not confirm presentation of the Rust wallpaper".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    pub fn stop_health(&self) {
        self.health.ready.store(false, Ordering::Release);
    }

    fn config(&self, body: &str) -> Result<Value, String> {
        let params = json!({"id":self.snapshot.desktop_id,"token":self.snapshot.token,"endpoint":self.health.url,"fallback":self.snapshot.fallback,"previous":self.snapshot.original_plugin,"image":self.snapshot.image,"imageToken":self.snapshot.image_token,"expectedPlugin":self.expected_plugin,"expectedToken":self.expected_bridge_token});
        let script = format!(
            "var d=desktopById(p.id);if(!d)throw new Error('Desktop unavailable');d.currentConfigGroup=['Wallpaper','{PLUGIN}','General'];{body}print('WANIENGINE_JSON:'+JSON.stringify({{ok:true}}));"
        );
        let value = eval_plasma(params, &script)?;
        if value["ok"] == true {
            Ok(value)
        } else {
            Err(value["error"]
                .as_str()
                .unwrap_or("Plasma bridge configuration failed")
                .into())
        }
    }

    fn wait_for(&self, key: &str) -> Result<(), String> {
        let status = if key == "PendingInstalled" {
            &self.installed
        } else {
            &self.ordered
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !status.load(Ordering::Acquire) {
            if Instant::now() >= deadline {
                return Err(format!(
                    "KWin did not confirm {key} for the private renderer ordering adapter"
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop_health();
        let restored = restore(&self.snapshot).is_ok();
        self.hub
            .members
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.snapshot.token);
        if restored {
            let _ = fs::remove_file(&self.state_path);
        } else {
            eprintln!("waniengine: Plasma bridge restoration remains pending");
        }
    }
}

fn restore(snapshot: &Snapshot) -> Result<(), String> {
    let value = eval_plasma(
        json!({"id":snapshot.desktop_id,"token":snapshot.token,"previous":snapshot.original_plugin}),
        r#"
        var d=desktopById(p.id);if(!d){print('WANIENGINE_JSON:'+JSON.stringify({ok:true}));return;}
        var plugin=String(d.wallpaperPlugin);
        d.currentConfigGroup=['Wallpaper','org.waniengine.rustbridge','General'];
        if(plugin==='org.waniengine.rustbridge'&&String(d.readConfig('SessionToken',''))===p.token){
            d.writeConfig('BridgeReady',false);d.wallpaperPlugin=p.previous;d.reloadConfig();
            if(String(d.wallpaperPlugin)!==p.previous)throw new Error('Plasma did not confirm desktop restoration');
        }
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true}));
    "#,
    )?;
    if value["ok"] == true {
        Ok(())
    } else {
        Err(value["error"]
            .as_str()
            .unwrap_or("Could not restore the Plasma desktop")
            .into())
    }
}

pub(crate) fn recover_stale() {
    let folder = state_directory();
    let Ok(entries) = fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("session-") || !name.ends_with(".json") {
            continue;
        }
        let Ok(file) = File::open(entry.path()) else {
            continue;
        };
        let mut bytes = Vec::new();
        if file.take(65537).read_to_end(&mut bytes).is_err() || bytes.len() > 65536 {
            continue;
        }
        let Ok(snapshot) = serde_json::from_slice::<Snapshot>(&bytes) else {
            continue;
        };
        if process_identity(snapshot.owner_pid).as_deref() == Some(snapshot.owner_identity.as_str())
        {
            continue;
        }
        if restore(&snapshot).is_ok() {
            if snapshot.token.len() <= 100
                && snapshot
                    .token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                if let Ok(connection) = Connection::session() {
                    if let Ok(proxy) = Proxy::new(
                        &connection,
                        "org.kde.KWin",
                        "/Scripting",
                        "org.kde.kwin.Scripting",
                    ) {
                        let _: Result<bool, _> = proxy.call(
                            "unloadScript",
                            &(format!(
                                "waniengine-rustbridge-{}",
                                if snapshot.ordering_token.is_empty() {
                                    &snapshot.token
                                } else {
                                    &snapshot.ordering_token
                                }
                            ),),
                        );
                    }
                }
                let _ = fs::remove_file(folder.join("scripts").join(format!(
                    "{}.js",
                    if snapshot.ordering_token.is_empty() {
                        &snapshot.token
                    } else {
                        &snapshot.ordering_token
                    }
                )));
            }
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn state_directory() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("WaniEngine/plasma-rust-bridge")
}

fn private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!("{}.tmp", new_token()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        e.to_string()
    })?;
    File::open(path.parent().ok_or("Missing adapter directory")?)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())
}

fn install_adapter() -> Result<(), String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("Could not find the user data directory")?;
    let folder = data.join("plasma/wallpapers").join(PLUGIN);
    let marker = folder.join(".waniengine-managed");
    if folder.is_symlink() || (folder.exists() && !marker.is_file()) {
        return Err("An unmanaged Plasma Rust bridge already exists at the adapter path".into());
    }
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    fs::write(&marker, b"org.waniengine.WaniEngine\n").map_err(|e| e.to_string())?;
    for (relative, bytes) in [
        ("metadata.json", METADATA),
        ("contents/config/main.xml", CONFIG),
        ("contents/ui/main.qml", QML),
    ] {
        let path = folder.join(relative);
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        if fs::read(&path).ok().as_deref() != Some(bytes.as_bytes()) {
            write_atomic(&path, bytes.as_bytes())?;
        }
    }
    fs::write(marker, b"org.waniengine.WaniEngine\n").map_err(|e| e.to_string())
}

fn fallback_image(value: &str, size: (i32, i32)) -> String {
    if value.is_empty() {
        return String::new();
    }
    let path = waniengine_core::library::file_path(value)
        .ok()
        .unwrap_or_else(|| PathBuf::from(value));
    if path.is_file() {
        return waniengine_core::library::file_url(&path);
    }
    let images = path.join("contents/images");
    let mut files = fs::read_dir(images)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        let parts = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .split('x')
            .filter_map(|n| n.parse::<i64>().ok())
            .collect::<Vec<_>>();
        if parts.len() == 2 {
            (parts[0] - i64::from(size.0)).abs() + (parts[1] - i64::from(size.1)).abs()
        } else {
            i64::MAX
        }
    });
    files
        .first()
        .map(|p| waniengine_core::library::file_url(p))
        .unwrap_or_default()
}

struct Health {
    url: String,
    ready: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    presented: Arc<AtomicBool>,
}

impl Health {
    fn new(pid: u32, token: &str, ordered: Arc<AtomicBool>) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let url = format!(
            "http://127.0.0.1:{}/{}",
            listener.local_addr().map_err(|e| e.to_string())?.port(),
            token
        );
        let ready = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let presented = Arc::new(AtomicBool::new(false));
        let acknowledged = presented.clone();
        let active = ready.clone();
        let stop = stopped.clone();
        let token = token.to_owned();
        let identity = process_identity(pid);
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, address)) if address.ip().is_loopback() => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(150)));
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(150)));
                        let mut line = Vec::new();
                        let read = BufReader::new(&stream)
                            .take(2049)
                            .read_until(b'\n', &mut line);
                        let presentation =
                            line.starts_with(format!("GET /{token}/visible HTTP/1.").as_bytes());
                        let valid = read.is_ok()
                            && line.len() <= 2048
                            && (presentation
                                || line.starts_with(format!("GET /{token} HTTP/1.").as_bytes()));
                        let running = identity
                            .as_ref()
                            .is_some_and(|id| process_identity(pid).as_ref() == Some(id))
                            && fs::read_to_string(format!("/proc/{pid}/stat"))
                                .ok()
                                .is_some_and(|s| {
                                    !s.rsplit_once(") ").is_some_and(|(_, fields)| {
                                        fields.starts_with('Z') || fields.starts_with('X')
                                    })
                                });
                        let healthy = running
                            && active.load(Ordering::Acquire)
                            && ordered.load(Ordering::Acquire);
                        if valid && presentation && healthy {
                            acknowledged.store(true, Ordering::Release);
                        }
                        let body = if valid {
                            json!({"token":token,"ready":healthy})
                        } else {
                            json!({"ready":false})
                        }
                        .to_string();
                        reply(
                            &mut stream,
                            if valid { "200 OK" } else { "404 Not Found" },
                            &body,
                        );
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(30))
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            url,
            ready,
            stopped,
            presented,
        })
    }
}

fn reply(stream: &mut TcpStream, status: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
}

impl Drop for Health {
    fn drop(&mut self) {
        self.ready.store(false, Ordering::Release);
        self.stopped.store(true, Ordering::Release);
    }
}
