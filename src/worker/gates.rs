use std::{
    fs::File,
    io::{ErrorKind, Read, Write},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command},
    time::{Duration, Instant},
};

use crate::workspace_manifest::RunGate;
use anyhow::{Context, Result};
use camino::Utf8Path;

const LOG_CAP: usize = 1_048_576;
const TAIL_CAP: usize = 4096;
const POLL: Duration = Duration::from_millis(10);

pub(super) fn run(gate: &RunGate, cwd: &Utf8Path, dir: &Utf8Path, round: usize) -> Result<bool> {
    let path = dir.join(format!("gate-{round}-{}.log", gate.name));
    let log = File::create(&path)?;
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    let stderr = writer.try_clone()?;
    let mut child = match Command::new(&gate.run[0])
        .args(&gate.run[1..])
        .current_dir(cwd)
        .stdout(OwnedFd::from(writer))
        .stderr(OwnedFd::from(stderr))
        .process_group(0)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            println!("{} fail (could not start: {error})\nlog: {path}", gate.name);
            return Ok(false);
        }
    };
    let (reason, tail) = collect(&mut child, reader, log, gate.timeout)?;
    let passed = reason.is_none();
    let outcome = reason.map_or_else(|| "pass".to_owned(), |reason| format!("fail ({reason})"));
    println!("{} {outcome}\nlog: {path}", gate.name);
    if !passed {
        // Lossy decoding can expand bytes. Reserve a byte for the printed newline.
        let text = String::from_utf8_lossy(&tail);
        let mut start = text.len().saturating_sub(TAIL_CAP - 1);
        while !text.is_char_boundary(start) {
            start += 1;
        }
        let mut lines: Vec<_> = text[start..].lines().rev().take(40).collect();
        lines.reverse();
        println!("{}", lines.join("\n"));
    }
    Ok(passed)
}

fn collect(
    child: &mut Child,
    mut reader: impl Read,
    mut log: impl Write,
    timeout: Duration,
) -> Result<(Option<String>, Vec<u8>)> {
    let started = Instant::now();
    let mut tail = Vec::new();
    let mut written = 0;
    let mut buffer = [0; 8192];
    let mut exited = None;
    // std 1.89 Child has no timed wait or capped output collector:
    // https://doc.rust-lang.org/1.89.0/std/process/struct.Child.html
    // Drain again after observing exit; never kill a reaped child's group.
    let status = (|| {
        loop {
            let n = match reader.read(&mut buffer) {
                Ok(n) => n,
                Err(error) if error.kind() == ErrorKind::WouldBlock => 0,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            let keep = n.min(LOG_CAP - written);
            log.write_all(&buffer[..keep])?;
            written += keep;
            tail.extend_from_slice(&buffer[..n]);
            tail.drain(..tail.len().saturating_sub(TAIL_CAP));
            let expired = started.elapsed() >= timeout;
            if n == 0 || expired {
                if exited.is_some() {
                    return Ok(exited);
                }
                exited = child.try_wait()?;
                if exited.is_some() {
                    continue;
                }
                if expired {
                    return Ok(None);
                }
                std::thread::sleep(POLL);
            }
        }
    })();
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            if exited.is_none() {
                terminate(child).with_context(|| format!("gate cleanup after {error:#}"))?;
            }
            return Err(error);
        }
    };
    let reason = match status {
        Some(status) => (!status.success()).then(|| {
            status
                .code()
                .map_or_else(|| status.to_string(), |code| format!("exit {code}"))
        }),
        None => {
            terminate(child)?;
            Some(format!("timeout after {}ms", timeout.as_millis()))
        }
    };
    Ok((reason, tail))
}

