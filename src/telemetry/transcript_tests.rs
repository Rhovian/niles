use super::super::{claude, codex, parse_lines, pi};
use super::*;
use serde_json::json;
use std::io::Write;

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

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE sessions (id TEXT, source TEXT, started_at REAL, model_config TEXT);
        CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT, active INTEGER,
            role TEXT, content TEXT, tool_calls TEXT, timestamp REAL);
        INSERT INTO sessions VALUES ('old','test',1,NULL),('main','test',2,NULL),
            ('delegate','test',3,'{\"_delegate_from\":\"main\"}');
        INSERT INTO messages VALUES (1,'main',1,'user','Hi',NULL,1792454400);",
    )
    .unwrap();
    db
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
    let db = db();
    db.execute("DELETE FROM messages", []).unwrap();
    db.execute(
        "INSERT INTO messages VALUES(1,'main',1,'assistant','Testing',?1,1792454401)",
        [r#"[{"function":{"name":"Bash","arguments":"{\"command\":\"cargo test\"}"}}]"#],
    )
    .unwrap();
    let hermes = TranscriptReader::hermes(db, "test")
        .unwrap()
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(claude.len(), 2);
    assert_eq!(claude, pi);
    assert_eq!(claude, codex);
    assert_eq!(claude, hermes);
    assert_eq!(
        serde_json::to_value(&claude[1]).unwrap(),
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
    assert_eq!(turns[0].role, TextRole::User);
    assert_eq!(turns[1].role, TextRole::Assistant);
}

#[test]
fn malformed_arguments_and_legacy_blocks_do_not_erase_codex_telemetry() {
    let body = lines(&[
        json!({"type":"session_meta","payload":{"cwd":"/w"}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user",
            "content":[{"text":"# Niles worker brief /w/report.md"}]}}),
        json!({"type":"response_item","payload":{"type":"function_call","name":"Bash","arguments":"{bad"}}),
        json!({"type":"event_msg","timestamp":"2026-10-20T00:00:00Z","payload":{"type":"task_started"}}),
        json!({"type":"event_msg","timestamp":"2026-10-20T00:00:00Z","payload":{"type":"token_count",
            "info":{"total_token_usage":{"input_tokens":1,"output_tokens":2,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}),
    ]);
    let parsed = parse_lines::<codex::CodexLine>(&body).unwrap();
    assert!(codex::codex_matches(
        &parsed,
        Utf8Path::new("/w"),
        Utf8Path::new("/w/report.md")
    ));
    assert_eq!(codex::codex_usage(&parsed).unwrap().output_tokens, 2);
    assert!(codex::codex_activity(&parsed).is_some());
    let turns = file_turns(
        Family::Codex,
        serde_json::from_str(body.lines().nth(2).unwrap()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        turns[0].content,
        TextBlock::ToolUse {
            name: "Bash".into(),
            input: json!("{bad")
        }
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
            assert_eq!(turns.len(), 2);
            assert_eq!(
                turns[1].content,
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
        assert!(pi::pi_session(&lines(&[message.clone()])).is_some());
        assert!(file_turns(Family::Pi, message).is_ok());
    }
}

#[test]
fn interrupted_user_text_is_preserved_and_repeated_blocks_are_not_deduplicated() {
    let repeated =
        assistant(json!([{"type":"text","text":"Again"},{"type":"text","text":"Again"}]));
    let user = json!({"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}});
    let turns = text(Family::Claude, &[repeated.clone(), repeated, user]);
    assert_eq!(turns.len(), 5);
    assert_eq!(turns[0], turns[3]);
    assert_eq!(turns[4].role, TextRole::User);
    assert_eq!(
        turns[4].content,
        TextBlock::Text {
            text: "[Request interrupted by user]".into()
        }
    );
}

#[test]
fn jsonl_cursor_waits_for_complete_utf8_records_and_does_not_replay() {
    let dir = crate::test_support::temp_test_path("transcript-tail");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("001.jsonl");
    let bytes = lines(&[assistant(json!([{"type":"text","text":"é"}]))]).into_bytes();
    let split = bytes.iter().position(|byte| *byte == 0xc3).unwrap() + 1;
    std::fs::write(&path, &bytes[..split]).unwrap();
    let mut file = BufReader::new(File::open(&path).unwrap());
    assert!(read_jsonl(&mut file, Family::Claude).unwrap().is_empty());
    let mut append = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    append.write_all(&bytes[split..]).unwrap();
    assert_eq!(read_jsonl(&mut file, Family::Claude).unwrap().len(), 1);
    assert!(read_jsonl(&mut file, Family::Claude).unwrap().is_empty());
    append.write_all(b"bad\n").unwrap();
    let before = file.stream_position().unwrap();
    assert!(read_jsonl(&mut file, Family::Claude).is_err());
    assert_eq!(file.stream_position().unwrap(), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn hermes_cursor_filters_sessions_roles_and_inactive_rows() {
    let db = db();
    db.execute_batch(
        "INSERT INTO messages VALUES (2,'old',1,'assistant','old',NULL,1),
        (3,'delegate',1,'assistant','delegate',NULL,1),(4,'main',0,'assistant','inactive',NULL,1),
        (5,'main',1,'tool','result',NULL,1);",
    )
    .unwrap();
    let mut reader = TranscriptReader::hermes(db, "test").unwrap().unwrap();
    assert_eq!(reader.read().unwrap().len(), 1);
    assert!(reader.read().unwrap().is_empty());
    let Source::Hermes { db, .. } = &reader.source else {
        panic!("Hermes");
    };
    db.execute(
        "INSERT INTO messages VALUES (6,'main',1,'assistant','New',NULL,1)",
        [],
    )
    .unwrap();
    assert_eq!(
        reader.read().unwrap()[0].content,
        TextBlock::Text { text: "New".into() }
    );
}

#[test]
fn hermes_structured_content_is_decoded() {
    let db = db();
    db.execute(
        "UPDATE messages SET content=?1",
        ["\0json:[{\"type\":\"text\",\"text\":\"Testing\"}]"],
    )
    .unwrap();
    let turns = TranscriptReader::hermes(db, "test")
        .unwrap()
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].content,
        TextBlock::Text {
            text: "Testing".into()
        }
    );
}

#[test]
fn hermes_retains_invalid_arguments_and_omits_unknown_tool_shapes() {
    let db = db();
    db.execute(
        "UPDATE messages SET tool_calls=?1",
        [r#"[{"function":{"name":"Bash","arguments":"{bad"}},{},null,42]"#],
    )
    .unwrap();
    let turns = TranscriptReader::hermes(db, "test")
        .unwrap()
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(
        turns[1].content,
        TextBlock::ToolUse {
            name: "Bash".into(),
            input: json!("{bad")
        }
    );
}

#[test]
fn opening_readers_uses_latest_pi_and_only_main_claude_file() {
    let home = crate::test_support::temp_test_path("transcript-open");
    let session_dir = home.join("pi");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("001.jsonl"), "bad\n").unwrap();
    std::fs::write(
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
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("sub.jsonl"), "bad\n").unwrap();
    std::fs::write(
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
    std::fs::remove_dir_all(home).unwrap();
}
