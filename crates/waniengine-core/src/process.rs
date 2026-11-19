//! Subprocesses belong to the manager's lifetime, including abrupt termination.
use std::process::Command;

/// Probe the exact pacing option used by the direct renderer, once per process.
/// Old FFmpeg builds can still serve still images; they must not advertise video.
pub fn direct_video_decoder_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        capture(
            Command::new("ffmpeg").args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-readrate_catchup",
                "1",
                "-f",
                "lavfi",
                "-i",
                "color=size=2x2:rate=1",
                "-frames:v",
                "0",
                "-f",
                "null",
                "-",
            ]),
            std::time::Duration::from_secs(3),
        )
        .is_ok_and(|output| output.status.success())
    })
}

/// The spawning thread must stay alive while its subprocess runs (Linux PDEATHSIG semantics).
pub fn die_with_parent(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let parent = unsafe { libc::getpid() };
        // Only async-signal-safe operations are performed between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    libc::_exit(1);
                }
                Ok(())
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
        time::{Duration, Instant},
    };
    #[test]
    #[ignore = "subprocess fixture for abrupt_parent_exit_terminates_its_decoder"]
    fn parent_fixture() {
        if std::env::var_os("WANI_PARENT_FIXTURE").is_none() {
            return;
        }
        let mut command = Command::new("sleep");
        command.arg("30");
        die_with_parent(&mut command);
        let mut child = command.spawn().unwrap();
        println!("CHILD_PID:{}", child.id());
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let _ = child.kill();
        let _ = child.wait();
    }
    #[test]
    fn abrupt_parent_exit_terminates_its_decoder() {
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process::tests::parent_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env("WANI_PARENT_FIXTURE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = BufReader::new(parent.stdout.take().unwrap());
        let mut line = String::new();
        let child = loop {
            line.clear();
            assert!(input.read_line(&mut line).unwrap() > 0);
            if let Some((_, pid)) = line.split_once("CHILD_PID:") {
                break pid.trim().parse::<u32>().unwrap();
            }
        };
        parent.kill().unwrap();
        parent.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let running = std::fs::read_to_string(format!("/proc/{child}/stat"))
                .ok()
                .is_some_and(|s| {
                    s.rsplit_once(") ").is_some_and(|(_, fields)| {
                        !fields.starts_with('Z') && !fields.starts_with('X')
                    })
                });
            if !running {
                break;
            }
            assert!(Instant::now() < deadline, "decoder survived parent");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Capture small command output with a deadline and a hard limit on both pipes.
pub fn capture(
    command: &mut Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::{
        io::Read,
        os::{fd::AsRawFd, unix::process::CommandExt},
        process::Stdio,
        time::Instant,
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    let group = child.id() as i32;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    for fd in [out.as_raw_fd(), err.as_raw_fd()] {
        // Read both pipes on the caller thread. A background grandchild holding
        // a pipe open must not turn the output-reader join into an unbounded wait.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
            let _ = child.wait();
            return Err(std::io::Error::last_os_error());
        }
    }
    fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> std::io::Result<bool> {
        let mut block = [0u8; 4096];
        loop {
            match pipe.read(&mut block) {
                Ok(0) => return Ok(true),
                Ok(n) if bytes.len() + n <= 65536 => bytes.extend_from_slice(&block[..n]),
                Ok(_) => {
                    return Err(std::io::Error::other(
                        "Backend command exceeded its output bound",
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
    }
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let deadline = Instant::now() + timeout;
    let result = (|| {
        loop {
            let out_done = drain(&mut out, &mut stdout)?;
            let err_done = drain(&mut err, &mut stderr)?;
            if let Some(status) = child.try_wait()? {
                if out_done && err_done {
                    return Ok(std::process::Output {
                        status,
                        stdout,
                        stderr,
                    });
                }
            }
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Backend command exceeded its deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })();
    if result.is_err() {
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod capture_tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn background_process_cannot_keep_capture_waiting() {
        let now = Instant::now();
        let error = capture(
            Command::new("sh").args(["-c", "sleep 30 & exit 0"]),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(now.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn noisy_command_has_bounded_output() {
        assert!(
            capture(
                Command::new("sh").args(["-c", "yes x"]),
                Duration::from_secs(2)
            )
            .unwrap_err()
            .to_string()
            .contains("output bound")
        );
    }
}
