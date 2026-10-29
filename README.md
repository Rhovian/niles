<img alt="Niles" src="docs/assets/niles-lockup-amber.svg" width="800">

[![CI](https://github.com/Rhovian/niles/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Rhovian/niles/actions/workflows/ci.yml?query=branch%3Amain)
[![crates.io](https://img.shields.io/crates/v/niles)](https://crates.io/crates/niles)

Niles orchestrates multi-agent and multi-family workflows across projects from one terminal
window. It runs Claude Code, Codex, and pi side by side in tmux, and you decide how they work
together by choosing skills. Any skill can be the entry agent you talk to: a lead that hands work
to other agents, or a solo agent that does the work itself. See the [roadmap](ROADMAP.md) for
what's next and the [releases](https://github.com/Rhovian/niles/releases) for what changed.

![Niles demo](docs/assets/niles-demo.gif)

## Is it for you

Niles suits you if you want the flexibility to use several model families instead of
committing to one. Small mechanical work can stay with one agent; complex work, or work that
benefits from scrutiny by a second family, can go to several. Most of the extra cost of using
several agents is in starting and stopping them, so you decide when that's worth it.

It isn't for you if you don't want to be in the terminal at least some of the time. With the
[remote PWA](#remote-access-opt-in) that time can be small.

> **Trust:** built-in worker defaults bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Requirements

- `tmux`. Rust 1.89+ is required when installing with Cargo or building from source.
- The agent CLIs you select must already be installed, on `PATH`, and logged in. One is enough.
- Unix only.

Niles tracks the agent CLIs closely and bumps its tested versions as often as it can. These are the
latest tested; older versions are not blocked. If a newer release breaks something, report it
with that version.

| CLI | Tested version |
| --- | --- |
| Claude Code | 2.1.312 |
| Codex | 0.165.0 |
| pi coding agent | 1.4.0 |

## Quickstart

1. Install Niles with Homebrew, the shell installer, or Cargo:

   ```sh
   brew tap Rhovian/niles && brew install niles
   ```

   ```sh
   curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh
   ```

   ```sh
   cargo install niles
   ```

   The shell installer puts `niles` in `${CARGO_HOME:-$HOME/.cargo}/bin`. Check that
   `niles --version` works.

2. Install the setup skill for your agent CLI. The first line is for Claude Code, the second for
   Codex and pi. Rerun it later to update the skill.

   ```sh
   mkdir -p ~/.claude/skills/niles-setup && curl -fsSL https://raw.githubusercontent.com/Rhovian/niles/main/skills/niles-setup/SKILL.md -o ~/.claude/skills/niles-setup/SKILL.md
   mkdir -p ~/.agents/skills/niles-setup && curl -fsSL https://raw.githubusercontent.com/Rhovian/niles/main/skills/niles-setup/SKILL.md -o ~/.agents/skills/niles-setup/SKILL.md
   ```

3. In your repository, start that agent CLI and ask it to "set up Niles here". It copies the skills
   you choose, writes the project's manifest with you, and runs `niles doctor`, which checks tmux
   and each bound CLI against its tested version; the agent fixes what doctor reports. Doctor does
   not check logins.

4. In a terminal in the repository, run `niles`. In the home view, select your project with the
   arrow keys and press Enter. If it isn't listed, press `r`, enter the repository's absolute path,
   accept or choose a project name, then select it and press Enter.

5. When the skills picker appears, press `c` to keep the manifest, or change an agent and save.
   Then type to your entry agent in the right pane.

To leave, detach with tmux (`Ctrl-b d` by default); agents keep running. To stop a project, select
it in the home view and press `c`. See [suggested setup](docs/setup.md) for the home view keys,
the status bar, and tmux key bindings.

## How it works

Each project is a tmux session. You are the operator, the person talking to the entry agent. What
the entry agent does is set by its skill, an ordinary `SKILL.md` file. A lead skill spawns workers, each in its own tmux window, and reads
their reports; a solo skill does the work in one session.

The manifest binds each skill to an agent, written `family:model:effort`, such as
`claude:sonnet:high`. A binding can list tiers (`frontier`, `standard`, `economy`, `mechanical`),
so a lead can propose a cheaper or stronger model for a task. `niles models` lists the models and
their relative cost and speed.

## Skills in this repository

The `niles` binary contains no skills; it loads `SKILL.md` files from `.niles/skills/` in the repository or
from `~/.niles/skills/`. The [skills](skills/) here are one workflow you can copy, or pin with
`niles skills import Rhovian/niles/skills/<name>@<rev>`.

- `lead` coordinates a session: it talks to you, spawns agents, and forwards their work. See the
  [lead workflow](skills/lead/SKILL.md).
- `solo` carries out your request directly in one session.
- `worker` implements a change and runs the focused checks CI doesn't cover.
- `reviewer` reviews correctness, idiom, economy, and test quality, and flags security concerns
  without reviewing them.
- `security` asks what an attacker can do with a change.
- `design` writes a design record before implementation.
- `research` answers one question with a source for every claim and edits nothing.
- `niles-setup` installs Niles and writes the manifest with you.

## What Niles changes on your machine

- `~/.niles/state/<mangled absolute workspace path>/`: the skill manifest, sessions, worker briefs, status logs, and reports. The key replaces `/` with `-` (for example `/Users/j/code/my-app` becomes `-Users-j-code-my-app`); slash/hyphen collisions are accepted. The key must fit the filesystem's component length limit (usually 255 bytes); longer paths fail with an OS error, with no shortening fallback.
- `~/.niles/projects`: the list of registered projects.
- `~/.niles/cache/`: checkouts of skills pinned with `niles skills import`, which also writes a
  `SKILL.ref` pointer under `.niles/skills/` or `~/.niles/skills/`.
- `~/.niles/config.yaml`: settings you save in the config panel.
- `~/.niles/trees/<absolute workspace path>/<branch>` and its Git branch when you spawn with
  `--worktree`.
- `~/.niles/remote/`: push keys and subscriptions, only if you use [remote access](#remote-access-opt-in).
- One tmux session per project, with its status bar options set on that session only.

Niles reads, but never writes, the model overrides in `~/.niles/roster.yaml` and a project's
`niles.yaml`. It
does not edit your agent CLIs' configuration or your tmux config. To remove Niles, stop its
projects, uninstall the binary the way you installed it, and delete `~/.niles`, the
`niles-setup` skill directories, any `.niles/` directory in your repositories, and any branches
worktree spawns left behind.

## Configuration

Each project has one manifest, at `~/.niles/state/<workspace key>/manifest.yaml`. The setup
skill writes it; a missing manifest fails, and Niles does not create a default. It names the entry
skill and binds every skill you use to a `standard` tier's `primary` agent, with optional
fallback `secondaries`:

```yaml
entry: lead
lead: {standard: {primary: claude:opus:medium, secondaries: []}}
worker: {standard: {primary: codex:gpt-6.1-sol:medium, secondaries: []}}
reviewer: {standard: {primary: claude:sonnet:high, secondaries: []}}
```

Only put trusted commands in this operator-owned manifest: worktree `setup` runs with your
permissions. See [configuration](docs/configuration.md) for tiers, model overrides, worktrees,
and check-ins, and [skill workflows](docs/skills.md) for design, implementation, and named setups.

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
