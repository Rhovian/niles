use super::*;

pub(crate) const CLAUDE_MAIN: &str = concat!(
    r#"{"type":"assistant","timestamp":"2026-10-01T09:05:00Z","message":{"id":"a","stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":300,"cache_creation_input_tokens":40}}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-10-01T09:05:00Z","message":{"id":"a","stop_reason":"tool_use","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":300,"cache_creation_input_tokens":40}}}"#,
    "\n",
    r#"{"type":"assistant","timestamp":"2026-10-01T11:40:00Z","message":{"id":"b","stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":7,"cache_read_input_tokens":600,"cache_creation_input_tokens":9}}}"#,
    "\n",
);
const CLAUDE_SUBAGENT: &str = concat!(
    r#"{"type":"assistant","timestamp":"2026-10-01T11:41:00Z","message":{"id":"c","usage":{"input_tokens":1000,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#,
    "\n",
);
const CODEX: &str = concat!(
    r#"{"type":"event_msg","timestamp":"2026-10-01T10:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":1},"last_token_usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":1},"model_context_window":1000}}}"#,
    "\n",
    r#"{"type":"event_msg","timestamp":"2026-10-01T10:00:00Z","payload":{"type":"token_count","info":null}}"#,
    "\n",
    r#"{"type":"event_msg","timestamp":"2026-10-01T13:30:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":800,"cache_write_input_tokens":50,"output_tokens":40,"reasoning_output_tokens":2},"last_token_usage":{"input_tokens":900,"cached_input_tokens":800,"cache_write_input_tokens":50,"output_tokens":30,"reasoning_output_tokens":1},"model_context_window":1000}}}"#,
    "\n",
);
pub(crate) const PI: &str = concat!(
    r#"{"type":"message","timestamp":"2026-10-01T08:00:00Z","message":{"role":"assistant","stopReason":"toolUse","usage":{"input":2,"output":3,"cacheRead":4,"cacheWrite":5}}}"#,
    "\n",
    r#"{"type":"message","timestamp":"2026-10-01T16:00:00Z","message":{"role":"assistant","stopReason":"stop","usage":{"input":20,"output":30,"cacheRead":40,"cacheWrite":50}}}"#,
    "\n",
);

pub(crate) fn turn(family: &str, body: &str) -> Result<Turn> {
    let turn = match family {
        "claude" => claude::claude_session(body, &[]).and_then(|record| record.turn),
        "codex" => codex::codex_activity(&parse_lines(body).context("invalid codex fixture")?),
        "pi" => pi::pi_session(body).and_then(|record| record.turn),
        _ => anyhow::bail!("missing conformance fixture for {family}"),
    };
    turn.context("fixture has no turn")
}

pub(crate) fn claude() -> Usage {
    claude::claude_session(CLAUDE_MAIN, &[CLAUDE_SUBAGENT.to_owned()])
        .unwrap()
        .usage
        .unwrap()
}

pub(crate) fn codex() -> Usage {
    let lines = parse_lines(CODEX).unwrap();
    codex::codex_usage(&lines).unwrap().unwrap()
}

pub(crate) fn pi() -> Usage {
    pi::pi_session(PI).unwrap().usage.unwrap()
}
