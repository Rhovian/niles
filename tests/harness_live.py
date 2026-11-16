#!/usr/bin/env python3
"""Prepare or manually run native conformance using the shipped niles spawn/send commands."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import time
import uuid


def save(path, value):
    # os.replace publishes a complete record. https://docs.python.org/3/library/os.html#os.replace
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2), encoding="utf-8")
    temporary.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("prepare", "run"):
        command = commands.add_parser(name)
        command.add_argument("plan", type=Path)
        command.add_argument("evidence", type=Path)
    execute = commands.add_parser("exec", help="record argv and exec the selected CLI")
    execute.add_argument("argv", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command == "exec":
        argv = args.argv[1:] if args.argv[:1] == ["--"] else args.argv
        values = {key: os.environ[key] for key in json.loads(os.environ["NILES_CONFORMANCE_ENV_KEYS"])}
        save(Path(os.environ["NILES_CONFORMANCE_RECORD"]), {"argv": argv, "env": values})
        cli = os.environ["NILES_CONFORMANCE_CLI"]
        os.execv(cli, [cli, *argv])
    plan = json.loads(args.plan.read_text(encoding="utf-8"))
    evidence = args.evidence.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    save(evidence / "plan.json", plan)
    print(f"Prepared {evidence / 'plan.json'}", flush=True)
    if args.command == "prepare":
        return
    cli, niles, workspace = (Path(plan[key]).resolve(strict=True) for key in ("cli", "niles", "workspace"))
    if not plan["auth_path"].strip():
        raise ValueError("record the operator-approved auth path")
    def version():
        return subprocess.check_output([str(cli), "--version"], text=True)
    before = version()
    if plan["pin"] not in before.split():
        raise ValueError(f"CLI does not match pin: {before}")
    binary = evidence / "bin"
    binary.mkdir()
    wrapper = binary / plan["agent"].split(":", 1)[0]
    wrapper.write_text(f'#!/bin/sh\nexec python3 {shlex.quote(str(Path(__file__).resolve()))} exec -- "$@"\n')
    wrapper.chmod(0o755)
    socket = f"/tmp/niles-live-{uuid.uuid4().hex}.sock"
    env = dict(os.environ, TMUX=f"{socket},0,0", PATH=f"{binary}:{os.environ['PATH']}",
               NILES_CONFORMANCE_CLI=str(cli), NILES_CONFORMANCE_ENV_KEYS=json.dumps(plan.get("environment_keys", [])))
    for key in ("NILES_AGENT_DIR", "NILES_SETUP"):
        env.pop(key, None)
    current, retired = None, None

    def command(argv, text=None, success=True):
        result = subprocess.run(argv, input=text, text=True, capture_output=True, cwd=workspace, env=env, timeout=120)
        record = {"argv": argv, "exit": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
        save(evidence / "last-command.json", record)
        if success:
            result.check_returncode()
        return record

    try:
        command(["tmux", "-f", "/dev/null", "-S", socket, "new-session", "-d", "-s", "conformance", "sleep 3600"])
        print(f"Inspect in another terminal: tmux -S {shlex.quote(socket)} attach", flush=True)
        for index, step in enumerate(plan["steps"]):
            folder = evidence / f"step-{index}"
            folder.mkdir()
            env["NILES_CONFORMANCE_RECORD"] = str(folder / "exec.json")
            action = step["action"]
            if action == "spawn":
                if current is not None:
                    raise ValueError("disconnect before spawning the next generation")
                current = f"conformance-{index}"
                result = command([str(niles), "spawn", "--skill", plan["skill"], "--agent", plan["agent"], "--checkin", "off", current, "-"], step["text"])
                # Parse the shipped command's labeled brief path once; never evaluate its output.
                brief, = [Path(line.removeprefix("brief: ")) for line in result["stdout"].splitlines() if line.startswith("brief: ")]
                deadline = time.monotonic() + 30
                while not (folder / "exec.json").exists():
                    if time.monotonic() >= deadline:
                        raise TimeoutError("CLI did not publish argv")
                    time.sleep(0.01)
                received = json.loads((folder / "exec.json").read_text(encoding="utf-8"))
                if received["argv"][-1] != brief.read_text(encoding="utf-8").rstrip("\n"):
                    raise ValueError("opening argv differs from the shipped brief")
                for name in ("brief.md", "launch.sh", "meta.json"):
                    shutil.copyfile(brief.parent / name, folder / name)
                if retired is not None:
                    refused = command([str(niles), "send", retired, "retired-generation probe", "--checkin", "off"], success=False)
                    if refused["exit"] == 0:
                        raise ValueError("retired handle delivered input")
                    save(folder / "retired-handle.json", refused)
            elif action == "input":
                result = command([str(niles), "send", current, "-", "--checkin", "off"], step["text"])
            elif action == "disconnect":
                result = command(["tmux", "-S", socket, "kill-window", "-t", f"conformance:niles-{current}"])
                retired, current = current, None
            elif action == "capture":
                result = {}
            else:
                raise ValueError(f"unknown action {action}")
            print(f"{step['case']}: inspect work and lifecycle, then press Enter to capture", flush=True)
            input()
            snapshot = command([str(niles), "workers"])
            pane = command([str(niles), "peek", current, "--lines", "0"]) if current else None
            save(folder / "result.json", {"case": step["case"], "result": "needs_review", "command": result,
                                           "workers": snapshot, "pane": pane, "auth_path": plan["auth_path"]})
        after = version()
        if before != after:
            raise ValueError("CLI version changed during run")
        save(evidence / "run.json", {"version_before": before, "version_after": after, "protocol": "none",
                                    "niles_sha256": hashlib.sha256(niles.read_bytes()).hexdigest(),
                                    "auth_path": plan["auth_path"], "result": "needs_review"})
    finally:
        subprocess.run(["tmux", "-S", socket, "kill-server"], check=False, timeout=30)


if __name__ == "__main__":
    main()
