use super::*;
use crate::agents::InvocationDefaults;
use crate::harness::native::NativeTui;
use crate::tmux::{SessionName, TmuxTarget};
use crate::{agent_window, agents, store::atomic_write, test_support};
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use std::{
    collections::BTreeMap,
    env, fs,
    fs::File,
    io::ErrorKind,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

// Like the existing real-tmux fixtures, the fake accepts bracketed paste without advertising
// paste mode. Perl's record separator preserves literal newlines until the submit CR.
// Rename publishes a complete receipt: https://perldoc.perl.org/functions/rename
const CLI: &str = r#"#!/bin/sh
set -eu
for arg do :; done
stty -icanon -icrnl
printf '%s' "$arg" > "$RECEIPT.tmp"
mv "$RECEIPT.tmp" "$RECEIPT"
printf 'ready\n'
exec perl -e '$| = 1; $/ = "\r"; while (<STDIN>) { s/^\e\[200~//; s/\e\[201~\r$//; open my $f, ">", "$ENV{RECEIPT}.tmp" or die $!; print $f $_; close $f or die $!; rename "$ENV{RECEIPT}.tmp", $ENV{RECEIPT} or die $!; print "\nreceived\n"; }'
"#;

struct Native {
    agent: String,
    cli: Utf8PathBuf,
    root: Utf8PathBuf,
    launched: Option<Launch>,
}

struct Launch {
    pane: String,
    root: Utf8PathBuf,
}
impl Launch {
    fn adapter(&self) -> NativeTui {
        NativeTui {
            target: TmuxTarget::pane(&self.pane).unwrap(),
        }
    }
}

fn launch(root: &Utf8Path, agent: &str, cli: &Utf8Path, first: &str) -> Launch {
    fs::create_dir_all(root.join("bin")).unwrap();
    let mut invocation = agents::invocation(agent, InvocationDefaults::Worker).unwrap();
    let profile_env = invocation.env.clone();
    let script = Utf8Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/harness_live.py");
    test_support::write_executable_script(
        &root.join("bin").join(invocation.binary),
        &format!(
            "#!/bin/sh\nexec python3 {} exec -- \"$@\"\n",
            agent_window::shell_quote(script.as_str())
        ),
    );
    for (key, value) in [
        (
            "PATH",
            format!("{}:{}", root.join("bin"), env::var("PATH").unwrap()),
        ),
        ("RECEIPT", root.join("receipt").into_string()),
        ("NILES_CONFORMANCE_CLI", cli.to_string()),
        (
            "NILES_CONFORMANCE_RECORD",
            root.join("exec.json").into_string(),
        ),
        (
            "NILES_CONFORMANCE_ENV_KEYS",
            serde_json::to_string(&profile_env.iter().map(|(key, _)| key).collect::<Vec<_>>())
                .unwrap(),
        ),
    ] {
        invocation.env.push((key.into(), value));
    }
    let brief = root.join("brief.md");
    atomic_write(&brief, first).unwrap();
    let target = agent_window::launch_worker_window(
        &SessionName::new("conformance").unwrap(),
        &format!("probe-{}", uuid::Uuid::new_v4()),
        root,
        &invocation,
        &agent_window::WorkerPaths {
            brief: &brief,
            launch: &root.join("launch.sh"),
            status: &root.join("status.log"),
        },
    )
    .unwrap();
    let pane = tmux_output(&[
        "display-message",
        "-p",
        "-t",
        &target.target_arg(),
        "#{pane_id}",
    ])
    .trim()
    .to_owned();
    wait_for("argv record", Duration::from_secs(30), || {
        Ok(root.join("exec.json").try_exists()?.then_some(()))
    })
    .unwrap();
    let record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("exec.json")).unwrap()).unwrap();
    let mut expected = invocation.args;
    expected.push(first.into());
    assert_eq!(record.get("argv").unwrap(), &serde_json::json!(expected));
    assert_eq!(
        record.get("env").unwrap(),
        &serde_json::to_value(profile_env.into_iter().collect::<BTreeMap<_, _>>()).unwrap()
    );
    Launch {
        pane,
        root: root.to_owned(),
    }
}

