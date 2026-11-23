use super::support::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::{fd::OwnedFd, unix::net::UnixStream};
use std::process::ChildStdin;

#[test]
fn stdio_tools_dispatch_and_share_the_workspace_wait_cursor() {
    let workspace = temp_workspace("niles-mcp-wait");
    let dir = worker_with_status(&workspace, "a", b"working: busy\n");
    edit_json(dir.join("meta.json"), |meta| {
        meta.as_object_mut().unwrap().insert(
            "gates".into(),
            json!({
                "post": [{"run": ["true"]}]
            }),
        );
    });
    atomic_write(dir.join("report.md"), "deliverable").unwrap();
    let (replies, writer) = UnixStream::pair().unwrap();
    replies
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut server = niles_bare(&workspace, &niles_home(&workspace))
        .arg("mcp")
        .env("NILES_AGENT_DIR", &dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(OwnedFd::from(writer)))
        .spawn()
        .unwrap();
    let mut input = server.stdin.take().unwrap();
    let mut replies = BufReader::new(replies).lines();
    let mut request = |input: &mut ChildStdin, method: &str, params: Value| {
        writeln!(
            input,
            "{}",
            json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params})
        )
        .unwrap();
        let response: Value = serde_json::from_str(&replies.next().unwrap().unwrap()).unwrap();
        assert_eq!(response.get("id"), Some(&json!(1)));
        assert!(response.get("error").is_none(), "{response}");
        response.get("result").unwrap().clone()
    };
    request(
        &mut input,
        "initialize",
        json!({
            "protocolVersion":"2025-11-25", "capabilities":{},
            "clientInfo":{"name":"niles-test","version":"1"}
        }),
    );
    // A real MCP client completes the initialization before issuing any calls.
    writeln!(
        input,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();

    let list = request(&mut input, "tools/list", json!({}));
    let tools = list.get("tools").and_then(Value::as_array).unwrap();
    let schema = |name| {
        tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
            .unwrap()
    };
    let spawn_schema = schema("spawn");
    for field in ["id", "skill", "worktree", "message"] {
        assert!(
            spawn_schema
                .pointer(&format!("/inputSchema/properties/{field}"))
                .is_some(),
            "{field}"
        );
    }
    assert_eq!(
        schema("wait").pointer("/inputSchema/properties/timeout/default"),
        Some(&json!("3600s"))
    );
    assert!(
        schema("wait")
            .pointer("/inputSchema/properties/ack")
            .is_some()
    );
    for name in ["spawn", "send"] {
        assert!(
            schema(name)
                .pointer("/inputSchema/properties/wait")
                .is_none()
        );
    }
    let mut tool = |name, arguments| {
        request(
            &mut input,
            "tools/call",
            json!({"name":name,"arguments":arguments}),
        )
    };
    for (name, typo, arguments) in [
        (
            "spawn",
            "worktre",
            json!({"id":"a","skill":"worker","message":"Task","worktre":"branch"}),
        ),
        (
            "send",
            "chekin",
            json!({"id":"a","message":"Task","chekin":"0"}),
        ),
    ] {
        let result = tool(name, arguments);
        assert!(
            result
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap()
                .contains(&format!("unknown field `{typo}`")),
            "{result}"
        );
    }

    let spawn = tool(
        "spawn",
        json!({
            "id":"job","skill":"worker","message":"Task","mode":"headless","billing":"subscription",
            "tier":"standard","agent":"codex"
        }),
    );
    assert_eq!(spawn.get("isError"), Some(&json!(true)));
    assert_eq!(
        spawn.pointer("/content/0/text"),
        Some(&json!("use tier or agent"))
    );

    let mut call = |name, arguments| {
        let result = tool(name, arguments);
        assert_ne!(result.get("isError"), Some(&json!(true)), "{result}");
        result.get("structuredContent").cloned()
    };
    for _ in 0..2 {
        assert_eq!(
            call("wait", json!({"timeout":"10ms"})),
            Some(json!({"outcome":"timeout"}))
        );
    }
    call(
        "agent_status",
        json!({"kind":"needs-decision","detail":"choose"}),
    );
    let first = call("wait", json!({"timeout":"0"})).unwrap();
    assert_eq!(
        first.get("wakes"),
        Some(&json!(["a needs-decision: choose"]))
    );
    let replay = call("wait", json!({"timeout":"0"})).unwrap();
    assert_eq!(
        replay.get("wakes"),
        first.get("wakes"),
        "unacknowledged wake was lost"
    );
    let cli = run_wait(&workspace, &["--timeout", "0"]);
    assert_command_success("CLI replay of an MCP wake", &cli);
    let batch = wake_batch(&cli);
    assert_eq!(batch.wakes, "a needs-decision: choose\n");
    append_status(&dir.join("status.log"), b"done: another\n");
    let next = call("wait", json!({"timeout":"0","ack":batch.ack})).unwrap();
    assert_eq!(next.get("wakes"), Some(&json!(["a done: another"])));
    let ack = next.get("ack").and_then(Value::as_str).unwrap();
    assert_drained(&run_wait(&workspace, &["--timeout", "0", "--ack", ack]));
    let stage = tool("agent_status", json!({"kind":"stage","detail":"ready"}));
    assert_ne!(stage.get("isError"), Some(&json!(true)), "{stage}");
    assert!(
        stage
            .pointer("/content/0/text")
            .and_then(Value::as_str)
            .unwrap()
            .contains("run-1 pass")
    );
    drop(input);
    assert!(server.wait().unwrap().success());
}