fn terminate(child: &mut Child) -> Result<()> {
    let group = child.id() as libc::pid_t;
    debug_assert!(group > 0);
    // SAFETY: kill takes an integer group id, no pointers. libc 0.2.189
    // src/unix/mod.rs provides only the unsafe extern, no safe group wrapper.
    if unsafe { libc::kill(-group, libc::SIGKILL) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error.into());
        }
    }
    child.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    #[test]
    fn output_written_between_empty_read_and_exit_check_is_drained() {
        struct ReleaseOnEmpty {
            reader: UnixStream,
            release: Option<(std::process::ChildStdin, u32)>,
        }
        impl Read for ReleaseOnEmpty {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let result = self.reader.read(buffer);
                if let Some((mut stdin, pid)) = self.release.take() {
                    assert_eq!(result.as_ref().unwrap_err().kind(), ErrorKind::WouldBlock);
                    stdin.write_all(b"go\n").unwrap();
                    let mut status = std::mem::MaybeUninit::<libc::siginfo_t>::uninit();
                    // SAFETY: status is writable; WNOWAIT observes exit without reaping,
                    // leaving collect's try_wait to observe the real child status.
                    assert_eq!(
                        unsafe {
                            libc::waitid(
                                libc::P_PID,
                                pid,
                                status.as_mut_ptr(),
                                libc::WEXITED | libc::WNOWAIT,
                            )
                        },
                        0
                    );
                }
                result
            }
        }
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut child = Command::new("sh")
            .args(["-c", "read line; printf private-output >&2; exit 7"])
            .stdin(std::process::Stdio::piped())
            .stderr(OwnedFd::from(writer))
            .process_group(0)
            .spawn()
            .unwrap();
        let reader = ReleaseOnEmpty {
            reader,
            release: Some((child.stdin.take().unwrap(), child.id())),
        };
        let mut log = Vec::new();
        let (reason, tail) = collect(&mut child, reader, &mut log, Duration::from_secs(5)).unwrap();
        assert_eq!(reason.as_deref(), Some("exit 7"));
        assert_eq!(log, b"private-output");
        assert_eq!(tail, b"private-output");
    }

    #[test]
    fn io_error_after_exit_does_not_kill_the_reaped_childs_group() {
        struct ErrorAfterEmpty(bool);
        impl Read for ErrorAfterEmpty {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                if std::mem::replace(&mut self.0, true) {
                    Err(ErrorKind::Other.into())
                } else {
                    Ok(0)
                }
            }
        }
        let mut child = Command::new("sh")
            .args(["-c", "read line"])
            .stdin(std::process::Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let mut holder = Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .process_group(child.id() as libc::pid_t)
            .spawn()
            .unwrap();
        drop(child.stdin.take());
        child.wait().unwrap();
        let result = collect(
            &mut child,
            ErrorAfterEmpty(false),
            std::io::sink(),
            Duration::from_secs(5),
        );
        // A live member makes an erroneous kill observable without PID reuse.
        let mut echo = [0];
        let survived = holder
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"x")
            .and_then(|()| holder.stdout.as_mut().unwrap().read_exact(&mut echo));
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert_eq!(
            result
                .unwrap_err()
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .kind(),
            ErrorKind::Other
        );
        survived.unwrap();
        assert_eq!(echo, *b"x");
    }

    #[test]
    fn read_and_log_write_errors_kill_and_reap_the_child() {
        let dir = tempfile::tempdir().unwrap();
        let readers: [Box<dyn Read>; 2] = [
            Box::new(File::open(dir.path()).unwrap()),
            Box::new(&b"x"[..]),
        ];
        for reader in readers {
            let mut child = Command::new("sleep")
                .arg("2")
                .process_group(0)
                .spawn()
                .unwrap();
            assert!(collect(&mut child, reader, &mut [][..], Duration::from_secs(1)).is_err());
            assert_eq!(
                child.try_wait().unwrap().unwrap().signal(),
                Some(libc::SIGKILL)
            );
        }
    }
    #[test]
    fn exited_gate_does_not_wait_for_output_held_outside_its_group() {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut holder = Command::new("sleep")
            .arg("2")
            .stdout(OwnedFd::from(writer.try_clone().unwrap()))
            .process_group(0)
            .spawn()
            .unwrap();
        let mut child = Command::new("true")
            .stdout(OwnedFd::from(writer))
            .process_group(0)
            .spawn()
            .unwrap();
        let result = collect(
            &mut child,
            reader,
            std::io::sink(),
            Duration::from_millis(100),
        );
        holder.kill().unwrap();
        holder.wait().unwrap();
        assert!(result.unwrap().0.is_none());
    }
}
