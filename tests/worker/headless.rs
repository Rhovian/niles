use super::support::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::os::unix::{fs::PermissionsExt, net::UnixStream};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PEER: &str = r#"#!/usr/bin/env python3
import sys,json,os,time
sid='conversation-533';prompt=None;cancelling=False
send=lambda v: print(json.dumps(dict(jsonrpc='2.0',**v)),flush=True)
reply=lambda id,result: send(dict(id=id,result=result))
update=lambda text: send(dict(method='session/update',params=dict(sessionId=sid,update=dict(sessionUpdate='agent_message_chunk',content=dict(type='text',text=text)))))
for line in sys.stdin:
    m=json.loads(line)
    with open('peer.log','a') as f:f.write(json.dumps(m)+'\n')
    method=m.get('method');p=m.get('params',{})
    if method=='initialize':
        assert not os.environ.get('CODEX_API_KEY') and not os.environ.get('OPENAI_API_KEY')
        reply(m['id'],dict(protocolVersion=1,agentCapabilities=dict(loadSession=True,_meta=dict(authStatus={}))))
        send(dict(method='_auth/status_update',params=dict(authStatus=dict(kind=('account' if os.environ.get('PEER_AUTH')=='account_then_api' else os.environ.get('PEER_AUTH','account'))))))
    elif method=='session/new':reply(m['id'],dict(sessionId=sid))
    elif method=='session/load':
        assert p['sessionId']==sid
        while os.path.exists('hold-load') and not os.path.exists('release-load'):time.sleep(.01)
        update('APPROVED');reply(m['id'],{})
    elif method=='session/prompt':
        prompt=m['id'];text=p['prompt'][0]['text'];cancelling=False
        if os.environ.get('PEER_AUTH')=='account_then_api':
            update('AUTH_PARTIAL')
            send(dict(method='_auth/status_update',params=dict(authStatus=dict(kind='api_key'))))
        elif text=='approval':
            send(dict(id=100,method='session/request_permission',params=dict(sessionId=sid,toolCall=dict(toolCallId='tool-533',title='Owned test command'),options=[dict(optionId='yes',name='Allow once',kind='allow_once') ])))
        elif text=='error':send(dict(id=prompt,error=dict(code=-32603,message='peer failed')))
        elif text!='hold':update(text);reply(prompt,dict(stopReason='end_turn'))
    elif method=='session/cancel':
        cancelling=True
        send(dict(id=101,method='session/request_permission',params=dict(sessionId=sid,toolCall=dict(toolCallId='late-533',title='Late approval'),options=[])))
    elif m.get('id')==101:
        assert m['result']['outcome']==dict(outcome='cancelled')
        reply(prompt,dict(stopReason='cancelled'))
    elif m.get('id')==100:
        if m['result']['outcome']==dict(outcome='cancelled'):assert cancelling
        else:
            assert m['result']['outcome']==dict(outcome='selected',optionId='yes')
            send(dict(method='session/update',params=dict(sessionId=sid,update=dict(sessionUpdate='agent_thought_chunk',content=dict(type='text',text='PRIVATE_THOUGHT')))))
            send(dict(method='session/update',params=dict(sessionId=sid,update=dict(sessionUpdate='tool_call',toolCallId='tool-533',title='Owned tool',rawInput={}))))
            update('APP');update('ROVED');reply(prompt,dict(stopReason='end_turn'))
            send(dict(id=102,method='session/request_permission',params=dict(sessionId=sid,toolCall=dict(toolCallId='idle-tool',title='Idle approval'),options=[])))
    elif m.get('id')==102:assert m['result']['outcome']==dict(outcome='cancelled')
