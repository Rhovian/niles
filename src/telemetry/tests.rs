use super::claude::claude_session;
use super::codex::{CodexLine, codex_matches, codex_usage};
use super::*;

#[test]
fn buckets_sum_to_the_total_for_every_family() {
    let max = u64::MAX.to_string();
    assert!(
        claude_session(
            &fixtures::CLAUDE_MAIN
                .replace("\"input_tokens\":5", &format!("\"input_tokens\":{max}")),
            &[]
        )
        .is_none()
    );
    assert!(
        pi::pi_session(&fixtures::PI.replace("\"input\":20", &format!("\"input\":{max}")))
            .is_none()
    );
    for usage in [fixtures::claude(), fixtures::codex(), fixtures::pi()] {
        assert_eq!(
            usage.buckets.iter().map(|(_, tokens)| tokens).sum::<u64>(),
            usage.total_tokens().unwrap()
        );
        assert!(
            usage
                .buckets
                .iter()
                .all(|(start, _)| start.timestamp() % 900 == 0)
        );
    }
    let claude = fixtures::claude();
    assert_eq!(
        claude
            .buckets
            .iter()
            .map(|(start, _)| start.to_rfc3339())
            .collect::<Vec<_>>(),
        ["2026-10-01T09:00:00+00:00", "2026-10-01T11:30:00+00:00"]
    );
    // The subagent's turn is spent, but the prompt is the main transcript's latest one.
    assert_eq!(claude.prompt_tokens, Some(5 + 600 + 9));
    assert_eq!(claude.context_window, None);

    let codex = fixtures::codex();
    assert_eq!(
        codex
            .buckets
            .iter()
            .map(|(_, tokens)| tokens)
            .collect::<Vec<_>>(),
        [110, 1090 - 110]
    );
    assert_eq!(codex.prompt_tokens, Some(900 + 50));
    assert_eq!(codex.context_window, Some(1000));
    assert_eq!(fixtures::pi().prompt_tokens, Some(20 + 40 + 50));
}

fn state(record: Option<SessionRecord>) -> Option<SessionState> {
    record
        .and_then(|record| record.turn)
        .map(|turn| match turn.state {
            TurnState::Running => SessionState::Working,
            TurnState::Idle(_) => SessionState::Waiting,
            TurnState::RequiresAction => SessionState::NeedsAttention,
        })
}
#[test]
fn claude_deduplicates_usage() {
    let main = "{\"type\":\"assistant\",\"timestamp\":\"2026-10-01T00:00:00Z\",\"message\":{\"id\":\"a\",\"model\":\"opus\",\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"cache_read_input_tokens\":4,\"cache_creation_input_tokens\":5}}}\n";
    let sub = main.replace("\"id\":\"a\"", "\"id\":\"b\"");
    let usage = claude_session(&format!("{main}{main}"), &[sub])
        .unwrap()
        .usage
        .unwrap();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens
        ),
        (4, 6, 8, Some(10))
    );
    assert!(claude_session("bad\n", &[]).is_none());
    assert_eq!(
        claude_session(&format!("{main}partial"), &[])
            .unwrap()
            .usage
            .unwrap()
            .input_tokens,
        2
    );
}

#[test]
fn codex_first_user_and_last_total() {
    let meta =
        r#"{"type":"session_meta","timestamp":"2026-10-01T00:00:00Z","payload":{"cwd":"/w"}}"#;
    let user = r##"{"type":"response_item","timestamp":"2026-10-01T00:00:00Z","payload":{"type":"message","role":"user","content":[{"text":"# Niles worker brief\nreport_file: /w/report.md"}]}}"##;
    let injected = user.replace("# Niles worker brief", "# AGENTS.md instructions");
    let other = user.replace("report.md", "other.md");
    let workspace = Utf8Path::new("/w");
    let needle = Utf8Path::new("/w/report.md");
    let count = |n, write| {
        format!(
            r#"{{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{n},"output_tokens":2,"cached_input_tokens":3,"cache_write_input_tokens":{write},"reasoning_output_tokens":4}}}}}}}}"#
        )
    };
    let lines = parse_lines::<CodexLine>(&format!(
        "{meta}\n{injected}\n{user}\n{}\n{}\n",
        count(4, 0),
        count(9, 0)
    ))
    .unwrap();
    assert!(codex_matches(&lines, workspace, needle));
    assert_eq!(codex_usage(&lines).unwrap().unwrap().input_tokens, 6);
    for body in [
        count(2, 0),
        format!("{}\n{}", count(9, 0), count(4, 0)),
        count(u64::MAX, 0),
        count(u64::MAX, 1),
    ] {
        let parsed = parse_lines::<CodexLine>(&format!("{body}\n")).unwrap();
        assert!(codex_usage(&parsed).is_err(), "{body}");
    }
    let lines =
        parse_lines::<CodexLine>(&format!("{meta}\n{injected}\n{other}\n{user}\n")).unwrap();
    assert!(!codex_matches(&lines, workspace, needle));
}

