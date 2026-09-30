# Niles

Niles coordinates coding agents from different model families in tmux, keeping work moving and
bringing worker reports back to the lead.

Named for the butler in *The Nanny*, it answers the door when a worker reports.

## Why Niles?

Using one model family for implementation and another for review can add independent judgment.
Pairing a frontier lead with a cheaper worker may also reduce cost. Those are motivations from
project experience, not benchmark results or guarantees of quality or cost.

## Requirements

- Rust 1.85+ and `tmux`.
- The agent CLIs you select must already be installed, on `PATH`, and authenticated.
- Unix only: Niles has been tested on macOS and Linux. Windows is unsupported; the implementation
  uses `flock`, `O_NOFOLLOW`, and `std::os::unix`.

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Quickstart

Run these four commands in a shell. Bare `niles` interactively creates or updates
`.niles/manifest.yaml`, lets you select the lead, and starts it in the current tmux pane.

```sh
cargo install --git https://github.com/Rhovian/niles
cd /path/to/your/project
tmux new-session -s niles # skip if already in tmux
niles
```

Ask the lead to run this fifth command, or run it from a second tmux pane in the same project:

```sh
niles spawn first-task "Inspect this project and propose one useful improvement"
```

Workers stay open after reporting so the lead can inspect and steer them; close them explicitly
after integration with `niles close <id>` or `niles close --task <label>`.

## Roles

| Role | Owns | Must not do |
| --- | --- | --- |
| Lead | Outcome, plan, delegation, integration | Implement the delegated change |
| Worker | Implementation and the project gate | Report `done:` without running the documented checks |
| Reviewer | Correctness, idiom, economy, test quality | Run the gate or perform the security pass |
| Security | Attacker-focused review of security boundaries | Run the gate or redo correctness/style review |

Only workers run the gate, avoiding duplicated test runs. Security is separate so ordinary review
does not turn every change into hardening against an unnamed attacker.

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
| `niles help [command]` | Show general or command-specific help |

`spawn` and `send` accept `--checkin 90s`, `5m`, `1h`, bare minutes, or `off`. For one worker,
their `--wait` forms fold in `wait`; for a fleet, dispatch first and use `niles wait --task LABEL`.

## Configuration

The workspace manifest binds roles. Bare `niles` creates it interactively; this is the minimal
valid `.niles/manifest.yaml`:

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

## Exit status

| Code | Meaning |
| ---: | --- |
| `0` | Success, or an actionable wake (`done:`, `failed:`, `blocked:`, or `needs-decision:`); a wake is not proof the task succeeded |
| `1` | Operational failure |
| `2` | Command-line argument parse failure |
| `10` | Worker closed or disappeared, after any queued wake was delivered |
| `22` | Wait timed out (default: 1 hour) |

The wait-specific codes also apply to `niles spawn --wait` and `niles send --wait`.

## Checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
```

Run these before reporting a change done: formatting, the clippy gate across all targets (tests
included), and the test suite. Keep `--no-fail-fast`: plain `cargo test` stops at the first failing
binary, so a failure in one test file hides every later one.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md), and consult the [threat model](docs/security.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
