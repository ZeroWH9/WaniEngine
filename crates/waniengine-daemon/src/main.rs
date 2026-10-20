use waniengine_core::{ActorInstance, Profile, ProfileStore, StoreError};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_REQUEST_BYTES: u64 = 64 * 1024;
const SOCKET_NAME: &str = "waniengine.sock";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("waniengine-daemon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<std::ffi::OsString>) -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(args)?;
    if options.help {
        print_help();
        return Ok(());
    }

    let socket_path = options.socket.unwrap_or(default_socket_path()?);
    let store_path = options.state.unwrap_or(default_state_path()?);
    let listener = bind_socket(&socket_path)?;
    let store = Arc::new(Mutex::new(ProfileStore::new(&store_path)));

    println!("Socket: {}", socket_path.display());
    println!("Profile: {}", store_path.display());
    println!("Ready. Press Ctrl+C to stop.");

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let request_store = Arc::clone(&store);
                if let Err(error) = thread::Builder::new()
                    .name("waniengine-ipc-client".to_owned())
                    .spawn(move || handle_client(stream, request_store))
                {
                    eprintln!("could not serve client: {error}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => break,
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

#[derive(Default)]
struct Options {
    socket: Option<PathBuf>,
    state: Option<PathBuf>,
    help: bool,
}

impl Options {
    fn parse(args: Vec<std::ffi::OsString>) -> Result<Self, Box<dyn std::error::Error>> {
        let mut result = Self::default();
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            match arg.to_str() {
                Some("--socket") => {
                    result.socket = Some(PathBuf::from(
                        iter.next().ok_or("--socket requires a path")?,
                    ));
                }
                Some("--state") => {
                    result.state = Some(PathBuf::from(
                        iter.next().ok_or("--state requires a path")?,
                    ));
                }
                Some("--help" | "-h") => result.help = true,
                Some(other) => return Err(format!("unknown option: {other}").into()),
                None => return Err("option contains non-UTF-8 text".into()),
            }
        }
        Ok(result)
    }
}

fn default_socket_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .ok_or("XDG_RUNTIME_DIR is not set; provide --socket")?;
    Ok(PathBuf::from(runtime).join(SOCKET_NAME))
}

fn default_state_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let state_home = if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
        PathBuf::from(state_home)
    } else {
        PathBuf::from(env::var_os("HOME").ok_or("HOME is not set; provide --state")?)
            .join(".local")
            .join("state")
    };
    Ok(state_home.join("waniengine").join("profile.json"))
}

fn bind_socket(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to replace a file that is not a socket: {}",
                    path.display()
                ),
            ));
        }
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("o serviço já está ativo em {}", path.display()),
            ));
        }
        fs::remove_file(path)?;
    }

    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn handle_client(mut stream: UnixStream, store: Arc<Mutex<ProfileStore>>) {
    let response = match read_request(&stream) {
        Ok(Some(request)) => match store.lock() {
            Ok(store) => dispatch(request, &store),
            Err(_) => Reply::Error {
                message: "service state is unavailable".to_owned(),
            },
        },
        Ok(None) => return,
        Err(message) => Reply::Error { message },
    };

    match serde_json::to_vec(&response) {
        Ok(bytes) => {
            if let Err(error) = stream
                .write_all(&bytes)
                .and_then(|()| stream.write_all(b"\n"))
            {
                eprintln!("failed to reply to client: {error}");
            }
        }
        Err(error) => eprintln!("failed to encode response: {error}"),
    }
}

fn read_request(stream: &UnixStream) -> Result<Option<Request>, String> {
    let cloned = stream.try_clone().map_err(|error| error.to_string())?;
    let reader = BufReader::new(cloned);
    let mut bytes = Vec::new();
    reader
        .take(MAX_REQUEST_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|error| error.to_string())?;

    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("a mensagem ultrapassa 64 KiB".to_owned());
    }

    parse_request(&bytes).map(Some)
}

fn parse_request(bytes: &[u8]) -> Result<Request, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid JSON command: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "o comando precisa ser um objeto JSON".to_owned())?;
    let command = object
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "o comando precisa informar o campo string command".to_owned())?;
    let allowed_fields: &[&str] = match command {
        "ping" | "get_state" => &["command"],
        "set_paused" => &["command", "paused"],
        "set_wallpaper" => &["command", "path"],
        "upsert_actor" => &["command", "actor"],
        "remove_actor" => &["command", "instance_id"],
        other => return Err(format!("comando desconhecido: {other}")),
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !allowed_fields.contains(&key.as_str()))
    {
        return Err(format!("unsupported field: {unknown}"));
    }

    serde_json::from_value(value).map_err(|error| format!("invalid command: {error}"))
}

fn dispatch(request: Request, store: &ProfileStore) -> Reply {
    match request {
        Request::Ping => Reply::Pong,
        Request::GetState => match store.load() {
            Ok(profile) => Reply::State { profile },
            Err(error) => Reply::from_store_error(error),
        },
        Request::SetPaused { paused } => update(store, |profile| {
            if profile.paused == paused {
                return Ok(false);
            }
            profile.paused = paused;
            Ok(true)
        }),
        Request::SetWallpaper { path } => {
            let Some(path) = path else {
                return update(store, |profile| {
                    if profile.wallpaper.is_none() {
                        return Ok(false);
                    }
                    profile.wallpaper = None;
                    Ok(true)
                });
            };
            if !path.is_absolute() || !path.is_file() {
                return Reply::Error {
                    message: "wallpaper must be an existing file with an absolute path".to_owned(),
                };
            }
            update(store, |profile| {
                if profile.wallpaper.as_ref() == Some(&path) {
                    return Ok(false);
                }
                profile.wallpaper = Some(path);
                Ok(true)
            })
        }
        Request::UpsertActor { actor } => update(store, |profile| {
            if let Some(existing) = profile
                .actors
                .iter_mut()
                .find(|item| item.instance_id == actor.instance_id)
            {
                *existing = actor;
            } else {
                profile.actors.push(actor);
            }
            Ok(true)
        }),
        Request::RemoveActor { instance_id } => update(store, |profile| {
            let old_len = profile.actors.len();
            profile
                .actors
                .retain(|actor| actor.instance_id != instance_id);
            Ok(old_len != profile.actors.len())
        }),
    }
}

