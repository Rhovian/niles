use super::super::{claude, codex, pi};
use super::*;
use crate::store::atomic_write;
use crate::test_support::{self, array, only};
use serde_json::json;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::slice;
use zstd::stream;

fn lines(values: &[Value]) -> String {
    values.iter().map(|value| format!("{value}\n")).collect()
}

fn text(family: Family, values: &[Value]) -> Vec<TextTurn> {
    values
        .iter()
        .flat_map(|value| file_turns(family, value.clone()).unwrap())
        .collect()
}

fn assistant(content: Value) -> Value {
    json!({"type":"assistant", "timestamp":"2026-10-20T00:00:01Z", "message":{
        "id":"a", "content":content,"stop_reason":"end_turn",
        "usage":{"input_tokens":1,"output_tokens":1,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}
    }})
}

#[test]
fn all_families_emit_one_visible_block_per_item() {
    let blocks = json!([{"type":"thinking","thinking":"private"},
        {"type":"text","text":"Testing"},
        {"type":"tool_use","name":"Bash","input":{"command":"cargo test"}},
        {"type":"image","data":"secret"}, {"type":"tool_result","content":"huge"}]);
    let claude = text(Family::Claude, &[assistant(blocks)]);
    let pi = text(
        Family::Pi,
        &[json!({"type":"message","timestamp":"2026-10-20T00:00:01Z",
        "message":{"role":"assistant","content":[{"type":"text","text":"Testing"},
            {"type":"toolCall","name":"Bash","arguments":{"command":"cargo test"}}]}})],
    );
    let codex = text(
        Family::Codex,
        &[
            json!({"type":"response_item","timestamp":"2026-10-20T00:00:01Z","payload":{
            "type":"message","role":"assistant","content":[{"type":"output_text","text":"Testing"}]}}),
            json!({"type":"response_item","timestamp":"2026-10-20T00:00:01Z","payload":{
            "type":"function_call","name":"Bash","arguments":"{\"command\":\"cargo test\"}"}}),
        ],
    );
    let [_, tool] = array(&claude);
    assert_eq!(claude, pi);
    assert_eq!(claude, codex);
    assert_eq!(
        serde_json::to_value(tool).unwrap(),
        json!({"role":"assistant",
        "timestamp":"2026-10-20T00:00:01Z","content":{"type":"tool_use","name":"Bash","input":{"command":"cargo test"}}})
    );
}

#[test]
fn codex_omits_developer_system_reasoning_and_event_mirrors() {
    let mut values = Vec::new();
    for role in ["developer", "system", "user", "assistant"] {
        values.push(
            json!({"type":"response_item","payload":{"type":"message","role":role,
            "content":[{"type":"input_text","text":role}]}}),
        );
    }
    values.extend([
        json!({"type":"response_item","payload":{"type":"reasoning","text":"private"}}),
        json!({"type":"event_msg","payload":{"type":"user_message","message":"user"}}),
    ]);
    let turns = text(Family::Codex, &values);
    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns.iter().map(|turn| turn.role).collect::<Vec<_>>(),
        [TextRole::User, TextRole::Assistant]
    );
}

#[test]
fn unknown_shapes_preserve_status_and_known_text() {
    for content in [
        Value::Null,
        json!({"unknown":true}),
        json!([{"text":"legacy"},
        {"type":"tool_use","name":"Bash"}, {"unknown":true}]),
    ] {
        let user = json!({"type":"user","message":{"content":content}});
        assert!(
            claude::claude_session(&lines(&[assistant(json!([])), user.clone()]), &[]).is_some()
        );
        let turns = file_turns(Family::Claude, user).unwrap();
        if content.is_array() {
            let [_, tool] = array(&turns);
            assert_eq!(
                tool.content,
                TextBlock::ToolUse {
                    name: "Bash".into(),
                    input: Value::Null
                }
            );
        } else {
            assert!(turns.is_empty());
        }
        let message = json!({"type":"message","timestamp":"2026-10-20T00:00:00Z",
            "message":{"role":"user","content":content}});
        assert!(pi::pi_session(&lines(slice::from_ref(&message))).is_some());
        assert!(file_turns(Family::Pi, message).is_ok());
    }
}

