#!/usr/bin/env python3
"""ACP v1 peers for the harness conformance sequence."""
import json
import os
from pathlib import Path
import subprocess
import sys


def send(**message):
    print(json.dumps(dict(jsonrpc="2.0", **message)), flush=True)


def receipt(text):
    temporary = Path("receipt.tmp")
    temporary.write_text(text, encoding="utf-8")
    # Publish the whole prompt in one rename: https://docs.python.org/3/library/os.html#os.replace
    temporary.replace("receipt")


def check_pi():
    if sys.argv[1:] == ["auth", "check", "--provider", "openai", "--json"]:
        Path("pi-auth-checked").touch()
        print(json.dumps(dict(status="ready", authType="oauth")))
        return
    assert sys.argv[1:-1] == [
        "--model", "openai/fixture", "--thinking", "high", "--provider", "openai",
        "--append-system-prompt", "conformance system prompt",
        "--extension",
    ], sys.argv
    assert Path(sys.argv[-1]).resolve(strict=True) == Path.cwd() / "acp-extension.js"


def peer(binary):
    sid = "conformance-session"
    pi = binary == "pi-acp"
    claude = binary == "claude-agent-acp"
    assert pi or claude or binary == "codex-acp", binary
    if pi:
        Path("pi-auth-checked").unlink()
        subprocess.run([os.environ["PI_ACP_PI_COMMAND"]], check=True)
    elif not claude:
        config = json.loads(os.environ["CODEX_CONFIG"])
        assert config["forced_login_method"] == "chatgpt"
        assert config["developer_instructions"] == "conformance system prompt"
        assert os.environ["INITIAL_AGENT_MODE"] == "agent-full-access"

    credential = "ANTHROPIC_API_KEY" if claude else "OPENAI_API_KEY"
    assert credential not in os.environ
    opened = resumed = ready = cancel_requested = False
    history_file = Path("peer-history.json")
    history = json.loads(history_file.read_text(encoding="utf-8")) if history_file.exists() else []
    pending = None

    def update(text):
        send(method="session/update", params=dict(sessionId=sid, update=dict(
            sessionUpdate="agent_message_chunk", content=dict(type="text", text=text))))

    def complete(text, reason="max_turn_requests"):
        nonlocal pending
        history.append(text)
        history_file.write_text(json.dumps(history), encoding="utf-8")
        update(text)
        send(id=pending, result=dict(stopReason=reason))
        pending = None

    for line in sys.stdin:
        message = json.loads(line)
        method = message.get("method")
        params = message.get("params", {})
        request_id = message.get("id")
        if method == "initialize":
            assert params["protocolVersion"] == 1
            capabilities = dict(loadSession=True)
            if not pi:
                capabilities["_meta"] = dict(authStatus={})
            send(id=request_id, result=dict(protocolVersion=1, agentCapabilities=capabilities))
            if not pi:
                send(method="_auth/status_update", params=dict(authStatus=dict(kind="account")))
        elif method in ("session/new", "session/load"):
            assert Path(params["cwd"]).resolve() == Path.cwd().resolve()
            if claude:
                assert params["_meta"]["systemPrompt"]["append"] == "conformance system prompt"
                assert params["_meta"]["claudeCode"]["options"]
            if method == "session/new":
                assert not opened and not history
                opened = True
                send(id=request_id, result=dict(sessionId=sid))
            else:
                assert params["sessionId"] == sid and history
                if Path("missing-history").exists():
                    send(id=request_id, error=dict(code=-32603, message="missing peer history"))
                    continue
                for reply in history:
                    update(reply)
                resumed = True
                send(id=request_id, result={})
            ready = not claude
        elif method == "session/set_mode":
            assert claude and params["sessionId"] == sid
            assert params["modeId"] == "bypassPermissions"
            ready = True
            send(id=request_id, result={})
        elif method == "session/prompt":
            assert ready and params["sessionId"] == sid and pending is None
            pending = request_id
            [prompt] = params["prompt"]
            assert prompt["type"] == "text"
            text = prompt["text"]
            assert not resumed or text != os.environ["NILES_CONFORMANCE_FIRST"], (
                "loaded session received the original assignment")
            receipt(text)
            if text == "history probe":
                assert resumed and history[0] == "peer opening reply"
                complete("loaded opening history")
            elif text == "approval":
                send(id=100, method="session/request_permission", params=dict(
                    sessionId=sid, toolCall=dict(toolCallId="approval", title="Allow fixture"),
                    options=[dict(optionId="yes", name="Allow once", kind="allow_once")]))
            elif text != "hold for cancel":
                complete("peer opening reply" if not history else "peer reply")
        elif request_id == 100:
            assert message["result"]["outcome"] == dict(outcome="selected", optionId="yes")
            complete("peer approved", reason="end_turn")
        elif method == "session/cancel":
            assert params["sessionId"] == sid
            if cancel_requested:
                send(id=pending, result=dict(stopReason="cancelled"))
                pending = None
            else:
                cancel_requested = True
                send(id=101, method="session/request_permission", params=dict(
                    sessionId=sid, toolCall=dict(toolCallId="late", title="Late fixture"), options=[]))
        elif request_id == 101:
            assert message["result"]["outcome"] == dict(outcome="cancelled")
            receipt("cancel requested")
        else:
            raise AssertionError(message)


if __name__ == "__main__":
    binary = Path(sys.argv[0]).name
    if binary == "pi":
        check_pi()
    else:
        peer(binary)
