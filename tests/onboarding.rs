#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;
use common::{TestEnv, state_dir_at, write_executable};
use std::{
    fs::{self, File},
    io::Write,
    os::fd::FromRawFd,
    process::Output,
    ptr::null_mut,
};
fn environment() -> TestEnv {
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
fn manifest(env: &TestEnv) -> std::path::PathBuf {
    state_dir_at(&env.home, &env.root).join("manifest.yaml")
}
fn terminal(
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
#[test]
fn first_run_registration_confirms_and_can_cancel() {
    for answer in [b"n".as_slice(), b"\x1b"] {
        let env = environment();
        let (output, _) = terminal(
            &env,
            &[],
            &[("directory:", b"\r"), ("]:", b"\r"), ("Register?", answer)],
            |_| {},
        );
        assert!(output.status.success(), "{output:?}");
        assert!(!env.home.join(".niles/projects").exists());
    }
    let env = environment();
    let (output, _) = terminal(
        &env,
        &[],
        &[("directory:", b"\r"), ("]:", b"\r"), ("Register?", b"y")],
        |_| {},
    );
    assert!(output.status.success(), "{output:?}");
    assert!(env.tmux_log().contains(" start ;"));
}
#[test]
fn wizard_solo_accepts_defaults_without_optional_imports_or_editor() {
    let env = environment();
    for name in ["design", "reviewer", "worker", "security", "research"] {
        fs::remove_dir_all(env.home.join(".niles/skills").join(name)).unwrap();
    }
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
            ("writes", b"\r"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?} {text}");
    let manifest = fs::read_to_string(manifest(&env)).unwrap();
    assert!(manifest.contains("entry: solo"), "{manifest}");
    assert!(!text.contains("lead's"));
    assert!(text.contains("docs/security.md"));
}
#[test]
fn lead_select_and_setup_cancellation_write_nothing() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
    let (output, _) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\r"),
            ("TIERS", b"\r"),
            ("Optional", b" \r"),
            ("Warning:", b"\r"),
            ("writes", b"\x1b"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?}");
    assert!(!manifest(&env).exists());
}
#[test]
fn missing_entry_fetch_failure_stops_before_manifest() {
    let env = environment();
    fs::remove_file(env.home.join(".niles/skills/solo/SKILL.md")).unwrap();
    let (output, _) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
            ("writes", b"\r"),
        ],
        |_| {},
    );
    assert!(!output.status.success());
    assert!(!manifest(&env).exists());
}

#[test]
fn existing_registry_does_not_start_first_run_registration() {
    let env = environment();
    let registry = env.home.join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    std::os::unix::fs::symlink(&env.root, registry.join("existing")).unwrap();
    let (output, text) = terminal(&env, &[], &[], |_| {});
    assert!(output.status.success(), "{output:?}");
    assert!(!text.contains("register"));
}

#[test]
fn taken_registration_name_can_be_replaced_after_confirmation() {
    let env = environment();
    let registry = env.home.join(".niles/projects");
    let other = env.root.join("other");
    fs::create_dir(&other).unwrap();
    let (output, text) = terminal(
        &env,
        &[],
        &[
            ("directory:", b"\r"),
            ("]:", b"taken\r"),
            ("Register?", b"y"),
            ("taken.", b"replacement\r"),
            ("Register?", b"y"),
        ],
        |step| {
            if step == 2 {
                fs::create_dir_all(&registry).unwrap();
                std::os::unix::fs::symlink(&other, registry.join("taken")).unwrap();
            }
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    assert_eq!(
        fs::read_link(registry.join("replacement")).unwrap(),
        env.root
    );
    assert_eq!(fs::read_link(registry.join("taken")).unwrap(), other);
}

#[test]
fn lead_saves_chosen_entry_and_edited_standard_tier() {
    let env = environment();
    let skills = env.home.join(".niles/skills");
    fs::remove_dir_all(&skills).unwrap();
    fs::create_dir_all(skills.join("custom-entry")).unwrap();
    fs::write(
        skills.join("custom-entry/SKILL.md"),
        "---\nname: custom-entry\ndescription: Coordinates this project\n---\nDo the work.\n",
    )
    .unwrap();
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("lead's", b"\x1b[B\r"),
            ("TIERS", b"\x1b[C"),
            ("Editing", b"\r\r\x1b[A\r\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
            ("writes", b"\r"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?} {text}");
    let value: serde_json::Value =
        serde_saphyr::from_str(&fs::read_to_string(manifest(&env)).unwrap()).unwrap();
    assert_eq!(value["entry"], "custom-entry");
    assert_eq!(
        value["tiers"]["standard"]["primary"],
        "claude:sonnet:medium"
    );
    assert_eq!(value["entry_agent"], "claude:sonnet:medium");
    assert_eq!(
        value["tiers"]["standard"]["secondaries"][0],
        "codex:gpt-6.1-sol:medium"
    );
}

#[test]
fn selected_starter_is_fetched_at_head_and_recorded_by_sha() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills/design")).unwrap();
    fs::write(
        env.home.join("skill.md"),
        "---\nname: design\ndescription: Plans changes\n---\nPlan the work.\n",
    )
    .unwrap();
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
"#,
    );
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\x1b[B \r"),
            ("Warning:", b"\r"),
            ("writes", b"\r"),
        ],
        |step| {
            if step == 4 {
                assert!(!env.home.join("git.log").exists());
            }
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    let sha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert_eq!(
        fs::read_to_string(env.home.join(".niles/skills/design/SKILL.ref")).unwrap(),
        format!("github.com/Rhovian/niles/skills/design@{sha}\n")
    );
    let log = fs::read_to_string(env.home.join("git.log")).unwrap();
    assert!(
        log.contains("ls-remote -- https://github.com/Rhovian/niles.git HEAD "),
        "{log}"
    );
    assert!(
        log.contains(&format!(
            "fetch --depth 1 --filter=blob:none -- https://github.com/Rhovian/niles.git {sha}"
        )),
        "{log}"
    );
    assert!(
        env.home
            .join(format!(
                ".niles/cache/skills/github.com/Rhovian/niles/{sha}/skills/design/@skill/SKILL.md"
            ))
            .exists()
    );
}

#[test]
fn concurrent_manifest_is_kept() {
    let env = environment();
    let winner = common::setup_manifest_body();
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\x1b[B\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
            ("writes", b"\r"),
        ],
        |step| {
            if step == 4 {
                fs::create_dir_all(manifest(&env).parent().unwrap()).unwrap();
                fs::write(manifest(&env), winner).unwrap();
            }
        },
    );
    assert!(output.status.success(), "{output:?} {text}");
    assert_eq!(fs::read_to_string(manifest(&env)).unwrap(), winner);
}

#[test]
fn lead_only_skips_entry_picker_and_reviews_default_entry() {
    let env = environment();
    fs::remove_dir_all(env.home.join(".niles/skills")).unwrap();
    let (output, text) = terminal(
        &env,
        &["start"],
        &[
            ("work?", b"\r"),
            ("TIERS", b"\r"),
            ("Optional", b"\r"),
            ("Warning:", b"\r"),
            ("writes", b"\x1b"),
        ],
        |_| {},
    );
    assert!(output.status.success(), "{output:?} {text}");
    assert!(!text.contains("lead's"));
    let summary = text.rsplit_once("Entry").unwrap().1;
    assert!(summary.contains("lead"), "{summary}");
    assert!(!manifest(&env).exists());
}