#[test]
fn interrupted_user_text_is_preserved_and_repeated_blocks_are_not_deduplicated() {
    let repeated =
        assistant(json!([{"type":"text","text":"Again"},{"type":"text","text":"Again"}]));
    let user = json!({"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}});
    let turns = text(Family::Claude, &[repeated.clone(), repeated, user]);
    let [first, _, _, fourth, interrupted] = array(&turns);
    assert_eq!(first, fourth);
    assert_eq!(interrupted.role, TextRole::Injected);
    assert_eq!(
        interrupted.content,
        TextBlock::Text {
            text: "[Request interrupted by user]".into()
        }
    );
    for text in [
        "niles: w1 reported (done) — check workers",
        "niles: no report from w1 — check workers",
        "<environment_context>cwd</environment_context>",
        "<command-name>/clear</command-name>",
        "<local-command-stdout>output</local-command-stdout>",
        "<local-command-caveat>generated</local-command-caveat>",
        "<system-reminder>hook output</system-reminder>",
    ] {
        for (family, line) in [
            (
                Family::Claude,
                json!({"type":"user","message":{"content":text}}),
            ),
            (
                Family::Codex,
                json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}}),
            ),
            (
                Family::Pi,
                json!({"type":"message","message":{"role":"user","content":text}}),
            ),
        ] {
            assert_eq!(
                only(&file_turns(family, line).unwrap()).role,
                TextRole::Injected
            );
        }
    }
    let hook = json!({"type":"user","isMeta":true,"message":{"content":"hook output"}});
    assert_eq!(
        only(&file_turns(Family::Claude, hook).unwrap()).role,
        TextRole::Injected
    );
}

#[test]
fn jsonl_cursor_waits_for_complete_utf8_records_and_does_not_replay() {
    let dir = test_support::temp_dir("transcript-tail");
    let path = dir.join("001.jsonl");
    let bytes = lines(&[assistant(json!([{"type":"text","text":"é"}]))]).into_bytes();
    let split = bytes.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
    let (head, tail) = bytes.split_at(split);
    atomic_write(&path, head).unwrap();
    let mut file = BufReader::new(File::open(&path).unwrap());
    assert!(read_jsonl(&mut file, Family::Claude).unwrap().is_empty());
    let mut append = OpenOptions::new().append(true).open(&path).unwrap();
    append.write_all(tail).unwrap();
    assert_eq!(read_jsonl(&mut file, Family::Claude).unwrap().len(), 1);
    assert!(read_jsonl(&mut file, Family::Claude).unwrap().is_empty());
    append.write_all(b"bad\n").unwrap();
    let before = file.stream_position().unwrap();
    assert!(read_jsonl(&mut file, Family::Claude).is_err());
    assert_eq!(file.stream_position().unwrap(), before);
}

