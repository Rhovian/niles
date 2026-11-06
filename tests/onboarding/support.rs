use crate::common::{TestEnv, state_dir_at, write_executable};
use std::{
    fs::{self, File},
    io::Write,
    os::fd::FromRawFd,
    process::Output,
    ptr::null_mut,
};
pub(super) fn environment() -> TestEnv {
    let mut env = TestEnv::with_tmux(
        "onboard",
        r#"#!/bin/sh
[ "$1" != -u ] || shift
printf '%s\n' "$*" >> "$TMUX_LOG"
case "$1" in
has-session) printf "can't find session\n" >&2; exit 1 ;;
display-message) case "$3" in *client_width*) printf '160 45\n' ;; *) printf 'onboard\n' ;; esac ;;
display) case "$*" in *pane_id*) printf '%%0\n' ;; *) printf '/tmp/onboard.sock\n' ;; esac ;;
list-panes) if [ -f "$HOME/view" ]; then printf '1\t%%2\t/dev/tty2\n'; fi ;;
split-window) touch "$HOME/view"; printf '%%2\t/dev/tty2\n' ;;
esac
"#,
    );
    let home = env.root.join("home with space");
    fs::rename(&env.home, &home).unwrap();
    env.home = home;
    write_executable(
        &env.bin.join("git"),
        "#!/bin/sh\necho unexpected import >&2\nexit 91\n",
    );

    env
}
pub(super) fn manifest(env: &TestEnv) -> std::path::PathBuf {
    state_dir_at(&env.home, &env.root).join("manifest.yaml")
}
pub(super) fn terminal(
    env: &TestEnv,
    args: &[&str],
    steps: &[(&str, &[u8])],
    mut on_prompt: impl FnMut(usize),
) -> (Output, String) {
    use std::{
        io::Read,
        os::unix::process::CommandExt,
        process::Stdio,
        time::{Duration, Instant},
    };
    let (mut pty, mut tty) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 45,
        ws_col: 180,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // libc 0.2.191 takes *const winsize on Linux and *mut winsize on macOS.
    assert_eq!(
        unsafe { libc::openpty(&mut pty, &mut tty, null_mut(), null_mut(), &raw mut size) },
        0
    );
    let (mut pty, tty) = unsafe { (File::from_raw_fd(pty), File::from_raw_fd(tty)) };
    let mut command = env.niles(&env.root, args);
    command
        .env("PATH", format!("{}:/usr/bin:/bin", env.bin.display()))
        .env("VISUAL", "false")
        .env("EDITOR", "false")
        .stdin(tty.try_clone().unwrap())
        .stdout(tty.try_clone().unwrap())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(command);
    drop(tty);
    let mut transcript = String::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    // ratatui 0.30.2's CrosstermBackend emits cursor moves between words and
    // only changed cells on redraw; steps use complete, unique prompt tokens.
    let mut step = 0;
    let mut seen = String::new();
    loop {
        let mut poll = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&pty),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut poll, 1, 50) } > 0 {
            let mut buffer = [0; 16384];
            match pty.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    let text = String::from_utf8_lossy(&buffer[..count]);
                    transcript.push_str(&text);
                    seen.push_str(&text);
                    if step < steps.len() && seen.contains(steps[step].0) {
                        on_prompt(step);
                        pty.write_all(steps[step].1).unwrap();
                        step += 1;
                        seen.clear();
                    }
                }
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("{error}"),
            }
        }
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            panic!("wizard timed out at step {step}: {transcript}");
        }
    }
    let output = child.wait_with_output().unwrap();
    assert_eq!(step, steps.len(), "{transcript}");
    (output, transcript)
}

pub(super) fn import_fixture(env: &TestEnv, name: &str, body: &str) {
    fs::write(env.home.join("skill.md"), body).unwrap();
    write_executable(
        &env.bin.join("git"),
        r#"#!/bin/sh
printf '%s\n' "$*" >> "$HOME/git.log"
while [ "$#" -gt 0 ]; do
  case "$1" in
    ls-remote) case "$*" in *' HEAD '*) printf 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tHEAD\n'; exit 0 ;; *) exit 91 ;; esac ;;
    init) mkdir .git; exit 0 ;;
    fetch|sparse-checkout) exit 0 ;;
    ls-tree) printf '040000 tree aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\tskills/design\n'; exit 0 ;;
    checkout) mkdir -p skills/design; cp "$HOME/skill.md" skills/design/SKILL.md; exit 0 ;;
    *) shift ;;
  esac
done
exit 91
"#.replace("design", name).as_str(),
    );
}
