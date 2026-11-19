//! Native Plasma desktop wallpaper control through its session-bus scripting API.
//! Using Plasma's built-in image wallpaper keeps the desktop's own icon surfaces
//! above the background and requires no WaniEngine Qt plugin.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use waniengine_core::lock::SessionLock;
use zbus::blocking::{Connection, Proxy};

const IMAGE_PLUGIN: &str = "org.kde.image";
const MARKER: &str = "WANIENGINE_JSON:";
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    screen: i32,
    containment_id: i32,
    original_plugin: String,
    original_config: HashMap<String, Value>,
    token: String,
    #[serde(default)]
    applied_image: Option<String>,
    #[serde(default)]
    applied_fill_mode: Option<i32>,
    #[serde(default)]
    applied_animation: Option<bool>,
}

#[derive(Default, Serialize, Deserialize)]
struct Recovery {
    owner_pid: u32,
    owner_identity: String,
    snapshots: Vec<Snapshot>,
}

struct Inner {
    path: PathBuf,
    recovery: Mutex<Recovery>,
    owns_recovery: AtomicBool,
    _session_lock: Option<SessionLock>,
}

#[derive(Clone)]
pub struct WallpaperController(Arc<Inner>);

impl WallpaperController {
    pub fn new() -> Self {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("waniengine-state-{}", process::id()))
            })
            .join("WaniEngine/plasma-image-session.json");
        let session_lock = lock_session(&state.with_extension("lock"));
        let mut recovery = read_recovery(&state).unwrap_or_default();
        let owner_is_other =
            session_lock.is_none() || (!recovery.snapshots.is_empty() && owner_alive(&recovery));
        if session_lock.is_some() && !recovery.snapshots.is_empty() && !owner_alive(&recovery) {
            if restore_recovery(&mut recovery).is_ok() {
                recovery = Recovery::default();
                let _ = fs::remove_file(&state);
            }
        }
        Self(Arc::new(Inner {
            path: state,
            recovery: Mutex::new(recovery),
            owns_recovery: AtomicBool::new(!owner_is_other),
            _session_lock: session_lock,
        }))
    }

    pub fn apply(
        &self,
        output_name: &str,
        position: (i32, i32),
        size: (i32, i32),
        image: &Path,
        fit: &str,
        animated: bool,
    ) -> Result<(), String> {
        if output_name.is_empty() {
            return Err("Select a connected display".into());
        }
        if !matches!(fit, "cover" | "contain" | "stretch" | "center") {
            return Err("Unsupported wallpaper fit mode".into());
        }
        if !image.is_file() {
            return Err("Wallpaper file is unavailable".into());
        }
        let image = image.canonicalize().map_err(|e| e.to_string())?;
        if image.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v"
            )
        }) {
            return Err(
                "Plasma's image backend accepts image files; use the Rust video bridge for videos"
                    .into(),
            );
        }
        let fit_mode = plasma_fill_mode(fit).expect("validated Plasma image fit mode");
        let image = image.canonicalize().map_err(|e| e.to_string())?;
        let image_path = waniengine_core::library::file_url(&image);
        let screen = resolve_screen(output_name, position, size)?;
        let mut recovery = self.0.recovery.lock().unwrap_or_else(|e| e.into_inner());
        if self.0._session_lock.is_none() {
            return Err(
                "Another WaniEngine instance currently controls the Plasma wallpaper".into(),
            );
        }
        if !self.0.owns_recovery.load(Ordering::Acquire) && owner_alive(&recovery) {
            return Err("Another WaniEngine session currently manages the Plasma wallpaper".into());
        }
        if !recovery.snapshots.is_empty() && !owner_alive(&recovery) {
            restore_recovery(&mut recovery)?;
            recovery.owner_pid = 0;
            recovery.owner_identity.clear();
        }
        let before = inspect(screen, -1)?;
        let plugin = before["plugin"]
            .as_str()
            .ok_or("Plasma returned no current wallpaper plugin")?
            .to_owned();
        let desktop_id = before["id"]
            .as_i64()
            .ok_or("Plasma returned no desktop identifier")? as i32;
        let token_before = before["sessionToken"].as_str().unwrap_or_default();

        let old = recovery.snapshots.clone();
        let prior = recovery
            .snapshots
            .iter()
            .find(|s| {
                s.screen == screen
                    && s.containment_id == desktop_id
                    && s.token == token_before
                    && plugin == IMAGE_PLUGIN
                    && s.applied_image.as_deref() == Some(before["image"].as_str().unwrap_or(""))
                    && s.applied_fill_mode
                        == before["imageConfig"]["FillMode"]
                            .as_i64()
                            .map(|value| value as i32)
                    && s.applied_animation == before["imageConfig"]["ForceImageAnimation"].as_bool()
            })
            .cloned();
        let mut snapshot = prior.unwrap_or_else(|| Snapshot {
            screen,
            containment_id: desktop_id,
            original_plugin: plugin.clone(),
            original_config: before["imageConfig"]
                .as_object()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            token: String::new(),
            applied_image: None,
            applied_fill_mode: None,
            applied_animation: None,
        });
        snapshot.token = new_token();
        snapshot.applied_image = Some(image_path.clone());
        snapshot.applied_fill_mode = Some(fit_mode);
        snapshot.applied_animation = Some(animated);
        recovery.snapshots.retain(|s| s.screen != screen);
        recovery.snapshots.push(snapshot.clone());
        recovery.owner_pid = process::id();
        recovery.owner_identity = process_identity(process::id()).unwrap_or_default();
        write_recovery(&self.0.path, &recovery)?;
        self.0.owns_recovery.store(true, Ordering::Release);

        let params = json!({"screen":screen,"id":desktop_id,"expectedPlugin":plugin,"expectedToken":token_before,"image":image_path,"fillMode":fit_mode,"animated":animated,"token":snapshot.token});
        let script = r#"
            var all=desktops(), d=desktopById(p.id);
            if (!d) throw new Error('The desktop was not found.');
            if (String(d.wallpaperPlugin)!==p.expectedPlugin) throw new Error('The wallpaper changed while applying.');
            if (p.expectedPlugin==='org.kde.image') {
                d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
                if (String(d.readConfig('WaniEngineSessionToken',''))!==p.expectedToken) throw new Error('The wallpaper changed outside this app.');
            }
            d.currentConfigGroup=['Wallpaper',p.expectedPlugin,'General'];
            d.wallpaperPlugin='org.kde.image';
            d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
            d.writeConfig('Image',p.image); d.writeConfig('FillMode',p.fillMode);
            d.writeConfig('ForceImageAnimation',p.animated); d.writeConfig('WaniEngineSessionToken',p.token);
            d.reloadConfig();
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,id:Number(d.id),plugin:String(d.wallpaperPlugin)}));
        "#;
        let applied = eval_plasma(params.clone(), script);
        let verified = inspect(screen, desktop_id);
        let result = match (applied, verified) {
            (Ok(applied), Ok(after))
                if applied["ok"] == true
                    && after["plugin"] == IMAGE_PLUGIN
                    && after["sessionToken"] == snapshot.token
                    && after["image"] == params["image"] =>
            {
                Ok(())
            }
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
            _ => Err("Plasma did not confirm the wallpaper change".into()),
        };
        if let Err(error) = result {
            // Keep a recovery snapshot if Plasma may have changed before its
            // confirmation was lost. Restore only when our token still owns it.
            match inspect(screen, desktop_id) {
                Ok(state) if state["sessionToken"] == snapshot.token => {
                    if let Err(restore_error) = restore_one(&snapshot) {
                        let _ = write_recovery(&self.0.path, &recovery);
                        return Err(format!(
                            "{error}; wallpaper restoration is pending: {restore_error}"
                        ));
                    }
                }
                Err(inspect_error) => {
                    let _ = write_recovery(&self.0.path, &recovery);
                    return Err(format!(
                        "{error}; wallpaper state could not be checked for restoration: {inspect_error}"
                    ));
                }
                _ => {}
            }
            recovery.snapshots = old;
            if recovery.snapshots.is_empty() {
                let _ = fs::remove_file(&self.0.path);
            } else {
                let _ = write_recovery(&self.0.path, &recovery);
            }
            return Err(error);
        }
        Ok(())
    }

    /// Stops or resumes Plasma's animated-image playback only while the
    /// currently applied image is still owned by this WaniEngine session.
    pub fn set_animation_enabled(
        &self,
        output_name: &str,
        position: (i32, i32),
        size: (i32, i32),
        enabled: bool,
    ) -> Result<(), String> {
        if output_name.is_empty() {
            return Err("Select a connected display".into());
        }
        let screen = resolve_screen(output_name, position, size)?;
        let mut recovery = self.0.recovery.lock().unwrap_or_else(|e| e.into_inner());
        if self.0._session_lock.is_none() {
            return Err(
                "Another WaniEngine instance currently controls the Plasma wallpaper".into(),
            );
        }
        if !self.0.owns_recovery.load(Ordering::Acquire) && owner_alive(&recovery) {
            return Err("Another WaniEngine session currently manages the Plasma wallpaper".into());
        }
        let before = inspect(screen, -1)?;
        if before["plugin"] != IMAGE_PLUGIN {
            return Err("The active wallpaper is not managed by WaniEngine".into());
        }
        let containment_id = before["id"]
            .as_i64()
            .ok_or("Plasma returned no desktop identifier")? as i32;
        let token = before["sessionToken"].as_str().unwrap_or_default();
        let image = before["image"].as_str().unwrap_or_default();
        let index = recovery
            .snapshots
            .iter()
            .position(|snapshot| {
                snapshot.screen == screen
                    && snapshot.containment_id == containment_id
                    && snapshot.token == token
                    && snapshot.applied_image.as_deref() == Some(image)
            })
            .ok_or("WaniEngine no longer owns the active Plasma wallpaper")?;
        if before["imageConfig"]["ForceImageAnimation"].as_bool() == Some(enabled) {
            if recovery.snapshots[index].applied_animation != Some(enabled) {
                recovery.snapshots[index].applied_animation = Some(enabled);
                write_recovery(&self.0.path, &recovery)?;
            }
            return Ok(());
        }

        let params = json!({"screen":screen,"id":containment_id,"token":token,"image":image,"enabled":enabled});
        let script = r#"
            var d=desktopById(p.id); if(!d) throw new Error('The desktop was not found.');
            if(String(d.wallpaperPlugin)!=='org.kde.image') throw new Error('The wallpaper changed outside this app.');
            d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
            if(String(d.readConfig('WaniEngineSessionToken',''))!==p.token || String(d.readConfig('Image',''))!==p.image) throw new Error('The wallpaper changed outside this app.');
            d.writeConfig('ForceImageAnimation',p.enabled); d.reloadConfig();
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,enabled:!!d.readConfig('ForceImageAnimation',false)}));
        "#;
        let result = eval_plasma(params.clone(), script);
        let after = inspect(screen, containment_id);
        match (result, after) {
            (Ok(result), Ok(after))
                if result["ok"] == true
                    && result["enabled"] == enabled
                    && after["plugin"] == IMAGE_PLUGIN
                    && after["sessionToken"] == token
                    && after["image"] == image
                    && after["imageConfig"]["ForceImageAnimation"] == enabled =>
            {
                recovery.snapshots[index].applied_animation = Some(enabled);
                write_recovery(&self.0.path, &recovery)?;
                Ok(())
            }
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
            _ => Err("Plasma did not confirm the animation state change".into()),
        }
    }

    pub fn restore(&self) -> Result<(), String> {
        let mut recovery = self.0.recovery.lock().unwrap_or_else(|e| e.into_inner());
        if self.0._session_lock.is_none() {
            return Err(
                "Another WaniEngine instance currently controls the Plasma wallpaper".into(),
            );
        }
        if !self.0.owns_recovery.load(Ordering::Acquire) && owner_alive(&recovery) {
            return Err("Another WaniEngine session currently manages the Plasma wallpaper".into());
        }
        self.0.owns_recovery.store(true, Ordering::Release);
        restore_recovery(&mut recovery)?;
        recovery.snapshots.clear();
        recovery.owner_pid = 0;
        recovery.owner_identity.clear();
        let _ = fs::remove_file(&self.0.path);
        Ok(())
    }
}