#[test]
fn opening_readers_uses_latest_pi_and_only_main_claude_file() {
    let home = test_support::temp_dir("transcript-open");
    let session_dir = home.join("pi");
    fs::create_dir_all(&session_dir).unwrap();
    atomic_write(session_dir.join("001.jsonl"), "bad\n").unwrap();
    atomic_write(
        session_dir.join("002.jsonl"),
        lines(&[json!({"type":"message","timestamp":null,
        "message":{"role":"user","content":"Hi"}})]),
    )
    .unwrap();
    let mut pi = TranscriptReader::open(&SessionLink::Pi { session_dir }, &home, &home, Utc::now())
        .unwrap()
        .unwrap();
    assert_eq!(pi.read().unwrap().len(), 1);
    let project = home.join(".claude/projects/test");
    let sub = project.join("session/subagents");
    fs::create_dir_all(&sub).unwrap();
    atomic_write(sub.join("sub.jsonl"), "bad\n").unwrap();
    atomic_write(
        project.join("session.jsonl"),
        lines(&[assistant(json!([{"type":"text","text":"Main"}]))]),
    )
    .unwrap();
    let mut claude = TranscriptReader::open_in(
        &home,
        &SessionLink::Claude {
            session_id: "session".into(),
        },
        &home,
        &home,
        Utc::now(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(claude.read().unwrap().len(), 1);
}

#[test]
fn codex_compressed_rollouts_share_telemetry_and_transcript_selection() {
    let home = test_support::temp_dir("codex-compressed");
    let created_at = Utc::now();
    let local = created_at.with_timezone(&chrono::Local);
    let dir = home.join(format!(".codex/sessions/{}", local.format("%Y/%m/%d")));
    fs::create_dir_all(&dir).unwrap();
    let plain = dir.join(format!(
        "rollout-{}-x.jsonl",
        local.format("%Y-%m-%dT%H-%M-%S")
    ));
    let compressed = plain.with_extension("jsonl.zst");
    let body = lines(&[
        json!({"type":"session_meta","payload":{"cwd":"/w"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user",
            "content":[{"text":"# Niles worker brief /w/report.md"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","name":"Bash","arguments":"{bad"}}),
        json!({"type":"event_msg","timestamp":"2026-10-20T00:00:00Z","payload":{"type":"task_complete"}}),
        json!({"type":"event_msg","timestamp":"2026-10-20T00:00:00Z","payload":{"type":"token_count",
            "info":{"total_token_usage":{"input_tokens":1,"output_tokens":2,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}),
    ]);
    let (head, tail) = body.as_bytes().split_at(20);
    let mut frames = stream::encode_all(head, 0).unwrap();
    frames.extend(stream::encode_all(tail, 0).unwrap());
    let (workspace, needle) = (Utf8Path::new("/w"), Utf8Path::new("/w/report.md"));
    atomic_write(plain.with_extension("jsonl.tmp"), &body).unwrap();
    atomic_write(&plain, "bad\n").unwrap();
    assert!(
        codex::read_codex_source(&home, workspace, needle, created_at)
            .unwrap()
            .is_none()
    );
    fs::remove_file(&plain).unwrap();
    atomic_write(&compressed, &frames).unwrap();
    for expected in [&compressed, &plain] {
        if expected == &plain {
            atomic_write(&plain, &body).unwrap();
            atomic_write(&compressed, "corrupt stale duplicate").unwrap();
        }
        let (path, parsed) = codex::read_codex_source(&home, workspace, needle, created_at)
            .unwrap()
            .unwrap();
        assert_eq!(&path, expected);
        let usage = codex::codex_usage(&parsed).unwrap().unwrap();
        assert_eq!(usage.output_tokens, 2);
        assert!(matches!(
            codex::codex_activity(&parsed).unwrap().state,
            super::super::TurnState::Ended
        ));
        let mut reader =
            TranscriptReader::open_in(&home, &SessionLink::Codex, workspace, needle, created_at)
                .unwrap()
                .unwrap();
        let turns = reader.read().unwrap();
        let [_, tool] = array(&turns);
        assert_eq!(
            tool.content,
            TextBlock::ToolUse {
                name: "Bash".into(),
                input: json!("{bad")
            }
        );
        assert!(reader.read().unwrap().is_empty());
    }
    atomic_write(
        &plain,
        body.replace(
            "\"input_tokens\":1",
            &format!("\"input_tokens\":{}", u64::MAX),
        ),
    )
    .unwrap();
    assert!(
        codex::read_codex(&home, workspace, needle, created_at)
            .unwrap()
            .is_none()
    );
    atomic_write(&plain, body.replace("report.md", "other.md")).unwrap();
    atomic_write(&compressed, frames).unwrap();
    assert!(
        codex::read_codex_source(&home, workspace, needle, created_at)
            .unwrap()
            .is_none()
    );
}

#[test]
fn compressed_transcript_errors_include_the_path() {
    let dir = test_support::temp_dir("compressed-transcript-errors");
    let path = dir.join("rollout.jsonl.zst");
    let mut valid = stream::encode_all(b"{}\n".as_slice(), 0).unwrap();
    valid.pop();
    for bytes in [
        b"not zstd".to_vec(),
        valid,
        stream::encode_all(b"\xff\n".as_slice(), 0).unwrap(),
    ] {
        atomic_write(&path, bytes).unwrap();
        let error = super::super::read_transcript(&path).unwrap_err();
        assert!(error.to_string().contains(path.as_str()));
    }
}
