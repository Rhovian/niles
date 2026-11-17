use super::launch::Billing;
use super::operations::{InputDisposition, PermissionAnswer};
use super::{Adapter, Capabilities, Prompt, SessionId, Turn};
use crate::telemetry::transcript::{TextBlock, TextTurn};
use crate::{agents::SessionConfig, store, util, wake};
use agent_client_protocol::v1::schema::CreateElicitationResponse;
use anyhow::{Context, Result, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::ErrorKind;
use std::net::Shutdown;
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt},
    net::UnixStream,
};
use std::time::Duration;

mod runner;
mod view;
pub(crate) use runner::run;

#[derive(Serialize, Deserialize)]
struct Config {
    family: String,
    billing: Billing,
    #[serde(flatten)]
    session: SessionConfig,
}
#[derive(Serialize, Deserialize)]
struct State {
    session: Option<SessionId>,
    assignment_submitted: bool,
    capabilities: Capabilities,
    turn: Turn,
}
pub(crate) fn prepare(
    dir: &Utf8Path,
    family: &str,
    billing: Billing,
    mut session: SessionConfig,
) -> Result<()> {
    session.prepare(dir)?;
    store::write_json(
        &dir.join("acp.json"),
        &Config {
            family: family.into(),
            billing,
            session,
        },
    )
}
fn state(dir: &Utf8Path) -> Result<State> {
    store::read_optional_json(&dir.join("acp-state.json"))?
        .context("ACP runner has not initialized")
}
pub(crate) fn capabilities(dir: &Utf8Path) -> Result<Capabilities> {
    Ok(state(dir)?.capabilities)
}
pub(crate) fn turn(dir: &Utf8Path) -> Result<Option<Turn>> {
    let state: Option<State> = store::read_optional_json(&dir.join("acp-state.json"))?;
    Ok(state.map(|state| state.turn))
}
// Hash state paths because sockaddr_un is limited to 104 bytes on macOS.
fn socket_dir(dir: &Utf8Path) -> Utf8PathBuf {
    let hash = format!("{:x}", Sha256::digest(dir.as_str().as_bytes()));
    Utf8PathBuf::from(format!("/tmp/niles-acp-{hash}"))
}
fn prepare_socket(dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let root = socket_dir(dir);
    match fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => (),
        Err(err) if err.kind() == ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err.into()),
    }
    checked_socket(root)
}
fn checked_socket(root: Utf8PathBuf) -> Result<Utf8PathBuf> {
    let meta = fs::symlink_metadata(&root)?;
    // SAFETY: geteuid has no memory or argument preconditions.
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "unsafe ACP socket directory {root}"
    );
    Ok(root.join("rpc"))
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "lowercase")]
enum Control {
    Prompt(String),
    Cancel,
    Answer {
        id: String,
        answer: CreateElicitationResponse,
    },
    Resume(SessionId),
    Permission {
        id: String,
        answer: PermissionAnswer,
    },
}
pub(crate) struct Headless<'a> {
    dir: &'a Utf8Path,
    connection: UnixStream,
}
impl Headless<'_> {
    pub(crate) fn connect(dir: &Utf8Path) -> Result<Headless<'_>> {
        let path = checked_socket(socket_dir(dir))
            .context("headless runner is not reachable; use niles resume")?;
        Ok(Headless {
            dir,
            connection: UnixStream::connect(path)
                .context("headless runner is not reachable; use niles resume")?,
        })
    }
    fn request(&mut self, command: Control) -> Result<InputDisposition> {
        self.connection
            .set_write_timeout(Some(Duration::from_secs(10)))?;
        self.connection
            .set_read_timeout(Some(Duration::from_secs(10)))?;
        serde_json::to_writer(&mut self.connection, &command)?;
        self.connection.shutdown(Shutdown::Write)?;
        let reply: Result<InputDisposition, String> = serde_json::from_reader(&mut self.connection)
            .context("headless control reply not confirmed (10s deadline); operation may still be pending")?;
        reply.map_err(anyhow::Error::msg)
    }
}
impl Adapter for Headless<'_> {
    fn negotiate(&mut self) -> Result<Capabilities> {
        Ok(state(self.dir)?.capabilities)
    }
    fn prompt(&mut self, prompt: Prompt) -> Result<InputDisposition> {
        self.request(Control::Prompt(prompt.0))
    }

    fn cancel(&mut self) -> Result<()> {
        self.request(Control::Cancel).map(drop)
    }
    fn resume(&mut self, session: &SessionId) -> Result<()> {
        self.request(Control::Resume(session.clone())).map(drop)
    }
    fn answer_permission(&mut self, request: &str, answer: PermissionAnswer) -> Result<()> {
        self.request(Control::Permission {
            id: request.into(),
            answer,
        })
        .map(drop)
    }
}
pub(crate) fn session(dir: &Utf8Path) -> Result<SessionId> {
    state(dir)?.session.context("session is not initialized")
}
pub(crate) fn peek(dir: &Utf8Path, count: usize) -> Result<()> {
    let body = util::read_optional_string(&dir.join("transcript.jsonl"))?
        .context("ACP transcript is not available yet")?;
    let text = wake::complete_lines(body.as_bytes())
        .map(|(_, line)| {
            let turn: TextTurn = serde_json::from_str(&line)?;
            Ok(match turn.content {
                TextBlock::Text { text } => text,
                TextBlock::ToolUse { name, input } => format!("[tool] {name}: {input}"),
            })
        })
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    let lines = text.lines();
    let start = if count == 0 {
        0
    } else {
        lines.clone().count().saturating_sub(count)
    };
    for line in lines.skip(start) {
        println!("{line}");
    }
    Ok(())
}
