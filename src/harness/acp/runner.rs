use super::{Config, Control, prepare_socket, view::View};
use crate::harness::operations::{InputDisposition, PermissionAnswer};
use crate::harness::{Capabilities, SessionId, StopReason, TurnState};
use crate::wait::cursor::open_lock_file;
use crate::{agents, store, util};
use agent_client_protocol as sdk;
use anyhow::{Context, Result, ensure};
use blocking::Unblock;
use camino::{Utf8Path, Utf8PathBuf};
use futures::{AsyncBufReadExt, StreamExt, future, io::BufReader};
use sdk::schema::ProtocolVersion;
use sdk::v1::{ConnectionTo, Responder, schema as acp};
use std::collections::VecDeque;
use std::fs;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream, unix::OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::timeout;

type PromptFuture = Pin<Box<dyn Future<Output = sdk::Result<acp::PromptResponse>> + Send>>;
enum Event {
    Update(acp::SessionNotification),
    Auth(Result<()>),
    Elicitation(
        acp::CreateElicitationRequest,
        Responder<acp::CreateElicitationResponse>,
    ),
    Permission(
        acp::RequestPermissionRequest,
        Responder<acp::RequestPermissionResponse>,
    ),
}
struct SocketPath(Utf8PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        if let Err(err) = fs::remove_file(&self.0) {
            eprintln!("ACP socket cleanup: {err}");
        }
        if let Some(parent) = self.0.parent()
            && let Err(err) = fs::remove_dir(parent)
        {
            eprintln!("ACP socket directory cleanup: {err}");
        }
    }
}
pub(crate) async fn run(dir: &Utf8Path, prompt: String) -> Result<()> {
    let lock = open_lock_file(&dir.join("acp.lock"))?;
    lock.try_lock().context("ACP runner is already active")?;
    let config: Config = store::read_optional_json(&dir.join("acp.json"))?
        .context("missing ACP launch configuration")?;
    let profile = agents::profile_for(&config.family)
        .and_then(|profile| profile.acp)
        .context("no qualified ACP adapter")?;
    println!(
        "Headless ACP · {} · billing: {}",
        profile.binary, profile.subscription
    );
    // SDK 5.3.0 acp_agent.rs exposes env setters, not removals. env -u removes
    // overrides after those setters, without mutating the CLI process environment.
    let removals = profile.remove_env.iter().flat_map(|key| ["-u", *key]);
    let agent = sdk::AcpAgent::new(
        sdk::AcpAgentConfig::new("env")
            .args(removals)
            .arg(profile.binary)
            .envs(config.session.env.clone()),
    );
    let (events, incoming) = mpsc::unbounded_channel();
    let updates = events.clone();
    let auth = events.clone();
    let questions = events.clone();
    let client = sdk::Client
        .v1()
        .on_receive_notification(
            async move |message: acp::SessionNotification, _| {
                updates
                    .send(Event::Update(message))
                    .map_err(sdk::Error::into_internal_error)
            },
            sdk::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: acp::RequestPermissionRequest, responder, _| {
                events
                    .send(Event::Permission(request, responder))
                    .map_err(sdk::Error::into_internal_error)
            },
            sdk::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: acp::CreateElicitationRequest, responder, _| {
                questions
                    .send(Event::Elicitation(request, responder))
                    .map_err(sdk::Error::into_internal_error)
            },
            sdk::on_receive_request!(),
        );
    let run = async move |connection: ConnectionTo<sdk::Agent>| {
        let mut runner = Runner {
            view: View::open(dir, profile)?,
            connection,
            events: incoming,
            authenticated: matches!(profile.auth, agents::AuthSource::Command(_)),
            profile,
            session: config.session,
            prompt: None,
            queue: VecDeque::new(),
        };
        let result = runner.serve(dir, prompt).await;
        runner.view.flush_reply()?;
        let cleanup = runner.view.cancel_requests();
        result.and(cleanup).map_err(Into::into)
    };
    let result = match profile.auth {
        agents::AuthSource::Notification(kind) => {
            client
                .on_receive_notification(
                    async move |status: agents::SubscriptionStatus, _| {
                        auth.send(Event::Auth(status.verify(kind)))
                            .map_err(sdk::Error::into_internal_error)
                    },
                    sdk::on_receive_notification!(),
                )
                .connect_with(agent, run)
                .await
        }
        agents::AuthSource::Command(readiness) => match readiness.check().await {
            Ok(()) => client.connect_with(agent, run).await,
            Err(err) => Err(err.into()),
        },
    };
    if let Err(err) = &result {
        View::open(dir, profile)?.failed(err.to_string())?;
    }
    result.map_err(Into::into)
}
struct Runner {
    view: View,
    connection: ConnectionTo<sdk::Agent>,
    events: mpsc::UnboundedReceiver<Event>,
    authenticated: bool,
    profile: agents::AcpProfile,
    session: agents::SessionConfig,
    prompt: Option<PromptFuture>,
    queue: VecDeque<String>,
}
impl Runner {
    fn event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Elicitation(request, responder) => {
                self.view
                    .elicitation(request, responder, self.prompt.is_some())
            }
            Event::Update(update) => self.view.notification(update),
            Event::Auth(auth) => {
                auth?;
                self.authenticated = true;
                Ok(())
            }
            Event::Permission(request, responder) => {
                self.view
                    .permission(request, responder, self.prompt.is_some())
            }
        }
    }
    fn drain(&mut self) -> Result<()> {
        while let Ok(event) = self.events.try_recv() {
            self.event(event)?;
        }
        Ok(())
    }
    // SDK 5.3.0 concepts/ordering.rs: callbacks run sequentially before routing
    // the response. Drain their queued events before leaving replay/initialization.
    async fn until<T>(&mut self, response: impl Future<Output = sdk::Result<T>>) -> Result<T> {
        tokio::pin!(response);
        loop {
            tokio::select! {
                result = &mut response => { self.drain()?; return Ok(result?); }
                event = self.events.recv() => self.event(event.context("ACP events closed")?)?,
            }
        }
    }
    async fn serve(&mut self, dir: &Utf8Path, prompt: String) -> Result<()> {
        let init = self
            .until(
                self.connection
                    .send_request(
                        acp::InitializeRequest::new(ProtocolVersion::V1)
                            .client_capabilities(
                                acp::ClientCapabilities::new().elicitation(
                                    acp::ElicitationCapabilities::new()
                                        .form(acp::ElicitationFormCapabilities::new()),
                                ),
                            )
                            .client_info(acp::Implementation::new(
                                "niles",
                                env!("CARGO_PKG_VERSION"),
                            )),
                    )
                    .block_task(),
            )
            .await?;
        ensure!(
            init.protocol_version == ProtocolVersion::V1,
            "adapter did not negotiate ACP v1"
        );
        self.profile.verify_initialize(&init)?;
        self.view.state.capabilities = Capabilities {
            cancel: true,
            attention: true,
            resume: init.agent_capabilities.load_session,
        };
        timeout(Duration::from_secs(20), async {
            while !self.authenticated {
                let event = self.events.recv().await.context("ACP events closed")?;
                self.event(event)?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("adapter did not report authentication")??;
        store::write_json(&dir.join("acp-state.json"), &self.view.state)?;
        // Bind before persisting a conversation: a bad socket must not leave an
        // empty session which restart mistakes for an already submitted assignment.
        let path = prepare_socket(dir)?;
        match fs::remove_file(&path) {
            Ok(()) => (),
            Err(err) if err.kind() == io::ErrorKind::NotFound => (),
            Err(err) => return Err(err.into()),
        }
        let listener = UnixListener::bind(&path)?;
        let _socket = SocketPath(path);
        let submitted = self.view.state.assignment_submitted;
        // codex-acp 2.9.0 does not persist an unprompted thread (live retry probe).
        // Only a submitted assignment has a conversation to resume.
        if submitted {
            self.load().await?;
        } else {
            let session = self
                .until(
                    self.connection
                        .send_request(
                            acp::NewSessionRequest::new(
                                util::current_dir_utf8()?.into_std_path_buf(),
                            )
                            .meta(self.session.meta.clone()),
                        )
                        .block_task(),
                )
                .await?;
            self.view.state.session = Some(SessionId::new(session.session_id.to_string())?);
            self.set_mode().await?;
            self.view.set_state(TurnState::Idle(StopReason::EndTurn))?;
        }
        // SDK 5.3.0 stdio.rs uses Unblock too: terminal reads stay off executor
        // threads, without making Tokio shutdown wait on an uncancellable stdin read.
        let mut input = BufReader::new(Unblock::new(io::stdin())).lines();
        let mut reads = JoinSet::new();
        println!(
            "Type a prompt, :cancel, :resume, :approve <request-id> <option-id>, or :answer <request-id> <JSON response>."
        );
        if !submitted && !prompt.is_empty() {
            self.start(prompt)?;
        }
        loop {
            // Completion wins over late permissions; window/socket input stays
            // ahead of a continuously streaming adapter's queued updates.
            tokio::select! {
                biased;
                response = async { match &mut self.prompt { Some(prompt) => prompt.await, None => future::pending().await } } => {
                    self.prompt = None;
                    self.drain()?;
                    self.view.completed(response)?;
                    if let Some(text) = self.queue.pop_front() { self.start(text)?; }
                }
                _ = self.connection.incoming_closed() => anyhow::bail!("ACP adapter closed its output"),
                accepted = listener.accept() => { reads.spawn(read_control(accepted?.0)); }
                Some(command) = reads.join_next() => {
                    let (control, reply) = command?;
                    match control {
                        Ok(control) => self.control(control, Some(reply)).await?,
                        Err(err) => send_reply(Some(reply), Err(err)).await?,
                    }
                }
                line = input.next() => match line {
                    Some(Ok(line)) => match self.window_command(line) {
                        Ok(control) => self.control(control, None).await?,
                        Err(err) => eprintln!("ACP command: {err:#}"),
                    },
                    Some(Err(err)) => return Err(err.into()),
                    None => { ensure!(self.prompt.is_none(), "window closed during an active turn"); return Ok(()); }
                },
                event = self.events.recv() => self.event(event.context("ACP events closed")?)?,
            }
        }
    }
    fn start(&mut self, text: String) -> Result<()> {
        // SDK 5.3.0 jsonrpc.rs enqueues before the dispatch marker is persisted.
        self.prompt = Some(Box::pin(
            self.connection
                .send_request(acp::PromptRequest::new(
                    self.view.protocol_session()?,
                    vec![text.clone().into()],
                ))
                .block_task(),
        ));
        self.view.start(&text)
    }
    async fn load(&mut self) -> Result<()> {
        ensure!(
            self.view.state.capabilities.resume,
            "adapter does not support resume"
        );
        self.view.replaying = true;
        let request = acp::LoadSessionRequest::new(
            self.view.protocol_session()?,
            util::current_dir_utf8()?.into_std_path_buf(),
        )
        .meta(self.session.meta.clone());
        let result = self
            .until(self.connection.send_request(request).block_task())
            .await;
        self.view.replaying = false;
        result?;
        self.set_mode().await?;
        self.view.set_state(TurnState::Idle(StopReason::EndTurn))
    }
    async fn set_mode(&mut self) -> Result<()> {
        if let Some(mode) = &self.session.mode {
            self.until(
                self.connection
                    .send_request(acp::SetSessionModeRequest::new(
                        self.view.protocol_session()?,
                        mode.clone(),
                    ))
                    .block_task(),
            )
            .await?;
        }
        Ok(())
    }
    fn window_command(&self, line: String) -> Result<Control> {
        let line = line.replace("\x1b[200~", "").replace("\x1b[201~", "");
        if let Some(answer) = line.strip_prefix(":answer ") {
            let (id, answer) = answer
                .split_once(' ')
                .context("use :answer <request-id> <JSON response>")?;
            return Ok(Control::Answer {
                id: id.into(),
                answer: serde_json::from_str(answer)?,
            });
        }
        if let Some(answer) = line.strip_prefix(":approve ") {
            let (id, answer) = answer
                .split_once(' ')
                .context("use :approve <request-id> <option-id>")?;
            return Ok(Control::Permission {
                id: id.into(),
                answer: PermissionAnswer::Selected(answer.into()),
            });
        }
        Ok(match line.as_str() {
            ":cancel" => Control::Cancel,
            ":resume" => Control::Resume(
                self.view
                    .state
                    .session
                    .clone()
                    .context("session is not initialized")?,
            ),
            _ => Control::Prompt(line),
        })
    }
    async fn control(&mut self, control: Control, reply: Option<OwnedWriteHalf>) -> Result<()> {
        let result = async {
            match control {
                Control::Prompt(text) => {
                    if self.prompt.is_some() {
                        self.queue.push_back(text);
                        return Ok(InputDisposition::Queued);
                    }
                    self.start(text)?;
                }
                Control::Cancel => {
                    ensure!(self.prompt.is_some(), "no active turn to cancel");
                    self.view.cancelling = true;
                    self.queue.clear();
                    self.connection
                        .send_notification(acp::CancelNotification::new(
                            self.view.protocol_session()?,
                        ))?;
                    self.view.cancel_requests()?;
                    println!("Cancel requested; waiting for cancellation confirmation.");
                }
                Control::Answer { id, answer } => self.view.answer_question(&id, answer)?,
                Control::Permission { id, answer } => self.view.answer(&id, answer)?,
                Control::Resume(session) => {
                    ensure!(
                        self.prompt.is_none(),
                        "cancel and wait for idle before resuming"
                    );
                    ensure!(
                        self.view.state.session.as_ref() == Some(&session),
                        "resume identity does not match this conversation"
                    );
                    self.load().await?;
                }
            }
            Ok(InputDisposition::Unconfirmed)
        }
        .await;
        send_reply(reply, result).await
    }
}
async fn read_control(stream: UnixStream) -> (Result<Control>, OwnedWriteHalf) {
    let (mut input, output) = stream.into_split();
    let mut bytes = Vec::new();
    let result = async {
        timeout(Duration::from_secs(10), input.read_to_end(&mut bytes)).await??;
        Ok(serde_json::from_slice(&bytes)?)
    }
    .await;
    (result, output)
}
async fn send_reply(reply: Option<OwnedWriteHalf>, result: Result<InputDisposition>) -> Result<()> {
    if let Some(mut reply) = reply {
        let result: Result<InputDisposition, String> = result.map_err(|err| format!("{err:#}"));
        if let Err(err) = reply.write_all(&serde_json::to_vec(&result)?).await {
            eprintln!("ACP command reply: {err}");
        }
    } else if let Err(err) = result {
        eprintln!("ACP command: {err:#}");
    }
    Ok(())
}
