use super::State;
use crate::harness::operations::PermissionAnswer;
use crate::harness::{Capabilities, SessionFailure, StopReason, Turn, TurnState};
use crate::telemetry::transcript::{TextBlock, TextRole, TextTurn};
use crate::{agents::AcpProfile, store, util, wake};
use agent_client_protocol as sdk;
use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};
use chrono::Utc;
use sdk::v1::{Responder, schema as acp};
use std::collections::{BTreeMap, btree_map::Entry};
use std::io::{self, Write};
use std::mem;

type PermissionResponder = Responder<acp::RequestPermissionResponse>;
pub(super) struct View {
    dir: Utf8PathBuf,
    profile: AcpProfile,
    pub(super) state: State,
    pub(super) replaying: bool,
    pub(super) cancelling: bool,
    permissions: BTreeMap<String, (Vec<acp::PermissionOption>, PermissionResponder)>,
    questions: BTreeMap<String, Responder<acp::CreateElicitationResponse>>,
    // Schema 1.18.0 v1/client.rs ContentChunk is a delta. The shared transcript
    // needs a complete message block, while the window streams each delta.
    reply: String,
}
impl View {
    pub(super) fn open(dir: &Utf8Path, profile: AcpProfile) -> Result<Self> {
        let state = match store::read_optional_json(&dir.join("acp-state.json"))? {
            Some(state) => state,
            None => State {
                session: None,
                assignment_submitted: false,
                capabilities: Capabilities::default(),
                turn: Turn {
                    state: TurnState::Running,
                    started_at: None,
                },
            },
        };
        Ok(Self {
            dir: dir.into(),
            profile,
            state,
            replaying: false,
            cancelling: false,
            permissions: BTreeMap::new(),
            questions: BTreeMap::new(),
            reply: String::new(),
        })
    }
    pub(super) fn protocol_session(&self) -> Result<acp::SessionId> {
        Ok(self
            .state
            .session
            .as_ref()
            .context("session is not initialized")?
            .0
            .clone()
            .into())
    }
    pub(super) fn start(&mut self, text: &str) -> Result<()> {
        self.render(format!("\n[user] {text}\n"))?;
        self.record(TextRole::User, TextBlock::Text { text: clean(text) })?;
        self.cancelling = false;
        self.state.assignment_submitted = true;
        self.set_state(TurnState::Running)
    }
    pub(super) fn cancel_requests(&mut self) -> Result<()> {
        for (_, responder) in mem::take(&mut self.questions) {
            responder.respond(acp::CreateElicitationResponse::new(
                acp::ElicitationAction::Cancel,
            ))?;
        }
        for (_, (_, responder)) in mem::take(&mut self.permissions) {
            responder.respond(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            ))?;
        }
        Ok(())
    }
    pub(super) fn answer(&mut self, id: &str, answer: PermissionAnswer) -> Result<()> {
        let Entry::Occupied(entry) = self.permissions.entry(id.into()) else {
            bail!("permission is no longer pending");
        };
        if let PermissionAnswer::Selected(option) = &answer {
            ensure!(
                entry
                    .get()
                    .0
                    .iter()
                    .any(|o| o.option_id.to_string() == *option),
                "unknown permission option"
            );
        }
        let outcome = match answer {
            PermissionAnswer::Cancelled => acp::RequestPermissionOutcome::Cancelled,
            PermissionAnswer::Selected(id) => {
                acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(id))
            }
        };
        entry
            .remove()
            .1
            .respond(acp::RequestPermissionResponse::new(outcome))?;
        if self.permissions.is_empty() && self.questions.is_empty() {
            self.set_state(TurnState::Running)?;
        }
        Ok(())
    }
    pub(super) fn permission(
        &mut self,
        request: acp::RequestPermissionRequest,
        responder: PermissionResponder,
        active: bool,
    ) -> Result<()> {
        ensure!(
            self.state
                .session
                .as_ref()
                .is_some_and(|session| session.0 == request.session_id.to_string()),
            "permission belongs to another session"
        );
        if self.cancelling || !active {
            responder.respond(acp::RequestPermissionResponse::new(
                acp::RequestPermissionOutcome::Cancelled,
            ))?;
        } else {
            let id = serde_json::to_string(responder.id())?;
            const UNTITLED_REQUEST: &str = "Operator input required";
            #[expect(
                clippy::disallowed_methods,
                reason = "the window owns the label for an untitled ACP request"
            )]
            let title = request
                .tool_call
                .fields
                .title
                .as_deref()
                .unwrap_or(UNTITLED_REQUEST);
            let mut display = format!("\n[permission {id}] {title}\n");
            for option in &request.options {
                display.push_str(&format!("  {}: {}\n", option.option_id, option.name));
            }
            display.push_str(&format!("Use :approve {id} <option-id>\n"));
            self.permissions.insert(id, (request.options, responder));
            self.attention(display)?;
        }
        Ok(())
    }
    // claude-agent-acp 0.107.3 elicitation.ts maps AskUserQuestion to SDK forms.
    pub(super) fn elicitation(
        &mut self,
        request: acp::CreateElicitationRequest,
        responder: Responder<acp::CreateElicitationResponse>,
        active: bool,
    ) -> Result<()> {
        let acp::ElicitationMode::Form(form) = request.mode else {
            bail!("elicitation mode is not supported");
        };
        let acp::ElicitationScope::Session(scope) = form.scope else {
            bail!("request-scoped form elicitation is not supported");
        };
        ensure!(
            self.state
                .session
                .as_ref()
                .is_some_and(|id| id.0 == scope.session_id.to_string()),
            "question belongs to another session"
        );
        if self.cancelling || !active {
            responder.respond(acp::CreateElicitationResponse::new(
                acp::ElicitationAction::Cancel,
            ))?;
        } else {
            let id = serde_json::to_string(responder.id())?;
            let display = format!(
                "\n[question {id}] {}\n{}\nUse :answer {id} {{\"action\":\"accept\",\"content\":{{<field>:<value>}}}} or {{\"action\":\"cancel\"}}\n",
                request.message,
                serde_json::to_string_pretty(&form.requested_schema)?
            );
            self.questions.insert(id, responder);
            self.attention(display)?;
        }
        Ok(())
    }
    pub(super) fn answer_question(
        &mut self,
        id: &str,
        answer: acp::CreateElicitationResponse,
    ) -> Result<()> {
        self.questions
            .remove(id)
            .context("question is no longer pending")?
            .respond(answer)?;
        if self.questions.is_empty() && self.permissions.is_empty() {
            self.set_state(TurnState::Running)?;
        }
        Ok(())
    }
    fn attention(&mut self, display: String) -> Result<()> {
        self.flush_reply()?;
        self.render(display)?;
        self.set_state(TurnState::RequiresAction)?;
        if self.dir.join("status.log").exists() {
            util::append_line(
                &wake::status_log_path(&self.dir),
                &wake::line(
                    wake::WakeKind::Attention,
                    "headless request requires operator input",
                ),
            )?;
        }
        Ok(())
    }
    pub(super) fn failed(&mut self, message: String) -> Result<()> {
        self.show_state(TurnState::Idle(StopReason::Error(SessionFailure {
            at: Utc::now(),
            message,
        })))
    }
    pub(super) fn completed(&mut self, response: sdk::Result<acp::PromptResponse>) -> Result<()> {
        self.cancel_requests()?;
        match response {
            Ok(response) => self.show_state(response.stop_reason.into()),
            Err(err) => self.failed(err.to_string()),
        }
    }
    pub(super) fn notification(&mut self, notification: acp::SessionNotification) -> Result<()> {
        ensure!(
            self.state
                .session
                .as_ref()
                .is_none_or(|session| session.0 == notification.session_id.to_string()),
            "update belongs to another session"
        );
        match notification.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                if let acp::ContentBlock::Text(text) = chunk.content {
                    if !self.replaying {
                        self.reply.push_str(&text.text);
                    }
                    self.render(text.text)?;
                }
            }
            acp::SessionUpdate::AgentThoughtChunk(chunk)
            | acp::SessionUpdate::UserMessageChunk(chunk) => {
                if let acp::ContentBlock::Text(text) = chunk.content {
                    self.render(text.text)?;
                }
            }
            acp::SessionUpdate::ToolCall(call) => {
                self.flush_reply()?;
                self.record(
                    TextRole::Assistant,
                    TextBlock::ToolUse {
                        name: clean(&call.title),
                        input: call.raw_input.clone().into(),
                    },
                )?;
                self.tool(call.into())?;
            }
            acp::SessionUpdate::ToolCallUpdate(update) => self.tool(update)?,
            acp::SessionUpdate::Plan(plan) => self.plan(plan.entries)?,
            acp::SessionUpdate::AvailableCommandsUpdate(_)
            | acp::SessionUpdate::CurrentModeUpdate(_)
            | acp::SessionUpdate::ConfigOptionUpdate(_)
            | acp::SessionUpdate::SessionInfoUpdate(_)
            | acp::SessionUpdate::UsageUpdate(_)
            | acp::SessionUpdate::Notice(_)
            | acp::SessionUpdate::CompactionUpdate(_)
            | acp::SessionUpdate::CompactionSummaryChunk(_) => {}
            _ => return Ok(()),
        }
        Ok(())
    }
    fn tool(&mut self, update: acp::ToolCallUpdate) -> Result<()> {
        let output = self.profile.terminal_output(&update).map(str::to_owned);
        let title = match update.fields.title {
            Some(title) => title,
            None => update.tool_call_id.to_string(),
        };
        let title = match update.fields.status {
            Some(status) => format!("{status:?}: {title}"),
            None => title,
        };
        self.render(format!("\n[tool {}] {title}\n", update.tool_call_id))?;
        for content in update.fields.content.into_iter().flatten() {
            if let acp::ToolCallContent::Content(content) = content
                && let acp::ContentBlock::Text(text) = content.content
            {
                self.render(text.text)?;
            }
        }
        if let Some(text) = output {
            self.render(text)?;
        }
        Ok(())
    }
    pub(super) fn set_state(&mut self, state: TurnState) -> Result<()> {
        if matches!(state, TurnState::Idle(_)) {
            self.flush_reply()?;
        }
        self.state.turn.state = state;
        store::write_json(&self.dir.join("acp-state.json"), &self.state)
    }
    fn render(&self, text: String) -> Result<()> {
        print!("{}", clean(&text));
        io::stdout().flush()?;
        Ok(())
    }
    fn record(&self, role: TextRole, content: TextBlock) -> Result<()> {
        if !self.replaying {
            util::append_line(
                &self.dir.join("transcript.jsonl"),
                &serde_json::to_string(&TextTurn {
                    role,
                    timestamp: Some(Utc::now()),
                    content,
                })?,
            )?;
        }
        Ok(())
    }
    pub(super) fn flush_reply(&mut self) -> Result<()> {
        if !self.reply.is_empty() {
            let text = mem::take(&mut self.reply);
            self.record(TextRole::Assistant, TextBlock::Text { text: clean(&text) })?;
        }
        Ok(())
    }
    fn plan(&self, entries: Vec<acp::PlanEntry>) -> Result<()> {
        let mut text = "\n[plan]\n".to_owned();
        for entry in entries {
            text.push_str(&format!("  [{:?}] {}\n", entry.status, entry.content));
        }
        self.render(text)
    }
    fn show_state(&mut self, state: TurnState) -> Result<()> {
        self.render(format!("\n[{state:?}]\n"))?;
        self.set_state(state)
    }
}

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
