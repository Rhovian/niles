use crate::commands::{Approve, Close, Message, Output, Peek, SendArgs, SpawnArgs, Target, Wait};
use crate::wait::Outcome;
use crate::worker;
use crate::worker::agent_status::{ORCHESTRATION_CONTRACT, Status};
use crate::workspace_manifest::setup::SetupSelection;
use anyhow::Result;
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::stdio_pipes,
};
use schemars::JsonSchema;
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, Error as _},
};
use serde_json::{Map, Value};

pub(crate) const NAME: &str = "niles";
pub(crate) const COMMAND: &str = "mcp";

pub(crate) async fn run(setup: Option<String>) -> Result<()> {
    Server {
        setup,
        tool_router: Server::tool_router(),
    }
    .serve(stdio_pipes())
    .await?
    .waiting()
    .await?;
    Ok(())
}

struct Server {
    setup: Option<String>,
    tool_router: ToolRouter<Self>,
}

type DispatchHandler<T, R> = fn(T, Message, Option<&SetupSelection>, &mut Output<'_>) -> Result<R>;

#[derive(Deserialize, JsonSchema)]
#[serde(bound(deserialize = "T: DeserializeOwned"))]
struct Dispatch<T> {
    #[serde(flatten, deserialize_with = "dispatch_args")]
    args: T,
    /// Assignment text.
    message: Message,
    /// Select a named setup. Defaults to the launch environment's NILES_SETUP.
    setup: Option<String>,
}

// Serde 1.0.229's flattened deserializer can't enforce unknown fields across
// nested structs. Decode the remaining top-level keys as a fresh command object.
// https://serde.rs/attr-flatten.html
fn dispatch_args<'de, T: DeserializeOwned, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
    let fields = Map::deserialize(d)?;
    serde_json::from_value(Value::Object(fields)).map_err(D::Error::custom)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(NAME, env!("CARGO_PKG_VERSION")))
    }
}

#[tool_router]
impl Server {
    #[tool(description = "Spawn a Niles worker with an installed primary skill and assignment.")]
    async fn spawn(&self, Parameters(input): Parameters<Dispatch<SpawnArgs>>) -> CallToolResult {
        self.dispatch(input, SpawnArgs::run).await
    }

    #[tool(
        description = "Send a follow-up assignment to a worker. Advances its wake cursor and returns any uncollected wakes it passed."
    )]
    async fn send(&self, Parameters(input): Parameters<Dispatch<SendArgs>>) -> CallToolResult {
        self.dispatch(input, SendArgs::run).await
    }

    #[tool(
        description = "Wait for a batch of worker wakes. Pass the returned ack on your next wait to acknowledge the batch. An omitted or expired ack leaves wakes eligible for replay. A newer wait supersedes the current one."
    )]
    async fn wait(
        &self,
        Parameters(args): Parameters<Wait>,
        context: RequestContext<RoleServer>,
    ) -> Result<Json<Outcome>, String> {
        blocking::unblock(move || {
            args.run(|| context.ct.is_cancelled())
                .map(Json)
                .map_err(|error| format!("{error:#}"))
        })
        .await
    }

    #[tool(description = "List live Niles workers and their current state.")]
    async fn workers(&self) -> CallToolResult {
        call(|output| Ok(Some(serde_json::to_value(worker::workers(output)?)?))).await
    }

    #[tool(description = "Read a live or archived worker's durable report.")]
    async fn report(&self, Parameters(args): Parameters<Target>) -> CallToolResult {
        text(move |output| worker::report(args.id, output)).await
    }

    #[tool(description = "Read the latest output from a worker's pane or headless transcript.")]
    async fn peek(&self, Parameters(args): Parameters<Peek>) -> CallToolResult {
        text(move |output| args.run(output)).await
    }

    #[tool(description = "Request cancellation of a headless worker's current turn.")]
    async fn cancel(&self, Parameters(args): Parameters<Target>) -> CallToolResult {
        text(move |output| worker::pane::cancel(args.id, output)).await
    }

    #[tool(description = "Answer a headless worker's pending permission request.")]
    async fn approve(&self, Parameters(args): Parameters<Approve>) -> CallToolResult {
        text(move |_| args.run()).await
    }

    #[tool(
        description = "Close and archive workers. Keep workers that reported done available for follow-up until the operator directs you to close them."
    )]
    async fn close(&self, Parameters(args): Parameters<Close>) -> CallToolResult {
        text(move |output| args.run(output)).await
    }

    #[tool(description = ORCHESTRATION_CONTRACT)]
    async fn agent_status(&self, Parameters(status): Parameters<Status>) -> CallToolResult {
        text(move |output| status.run(output)).await
    }

    async fn dispatch<T: Send + 'static, R: 'static>(
        &self,
        input: Dispatch<T>,
        dispatch: DispatchHandler<T, R>,
    ) -> CallToolResult {
        let setup = input.setup.or_else(|| self.setup.clone());
        text(move |output| {
            let selection = setup.as_deref().map(str::parse).transpose()?;
            dispatch(input.args, input.message, selection.as_ref(), output).map(drop)
        })
        .await
    }
}

async fn text(run: impl FnOnce(&mut Output<'_>) -> Result<()> + Send + 'static) -> CallToolResult {
    call(move |output| run(output).map(|()| None)).await
}

async fn call(
    run: impl FnOnce(&mut Output<'_>) -> Result<Option<Value>> + Send + 'static,
) -> CallToolResult {
    blocking::unblock(move || command_result(run)).await
}

fn command_result(run: impl FnOnce(&mut Output<'_>) -> Result<Option<Value>>) -> CallToolResult {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = run(&mut Output {
        out: &mut out,
        err: &mut err,
    });
    let content = [out, err]
        .into_iter()
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| ContentBlock::text(String::from_utf8_lossy(&bytes)))
        .collect::<Vec<_>>();
    match result {
        Ok(Some(data)) => {
            let mut result = CallToolResult::structured(data);
            result.content.extend(content);
            result
        }
        Ok(None) => CallToolResult::success(content),
        Err(error) => {
            let mut result = CallToolResult::error(content);
            result
                .content
                .push(ContentBlock::text(format!("{error:#}")));
            result
        }
    }
}