fn update(
    store: &ProfileStore,
    change: impl FnOnce(&mut Profile) -> Result<bool, String>,
) -> Reply {
    let mut profile = match store.load() {
        Ok(profile) => profile,
        Err(error) => return Reply::from_store_error(error),
    };

    match change(&mut profile) {
        Ok(false) => Reply::State { profile },
        Err(message) => Reply::Error { message },
        Ok(true) => {
            if let Err(error) = profile.bump_revision() {
                return Reply::from_store_error(StoreError::Profile(error));
            }
            if let Err(error) = profile.validate() {
                return Reply::from_store_error(StoreError::Profile(error));
            }
            match store.save(&profile) {
                Ok(()) => Reply::State { profile },
                Err(error) => Reply::from_store_error(error),
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Ping,
    GetState,
    SetPaused { paused: bool },
    SetWallpaper { path: Option<PathBuf> },
    UpsertActor { actor: ActorInstance },
    RemoveActor { instance_id: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply {
    Pong,
    State { profile: Profile },
    Error { message: String },
}

impl Reply {
    fn from_store_error(error: StoreError) -> Self {
        Self::Error {
            message: error.to_string(),
        }
    }
}

fn print_help() {
    println!(
        "WaniEngine profile daemon\n\
         Usage: waniengine-daemon [--socket PATH] [--state PATH]\n\
         O IPC aceita uma mensagem JSON por conexão e usa um socket privado do usuário.\n\
         The get_state command always responds with the current profile."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use waniengine_core::AnimationState;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    struct TempProfile(PathBuf);

    impl TempProfile {
        fn new() -> Self {
            let id = NEXT_TEST.fetch_add(1, Ordering::Relaxed);
            let directory = env::temp_dir().join(format!(
                "waniengine-daemon-{}-{id}",
                std::process::id()
            ));
            Self(directory.join("profile.json"))
        }

        fn store(&self) -> ProfileStore {
            ProfileStore::new(&self.0)
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            if let Some(parent) = self.0.parent() {
                let _ = fs::remove_dir_all(parent);
            }
        }
    }

    fn actor(id: &str, x: f64) -> ActorInstance {
        ActorInstance {
            instance_id: id.to_owned(),
            asset_id: "local.chibi".to_owned(),
            x,
            y: 0.9,
            scale: 1.0,
            state: AnimationState::Dance,
        }
    }

    #[test]
    fn pause_commands_only_advance_revision_when_state_changes() {
        let temp = TempProfile::new();
        let store = temp.store();

        let first = dispatch(Request::SetPaused { paused: true }, &store);
        assert!(
            matches!(first, Reply::State { profile } if profile.paused && profile.revision == 1)
        );

        let second = dispatch(Request::SetPaused { paused: true }, &store);
        assert!(
            matches!(second, Reply::State { profile } if profile.paused && profile.revision == 1)
        );

        let third = dispatch(Request::SetPaused { paused: false }, &store);
        assert!(
            matches!(third, Reply::State { profile } if !profile.paused && profile.revision == 2)
        );
    }

    #[test]
    fn actor_updates_replace_by_instance_id_and_invalid_ones_do_not_persist() {
        let temp = TempProfile::new();
        let store = temp.store();

        assert!(matches!(
            dispatch(Request::UpsertActor { actor: actor("actor-1", 0.5) }, &store),
            Reply::State { profile } if profile.actors.len() == 1
        ));
        assert!(matches!(
            dispatch(Request::UpsertActor { actor: actor("actor-1", 0.8) }, &store),
            Reply::State { profile } if profile.actors.len() == 1 && profile.actors[0].x == 0.8 && profile.revision == 2
        ));
        assert!(matches!(
            dispatch(
                Request::UpsertActor {
                    actor: actor("../escape", 0.1)
                },
                &store
            ),
            Reply::Error { .. }
        ));
        assert_eq!(store.load().expect("load valid actor"), {
            let mut profile = Profile {
                revision: 2,
                ..Profile::default()
            };
            profile.actors.push(actor("actor-1", 0.8));
            profile
        });
    }

    #[test]
    fn rejects_a_wallpaper_path_that_does_not_exist() {
        let temp = TempProfile::new();
        let response = dispatch(
            Request::SetWallpaper {
                path: Some(PathBuf::from("/does/not/exist.mp4")),
            },
            &temp.store(),
        );
        assert!(matches!(response, Reply::Error { .. }));
        assert!(!temp.0.exists());
    }

    #[test]
    fn parses_one_json_command_and_rejects_unknown_fields() {
        let valid = br#"{"command":"set_paused","paused":true}"#;
        assert!(matches!(
            parse_request(valid),
            Ok(Request::SetPaused { paused: true })
        ));

        let invalid = br#"{"command":"ping","arbitrary":"value"}"#;
        assert!(parse_request(invalid).is_err());
    }
}
