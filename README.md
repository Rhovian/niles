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

**Run Claude Code, Codex, Hermes and pi as one team in tmux.**

You talk to one agent. It plans the change, hands the work to others, often from another model
family, and reviews what comes back before it reaches you.

- **Diffs stay small.** Every handoff carries a line budget. A worker that passes it stops, and the
  overrun is trimmed line by line before review.
- **Independent review.** One model family writes the change and another reviews it.
- **Nothing goes quiet.** Niles checks in on quiet workers, wakes the lead when they report, and
  shows ⚠ in your status bar when a lead is waiting on you.
- **One Rust binary.** tmux and plain files: no server, no plugins, and your agent CLIs run
  unmodified.

## Requirements

- `tmux`. Rust 1.89+ is required when installing with Cargo or building from source.
- The agent CLIs you select must already be installed, on `PATH`, and authenticated.
  One CLI is enough: bind every role to `claude` or to `codex`. Mixing families adds independent
  review.
- Unix only.

Niles tracks the agent CLIs closely and bumps its tested versions as often as it can. These are the
latest tested; if a newer release breaks something, report it with that version.

| CLI | Tested version |
| --- | --- |
| Claude Code | 2.1.280 |
| Codex | 0.159.2 |
| Hermes Agent | 0.21.3 |
| pi coding agent | 0.73.1 |

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Quickstart

Install with the shell installer or with Cargo:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh
```

```sh
cargo install niles
```

Then run `niles`. It opens the project list: type a project's number to open it, `n` to register a
directory, or `q` to quit, then press Enter. Opening a project creates or switches to its own tmux
session with the lead running in a window named `niles`. The first launch in a workspace prompts
for all four roles and writes `.niles/manifest.yaml`; later launches show the roles before
starting.

See [suggested setup](docs/setup.md) for the status bar and tmux key bindings.

## What niles changes on your machine

- `.niles/` in each workspace: the role manifest, worker briefs, status logs, and reports.
- `~/.niles/projects`: the list of registered projects.
- One tmux session per project, with its status bar options set on that session only.
- `../<repo>-trees/<branch>` when you spawn with `--worktree`.

Niles does not edit your agent CLIs' configuration or your tmux config.

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
`--agent` overrides a role binding. Built-in families are `codex`, `claude`, `hermes`, and `pi`.
Set `reviewer: lead` to have the lead review worker diffs inline. This saves a separate reviewer
session, but the lead reviews its own plan and must question its design during the economy pass.
Run `niles models` to list the effective models and effort levels for the current workspace.

Optional manifest keys include `worker_planning`, a mapping from exact `family:model` names to
planning guidance the lead reads, and `checkin` / `recheck` for watcher cadence. Check-ins default
to five minutes, then back off to hourly reminders. Both take a duration: a non-negative integer
followed by `ms`, `s`, `m`, or `h`, such as `90s` or `5m`. `0` and `off` disable a check-in, a
fixed recheck must be greater than zero, and `recheck: backoff` selects backoff. Per-command
`--checkin` overrides the manifest.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
