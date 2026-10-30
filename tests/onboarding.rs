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
    let editor = r#"#!/bin/sh
[ "$1" != -w ] || shift
ls -ld "$1" > "$HOME/mode"
printf '%s' "$1" > "$HOME/draft"
cp "$HOME/edit" "$1"
[ ! -f "$HOME/next" ] || mv "$HOME/next" "$HOME/edit"
[ ! -f "$HOME/winner" ] || cp "$HOME/winner" "$MANIFEST"
"#;
    for name in ["vi", "visual", "editor"] {
        write_executable(&env.bin.join(name), editor);
    }
    env
}
fn manifest(env: &TestEnv) -> std::path::PathBuf {
    state_dir_at(&env.home, &env.root).join("manifest.yaml")
}
fn terminal(env: &TestEnv, args: &[&str], input: &[u8], editors: &[(&str, &str)]) -> Output {
    let (mut pty, mut tty) = (-1, -1);
    let opened = unsafe { libc::openpty(&mut pty, &mut tty, null_mut(), null_mut(), null_mut()) };
    assert_eq!(opened, 0);
    let (mut pty, tty) = unsafe { (File::from_raw_fd(pty), File::from_raw_fd(tty)) };
    pty.write_all(input).unwrap();
    env.niles(&env.root, args)
        .env("PATH", format!("{}:/usr/bin:/bin", env.bin.display()))
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .envs(editors.iter().copied())
        .env("MANIFEST", manifest(env))
        .stdin(tty)
        .output()
        .unwrap()
}
fn clean_draft(env: &TestEnv) {
    let path = fs::read_to_string(env.home.join("draft")).unwrap();
    assert!(!std::path::Path::new(&path).exists());
    assert!(std::path::Path::new(&path).starts_with(env.home.join(".niles/state/v2")));
    let mode = fs::read_to_string(env.home.join("mode")).unwrap();
    assert!(mode.starts_with("-rw-------"));
}
const EDIT: &str = "# keep my text\nentry: solo\ntiers: {standard: {primary: claude:sonnet:high, secondaries: [codex:gpt-6.1-sol:medium]}, economy: {primary: claude:sonnet:medium, secondaries: []}}\n";
#[test]
fn first_run_registers_then_setup_launches_without_picker_or_reimport() {
    let env = environment();
    let skill = env.home.join(".niles/skills/design");
    fs::remove_file(skill.join("SKILL.md")).unwrap();
    let pin = "github.com/Rhovian/niles/skills/design@aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n";
    fs::write(skill.join("SKILL.ref"), pin).unwrap();
    assert!(terminal(&env, &[], b"\n", &[]).status.success());
    let log = env.tmux_log();
    assert!(log.contains(" start ;"));
    assert!(!log.contains("--keep-skills"));
    assert!(
        log.lines()
            .any(|s| s.starts_with("switch-client") && s.ends_with(":=niles"))
    );
    assert!(terminal(&env, &["start"], b"\n", &[]).status.success());
    assert!(manifest(&env).exists());
}
#[test]
fn existing_registry_never_prompts_in_subdirectory_or_home() {
    let mut env = environment();
    let registry = env.home.join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    std::os::unix::fs::symlink(&env.root, registry.join("existing")).unwrap();
    for cwd in [env.root.join("bin"), env.home.clone()] {
        let root = std::mem::replace(&mut env.root, cwd);
        let output = terminal(&env, &[], b"\x04", &[]);
        assert!(output.status.success(), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("No projects yet"));
        env.root = root;
    }
}
#[test]
fn skip_and_eof_leave_setup_or_registration_untouched() {
    let env = environment();
    for args in [&[][..], &["start"]] {
        for input in [b"junk\nn\n".as_slice(), b"\x04"] {
            let output = terminal(&env, args, input, &[]);
            assert_eq!(output.status.success(), input != b"\x04");
            assert!(!manifest(&env).exists());
            assert!(!env.home.join(".niles/projects").exists());
        }
    }
    assert!(!env.run(&["start"]).status.success());
    let unattended = terminal(&env, &["start", "--keep-skills"], b"\n", &[]);
    assert!(!unattended.status.success());
    for name in ["claude", "codex"] {
        fs::remove_file(env.bin.join(name)).unwrap();
    }
    assert!(!terminal(&env, &["start"], b"n\n", &[]).status.success());
}
#[test]
fn custom_preserves_text_and_editor_precedence() {
    for editors in [
        &[("VISUAL", "visual -w"), ("EDITOR", "false")][..],
        &[("EDITOR", "editor -w")],
        &[],
    ] {
        let env = environment();
        fs::write(env.home.join("edit"), EDIT).unwrap();
        assert!(terminal(&env, &["start"], b"c\n", editors).status.success());
        assert_eq!(fs::read_to_string(manifest(&env)).unwrap(), EDIT);
        clean_draft(&env);
    }
}
#[test]
fn custom_validation_abort_retry_and_failed_pin() {
    for (invalid, input) in [
        ("[".to_owned(), "c\nq\n"),
        ("[".to_owned(), "c\n\x04"),
        (EDIT.replace("primary:", "unknown:"), "c\nq\n"),
        (
            EDIT.replace("claude:sonnet:high", "unknown:model:high"),
            "c\nq\n",
        ),
        (EDIT.replace("solo", "unavailable"), "c\n"),
    ] {
        let env = environment();
        fs::write(env.home.join("edit"), invalid).unwrap();
        let output = terminal(&env, &["start"], input.as_bytes(), &[]);
        assert_eq!(output.status.success(), input.ends_with("q\n"));
        assert!(!manifest(&env).exists());
        assert!(!env.home.join(".niles/projects").exists());
        clean_draft(&env);
    }
    let env = environment();
    fs::write(env.home.join("edit"), "[").unwrap();
    fs::write(env.home.join("next"), EDIT).unwrap();
    assert!(terminal(&env, &["start"], b"c\n\n", &[]).status.success());
    assert_eq!(fs::read_to_string(manifest(&env)).unwrap(), EDIT);
}
#[test]
fn concurrent_manifest_is_kept() {
    let env = environment();
    fs::write(env.home.join("edit"), EDIT).unwrap();
    let winner = common::setup_manifest_body();
    fs::write(env.home.join("winner"), winner).unwrap();
    assert!(terminal(&env, &["start"], b"c\n", &[]).status.success());
    assert_eq!(fs::read_to_string(manifest(&env)).unwrap(), winner);
}
#[test]
fn invalid_name_reprompts() {
    let mut env = environment();
    let root = env.root.clone();
    let path = root.join("invalid name");
    fs::create_dir(&path).unwrap();
    env.root = path;
    let output = terminal(&env, &[], b"\nreplacement\n", &[]);
    assert!(output.status.success(), "{output:?}");
    let target = fs::read_link(env.home.join(".niles/projects/replacement")).unwrap();
    assert_eq!(target, env.root);
    env.root = root;
}
