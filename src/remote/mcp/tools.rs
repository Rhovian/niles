use super::oidc::Grants;
use crate::config::user::McpAccess::{self, Operate, Read};
use crate::projects::lifecycle::{self, Closed, Start, Stop};
use crate::projects::{navigation, status};
use crate::remote::chat::{Chat, find_worker};
use crate::remote::message::Delivery;
use crate::remote::{Session, blocking, feed, message};
use crate::tmux;
use anyhow::Result;
use axum::http::request::Parts;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

type Context = RequestContext<RoleServer>;
type ToolResult = Result<CallToolResult, ErrorData>;

#[derive(Clone)]
pub(super) struct LeadTools {
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Project {
    /// Registered project name.
    project: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Worker {
    /// Registered project name.
    project: String,
    /// Worker ID from list_projects.
    worker_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct LeadMessage {
    /// Registered project name.
    project: String,
    /// Text to send to the lead.
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct WorkerMessage {
    /// Registered project name.
    project: String,
    /// Worker ID from list_projects.
    worker_id: String,
    /// Text to send to the worker.
    text: String,
}

#[tool_handler(router = self.tool_router, name = "Niles")]
impl ServerHandler for LeadTools {}

impl LeadTools {
    pub(super) fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl LeadTools {
    #[tool(
        description = "List authorized registered projects and their workers",
        annotations(read_only_hint = true)
    )]
    async fn list_projects(&self, context: Context) -> ToolResult {
        let access = grants(&context)?;
        Ok(success(json!({"projects":
            blocking(move || navigation::collect_for(|name| access.allows(name, Read)))
                .await.map_err(internal)?
        })))
    }

    #[tool(
        description = "List authorized active lead sessions",
        annotations(read_only_hint = true)
    )]
    async fn list_sessions(&self, context: Context) -> ToolResult {
        let access = grants(&context)?;
        let sessions = blocking(move || {
            Ok(navigation::sessions_for(|name| access.allows(name, Read))?
                .iter()
                .map(Session::from_record)
                .collect::<Vec<_>>())
        })
        .await
        .map_err(internal)?;
        Ok(success(json!({"sessions": sessions})))
    }

    #[tool(description = "Start the project's saved lead session")]
    async fn start_lead(
        &self,
        context: Context,
        Parameters(input): Parameters<Project>,
    ) -> ToolResult {
        for_project(&context, input.project, Operate, |project| {
            Ok(match lifecycle::start(project)? {
                Start::Running => success(json!({"status": "running"})),
                Start::Unregistered => failure("Unregistered project"),
                Start::Conflict(reason) => failure(reason),
            })
        })
        .await
    }

    #[tool(description = "Stop the project's lead session and close its workers")]
    async fn stop_lead(
        &self,
        context: Context,
        Parameters(input): Parameters<Project>,
    ) -> ToolResult {
        for_project(
            &context,
            input.project,
            Operate,
            |project| match lifecycle::stop(project, &lifecycle::niles_in)? {
                Stop::Stopped => Ok(success(json!({"status": "stopped"}))),
                Stop::NotRunning => missing(project, "No running lead session"),
            },
        )
        .await
    }

    #[tool(
        description = "Read the project's lead status and recent transcript turns. Call again to refresh.",
        annotations(read_only_hint = true)
    )]
    async fn read_lead_events(
        &self,
        context: Context,
        Parameters(input): Parameters<Project>,
    ) -> ToolResult {
        for_project(&context, input.project, Read, |project| {
            with_lead_chat(project, snapshot)
        })
        .await
    }

    #[tool(
        description = "Read the worker's recent transcript turns. Call again to refresh.",
        annotations(read_only_hint = true)
    )]
    async fn read_worker_events(
        &self,
        context: Context,
        Parameters(input): Parameters<Worker>,
    ) -> ToolResult {
        for_project(&context, input.project, Read, move |project| {
            with_worker_chat(project, &input.worker_id, snapshot)
        })
        .await
    }

    #[tool(description = "Send a message to the project's foreground lead session")]
    async fn message_lead(
        &self,
        context: Context,
        Parameters(input): Parameters<LeadMessage>,
    ) -> ToolResult {
        for_project(&context, input.project, Operate, move |project| {
            with_lead_chat(project, |chat| send(chat, input.text))
        })
        .await
    }

    #[tool(description = "Send a message to the specified worker")]
    async fn message_worker(
        &self,
        context: Context,
        Parameters(input): Parameters<WorkerMessage>,
    ) -> ToolResult {
        for_project(&context, input.project, Operate, move |project| {
            with_worker_chat(project, &input.worker_id, |chat| send(chat, input.text))
        })
        .await
    }

    #[tool(description = "Close the specified worker")]
    async fn close_worker(
        &self,
        context: Context,
        Parameters(input): Parameters<Worker>,
    ) -> ToolResult {
        for_project(
            &context,
            input.project,
            Operate,
            move |project| match lifecycle::close_worker(
                project,
                &input.worker_id,
                &lifecycle::niles_in,
            )? {
                Closed::Closed => Ok(success(json!({"status": "closed"}))),
                Closed::NotFound => missing(project, "Unknown worker"),
            },
        )
        .await
    }
}