impl Default for WallpaperController {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        if !self.owns_recovery.load(Ordering::Acquire) {
            return;
        }
        let recovery = self.recovery.get_mut().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = restore_recovery(recovery) {
            eprintln!("waniengine: wallpaper restoration will be retried next launch: {error}");
        }
    }
}

fn restore_recovery(recovery: &mut Recovery) -> Result<(), String> {
    let mut remaining = Vec::new();
    let mut first_error = None;
    for snapshot in &recovery.snapshots {
        match restore_one(snapshot) {
            Ok(true) => {}
            Ok(false) => remaining.push(snapshot.clone()),
            Err(error) => {
                remaining.push(snapshot.clone());
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    recovery.snapshots = remaining;
    if let Some(error) = first_error {
        return Err(error);
    }
    if recovery.snapshots.is_empty() {
        Ok(())
    } else {
        Err("Plasma has not confirmed wallpaper restoration".into())
    }
}

fn restore_one(snapshot: &Snapshot) -> Result<bool, String> {
    let params = json!({"screen":snapshot.screen,"id":snapshot.containment_id,"token":snapshot.token,"plugin":snapshot.original_plugin,"config":snapshot.original_config,"appliedImage":snapshot.applied_image,"appliedFillMode":snapshot.applied_fill_mode,"appliedAnimation":snapshot.applied_animation});
    let script = r#"
        var d=desktopById(p.id); if(!d) throw new Error('The desktop was not found.');
        if(String(d.wallpaperPlugin)!=='org.kde.image') { print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true})); return; }
        d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
        if(String(d.readConfig('WaniEngineSessionToken',''))!==p.token) { print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true})); return; }
        if(p.appliedImage!=null && String(d.readConfig('Image',''))!==p.appliedImage) { print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true})); return; }
        if(p.appliedFillMode!=null && Number(d.readConfig('FillMode',2))!==p.appliedFillMode) { print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true})); return; }
        if(p.appliedAnimation!=null && !!d.readConfig('ForceImageAnimation',false)!==p.appliedAnimation) { print('WANIENGINE_JSON:'+JSON.stringify({ok:true,skipped:true})); return; }
        for(var key in p.config) if(Object.prototype.hasOwnProperty.call(p.config,key)) d.writeConfig(key,p.config[key]);
        d.wallpaperPlugin=p.plugin; d.reloadConfig();
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,restored:String(d.wallpaperPlugin)===p.plugin,plugin:String(d.wallpaperPlugin)}));
    "#;
    let result = eval_plasma(params, script)?;
    if result["skipped"] == true {
        return Ok(true);
    }
    if result["restored"] == true {
        return Ok(true);
    }
    Err(result["error"]
        .as_str()
        .unwrap_or("Plasma did not confirm wallpaper restoration")
        .to_owned())
}

