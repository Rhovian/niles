---
name: niles
description: Use when the user asks to install Niles, set it up for a repository, or get started with Niles from their coding agent. Installs and configures Niles, then hands off to its entry agent. Not for implementing Niles features or coordinating existing workers.
---
Get the user to a running Niles entry agent in their repository.

1. Inspect the OS, PATH, project directory, and available agent CLIs (`claude`, `codex`, `pi`).
   Niles requires Unix, tmux, git for skill fetching, and an authenticated agent CLI. Reuse the
   current agent's CLI when possible. Confirm the target repository from context; ask if ambiguous.

2. Before changing anything, state the concrete plan: installation method and missing
   prerequisites, entry skill, agent, tiers and starter skills; fetching missing skills from
   github.com/Rhovian/niles at latest Git HEAD into `~/.niles/skills` with pinned commit SHAs;
   writing project state under `~/.niles/state`; registering the project; and starting detached
   tmux. Explain that workers bypass approval prompts and Niles provides no sandbox; the entry
   agent keeps its harness's normal prompts. Follow the user's existing permissions for the plan.
   Install missing prerequisites through the machine's package manager. If authentication requires
   the user, stop with the exact remaining login step.

3. If Niles is missing, install with the available package manager:
   `brew tap Rhovian/niles && brew install niles` on Homebrew, or `cargo install niles` with
   Rust 1.89+. Otherwise download the official shell installer from
   `https://github.com/Rhovian/niles/releases/latest/download/niles-installer.sh`, inspect it,
   and run it with `sh`. Resolve its reported binary location into PATH for subsequent commands.
   Run `niles setup --help`. If this installation lacks setup flags, update it with its package
   manager or `niles update`; if the release still lacks them, report that blocker.

4. Use `niles setup --help` for setup flags and `niles models` for available agents. Choose
   `solo` for direct work or `lead` for coordinating workers, and use only authenticated installed
   families. Select starter skills only as needed for the user's workflow. For a custom entry,
   import its supplied reference with `niles skills import --global <reference>` and use its name.

5. Run setup from the target repository, using a unique project name with ASCII letters,
   digits, underscores or hyphens. For example, with an authenticated Claude CLI:

   ```sh
   niles setup --entry-skill solo --entry-agent claude:sonnet:high \
     --tiers '{standard: {primary: "claude:sonnet:high", secondaries: []}}' \
     --start my-project
   ```

   Add `--starter-skills worker,reviewer` for selected optional skills. This writes once and
   starts without a wizard or tier picker. Existing skills are kept; missing named skills are
   fetched from the official repository. If a manifest exists, don't delete it or re-register:
   run `niles doctor` and hand the user `niles` to start the project from home.
   Clear an inherited `NILES_SETUP` when configuring the default project; setup rejects named
   setup selection.

6. Run `niles doctor` and check the entry pane with
   `tmux list-panes -t '=my-project:=niles' -F '#{pane_dead}'` and
   `tmux capture-pane -p -t '=my-project:=niles'`. Substitute the chosen project name.
   Resolve startup failures before claiming success. A login or repository trust prompt needs
   the user's attention; don't accept it for them. Hand off with the project name, chosen
   entry agent, and `niles` to open the home view (or `tmux attach -t '=my-project'` from an
   interactive terminal).