"#;
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        self.0.wait().unwrap();
    }
}
fn observed(dir: &Path, predicate: impl Fn(&Value) -> bool) -> Value {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(bytes) = fs::read(dir.join("acp-state.json")) {
            let state: Value = serde_json::from_slice(&bytes).unwrap();
            if predicate(&state) {
                return state;
            }
        }
        assert!(Instant::now() < until, "runner state timed out at {dir:?}");
        thread::sleep(Duration::from_millis(10));
    }
}
fn fixture(label: &str) -> (TestEnv, PathBuf) {
    let env = TestEnv::with_manifest(label, "codex");
    write_executable(&env.bin.join("codex-acp"), PEER);
    fs::remove_file(env.bin.join("codex")).unwrap();
    assert_command_success(
        "headless spawn",
        &env.spawn(
            "solo",
            &[
                "job",
                "--mode",
                "headless",
                "--billing",
                "subscription",
                "Task",
            ],
        ),
    );
    let dir = state_dir(&env.root).join("worker/job");
    (env, dir)
}
fn refused_send_preserves_wakes(env: &TestEnv, dir: &Path) {
    let log = fs::read(dir.join("status.log")).unwrap();
    let cursor = fs::read(dir.join("status.cursor")).ok();
    assert!(
        !env.run(&["send", "job", "--checkin", "off", "not sent"])
            .status
            .success()
    );
    assert_eq!(fs::read(dir.join("status.log")).unwrap(), log);
    assert_eq!(fs::read(dir.join("status.cursor")).ok(), cursor);
}
#[test]
fn headless_permissions_queue_cancel_resume_and_errors_share_the_harness() {
    let (env, dir) = fixture("headless-contract");
    refused_send_preserves_wakes(&env, &dir);
    let socket_dir = PathBuf::from(format!(
        "/tmp/niles-acp-{:x}",
        Sha256::digest(dir.to_str().unwrap().as_bytes())
    ));
    assert!(
        !socket_dir.exists(),
        "failed clients must not create socket directories"
    );
    fs::create_dir(&socket_dir).unwrap();
    fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o755)).unwrap();
    let failed = env
        .niles(&env.root, &["acp", dir.to_str().unwrap(), "approval"])
        .output()
        .unwrap();
    assert_failure_contains("unsafe socket", &failed, "unsafe ACP socket directory");
    assert!(
        !fs::read_to_string(env.root.join("peer.log"))
            .unwrap()
            .contains("session/new")
    );
    fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700)).unwrap();
    // An unsubmitted conversation may not be durable; retry the launch and its
    // assignment, rather than loading it as an already dispatched conversation.
    let mut state: Value =
        serde_json::from_slice(&fs::read(dir.join("acp-state.json")).unwrap()).unwrap();
    state["session"] = json!("conversation-533");
    state["assignment_submitted"] = json!(false);
    atomic_write(
        dir.join("acp-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();

    let child = Running(
        env.niles(&env.root, &["acp", dir.to_str().unwrap(), "approval"])
            .env("CODEX_API_KEY", "must-not-reach-adapter")
            .env("OPENAI_API_KEY", "must-not-reach-adapter")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    observed(&dir, |s| {
        s.pointer("/turn/state") == Some(&json!("RequiresAction"))
    });
    assert!(stdout_of(&env.run(&["workers"])).contains("needs_attention"));
    assert_eq!(
        fs::read_to_string(dir.join("status.log"))
            .unwrap()
            .matches("attention:")
            .count(),
        1
    );
    assert_failure_contains(
        "bad option",
        &env.run(&["approve", "job", "100", "wrong"]),
        "unknown permission option",
    );
    let _partial_client = UnixStream::connect(socket_dir.join("rpc")).unwrap();
    let approval_started = Instant::now();
    assert_command_success("approval", &env.run(&["approve", "job", "100", "yes"]));
    observed(&dir, |state| {
        state.pointer("/turn/state/Idle") == Some(&json!("EndTurn"))
            && fs::read_to_string(env.root.join("peer.log"))
                .unwrap()
                .lines()
                .any(|line| {
                    let message: Value = serde_json::from_str(line).unwrap();
                    message["id"] == 102
                        && message.pointer("/result/outcome/outcome") == Some(&json!("cancelled"))
                })
    });
    assert!(
        approval_started.elapsed() < Duration::from_secs(5),
        "a partial socket client stalled approval"
    );
    let turns: Vec<Value> = fs::read_to_string(dir.join("transcript.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let text: Vec<_> = turns
        .iter()
        .filter(|turn| turn["content"]["type"] == "text")
        .map(|turn| (&turn["role"], &turn["content"]["text"]))
        .collect();
    assert_eq!(
        text,
        vec![
            (&json!("user"), &json!("approval")),
            (&json!("assistant"), &json!("APPROVED"))
        ]
    );
    assert_command_success(
        "pending turn",
        &env.run(&["send", "job", "--checkin", "off", "approval"]),
    );
    observed(&dir, |s| {
        s.pointer("/turn/state") == Some(&json!("RequiresAction"))
    });
    assert_command_success("cancel permission", &env.run(&["cancel", "job"]));
    observed(&dir, |s| {
        s.pointer("/turn/state/Idle") == Some(&json!("Cancelled"))
    });
    assert_command_success(
        "send",
        &env.run(&["send", "job", "--checkin", "off", "hold"]),
    );
    observed(&dir, |s| {
        s.pointer("/turn/state") == Some(&json!("Running"))
    });
    let queued = env.run(&["send", "job", "--checkin", "off", "queued"]);
    assert_command_success("queue", &queued);
    assert!(stderr_of(&queued).contains("queued:"));
    assert_command_success("cancel", &env.run(&["cancel", "job"]));
    observed(&dir, |s| {
        s.pointer("/turn/state/Idle") == Some(&json!("Cancelled"))
    });
    assert_command_success("resume", &env.run(&["resume", "job"]));
    let peek = env.run(&["peek", "job", "--lines", "0"]);
    assert_command_success("peek", &peek);
    assert_eq!(
        stdout_of(&peek).matches("APPROVED").count(),
        1,
        "resume must not duplicate replayed history"
    );
    assert_command_success(
        "error turn",
        &env.run(&["send", "job", "--checkin", "off", "error"]),
    );
    observed(&dir, |s| {
        s.pointer("/turn/state/Idle/Error/message")
            .and_then(Value::as_str)
            .is_some_and(|message| message.contains("peer failed"))
    });
    drop(child);
    let _resumed = Running(
        env.niles(
            &env.root,
            &["acp", dir.to_str().unwrap(), "must-not-repeat-original"],
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap(),
    );
    observed(&dir, |s| {
        s.pointer("/turn/state/Idle") == Some(&json!("EndTurn"))
    });
    let calls = fs::read_to_string(env.root.join("peer.log")).unwrap();
    assert_eq!(calls.matches("session/new").count(), 1);
    assert_eq!(calls.matches("session/load").count(), 2);
    assert!(
        !calls.contains("\"text\": \"queued\""),
        "cancel must discard queued input before submitting it"
    );
    assert!(
        !calls.contains("must-not-repeat-original"),
        "restart must load without resubmitting the initial assignment"
    );
    atomic_write(env.root.join("hold-load"), "").unwrap();
    let mut resume = env
        .niles(&env.root, &["resume", "job"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while resume.try_wait().unwrap().is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    atomic_write(env.root.join("release-load"), "").unwrap();
    assert_failure_contains(
        "unanswered load is bounded",
        &resume.wait_with_output().unwrap(),
        "reply not confirmed (10s deadline); operation may still be pending",
    );
}
#[test]
fn headless_rejects_an_authentication_identity_that_disagrees_with_billing() {
    let (env, dir) = fixture("headless-billing");
    for identity in ["api_key", "account_then_api"] {
        let mut child = env
            .niles(&env.root, &["acp", dir.to_str().unwrap(), "must not run"])
            .env("PEER_AUTH", identity)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _input = child.stdin.take();
        let output = child.wait_with_output().unwrap();
        assert_failure_contains("auth mismatch", &output, "not the declared subscription");
        if identity == "account_then_api" {
            assert!(
                fs::read_to_string(dir.join("transcript.jsonl"))
                    .unwrap()
                    .contains("AUTH_PARTIAL")
            );
        }
        if identity == "api_key" {
            assert!(
                !fs::read_to_string(env.root.join("peer.log"))
                    .unwrap()
                    .contains("session/new")
            );
        }
    }
}