fn inspect(screen: i32, id: i32) -> Result<Value, String> {
    let params = json!({"screen":screen,"id":id});
    let script = r#"
        var d=p.id>=0?desktopById(p.id):desktopForScreen(p.screen);
        if(!d) throw new Error('No desktop exists for the selected monitor.');
        d.currentConfigGroup=['Wallpaper',String(d.wallpaperPlugin),'General'];
        var imageConfig={};
        d.currentConfigGroup=['Wallpaper','org.kde.image','General'];
        imageConfig={Image:d.readConfig('Image',''),FillMode:Number(d.readConfig('FillMode',2)),ForceImageAnimation:!!d.readConfig('ForceImageAnimation',false),WaniEngineSessionToken:String(d.readConfig('WaniEngineSessionToken',''))};
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,id:Number(d.id),screen:Number(d.screen),plugin:String(d.wallpaperPlugin),image:String(imageConfig.Image||''),sessionToken:String(imageConfig.WaniEngineSessionToken||''),imageConfig:imageConfig}));
    "#;
    let result = eval_plasma(params, script)?;
    if result["ok"] == true {
        Ok(result)
    } else {
        Err(result["error"]
            .as_str()
            .unwrap_or("Could not inspect Plasma desktop")
            .to_owned())
    }
}

pub(crate) fn resolve_screen(
    output_name: &str,
    position: (i32, i32),
    size: (i32, i32),
) -> Result<i32, String> {
    let params =
        json!({"output":output_name,"x":position.0,"y":position.1,"width":size.0,"height":size.1});
    let script = r#"
        if(typeof screenForConnector==='function') {
            var screen=Number(screenForConnector(p.output));
            if(screen<0)throw new Error('The selected monitor is disconnected or Plasma is still updating its outputs.');
            print('WANIENGINE_JSON:'+JSON.stringify({ok:true,screen:screen}));return;
        }
        var ids=[],matches=[],all=desktops();
        for(var i=0;i<all.length;i++){var n=Number(all[i].screen);if(n>=0&&ids.indexOf(n)<0)ids.push(n);}
        for(var i=0;i<ids.length;i++){var g=screenGeometry(ids[i]);if(g.x===p.x&&g.y===p.y&&g.width===p.width&&g.height===p.height)matches.push(ids[i]);}
        if(matches.length!==1) throw new Error('Plasma could not uniquely identify the selected monitor.');
        print('WANIENGINE_JSON:'+JSON.stringify({ok:true,screen:matches[0]}));
    "#;
    let result = eval_plasma(params, script)?;
    if result["ok"] == true {
        result["screen"]
            .as_i64()
            .map(|n| n as i32)
            .ok_or_else(|| "Plasma returned no screen id".into())
    } else {
        Err(result["error"]
            .as_str()
            .map(|error| format!("{output_name}: {error}"))
            .unwrap_or_else(|| format!("Could not identify Plasma display ‘{output_name}’")))
    }
}

