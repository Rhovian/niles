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

[![CI](https://github.com/Rhovian/niles/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Rhovian/niles/actions/workflows/ci.yml?query=branch%3Amain)
[![crates.io](https://img.shields.io/crates/v/niles)](https://crates.io/crates/niles)

Niles coordinates coding agents from different model families, keeping work moving and allowing
for workflows where each model does what it does best.

Using one model family for implementation and another for review can add independent judgment.
Pairing a frontier lead with a cheaper worker may also reduce cost.

## Requirements

- `tmux`. Rust 1.85+ is required when installing with Cargo or building from source.
- The agent CLIs you select must already be installed, on `PATH`, and authenticated.
- Unix only.

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Quickstart

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh
niles
```

Alternatively, install with `cargo install niles`, or build from source with
`cargo install --git https://github.com/Rhovian/niles`.

Bare `niles` opens the project list. Type a project's number to open it, `n` to register a
directory, or `q` to quit, then press Enter. Opening a project creates or switches to its own tmux
session with the lead running in a window named `niles`. The first launch in a workspace prompts
for all four roles and writes `.niles/manifest.yaml`; later launches show the roles before
starting.

To open the list from anywhere in tmux, bind it to a popup:

```tmux
bind-key -n M-n display-popup -E niles
```

## Projects and the status bar

Each project is a tmux session, and each worker is a window in it. Project sessions show a
two-line status bar at the top:

- **Projects:** every live project and its lead's state. `●` the lead or one of its workers is
  working; `⚠` the lead is idle with no worker working, so it is waiting for you, with how long
  it has waited.
- **Windows:** the current project's lead and workers, with each agent's model, state (`●`
  working, `○` idle, `⚠` lead waiting for you), token total, and for workers, time since spawn.

Switch projects and windows with tmux's own keys (`switch-client -n/-p`, `next-window`,
`previous-window`), or bind them, for example:

```tmux
bind -n M-[ switch-client -p
bind -n M-] switch-client -n
bind -n M-\; previous-window
bind -n "M-'" next-window
```

## Parallel implementation

Workers in a workspace share one working tree, so two workers editing the same files overwrite
each other. To run a second implementation at once, give it its own tree:

```sh
git worktree add ../repo-b -b feature-b
niles spawn b --tree ../repo-b "Implement feature B"
```

The worker runs in `../repo-b`, while `niles workers`, `wait`, `close`, and the watcher still
track it from the lead's workspace. Niles never creates, cleans, or removes the tree; that stays
with `git worktree`.

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
| `niles` | List, register, and open projects |
| `niles doctor` | Show binary identity and dev-build staleness |
| `niles spawn [options] <id> (<text...> \| - \| -m <text>...)` | Start a worker window; `--tree` runs it in another working tree, `--wait` awaits its first wake |
| `niles close [options] [id]` | Close and archive workers by ID, `--task`, or `--all` |
| `niles workers` | Print this workspace's live workers, window health, and pending wakes as JSON |
| `niles usage` | Print usage for live lead and worker sessions as JSON |
| `niles models` | List the effective model and effort roster for this workspace |
| `niles report <id>` | Print a live or most recently archived worker report |
| `niles peek <id>` | Print recent pane output; `--lines 0` captures all history |
| `niles send [options] <id> (<text...> \| - \| -m <text>...)` | Steer a worker; add `--wait` to await its next wake |
| `niles wait [options] <id...>` | Consume the next wake; also supports `--task` and `--timeout` |
| `niles quiet <id>` | Disarm an intentionally idle worker's check-in |

Every duration is a non-negative integer followed by `ms`, `s`, `m`, or `h`, such as `500ms`,
`90s`, `5m`, or `1h`; plain `0` is also accepted. `spawn` and `send` accept `off` for
`--checkin`, whose nonzero delays must be at least `1s`. For one worker, their `--wait` forms fold
in `wait`; for a fleet, dispatch first and use `niles wait --task LABEL`.

## Configuration

The workspace manifest binds roles. This is the minimal valid `.niles/manifest.yaml`:

```yaml
lead: codex
worker: codex
reviewer: claude
security: claude
```

The first of `niles.yaml` or `.niles.yaml` defines custom agent executables:

```yaml
agents:
  local-reviewer:
    binary: review-agent
    args: ["--format", "plain"]

models:
  codex:
    gpt-5.7:
      efforts: [low, med, high, xhigh]
```

Model entries extend the built-in roster; listing an existing model replaces its effort list.
An empty `efforts: []` marks a model that takes no effort qualifier.

Bindings accept `family:model[:effort]`, such as `codex:gpt-6-astra:high` or `claude:opus:medium`;
`--agent` overrides a role binding. Built-in families are `codex`, `claude`, and `hermes`.
Set `reviewer: lead` to have the lead review worker diffs inline. This saves a separate reviewer
session, but the lead reviews its own plan and must question its design during the economy pass.
Run `niles models` to list the effective models and effort levels for the current workspace.

Optional manifest keys include `worker_planning`, a mapping from exact `family:model` names to
planning guidance the lead reads, and `checkin` / `recheck` for watcher cadence. Check-ins default
to five minutes, then back off to hourly reminders. Both take a duration as above; `0` and `off`
disable a check-in, a fixed recheck must be greater than zero, and `recheck: backoff` selects
backoff. Per-command `--checkin` overrides the manifest.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
