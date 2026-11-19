//! Reentrant per-thread advisory lock, exclusive across threads and processes.
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::{File, OpenOptions},
    io,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    thread,
    time::{Duration, Instant},
};

thread_local! { static HELD: RefCell<HashMap<PathBuf, Weak<File>>> = RefCell::default(); }

pub struct FileLock {
    _file: Rc<File>,
}

/// Non-reentrant session lock, transferable between manager threads. Explicit
/// unlock prevents a concurrent fork retaining the lock until its exec/exit.
pub struct SessionLock(File);
impl SessionLock {
    pub fn try_lock(file: File) -> Option<Self> {
        (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0)
            .then_some(Self(file))
    }
}
impl Drop for SessionLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
impl Drop for FileLock {
    fn drop(&mut self) {
        if Rc::strong_count(&self._file) == 1 {
            unsafe {
                libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

impl FileLock {
    pub fn acquire(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("Lock has no parent"))?;
        std::fs::create_dir_all(parent)?;
        let key = parent.canonicalize()?.join(
            path.file_name()
                .ok_or_else(|| io::Error::other("Invalid lock path"))?,
        );
        if let Some(file) = HELD.with(|held| held.borrow().get(&key).and_then(Weak::upgrade)) {
            return Ok(Self { _file: file });
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&key)?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::WouldBlock {
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Another operation is updating this library",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        let file = Rc::new(file);
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            held.retain(|_, weak| weak.strong_count() > 0);
            held.insert(key, Rc::downgrade(&file));
        });
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forked_descriptor_does_not_delay_session_unlock() {
        let path = std::env::temp_dir().join(format!("wani-fork-lock-{}", std::process::id()));
        let open = || {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap()
        };
        let first = SessionLock::try_lock(open()).unwrap();
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            // No allocation or Rust teardown after fork in a threaded test.
            unsafe {
                libc::raise(libc::SIGSTOP);
                libc::_exit(0);
            }
        }
        let mut status = 0;
        unsafe {
            libc::waitpid(child, &mut status, libc::WUNTRACED);
        }
        drop(first);
        let replacement = SessionLock::try_lock(open());
        unsafe {
            libc::kill(child, libc::SIGKILL);
            libc::waitpid(child, &mut status, 0);
        }
        assert!(
            replacement.is_some(),
            "a forked child retained the released session lock"
        );
        drop(replacement);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn nested_access_is_reentrant_but_other_threads_wait() {
        let directory = std::env::temp_dir().join(format!("wani-lock-{}", std::process::id()));
        let path = directory.join("lock");
        let outer = FileLock::acquire(&path).unwrap();
        let inner = FileLock::acquire(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || {
            let _guard = FileLock::acquire(&path).unwrap();
            tx.send(()).unwrap();
        });
        drop(inner);
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(outer);
        rx.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