pub(crate) fn eval_plasma(params: Value, body: &str) -> Result<Value, String> {
    let params = serde_json::to_string(&params).map_err(|e| e.to_string())?;
    let script = format!(
        "(function(){{var p={params};try{{{body}}}catch(e){{print('{MARKER}'+JSON.stringify({{ok:false,error:String(e)}}));}}}})();"
    );
    let connection = Connection::session()
        .map_err(|e| format!("Could not connect to the desktop session bus: {e}"))?;
    let proxy = Proxy::new(
        &connection,
        "org.kde.plasmashell",
        "/PlasmaShell",
        "org.kde.PlasmaShell",
    )
    .map_err(|e| format!("Plasma wallpaper control is unavailable: {e}"))?;
    let output: String = proxy
        .call("evaluateScript", &(script,))
        .map_err(|e| format!("Plasma rejected the wallpaper operation: {e}"))?;
    let at = output
        .rfind(MARKER)
        .ok_or("Plasma did not confirm the wallpaper operation")?
        + MARKER.len();
    serde_json::from_str(output[at..].trim())
        .map_err(|e| format!("Plasma returned an invalid confirmation: {e}"))
}

pub(crate) fn new_token() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{}-{time}-{}",
        process::id(),
        NEXT_TOKEN.fetch_add(1, Ordering::Relaxed)
    )
}