fn grants(context: &Context) -> Result<Arc<Grants>, ErrorData> {
    context
        .extensions
        .get::<Parts>()
        .and_then(|parts| parts.extensions.get::<Arc<Grants>>())
        .cloned()
        .ok_or_else(|| ErrorData::internal_error("Missing request authorization", None))
}

async fn for_project(
    context: &Context,
    project: String,
    access: McpAccess,
    operation: impl FnOnce(&str) -> Result<CallToolResult> + Send + 'static,
) -> ToolResult {
    if !grants(context)?.allows(&project, access) {
        let level = match access {
            Read => "A read",
            Operate => "An operate",
        };
        return Ok(failure(format!(
            "{level} grant is required for project {project}"
        )));
    }
    blocking(move || operation(&project))
        .await
        .map_err(internal)
}

fn with_lead_chat(
    project: &str,
    operation: impl FnOnce(Chat) -> Result<CallToolResult>,
) -> Result<CallToolResult> {
    let chat = navigation::sessions_for(|name| name == project)?
        .first()
        .map(Chat::lead)
        .transpose()?;
    with_chat(project, chat, "No running lead session", operation)
}

fn with_worker_chat(
    project: &str,
    id: &str,
    operation: impl FnOnce(Chat) -> Result<CallToolResult>,
) -> Result<CallToolResult> {
    with_chat(
        project,
        find_worker(project, id)?,
        "Unknown worker",
        operation,
    )
}

fn with_chat(
    project: &str,
    chat: Option<Chat>,
    absent: &str,
    operation: impl FnOnce(Chat) -> Result<CallToolResult>,
) -> Result<CallToolResult> {
    match chat {
        Some(chat) => operation(chat),
        None => missing(project, absent),
    }
}

fn snapshot(chat: Chat) -> Result<CallToolResult> {
    const MAX_TEXT_BYTES: usize = 24 * 1024;
    let status = chat.status.clone();
    let (_, mut snapshot) = feed::Feed::open(chat)?;
    snapshot.bound_text(MAX_TEXT_BYTES);
    Ok(success(json!({"status": status, "snapshot": snapshot})))
}

fn send(chat: Chat, text: String) -> Result<CallToolResult> {
    let message = match message::Message::from_text(text) {
        Ok(message) => message,
        Err(error) => return Ok(failure(error.to_string())),
    };
    Ok(
        match message::deliver(chat, message, None, tmux::send_line)? {
            Delivery::Sent => success(json!({"status": "sent"})),
            Delivery::PaneUnavailable => failure("Agent pane is unavailable"),
        },
    )
}

fn missing(project: &str, if_registered: &str) -> Result<CallToolResult> {
    Ok(failure(if status::project(project)?.is_some() {
        if_registered
    } else {
        "Unregistered project"
    }))
}

fn success(data: Value) -> CallToolResult {
    CallToolResult::structured(data)
}

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

fn internal(error: anyhow::Error) -> ErrorData {
    eprintln!("remote MCP tool: {error:#}");
    ErrorData::internal_error("Niles operation failed", None)
}
