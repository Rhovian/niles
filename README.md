```text
▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄    ▄▄▄▄▄▄▄▄ ▄▄▄▄▄▄▄▄▄            ▄▄▄▄▄▄▄▄▄▄▄▄▄▄     ▄▄▄▄▄▄▄▄▄▄▄▄▄▄
▄              ▀▄  ▄      ▓ ▄       ▓     ░    ▄▀        ░ ░▒▓░  ▄▀▀             ░
█               ▐▌ █      ▒ █       ▒   ·░░░  ▐▌            ░▒▒ ▐▌     ▄▄▄▄     ▒▒
▓               ░█ ▓      ░ █       ░   ░▒▒░  █              ░▓ █      ░  ░     ░▓
▒      █▀▀█      █ ▒      █ ▓       █   ░░░   ▓       ▄▄▄▄▄▄▄▄█ ▒      ▒  ▀▀▀▀▀▀▀▀
░      █  ░     ┼█ ░      █ ░       █    ░ ·  ░       ▓▄▄▄▄ ·   ▐▌      ▀▄▄▄▄▄▄
█┼     █  ▒    ┼┼█ █┼     █ █ ┼     █       · █ ┼     ▄▄▄▄▒   ·  ▀▄▄         ┼┼▀▄
█┼    ┼█  ▓┼    ┼█ █┼    ┼█ █┼┼┼   ┼█▄▄▄▄▄▄▄▄ █┼┼┼   ┼▓▄▄▄▄▄▄▄▄     ▀▀▀▀▀▄▄┼├├┼├├▌
█┼┼  ┼┼░  ▒┼    ┼░ █┼┼  ┼┼░ █┼┼┼ ┼┼┼┼┼┼┼┼┼┼┼░ █┼┼┼ ┼┼┼┼┼┼┼┼┼┼┼░ ▀▀▀▀▀▀▀░  ▒┼├├┼├├▓
█┼┼┼┼┼┼▒  ░┼┼┼ ┼┼▒ █┼┼┼┼┼┼▒ ▐▌┼┼┼┼┼┼┼┼┼┼┼┼┼┼▒ ▐▌┼┼┼┼┼┼┼┼┼┼┼┼┼┼▒ ▒┼┼├┼┼┼▒▄▄▓┼┼├┼┼▐▌
█┼┼┼┼┼┼▓  █┼┼┼┼┼┼▓ █┼┼┼┼┼┼▓ ·▀▄┼┼┼┼┼┼┼┼┼┼┼┼┼▓ ·▀▄┼┼┼┼┼┼┼┼┼┼┼┼┼▓ ░┼├┼┼┼┼┼┼┼┼┼┼┼┼▄▀
▀▀▀▀▀▀▀▀  ▀▀▀▀▀▀▀▀ ▀▀▀▀▀▀▀▀    ▀▀▀▀▀▀▀▀▀▀▀▀▀▀    ▀▀▀▀▀▀▀▀▀▀▀▀▀▀ ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
```

Niles coordinates coding agents from different model families, keeping work moving and allowing
for workflows where each model does what it does best.

Using one model family for implementation and another for review can add independent judgment.
Pairing a frontier lead with a cheaper worker may also reduce cost.

## Requirements

- Rust 1.85+ and `tmux`.
- The agent CLIs you select must already be installed, on `PATH`, and authenticated.
- Unix only.

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Quickstart

Bare `niles` interactively creates or updates `.niles/manifest.yaml`, lets you select the lead,
and starts it in the current tmux pane.

```sh
cargo install --git https://github.com/Rhovian/niles
cd /path/to/your/project
tmux new-session -s niles # skip if already in tmux
niles
```

## Roles

The lead is the agent you talk to. It owns the outcome: it reads the code, settles the plan, and
decides who does what. Anything cheaper to do than to delegate, it does itself, including reviewing
a diff. The rest it hands to other roles, commissioning as much review as the risk warrants.

- **Worker** implements the change and owns the gate: it runs the project's checks before reporting
  `done:` and says what printed, so nobody else re-runs them.
- **Reviewer** gives an independent read on correctness, idiom, economy, and test quality. It never
  does security review; it flags anything security-relevant in one line.
- **Security** asks what an attacker can do with the change. The lead commissions it only when the
  change is itself a security boundary, so ordinary work is not hardened against an unnamed attacker.

## Command reference

| Command | Purpose |
| --- | --- |
| `niles` | Configure the manifest and start the foreground lead |
| `niles doctor` | Show binary identity, schema state, and dev-build staleness |
| `niles spawn [options] <id> <task...>` | Start a worker window; add `--wait` to await its first wake |
| `niles close [options] [id]` | Close and archive workers by ID, `--task`, or `--all` |
| `niles workers` | List this workspace's live workers, window health, and pending wakes |
| `niles report <id>` | Print a live or most recently archived worker report |
| `niles peek <id>` | Print recent pane output; `--lines 0` captures all history |
| `niles send [options] <id> <message...>` | Steer a worker; add `--wait` to await its next wake |
| `niles wait [options] <id...>` | Consume the next wake; also supports `--task` and `--timeout` |
| `niles quiet <id>` | Disarm an intentionally idle worker's check-in |

`spawn` and `send` accept `--checkin 90s`, `5m`, `1h`, bare minutes, or `off`. For one worker,
their `--wait` forms fold in `wait`; for a fleet, dispatch first and use `niles wait --task LABEL`.

## Configuration

The workspace manifest binds roles. This is the minimal valid `.niles/manifest.yaml`:

```yaml
lead: codex
worker: codex
reviewer: claude
security: claude
niles_schema: 2
```

The first of `niles.yaml` or `.niles.yaml` defines custom agent executables:

```yaml
agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]
```

Bindings accept `family:model[:effort]`, such as `codex:gpt-6-astra:high` or `claude:opus:medium`;
`--agent` overrides a role binding. Built-in families are `codex`, `claude`, and `hermes`.
Supported models and effort levels are listed in [the agent profiles](src/agents/families.rs).

Optional manifest keys include `worker_planning`, a mapping from exact `family:model` names to
planning guidance the lead reads, and `checkin` / `recheck` for watcher cadence. Check-ins default
to five minutes, then back off to hourly reminders; `recheck: 10m` selects a fixed gap instead.
Per-command `--checkin` overrides the manifest, and `off` disables the check-in.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
