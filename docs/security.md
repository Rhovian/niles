# Niles Threat Model

This document describes Niles's current security boundaries. It is not a claim that running an
agent on an untrusted repository is safe, and it is not a broad audit of Niles or the agent CLIs it
launches.

Report suspected vulnerabilities according to the [security policy](../SECURITY.md).

## Trust model

The important repository attack begins when a malicious repository author convinces an operator
to clone the repository and launch Niles or one of its agents there. Repository files are attacker
input at that point. That is different from an adversary who already controls the host, the
workspace, or another process running as the operator: such an adversary already has authority
outside the boundary Niles can provide.

Niles keeps only operator-added projects as symlinks in `~/.niles/projects/`. Opening a
registered project creates its named tmux session and starts the lead in that path; workers run
in the same session. Niles writes orchestration state beneath the workspace's `.niles` directory.
It never accepts an agent CLI workspace-trust prompt. Niles does not create an isolation boundary
between the operator, agents, and workspace. Agent permissions ultimately depend on the selected
CLI, its launch flags, and operating-system controls.

## Boundaries

### Cloned repository manifest and configuration

**Attacker.** A malicious repository author who can choose committed repository content and induce
an operator to launch Niles from the clone.

**Attacker-controlled input.** Niles reads the first present project configuration file,
`niles.yaml` or `.niles.yaml`. An agent entry can replace its executable and argument list. The
`.niles/manifest.yaml` file selects the agent for each role (lead, worker, reviewer, and
security), and its `worker_planning` strings can influence lead instructions for implementation
assignments. See
[`src/config/spec.rs`](../src/config/spec.rs),
[`src/workspace_manifest/types.rs`](../src/workspace_manifest/types.rs), and
[`src/templates/lead_brief.md`](../src/templates/lead_brief.md).

**Existing mitigations.** Deserializing YAML does not itself execute shell commands. Unknown
manifest fields are rejected, and built-in agent families reject unsupported model names. Worker
launch scripts quote the selected binary and arguments, while foreground launches pass them as
separate `Command` arguments; these measures prevent incidental shell parsing. See
[`src/agent_window.rs`](../src/agent_window.rs),
[`src/session/foreground.rs`](../src/session/foreground.rs), and
[`src/agents/mod.rs`](../src/agents/mod.rs).

**Known gaps.** Niles has no repository-trust gate of its own. Once the operator launches it, Niles
may intentionally execute the configured binary with the configured arguments, so quoting is not
a trust mechanism for a deliberately selected executable. Role bindings select what is launched,
and planning text is instruction input to the lead. Niles does not currently execute
repository-defined hooks; configuration parsing should not be interpreted as a hook facility.

### Local `.niles` state, symlinks, and locks

**Attacker.** A malicious repository author who supplies tracked `.niles` paths or symlinks before
the operator launches Niles. Tampering after launch by another process with the operator's account
is outside the isolation guarantees described here.

**Attacker-controlled input.** Repository content may shape paths already present under `.niles`.
Niles subsequently creates and reads manifests, session artifacts, worker directories, metadata,
briefs, launch scripts, reports, status logs, and cursors in that tree. See
[`src/store/paths.rs`](../src/store/paths.rs) and [`src/store/worker.rs`](../src/store/worker.rs).

