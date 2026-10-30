<img alt="Niles" src="assets/niles-lockup-amber.svg" width="800">

[![CI](https://github.com/Rhovian/niles/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Rhovian/niles/actions/workflows/ci.yml?query=branch%3Amain)
[![crates.io](https://img.shields.io/crates/v/niles)](https://crates.io/crates/niles)

Niles orchestrates multi-agent and multi-family workflows across projects from one terminal
window. It runs Claude Code, Codex, and pi side by side in tmux; the skills you choose decide how
they work together. See the [roadmap](ROADMAP.md) and [releases](https://github.com/Rhovian/niles/releases).

![Niles demo](assets/niles-demo.gif)

## Is it for you

It suits you if you want to use several model families rather than commit to one. Small work can
stay with one agent; complex work can get a second family's scrutiny. Most of the extra cost is
in starting and stopping agents, so you choose when it's worth it.

It isn't for you if you don't want to be in the terminal at all; the [remote PWA](#remote-access-opt-in)
keeps that time small.

> **Trust:** Niles's default worker launch flags bypass agent approval prompts, and Niles provides no
> sandbox. Read the [threat model](docs/security.md) before running agents on a repository.

## Requirements

Unix, `tmux`, and at least one agent CLI installed, on `PATH`, and logged in. Rust 1.89+ only if
you install with Cargo. Latest tested CLI versions (older ones aren't blocked; report breakage
with the version):

| CLI | Tested version |
| --- | --- |
| Claude Code | 2.1.312 |
| Codex | 0.165.0 |
| pi coding agent | 1.4.0 |

## Quickstart

1. Install Niles with one of:

   ```sh
   brew tap Rhovian/niles && brew install niles
   curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh
   cargo install niles
   ```

2. Run Niles in your repository and answer the setup prompts:

   ```sh
   cd <repo> && niles
   ```

   Register the folder when prompted, then choose defaults or edit the full manifest in the
   project pane. Niles detects your agent CLIs, pins release-matched skills from GitHub, and starts
   the entry agent there. Registration is optional; `r` in the home view registers later projects.

Detach with `Ctrl-b d`; agents keep running. See the [interface reference](docs/setup.md) for keys and
the status bar.

## How it works

Each project is a tmux session. You, the operator, talk to the entry agent, whose skill sets what
it does: a lead spawns workers in their own windows and reads their reports; solo does the work
itself. The manifest defines project-wide model tiers (`family:model:effort`) usable by every skill;
`niles models` lists the approved agents.

## Skills in this repository

The binary embeds no skill text. Setup in the project pane pins the default set from this
repository, at the binary's release tag, into `~/.niles/skills`; copy or pin others with
`niles skills import Rhovian/niles/skills/<name>@<rev>`.

- `lead` coordinates: talks to you, spawns agents, forwards their work.
- `solo` does your request directly in one session.
- `worker` implements a change and runs focused checks.
- `reviewer` reviews correctness, idiom, economy, and tests.
- `security` asks what an attacker can do with a change.
- `design` writes a design record before implementation.
- `research` answers one question with sources and edits nothing.

## What Niles changes on your machine

Niles writes under `~/.niles/` (state, project list, skill cache, config, worktrees, remote keys),
pins skills from GitHub at the binary's release tag on first setup,
creates one tmux session per project, and creates branches for worktree spawns. It doesn't touch your agent CLIs' or tmux's
config. To remove it, close its projects in the home view (`c`), uninstall the binary, and delete `~/.niles` and
any `.niles/` in your repositories. See
[state layout](docs/configuration.md#state-layout) for details.

## Configuration

Each project's manifest lives at `~/.niles/state/v2/<workspace key>/@state/manifest.yaml`
(see [state layout](docs/configuration.md#state-layout) for the encoding). First-run setup
generates it from your installed agent CLIs. It names the entry skill and defines project-wide tiers with a `primary`
and ordered fallback `secondaries`:

```yaml
entry: lead
tiers:
  standard: {primary: claude:sonnet:high, secondaries: [codex:gpt-6.1-sol:medium]}
  frontier: {primary: claude:opus:high, secondaries: [codex:gpt-6-astra:high]}
```

The manifest's worktree `setup` runs with your permissions, so keep it trusted. See
[configuration](docs/configuration.md) and [skill workflows](docs/skills.md) for the rest.

## Remote access (opt-in)

The installable [PWA](web/README.md) lists sessions, chats with the foreground agent, and sends
Web Push when attention is needed. Phone access is shell access on the host. Prebuilt binaries
omit the `remote` feature; follow the [remote setup guide](docs/remote.md) and read the
[threat model](docs/security.md#remote-api).

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) to contribute. Report suspected vulnerabilities privately
according to the [security policy](SECURITY.md).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
