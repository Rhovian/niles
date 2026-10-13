---
name: niles-setup
description: Install Niles, write .niles/manifest.yaml for this repository with the user, check it with niles doctor, and tell the user how to start it. Use when the user asks to set up, install, or configure Niles.
---

# Set up Niles

## 1. Prerequisites

Niles is Unix only. Work in the repository root the user named; “here” means the current directory.
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

If `.niles/manifest.yaml` exists, keep it; edit only when the user requests it. Otherwise ask in
one message which installed CLI should lead (the agent they talk to) and, when both Claude Code
and Codex are present, whether to mix families for independent review. Use the mixed template
when they choose to mix, setting `lead` to their choice; otherwise use their chosen CLI's template.
Create the file exclusively: `mkdir -p .niles && set -C && cat > .niles/manifest.yaml` with the
chosen YAML as input. If the file appears before creation, stop and keep it.

Claude Code and Codex (built-in default; change `lead` to the user's choice):

```yaml
lead: claude
worker: codex
reviewer: claude
security: claude
design:
  - models: [claude, codex]
research: codex:gpt-6-luna:medium
```

Claude Code only:

```yaml
lead: claude
worker: claude
reviewer: claude
security: claude
design: claude
research: claude
```

Codex only:

```yaml
lead: codex
worker: codex
reviewer: codex
security: codex
design: codex
research: codex
```

For Hermes, pi, or specific models/efforts, run `niles models` and read
[Configuration](https://github.com/Rhovian/niles/blob/main/README.md#configuration).
Bindings are `family[:model[:effort]]`; all six role keys are required. Your only repository
write is `.niles/manifest.yaml`.

## 4. Check with niles doctor

Before running doctor, inspect `niles.yaml` and `.niles.yaml`. If either has an `agents:` block
you did not write, show it to the user and get a yes first: doctor executes configured binaries
with `--version`. Run `niles doctor` in the repository root, then triage its output:

- Non-zero exit: fix the manifest/config problem named in the error and rerun. Get permission
  before changing an existing manifest; have the user correct project configuration.
- `tmux: not found on PATH`: return to step 1.
- `agents: no workspace manifest`: return to step 3.
- `agent <x>: not found on PATH`: install that CLI or agree with the user to rebind its roles; rerun.
- A version older than `tested`: tell the user; this is not a blocker.

Doctor does not check logins. Ask the user to confirm each bound CLI is logged in.

## 5. Hand off to start

Do not run bare `niles`, `niles lead`, or any command that attaches or switches tmux yourself:
it takes over the user's terminal. Tell the user: “In a terminal, `cd <repo>` and run `niles`.
Press `r`, Enter to accept this directory, Enter to accept the name, then Enter on the project
to open it. The roles picker shows the manifest; press `c` to keep it.”

Built-in worker defaults bypass approval prompts. Point the user to the
[threat model](https://github.com/Rhovian/niles/blob/main/docs/security.md) before they start.