fn plasma_fill_mode(fit: &str) -> Option<i32> {
    // Qt Quick Image.FillMode values used by Plasma's built-in image wallpaper.
    match fit {
        "cover" => Some(2),
        "stretch" => Some(0),
        "contain" => Some(1),
        "center" => Some(6),
        _ => None,
    }
}

#[cfg(unix)]
fn lock_session(path: &Path) -> Option<SessionLock> {
    let parent = path.parent()?;
    fs::create_dir_all(parent).ok()?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
        .ok()?;
    SessionLock::try_lock(file)
}

#[cfg(not(unix))]
fn lock_session(_path: &Path) -> Option<SessionLock> {
    None
}

#[cfg(test)]
mod tests {
    use super::{Snapshot, lock_session, plasma_fill_mode};
    use serde_json::json;

    #[test]
    fn maps_wallpaper_fit_to_plasma_image_modes() {
        assert_eq!(plasma_fill_mode("cover"), Some(2));
        assert_eq!(plasma_fill_mode("stretch"), Some(0));
        assert_eq!(plasma_fill_mode("contain"), Some(1));
        assert_eq!(plasma_fill_mode("center"), Some(6));
        assert_eq!(plasma_fill_mode("unknown"), None);
    }

    #[cfg(unix)]
    #[test]
    fn wallpaper_session_lock_is_exclusive_and_released_on_drop() {
        let directory = std::env::temp_dir().join(format!(
            "wani-plasma-lock-{}-{}",
            std::process::id(),
            super::NEXT_TOKEN.fetch_add(1, super::Ordering::Relaxed)
        ));
        let path = directory.join("session.lock");
        let first = lock_session(&path).unwrap();
        assert!(lock_session(&path).is_none());
        drop(first);
        let third = lock_session(&path).unwrap();
        drop(third);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn recovery_accepts_snapshots_saved_before_expected_state_tracking() {
        let snapshot: Snapshot = serde_json::from_value(json!({
            "screen": 0,
            "containment_id": 12,
            "original_plugin": "org.kde.image",
            "original_config": {},
            "token": "legacy-session"
        }))
        .unwrap();
        assert!(snapshot.applied_image.is_none());
        assert!(snapshot.applied_fill_mode.is_none());
        assert!(snapshot.applied_animation.is_none());
    }
}

fn read_recovery(path: &Path) -> Option<Recovery> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
fn write_recovery(path: &Path, recovery: &Recovery) -> Result<(), String> {
    waniengine_core::library::write_json(
        path,
        &serde_json::to_value(recovery).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn process_identity(pid: u32) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.get(stat.rfind(')')? + 1..)?;
    after.split_ascii_whitespace().nth(19).map(str::to_owned)
}
fn owner_alive(recovery: &Recovery) -> bool {
    recovery.owner_pid > 0
        && process_identity(recovery.owner_pid).is_some_and(|id| id == recovery.owner_identity)
}