#[test]
fn claude_main_lines_classify_both_states() {
    let assistant = r#"{"type":"assistant","timestamp":"2026-10-01T00:00:00Z","message":{"id":"a","model":"opus","stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
    let user = r#"{"type":"user","timestamp":"2026-10-01T00:01:00Z","message":{"role":"user","content":"next"}}"#;
    let interrupted = r#"{"type":"user","timestamp":"2026-10-01T00:02:00Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#;
    let error = assistant.replace("end_turn", "stop_sequence");
    let tool_result = interrupted.replace("\"type\":\"text\"", "\"type\":\"tool_result\"");
    for (body, expected) in [
        (format!("{assistant}\n"), SessionState::Waiting),
        (format!("{assistant}\n{user}\n"), SessionState::Working),
        (
            format!("{assistant}\n{interrupted}\n"),
            SessionState::Waiting,
        ),
        (format!("{error}\n"), SessionState::Waiting),
        (
            format!("{assistant}\n{tool_result}\n"),
            SessionState::Working,
        ),
    ] {
        assert_eq!(
            state(claude::claude_session(&body, &[])),
            Some(expected),
            "{body}"
        );
    }
}

#[test]
fn pi_message_lines_classify_both_states() {
    let assistant = r#"{"type":"message","timestamp":"2026-10-01T00:00:00Z","message":{"role":"assistant","stopReason":"stop","usage":{"input":2,"output":3,"cacheRead":4,"cacheWrite":5}}}"#;
    let next = assistant.replace("00:00:00", "00:01:00");
    let body = format!("{assistant}\n{next}\n");
    let waiting = pi::pi_session(&body).unwrap();
    assert!(matches!(
        waiting.turn.unwrap().state,
        TurnState::Idle(StopReason::EndTurn)
    ));
    let waiting = waiting.usage.unwrap();
    assert_eq!(
        (
            waiting.input_tokens,
            waiting.output_tokens,
            waiting.cache_read_tokens,
            waiting.cache_write_tokens
        ),
        (4, 6, 8, Some(10))
    );
    assert_eq!(
        waiting.last_turn_at,
        Some("2026-10-01T00:01:00Z".parse().unwrap())
    );
    assert!(waiting.reasoning_tokens.is_none());
    assert!(waiting.estimated_cost_usd.is_none());
    for role in ["user", "toolResult"] {
        let message = format!(
            r#"{{"type":"message","timestamp":"2026-10-01T00:02:00Z","message":{{"role":"{role}"}}}}"#
        );
        let working = pi::pi_session(&format!("{body}{message}\n")).unwrap();
        assert!(matches!(working.turn.unwrap().state, TurnState::Running));
        assert_eq!(working.usage.unwrap().last_turn_at, waiting.last_turn_at);
        assert!(
            pi::pi_session(&format!("{message}\n"))
                .unwrap()
                .usage
                .is_none()
        );
    }
    let custom =
        r#"{"type":"message","timestamp":"2026-10-01T00:02:00Z","message":{"role":"custom"}}"#;
    assert_eq!(
        state(pi::pi_session(&format!("{body}{custom}\n"))),
        Some(SessionState::Waiting)
    );
    assert_eq!(
        state(pi::pi_session(&body.replace("\"stop\"", "\"toolUse\""))),
        Some(SessionState::Working)
    );
    assert_eq!(
        state(pi::pi_session(&format!(
            "{body}{{\"type\":\"model_change\"}}\npartial"
        ))),
        Some(SessionState::Waiting)
    );
    assert!(pi::pi_session("bad\n").is_none());
}

#[test]
fn codex_events_classify_both_states() {
    let count = r#"{"type":"event_msg","timestamp":"2026-10-01T00:00:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1,"output_tokens":1,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}"#;
    let started = r#"{"type":"event_msg","timestamp":"2026-10-01T00:01:00Z","payload":{"type":"task_started"}}"#;
    let complete = r#"{"type":"event_msg","timestamp":"2026-10-01T00:02:00Z","payload":{"type":"task_complete"}}"#;
    let codex_state = |lines: &[CodexLine]| {
        state(Some(SessionRecord {
            turn: codex::codex_activity(lines),
            usage: None,
        }))
    };
    let lines = parse_lines::<CodexLine>(&format!("{count}\n{started}\n")).unwrap();
    assert_eq!(codex_state(&lines), Some(SessionState::Working));
    let lines = parse_lines::<CodexLine>(&format!("{count}\n{started}\n{complete}\n")).unwrap();
    assert_eq!(
        codex::codex_activity(&lines)
            .unwrap()
            .started_at
            .unwrap()
            .to_rfc3339(),
        "2026-10-01T00:01:00+00:00"
    );
    assert_eq!(codex_state(&lines), Some(SessionState::Waiting));
}
