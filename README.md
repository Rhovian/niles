<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/niles-lockup-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset="docs/assets/niles-lockup-light.svg">
  <img alt="Niles" src="docs/assets/niles-lockup-light.svg" width="800">
</picture>

[![CI](https://github.com/Rhovian/niles/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Rhovian/niles/actions/workflows/ci.yml?query=branch%3Amain)
[![crates.io](https://img.shields.io/crates/v/niles)](https://crates.io/crates/niles)

An opinionated orchestration harness across your agent CLIs, featuring research, design, and security
skills; small diffs, telemetry, and options to make it your own. See the [roadmap](ROADMAP.md) for
what's next and the [releases](https://github.com/Rhovian/niles/releases) for what changed.

![Niles demo](docs/assets/niles-demo.gif)

## Requirements

- `tmux`. Rust 1.89+ is required when installing with Cargo or building from source.
- The agent CLIs you select must already be installed, on `PATH`, and authenticated.
  One CLI is enough: bind every skill to `claude` or to `codex`. Mixing families adds independent
  review.
- Unix only.

Niles tracks the agent CLIs closely and bumps its tested versions as often as it can. These are the
latest tested; if a newer release breaks something, report it with that version.

| CLI | Tested version |
| --- | --- |
| Claude Code | 2.1.296 |
| Codex | 0.162.1 |
| Hermes Agent | 0.21.3 |
| pi coding agent | 1.2.0 |

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## For agents

Agents: if you are setting Niles up for someone, load the `niles-setup` skill and follow it.
Without it installed, read [skills/niles-setup/SKILL.md](skills/niles-setup/SKILL.md) and follow it.

- [Install Niles](skills/niles-setup/SKILL.md#2-install).
- [Write the global project `manifest.yaml` with the user](skills/niles-setup/SKILL.md#3-write-the-manifest) and [run `niles doctor`](skills/niles-setup/SKILL.md#4-check-with-niles-doctor).
- [Have the user run `niles` to start the workspace](skills/niles-setup/SKILL.md#5-hand-off-to-start).

Install the skill for the CLIs you use: Claude Code (first line), Codex and pi (second line),
or Hermes (third line). Then ask "set up Niles here":

```sh
mkdir -p ~/.claude/skills/niles-setup && curl -fsSL https://raw.githubusercontent.com/Rhovian/niles/main/skills/niles-setup/SKILL.md -o ~/.claude/skills/niles-setup/SKILL.md
mkdir -p ~/.agents/skills/niles-setup && curl -fsSL https://raw.githubusercontent.com/Rhovian/niles/main/skills/niles-setup/SKILL.md -o ~/.agents/skills/niles-setup/SKILL.md
mkdir -p ~/.hermes/skills/niles-setup && curl -fsSL https://raw.githubusercontent.com/Rhovian/niles/main/skills/niles-setup/SKILL.md -o ~/.hermes/skills/niles-setup/SKILL.md
```

Rerun these lines to update the skill.

## Quickstart

1. Install with Homebrew, the shell installer or Cargo:

```sh
brew tap Rhovian/niles
brew install niles
```

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh
```

```sh
cargo install niles
```

2. Run `niles` from your repository. In the home view, select your project with the arrow
keys and press Enter. If it isn't listed, press `r`, enter the repository's absolute path,
then accept or choose a project name. Select the registered project and press Enter.
Choose and save your agent skills when prompted, then type to the lead in the right pane.

3. Check with `niles doctor` in another terminal in the repository after saving the skills.
It reports tmux and each bound agent CLI with its tested version; it does not check logins.
Without the global project `manifest.yaml`, it reports `agents: no workspace manifest` and checks no agent CLI.

See [suggested setup](docs/setup.md) for the status bar and tmux key bindings.

## What niles changes on your machine

- `~/.niles/state/<mangled absolute workspace path>/`: the skill manifest, sessions, worker briefs, status logs, and reports. The key replaces `/` with `-` (for example `/Users/j/code/my-app` becomes `-Users-j-code-my-app`); slash/hyphen collisions are accepted. The key must fit the filesystem's component length limit (usually 255 bytes); longer paths fail with an OS error, with no shortening fallback.
- `~/.niles/projects`: the list of registered projects.
- One tmux session per project, with its status bar options set on that session only.
- `~/.niles/trees/<absolute workspace path>/<branch>` when you spawn with `--worktree`.

Niles does not edit your agent CLIs' configuration or your tmux config.

## Skills

The lead is the agent you talk to. It owns the outcome: it reads the code, settles the plan, and
decides who does what. Anything cheaper to do than to delegate, it does itself, including reviewing
a diff. The rest it hands to other skills, commissioning as much review as the risk warrants,
which may be none.

- **Worker** implements the change and owns the gate: it runs the project's checks before reporting
  `done:` and says what printed, so nobody else re-runs them.
- **Reviewer** gives an independent read on correctness, idiom, economy, and test quality. It never
  does security review; it flags anything security-relevant in one line.
- **Security** asks what an attacker can do with the change. The lead commissions it only when the
  change is itself a security boundary, so ordinary work is not hardened against an unnamed attacker.
- **Research** answers one question with a source for every claim. It edits nothing and runs no
  checks, so the lead keeps the answer without spending its own context on the reading.

## Configuration

The workspace manifest binds skills. Here is a global project `manifest.yaml`:

```yaml
lead: {standard: {primary: codex:gpt-6.1-sol:medium, secondaries: []}}
worker: {standard: {primary: codex:gpt-6.1-sol:medium, secondaries: []}}
reviewer: {standard: {primary: claude:sonnet:high, secondaries: []}}
security: {standard: {primary: codex:gpt-6.1-sol:high, secondaries: []}}
design: {standard: {primary: claude:sonnet:high, secondaries: []}}
research: {standard: {primary: codex:gpt-6.1-sol:medium, secondaries: []}}
```

For ready-made setups with all shipped skill tiers, see [examples/](examples/):
[Claude lead](examples/claude-lead.yaml) and [Codex lead](examples/codex-lead.yaml).
From the project root, copy a template to
`"$HOME/.niles/state/$(pwd -P | tr '/' '-')/manifest.yaml"` (create its parent directory first).
Run `niles doctor` to check the required CLIs.

The first of `niles.yaml` or `.niles.yaml` extends the model roster for the built-in agent families:

```yaml
models:
  codex:
    gpt-5.7:
      efforts: [low, med, high, xhigh]
      context_window: 400000
```

Model entries extend the built-in roster; listing an existing model replaces its entry.
An empty `efforts: []` marks a model that takes no effort qualifier. The optional
`context_window`, in tokens, is what the telemetry panel measures a live session's context
against; without one, the panel shows the session's tokens and no percentage.

The shipped `src/agents/roster.yaml` registers each effort's tier, description, relative cost,
relative speed, and context window. `~/.niles/roster.yaml` replaces the complete entry list for
any model it names, or adds a model within a built-in family:

```yaml
codex:
  gpt-6-sol:
    - effort: high
      tier: standard
      description: Complex work with a balance of intelligence and cost
      cost: 2
      speed: null
      context_window: 1050000
```

Tiers are `frontier`, `standard`, `economy`, and `mechanical`. Omit `effort` for a model that takes
no effort flag. Cost ranks run from 1 (cheapest) to 4 (most expensive); speed ranks from 1
(fastest) to 4 (slowest). Ranks are model-level proxies; `null` represents unverified metadata.
`niles models` displays unverified ranks as `unknown` and lists only pairs allowed by at least
one workspace skill. Without a workspace manifest it lists all effective pairs. User overrides
load before project `models:` overrides; new workspace pairs need user-registry metadata to be
listed. Invalid registry overrides error.

Only the entry skill must be bound. `entry:` defaults to `lead`; other bindings are optional.
Each binding requires a `standard` tier and may list `frontier`, `economy`,
or `mechanical`. Every tier has one `primary` and an ordered `secondaries` list of exact
`family:model:effort` entries. Models without effort support, such as `claude:haiku`, omit
that suffix. Scalar bindings and `when:` groups are rejected.

`niles spawn <id> --skill <name>` uses its standard primary; `--tier <tier>` selects another listed
tier's primary. An unlisted tier errors. `--agent family:model:effort` selects an exact listed
primary or secondary and conflicts with `--tier`. The lead starts at its standard primary,
looks up metadata with `niles models`, and proposes tier or secondary changes with the reason
and cost difference, waiting for the operator's yes. It proposes economical step-downs too.

A manifest can configure managed worktrees without tying Niles to a build tool:

```yaml
worktree:
  env:
    CARGO_TARGET_DIR: /absolute/path/to/main/target
  setup: 'test -d "$CARGO_TARGET_DIR"'
```

`env` overrides the inherited worker environment for `spawn --worktree`, including when joining
an existing tree. Values are literal strings: use absolute paths, since `$HOME`, `~`, and shell
substitutions are not expanded. Sharing the main checkout's Cargo target directory reuses its
warm dependency cache; concurrent Cargo builds serialize on Cargo's lock. Niles sets no
language-specific defaults.

Optional `setup` runs synchronously via `sh -c` in a newly created tree, with the same
configured environment, before launching the worker. Use it for project preparation such as
`uv sync` or `pnpm install`. It runs only on creation, never on reuse. Stdin is `/dev/null` (EOF),
so setup must be non-interactive; both output streams go to Niles's stderr, keeping spawn stdout
reserved for its result. A nonzero exit fails the spawn and removes the new tree (including
setup-created files), so a retry runs setup again. The branch remains; omit `--base` when
retrying. Neither setting applies to plain spawns or `--tree`.

Concurrent creation/join of the same branch fails while creation or setup holds its lock. Niles
records pending creation before adding the tree, so failed Git creation, interrupted setup, or
failed cleanup block subsequent joins, even if setup later finishes in an orphan process. Setup
is never rerun on that existing tree. To recover, stop any remaining setup process, inspect/save
needed files, remove the incomplete worktree with Git, then remove the `.pending` file named in
the error (`~/.niles/state/<workspace-key>/worktrees/<branch>/.pending`). Recreate without
`--base` if the branch already exists. Keep the sibling `.lock` file; removing a live lock can
break exclusion. Trees created before this tracking existed are treated as existing trees and
skip setup.

Only put trusted commands in this operator-owned manifest: setup runs with your permissions.

Optional manifest keys include `checkin` / `recheck` for watcher cadence. Check-ins default
to five minutes, then back off to hourly reminders. Both take a duration: a non-negative integer
followed by `ms`, `s`, `m`, or `h`, such as `90s` or `5m`. `0` and `off` disable a check-in, a
fixed recheck must be greater than zero, and `recheck: backoff` selects backoff. Per-command
`--checkin` overrides the manifest.

See [skill workflows](docs/skills.md#design-and-implementation) for design, implementation,
and the `--from` and `--mechanical` spawn flags.

## Remote access (opt-in)

Use the installable [PWA](web/README.md) to list sessions, chat with the foreground
agent (including solo sessions), and receive Web Push when attention is needed.
Phone access is shell access on the host: Niles does not sandbox agents or workers.

Follow the [remote setup guide](docs/remote.md) for the optional Rust feature,
OpenSSL, allowlist, matching release asset, and exact `tailscale serve` commands
for static `/` plus the loopback `/api` proxy. Prebuilt release binaries omit
`remote`. Serve the PWA from the Niles host on the API's origin, never a public site.
See the [threat model](docs/security.md#remote-api) and documented framing gap.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.

`niles wait` covers every live worker in the current workspace, including workers spawned
while it runs. It returns immediately with all pending reports, one escaped
`<id> <kind>: <detail>` line per worker (at most 4096 bytes each). Run one wait, act on
the batch, then wait again; reports accumulate while you act, with no batching timer.
Status logs must be appended to (`>>`). An unsupported in-place truncate and rewrite that
regrows to the cached scan offset or beyond between polls can skip the rewritten prefix;
normal appends and respawns use the supported delivery path.
Unreadable or corrupt worker state, probe errors, and failures to append a generated wake stop
the workspace wait with an error on stderr. Repair the reported state and run wait again;
workers whose lines were not printed keep their cursors. Any lines already printed on stdout
stand as delivered even if a later worker causes a nonzero exit; process those lines before retrying.
A newer wait supersedes the old one. An unreported worker whose window or agent exits
produces `<id> gone: ...`; a worker confirmed idle for 30 seconds without reporting does too.
Gone lines use the same log/cursor delivery path as reports, and reported workers remain quiet.
A kill after printing but before advancing the cursor may replay that printed line. Exit codes: 0 wakes (reports or `gone:`), 11 superseded, 22 timeout.
`spawn --wait` and `send --wait` start this same workspace wait after dispatch.
The watcher also combines pending reports and overdue check-ins in one `niles:` line;
while a wait runs, it leaves report delivery to that wait.
