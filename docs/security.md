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

The operator's `~/.niles/config.yaml` controls the theme and optional tmux bindings; those bindings run only the shell-quoted Niles executable.

## Boundaries

### Maintainer CLI updates

`scripts/cli-versions.sh` reads installed CLI versions and GitHub release metadata. Its explicit
`--update` mode also changes host installations without confirmation. It preflights all four
CLIs before invoking npm or Hermes's native updater, identifies npm ownership by the resolved
executable's location in npm's global tree, and admits Hermes Git updates only for the official
repository. Unsupported ownership fails rather than selecting an installer. `--plan` performs
the same lookups without running updates. Release versions are parsed as numeric stable versions
and commands use argument arrays; feed and CLI output are never evaluated as shell code.
The operator trusts the installed tools and upstream release/package publishers, including npm
install scripts and Hermes's dependency preparation and service restarts. Updates across CLIs
are sequential and are not a transaction; a later failure does not undo earlier updates.

`niles compat` launches built-in CLIs with approval-bypass flags and operator credentials in a
fresh temp directory, using a fixed prompt with no outside input. It tests shipped profiles,
ignoring workspace overrides; the directory is evidence, not a sandbox. Release notes and compare
text are upstream-controlled inputs to an agent that may run commands; the maintainer procedure
treats them as data, never executable instructions. Updates require operator approval of the plan.

### Cloned repository manifest and configuration

**Attacker.** A malicious repository author who can choose committed repository content and induce
an operator to launch Niles from the clone.

**Attacker-controlled input.** Niles reads the first present project configuration file,
`niles.yaml` or `.niles.yaml`. An agent entry can replace its executable and argument list. The
`.niles/manifest.yaml` file selects the agent for each role (lead, worker, reviewer, security,
design, and research), and the `when` text of its role groups can influence which listed agent the
lead picks. See
[`src/config/spec.rs`](../src/config/spec.rs),
[`src/workspace_manifest/types.rs`](../src/workspace_manifest/types.rs), and
[`src/templates/lead_brief.md`](../src/templates/lead_brief.md).

**Existing mitigations.** The manifest's listed agents bound what spawn will launch. Deserializing YAML does not itself execute shell commands. Unknown
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
`niles doctor` also executes repository-chosen agent binaries with `--version`; the `niles-setup`
skill has the agent show a project `agents:` block to the user and get approval before running it.

### Local `.niles` state, symlinks, and locks

**Attacker.** A malicious repository author who supplies tracked `.niles` paths or symlinks before
the operator launches Niles. Tampering after launch by another process with the operator's account
is outside the isolation guarantees described here.

**Attacker-controlled input.** Repository content may shape paths already present under `.niles`.
Niles subsequently creates and reads manifests, session artifacts, worker directories, metadata,
briefs, launch scripts, reports, status logs, and cursors in that tree. See
[`src/store/paths.rs`](../src/store/paths.rs) and [`src/store/worker.rs`](../src/store/worker.rs).

**Existing mitigations.** Worker IDs and task labels accept only ASCII letters, digits, hyphens, and
underscores (and reserve `archive`). The final open of `status.log` for `niles wait`, Niles's own
file appends (through `append_line` or the trust watcher's held descriptor), spawn's log creation,
and the opens of `status.cursor`, `status.waiter`, `status.waiting` and `checkin.lock` use
`O_NOFOLLOW`. An exclusive advisory `flock` on `status.cursor` covers the cursor read, status scan,
and cursor advance, serializing a wait against `send`'s cursor skip. A wait holds an exclusive
`flock` on `status.waiter` for its whole life, so a second wait on the same worker is refused, and a
shared `flock` on `status.waiting`, which the watcher probes to leave the lines that wait will
deliver unreported. Every write or removal of a worker's check-in holds an exclusive `flock` on
`checkin.lock`, so the watcher's re-read and the action that follows cannot interleave with a `send`
arming a fresh one. See [`src/worker/validation.rs`](../src/worker/validation.rs) and
[`src/wait.rs`](../src/wait.rs).

`niles prune` previews deletions unless `--apply` is supplied. It removes registry links whose
targets are not directories, and dated archive and session directories, always retaining the
newest session. Archive and session entries are checked with `symlink_metadata`, so symlinks and
files are left alone. Unparseable names are also retained. See
[`src/projects/prune.rs`](../src/projects/prune.rs).

**Known gaps.** `O_NOFOLLOW` protects only those final path components; parent directories and
other filesystem operations are not protected globally against symlink traversal or replacement.
The worker launch script's shell append (`echo closed: ... >> "$STATUS"`) does not use
`O_NOFOLLOW`.
The advisory locks authenticate neither writers nor status content, and any process with the
operator's permissions can hold `status.waiter`'s lock and so refuse every wait on that worker, or
hold `status.waiting`'s and so silence the watcher's report nudges for it.
Other `.niles` state is not universally locked. Niles does not defend its state from a process that
already has the operator's filesystem permissions.

Theme config text never reaches tmux formats. The theme name selects a palette; only typed RGB
colors and styling modifiers are rendered into tmux formats.

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

Agent-written session records decide worker states and due check-ins; same-user processes can
forge them. A transcript Niles cannot parse is no record: after the initial check-in interval the
worker is Unknown and gets `blocked: session state unknown ...`; a read error or uncertain pane
liveness nudges and re-arms instead. A structured terminal failure in a Codex, Claude or pi record
becomes `blocked: provider refused (<label>): <first line>`, never from pane text: the message is
stripped of controls and capped at 1,024 characters, the label at 256; nudges carry only the worker
ID and status kind. Appends use `O_NOFOLLOW` and recheck the check-in under its lock and the log
length; worker echo does not take that lock, so a racing report can interleave (accepted, #348).

`niles panel config` no longer prints raw file bytes: parsed role cells, model names, and load errors
are clamped and stripped of control characters before terminal rendering. The shared roles picker
also clamps repository-authored family names and role values. It probes configured executables
with the same PATH lookup as doctor; it does not execute them.
When a manifest cannot deserialize, the interactive roles picker displays a clamped, control-stripped
parse error and starts from built-in defaults. Only an explicit save replaces the invalid manifest;
quitting leaves it unchanged, and non-interactive launches still fail.

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
With `--worktree`, Niles uses git to create a worktree under `~/.niles/trees` and can
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

Niles reads Claude, Codex, Hermes, and pi session stores to report agent token usage. It uses
read-only access to those stores and links sessions through launch identifiers or a Codex brief.
For Hermes, it opens the local `state.db` read-only to read session usage and state.
For pi, Niles chooses transcript directories under its own `.niles` worker and lead session
state directories using `--session-dir`, and reads the newest JSONL transcript there.

**Known gaps.** Built-in worker profiles explicitly launch Codex with
`--dangerously-bypass-approvals-and-sandbox`, Claude with `--dangerously-skip-permissions`, and
Hermes `chat` with `--yolo`. Pi runs with no permission prompts in either launch path, like
Hermes with `--yolo`. Niles launches pi with `PI_OFFLINE=1`, which disables its startup update
check and install telemetry only. Foreground defaults differ, and project configuration can replace
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
