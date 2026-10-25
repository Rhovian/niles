---
name: niles-setup
description: Install Niles, write the global project manifest for this repository with the user, check it with niles doctor, and tell the user how to start it. Use when the user asks to set up, install, or configure Niles.
---

# Set up Niles

## 1. Prerequisites

Niles is Unix only. Work in the repository root the user named; "here" means the current directory.
Check `command -v tmux claude codex hermes pi`. You need tmux and at least one agent CLI,
installed and logged in. If tmux is missing, ask before installing it: `brew install tmux` on
macOS, or the system package manager on Linux. If no agent CLI is installed, ask which to install.

## 2. Install

Skip installation if `niles --version` succeeds. Otherwise choose any of these methods:

- Homebrew: `brew tap Rhovian/niles && brew install niles`.
- Shell installer: `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh | sh`.
- Cargo (requires Rust 1.89+): `cargo install niles`.

The shell installer puts `niles` in `${CARGO_HOME:-$HOME/.cargo}/bin`, which may not yet be on
this shell's PATH. Verify `niles --version` before continuing.

## 3. Write the manifest

Run `niles workers` from the project directory to migrate any legacy `.niles/` directory;
stop and triage an error before writing a manifest. Migration copies to staging, publishes the
complete copy, then removes the legacy source; stop running agents before upgrading because
copying cannot synchronize live writers. Set
`niles_project_dir="$HOME/.niles/state/$(pwd -P | tr '/' '-')"`.
This plain Claude-style key replaces slashes with hyphens and accepts separator/hyphen collisions.
If `"$niles_project_dir/manifest.yaml"` exists, keep it; edit only when the user requests it. Otherwise ask in
one message which installed CLI should lead (the agent they talk to) and, when both Claude Code
and Codex are present, whether to mix families for independent review. Use the mixed template
when they choose to mix; if Codex should lead, swap each lead tier's primary with its Codex
secondary. Otherwise use their chosen CLI's template.
Create the file exclusively: `mkdir -p "$niles_project_dir" && set -C && cat > "$niles_project_dir/manifest.yaml"` with the
chosen YAML as input. If the file appears before creation, stop and keep it.

Claude Code and Codex (built-in default; change `lead` to the user's choice):

```yaml
lead:
  standard:
    primary: claude:sonnet:high
    secondaries: [codex:gpt-6.1-sol:medium]
  frontier:
    primary: claude:opus:high
    secondaries: [codex:gpt-6-astra:high]
worker:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: [claude:sonnet:high]
  frontier:
    primary: codex:gpt-6-astra:high
    secondaries: [claude:opus:high]
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: [claude:sonnet:medium]
  mechanical:
    primary: codex:gpt-6-luna:low
    secondaries: [claude:haiku]
design:
  standard:
    primary: claude:sonnet:high
    secondaries: [codex:gpt-6.1-sol:medium]
  frontier:
    primary: claude:opus:high
    secondaries: [codex:gpt-6-astra:high]
reviewer:
  standard:
    primary: claude:sonnet:high
    secondaries: [codex:gpt-6.1-sol:medium]
  frontier:
    primary: claude:opus:high
    secondaries: [codex:gpt-6-astra:high]
  economy:
    primary: claude:sonnet:medium
    secondaries: [codex:gpt-6-luna:medium]
security:
  standard:
    primary: codex:gpt-6.1-sol:high
    secondaries: []
  economy:
    primary: hermes:z-ai/glm-5.3-flash:high
    secondaries: [pi:deepseek/deepseek-v4.1-flash:high]
  mechanical:
    primary: codex:gpt-6-luna:low
    secondaries: []
research:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: [claude:sonnet:high]
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: [claude:sonnet:medium]
```

Claude Code only:

```yaml
lead:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  frontier:
    primary: claude:opus:high
    secondaries: []
worker:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  frontier:
    primary: claude:opus:high
    secondaries: []
  economy:
    primary: claude:sonnet:medium
    secondaries: []
  mechanical:
    primary: claude:sonnet:low
    secondaries: []
design:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  frontier:
    primary: claude:opus:high
    secondaries: []
reviewer:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  frontier:
    primary: claude:opus:high
    secondaries: []
  economy:
    primary: claude:sonnet:medium
    secondaries: []
security:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  economy:
    primary: claude:sonnet:medium
    secondaries: []
  mechanical:
    primary: claude:sonnet:low
    secondaries: []
research:
  standard:
    primary: claude:sonnet:high
    secondaries: []
  economy:
    primary: claude:sonnet:medium
    secondaries: []
```

Codex only:

```yaml
lead:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  frontier:
    primary: codex:gpt-6-astra:high
    secondaries: []
worker:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  frontier:
    primary: codex:gpt-6-astra:high
    secondaries: []
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: []
  mechanical:
    primary: codex:gpt-6-luna:low
    secondaries: []
design:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  frontier:
    primary: codex:gpt-6-astra:high
    secondaries: []
reviewer:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  frontier:
    primary: codex:gpt-6-astra:high
    secondaries: []
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: []
security:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: []
  mechanical:
    primary: codex:gpt-6-luna:low
    secondaries: []
research:
  standard:
    primary: codex:gpt-6.1-sol:medium
    secondaries: []
  economy:
    primary: codex:gpt-6-luna:medium
    secondaries: []
```

For Hermes, pi, or specific models/efforts, run `niles models` and read
[Configuration](https://github.com/Rhovian/niles/blob/main/README.md#configuration).
Each role lists tiers with one exact `family:model:effort` primary and ordered secondaries.
All six roles and their standard tier are required; no-effort models omit the effort suffix. The only file you write is `"$niles_project_dir/manifest.yaml"`.

## 4. Check with niles doctor

Run `niles doctor` in the repository root, then triage its output:

- Non-zero exit: fix the manifest/config problem named in the error and rerun. Get permission
  before changing an existing manifest; have the user correct project configuration.
- `tmux: not found on PATH`: return to step 1.
- `agents: no workspace manifest`: return to step 3.
- `agent <x>: not found on PATH`: install that CLI or agree with the user to rebind its roles; rerun.
- A version older than `tested`: tell the user; this is not a blocker.

Doctor does not check logins. Ask the user to confirm each bound CLI is logged in.

## 5. Hand off to start

Do not run bare `niles`, `niles lead`, or any command that attaches or switches tmux yourself:
it takes over the user's terminal. Give the user the repository's absolute path and say:
"In a terminal, `cd <absolute-repo-path>` and run `niles`. If the project is already listed,
use the arrow keys to select it and press Enter. Otherwise press `r`, type
`<absolute-repo-path>` in the directory prompt and press Enter, then Enter to accept the name
(or enter an unused name if it is taken). Use the arrow keys to select the registered project
and press Enter. If registration says 'already registered as <name>', press Esc, select that
project with the arrow keys and press Enter. When the roles picker appears, press `c` to keep
the manifest." Never rely on the directory prompt's default: an existing home session may
have started in another directory.

Built-in worker defaults bypass approval prompts. Point the user to the
[threat model](https://github.com/Rhovian/niles/blob/main/docs/security.md) before they start.