impl Fixture for Native {
    type Adapter = NativeTui;
    fn start(&mut self, first: &str) -> Self::Adapter {
        let launched = launch(
            &self.root.join(uuid::Uuid::new_v4().to_string()),
            &self.agent,
            &self.cli,
            first,
        );
        wait_for("fake CLI startup", Duration::from_secs(30), || {
            Ok(launched.root.join("receipt").try_exists()?.then_some(()))
        })
        .unwrap();
        let adapter = launched.adapter();
        self.launched = Some(launched);
        adapter
    }
    fn received(&self) -> String {
        take_receipt(
            &self.launched.as_ref().unwrap().root,
            Duration::from_secs(30),
        )
        .unwrap()
    }
    fn disposition(&self) -> InputDisposition {
        InputDisposition::Unconfirmed
    }
    fn disconnect(&mut self) {
        tmux_output(&["kill-pane", "-t", &self.launched.as_ref().unwrap().pane]);
    }
    fn restart(&mut self) -> Self::Adapter {
        let adapter = self.start("restart");
        assert_eq!(self.received(), "restart");
        adapter
    }
    fn session(&self) -> SessionId {
        SessionId::new("native-identity-not-resumable".into()).unwrap()
    }
    fn auth_path(&self) -> &str {
        "fixture/no-auth"
    }
}

fn tmux_output(args: &[&str]) -> String {
    let output = Command::new("tmux").args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
#[ignore = "subprocess entry point; native_families supplies an isolated tmux server and PATH"]
fn native_child() {
    let root = Utf8PathBuf::from(env::var("NILES_CONFORMANCE_ROOT").unwrap());
    for profile in agents::profiles() {
        let dir = root.join(format!("{}-run", profile.id));
        fs::create_dir_all(&dir).unwrap();
        eprintln!(
            "adapter=native-tui family={} protocol=none cli=fixture",
            profile.id
        );
        let catalogue = agents::ModelCatalogue::fixture();
        let (model, efforts) = catalogue.models(profile.id).unwrap().next().unwrap();
        let agent = agents::AgentSpec::from_parts(
            profile.id,
            Some(model),
            efforts.first().map(String::as_str),
        )
        .unwrap()
        .canonical();
        super::run(&mut Native {
            agent,
            cli: root.join("fake-cli"),
            root: dir,
            launched: None,
        });
    }
}

#[test]
fn native_families() {
    // Environment changes belong in a child: unit tests share a process and run concurrently.
    let root = test_support::temp_dir("harness-conformance");
    test_support::write_executable_script(&root.join("fake-cli"), CLI);
    let server = Server::start();
    let mut child = Command::new(env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "harness::conformance::native::native_child",
            "--nocapture",
        ])
        .env("TMUX", format!("{},0,0", server.socket))
        .env(
            "PATH",
            format!("{}:{}", root.as_str(), env::var("PATH").unwrap()),
        )
        .env("NILES_CONFORMANCE_ROOT", root.as_str())
        .stdout(File::create(root.join("child.stdout")).unwrap())
        .stderr(File::create(root.join("child.stderr")).unwrap())
        .spawn()
        .unwrap();
    let result = wait_for("conformance child", Duration::from_secs(120), || {
        Ok(child.try_wait()?)
    });
    if result.is_err() {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    let stdout = fs::read_to_string(root.join("child.stdout")).unwrap();
    let stderr = fs::read_to_string(root.join("child.stderr")).unwrap();
    eprintln!("{stderr}");
    assert!(
        result.as_ref().is_ok_and(|status| status.success()),
        "{result:?}\n{stdout}\n{stderr}"
    );
}

// std has no deadline-aware process or file wait. Child::try_wait is nonblocking:
// https://doc.rust-lang.org/1.89.0/std/process/struct.Child.html#method.try_wait
fn wait_for<T>(
    label: &str,
    timeout: Duration,
    mut poll: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .context("deadline overflow")?;
    loop {
        if let Some(value) = poll()? {
            return Ok(value);
        }
        ensure!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn take_receipt(root: &Utf8Path, timeout: Duration) -> Result<String> {
    let path = root.join("receipt");
    let text = wait_for("fake CLI receipt", timeout, || {
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    })?;
    // Consume each publication so repeated identical prompts need their own CLI receipt.
    fs::remove_file(path)?;
    Ok(text)
}

struct Server {
    socket: String,
}
impl Server {
    fn start() -> Self {
        let server = Self {
            socket: format!("/tmp/nc-{}.sock", uuid::Uuid::new_v4()),
        };
        let output = Command::new("tmux")
            .args([
                "-f",
                "/dev/null",
                "-S",
                &server.socket,
                "new-session",
                "-d",
                "-s",
                "conformance",
                "sleep 600",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        server
    }
}
#[expect(
    clippy::let_underscore_must_use,
    reason = "cleanup must also tolerate an unwinding test or an exited server"
)]
impl Drop for Server {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-S", &self.socket, "kill-server"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_file(&self.socket);
    }
}