**Existing mitigations.** Worker IDs and task labels accept only ASCII letters, digits, hyphens,
and underscores (and reserve `archive`). The final open of `status.log` for `niles wait`, Niles's
own file appends (through `append_line` or the trust watcher's held descriptor), spawn's log
creation, and the open of `status.cursor` use `O_NOFOLLOW`. An exclusive advisory `flock` on
`status.cursor` covers the cursor read, status scan, and cursor advance, serializing concurrent
waiters. See
[`src/worker/validation.rs`](../src/worker/validation.rs) and [`src/wait.rs`](../src/wait.rs).

`niles prune` previews deletions unless `--apply` is supplied. It removes registry links whose
targets are not directories, and dated archive and session directories, always retaining the
newest session. Archive and session entries are checked with `symlink_metadata`, so symlinks and
files are left alone. Unparseable names are also retained. See
[`src/projects/prune.rs`](../src/projects/prune.rs).

**Known gaps.** `O_NOFOLLOW` protects only those final path components; parent directories and
other filesystem operations are not protected globally against symlink traversal or replacement.
The worker launch script's shell append (`echo closed: ... >> "$STATUS"`) does not use
`O_NOFOLLOW`.
The cursor itself is the locked file—there is no separate persistent lock file—and the advisory
lock authenticates neither writers nor status content. Other `.niles` state is not universally
locked. Niles does not defend its state from a process that already has the operator's filesystem
permissions.

### Status lines, pane capture, and terminal or lead injection

**Attacker.** A malicious repository author whose content is repeated or acted on by an agent, or
another source able to influence agent output. A deliberately malicious same-user worker is a
non-goal, but its output would reach the same display and prompt paths.

**Attacker-controlled input.** Influenced text can enter worker status lines and reports, appear in
captured tmux panes, or be included in the repository text an agent reads. This creates both
terminal-control risks and semantic prompt-injection risks.

**Existing mitigations.** `niles wait` escapes characters for which Rust's `char::is_control` is
true and limits the escaped rendered line to 4,096 bytes plus the `... (truncated)` suffix. Worker
IDs and task labels have the restricted grammar described above. Watcher nudges identify only the
worker and wake kind; they do not copy status-line content into the lead pane. Text sent through
tmux uses literal-key mode. During a new worker's first 30 seconds, while its status log is still
empty, the watcher recognizes specific Claude and Codex workspace-trust prompts displaying the
exact workspace, appends a `blocked:` status, and never accepts the prompt or sends keys. See
[`src/wait.rs`](../src/wait.rs), [`src/watch/decide.rs`](../src/watch/decide.rs),
[`src/tmux.rs`](../src/tmux.rs), and [`src/watch/trust.rs`](../src/watch/trust.rs).
Before sending a lead nudge, the watcher captures the visible lead pane and queries its cursor.
It holds the nudge only when the cursor is visible on a typed Claude or Codex composer row, or an
indented continuation of one. An absent marker, hidden cursor, or failed query delivers; a
continuous hold ends after five minutes. The recognizer reads terminal output and cursor position
as state only and never submits captured text.

Agent-written session telemetry gates due worker check-ins. Agents can delay their own check-ins
by writing to their own logs.

**Known gaps.** The wait renderer is not a global sanitizer. `niles workers` prints stored status
strings, `niles report` prints a report body, and `niles peek` prints tmux capture without a shared
universal sanitizer. See [`src/worker/list.rs`](../src/worker/list.rs),
[`src/worker/report.rs`](../src/worker/report.rs), and
[`src/worker/pane.rs`](../src/worker/pane.rs). Escaping terminal control characters, excluding
status content from nudges, and using tmux literal-key mode do not establish semantic authorization:
ordinary text can still instruct or mislead an agent or operator. Startup trust-prompt detection is
bounded and recognizes only known exact-workspace Claude and Codex prompt shapes; it is not general
prompt or injection detection.
Composer recognition depends on the CLIs' current screen layout and styling. A keystroke between
capture and submit can still merge with a nudge.

### Agent CLI execution

With `--tree`, the worker agent runs in a directory chosen by the lead; Niles validates only that it
exists and is a directory.
With `--worktree`, Niles uses git to create a sibling directory outside the workspace and can
remove it on close. The branch name is checked by git before it becomes a path. Close removes only
registered worktrees under that workspace's managed tree root after checking live workers, status,
and remote reachability; it never deletes branches.

**Attacker.** A malicious repository author who can influence selected configuration, manifest
bindings, briefs, or workspace content and induce the operator to launch an agent.

**Attacker-controlled input.** The selected executable, its configured arguments, role selection,
planning guidance, briefs, and repository content all affect the process Niles launches and the
work it is asked to perform.

**Existing mitigations.** Foreground agents are launched with a process argument vector, and worker
script values are shell-quoted. Static built-in profiles validate their supported model and effort
values. These are launch-integrity measures, not containment. See
[`src/session/foreground.rs`](../src/session/foreground.rs),
[`src/agent_window.rs`](../src/agent_window.rs), and
[`src/agents/families.rs`](../src/agents/families.rs).

Niles reads Claude, Codex, and Hermes session stores to report agent token usage. It uses
read-only access to those stores and links sessions through launch identifiers or a Codex brief.
For Hermes, it opens the local `state.db` read-only to read session usage and state.

**Known gaps.** Built-in worker profiles explicitly launch Codex with
`--dangerously-bypass-approvals-and-sandbox`, Claude with `--dangerously-skip-permissions`, and
Hermes `chat` with `--yolo`. Foreground defaults differ, and project configuration can replace
default arguments for either launch path. Actual access therefore depends on the agent CLI, launch
flags, credentials, and OS permissions. Niles adds no sandbox or process isolation. Agents share
the workspace tree and inherited environment, and `.niles` artifacts, reports, pane captures, and
logs may contain secrets. Treat them as sensitive.

## Non-goals

This model does not promise protection from:

- hostile local users or shared-host and multi-user isolation failures;
- malicious workers already running as the same OS user; or
- sandboxing or otherwise containing the agent CLIs Niles launches.

These non-goals do not make a cloned repository trustworthy. They clarify that Niles coordinates
the operator's existing tools and permissions rather than reducing them.
