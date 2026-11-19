//! Per-user shutdown socket used by the installer before replacing a running app.
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const SOCKET_NAME: &str = "waniengine-control.sock";
const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub struct ManagerControl {
    receiver: Receiver<ControlRequest>,
    // Drop this last so wallpaper and overlay cleanup finishes before the socket disappears.
    _server: ControlServer,
}

impl ManagerControl {
    pub fn take_exit(&self) -> bool {
        matches!(self.receiver.try_recv(), Ok(ControlRequest::Exit))
    }
}

enum ControlRequest {
    Exit,
}

struct ControlServer {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if fs::symlink_metadata(&self.path).is_ok_and(|metadata| metadata.file_type().is_socket()) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn start() -> io::Result<ManagerControl> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR is not set"))?;
    let path = runtime.join(SOCKET_NAME);
    let listener = bind_socket(&path)?;
    let (sender, receiver) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("waniengine-control".into())
        .spawn(move || serve(listener, sender, thread_stop))?;
    Ok(ManagerControl {
        receiver,
        _server: ControlServer {
            path,
            stop,
            thread: Some(worker),
        },
    })
}

fn bind_socket(path: &Path) -> io::Result<UnixListener> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to replace a non-socket file: {}", path.display()),
            ));
        }
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("WaniEngine is already running: {}", path.display()),
            ));
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn serve(listener: UnixListener, sender: Sender<ControlRequest>, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_request(stream, &sender),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                eprintln!("waniengine control socket: {error}");
                break;
            }
        }
    }
}

fn handle_request(mut stream: UnixStream, sender: &Sender<ControlRequest>) {
    if stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .is_err()
    {
        return;
    }
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    let mut request = Vec::new();
    let read = BufReader::new(reader_stream)
        .take(MAX_REQUEST_BYTES as u64 + 1)
        .read_until(b'\n', &mut request);
    if !matches!(read, Ok(length) if length <= MAX_REQUEST_BYTES) {
        let _ = stream.write_all(b"{\"error\":\"invalid request\"}\n");
        return;
    }
    if is_exit_request(&request) && sender.send(ControlRequest::Exit).is_ok() {
        let _ = stream.write_all(b"{\"ok\":true}\n");
    } else {
        let _ = stream.write_all(b"{\"error\":\"unknown action\"}\n");
    }
}

fn is_exit_request(request: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(request)
        .ok()
        .and_then(|value| value["action"].as_str().map(str::to_owned))
        .as_deref()
        == Some("exit")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn temp_runtime() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "waniengine-control-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn only_accepts_shutdown_from_a_local_client_and_cleans_up_socket() {
        let runtime = temp_runtime();
        let path = runtime.join(SOCKET_NAME);
        let listener = match bind_socket(&path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                eprintln!("skipping AF_UNIX integration because the sandbox denies socket bind");
                fs::remove_dir_all(runtime).unwrap();
                return;
            }
            Err(error) => panic!("could not bind test socket: {error}"),
        };
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, sender, worker_stop));
        let server = ControlServer {
            path: path.clone(),
            stop,
            thread: Some(worker),
        };

        let mut client = UnixStream::connect(&path).unwrap();
        client.write_all(b"{\"action\":\"exit\"}\n").unwrap();
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .is_ok_and(|request| matches!(request, ControlRequest::Exit))
        );
        drop(server);
        assert!(!path.exists());
        fs::remove_dir(runtime).unwrap();
    }

    #[test]
    fn only_the_exit_action_is_accepted() {
        assert!(is_exit_request(br#"{"action":"exit"}"#));
        assert!(!is_exit_request(br#"{"action":"restart"}"#));
        assert!(!is_exit_request(b"not json"));
    }

    #[test]
    fn refuses_to_replace_a_regular_file_at_the_socket_path() {
        let runtime = temp_runtime();
        let path = runtime.join(SOCKET_NAME);
        fs::write(&path, b"keep").unwrap();
        assert_eq!(
            bind_socket(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep");
        fs::remove_dir_all(runtime).unwrap();
    }
}
