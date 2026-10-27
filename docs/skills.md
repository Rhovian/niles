# Skill workflows

## Entry sessions

Each setup must explicitly select its entry skill with `entry:`. Niles has no embedded skill
bodies. This repository uses the skills in [`skills/`](../skills/); copy
any directories you want to use into `.niles/skills/` or `~/.niles/skills/`, or import
them from GitHub as described below. Its `lead`
coordinates workers and its `solo` edits directly, spawning workers only on request. Native sub-agents have no Niles status line, wake or usage rollup.
Select solo in the workspace manifest; its binding supplies the agent and tiers:

```yaml
entry: solo
solo:
  standard:
    primary: claude:sonnet:high
    secondaries: []
```

The setup examples do not bind `solo`; set `entry: solo` to select it. Entry bodies render
`{workspace}`, `{agent}`, `{dir}`, `{manifest}` and `{startup_context}` (the same startup context
for either skill); unknown placeholders stay literal.

## Named setups

Select a complete manifest for a workspace command with `--setup tdd` or `NILES_SETUP=tdd` (the flag wins).
`<workspace>/.niles/setups/tdd.yaml` overrides `~/.niles/setups/tdd.yaml`;
`--setup global:tdd` or `--setup workspace:tdd` pins one root. Names contain ASCII letters,
digits, `-` and `_`. Without selection, the per-workspace global-state manifest remains active.
Files are never merged. Picker saves write back to the selected file, including global files.
Launches print name/origin/path and record name/origin in session and worker metadata.
The entry environment carries the qualified selection, so its `niles spawn` calls inherit it
and a new workspace shadow cannot redirect them. Multi-project services use each project's
default manifest; selection is not forwarded to home, panels or explorer child processes.
Switch by starting a new lead; live sessions are unchanged. Workspace setup symlinks are rejected; global links are allowed
([security](security.md#named-setups)).

## Design and implementation

`niles spawn --skill <name>` requires a binding in the workspace manifest. The entry skill
(selected with the required `entry:`) cannot be spawned. Only the entry must be bound when
the manifest loads; other skills are optional. Each binding must resolve to a skill directory containing exactly one of `SKILL.md` or `SKILL.ref`. Lookup is by exact name: `<workspace>/.niles/skills/<name>/SKILL.md` overrides
`~/.niles/skills/<name>/SKILL.md`. Missing skills fail; there is no fallback. Unbound directories are never scanned. `niles skills` lists bound skills,
entry first, with sources, paths, descriptions and tier primaries.

A `SKILL.md` starts with YAML frontmatter between `---` lines: `name` must equal the directory
name and `description` must be nonempty and at most 1,024 characters. Other frontmatter keys
are allowed. Disk documents are limited to 131,072 bytes including frontmatter.
The Markdown body follows unchanged. Workers receive the primary body, then `always`,
active `on_from` and `--with` bodies, then `skills/skill_common.md`; entry sessions retain
their template rendering. Primary text must be nonempty, contain no NUL and fit 65,536 bytes
after assembly/rendering. Codex's encoded argument
must also fit 131,071 bytes. Invalid content fails before worker/session creation. Manifest load
checks existence only; launch reads content. Workspace skill directories/files must not be
symlinks; global skill symlinks are allowed. See [the threat model](security.md#skills-as-an-instruction-source).

`--from <id>` passes a live worker's report path as `from_report:`; its latest status must be
`done:`. Any skill can build on a finished report. `--mechanical <reason>` optionally logs
mechanical work in the latest lead session. It accepts any skill and requires a nonempty
reason without control characters. The flags conflict.

The lead runs design when the operator asks for it. Otherwise, it judges from what the operator
has said, not from the issue: when that settles the way forward, it sends the work to a worker
without asking. It asks “Do you want a design?” only when the approach is genuinely open.
A trivial change, such as a line or a wording fix, never gets the question.

Design uses one designer by default; a second runs only on operator request and comes from a
different family in the manifest's selected `design` tier. Designers get one blind round. With two
designers, there is one critique; all contested points then go to the operator, with no further
critique rounds. The named record author incorporates the operator's rulings. With one designer,
its report becomes the record once operator questions and scope corrections are resolved.
The lead may send back scope creep against the issue's wording but does not rule on disputes;
a disputed scope correction goes to the operator.

## Primary instructions

The skill's primary instructions reach every worker and lead through the family's system-prompt
argument: Claude and pi use `--append-system-prompt`, Codex uses `--config developer_instructions`,
and Hermes uses `HERMES_EPHEMERAL_SYSTEM_PROMPT`. The opening user turn carries the task and paths.
Codex and Hermes leads now receive their standing brief at launch; its startup context is frozen,
as it already was for Claude and pi. `lead.md` remains the written brief.

| Family (tested version) | Primary instruction lifetime (source-backed) |
|---|---|
| Claude 2.1.296 | System prompt resent per request; survives compaction (documented). |
| Codex 0.162.1 | Developer message rebuilt from launch config after local and remote compaction. |
| Hermes 0.21.3 | Request-time overlay outside compacted history in researched 0.21.7; 0.21.3 unverified. |
| pi 1.2.0 | Effective system message carried through compaction (source and offline probe). |

These are source-backed claims, not live compaction observations. `niles compat` tests first delivery
only: a secret present solely in primary instructions must be reported using `niles agent status`.
Text is supplied at launch, not restored from saved history; Niles does not resume workers.
Retention across `/clear` is documented or inferred for Claude and Codex and is not tested.

Bindings can also name discovery and trigger skills:

```yaml
worker:
  standard: {primary: claude:sonnet:high, secondaries: []}
  skills: [style-guide]
  always: [coding-rules]
  on_from: [follow-design]
  on_stage: [self-review]
```

`skills:` supplies the menu; repeatable `--menu <name>` appends unbound names,
deduplicated in first-occurrence order, up to 16. Menus require disk `SKILL.md`.
Descriptions must be nonempty and at most 1,024 characters; missing the whole word
“when” warns after a successful spawn. Descriptions have control and format
characters stripped and are clamped to 200 characters for display. The opening
turn lists names, descriptions and sources.
`niles agent skills` refreshes the menu; `niles agent skill <name>` prints a listed
body without frontmatter, with its source. These use `NILES_AGENT_DIR`'s snapshot,
resolving names at call time. Fetched bodies last until compaction; rerun to refresh.
Copy a skill directory to share it.

`always` bodies follow the primary in the system prompt, then `on_from` bodies
when spawned with `--from`, then repeatable `--with <name>` injections. They share the 65,536-byte assembled prompt cap, including Niles notes;
they last as long as the system prompt. Injected names are deduplicated in
primary → always → active on_from → --with order; the first occurrence wins.
Spawn prints every list's provenance.
`on_stage` names are resolved and snapshotted at spawn, capped at 32,768 body bytes
total; staging delivers them. There is no `on_send` or `done` trigger. Editing a
binding does not change a running worker's lists. Discovery serves only its menu,
trigger and explicit `--with` names.

## Imports

Skills are ordinary shareable directories. Pin one from any GitHub repository:

```sh
niles skills import anthropics/skills/skills/pdf@main
niles skills import Rhovian/niles/skills/lead@<full-sha> --global
```

A ref is `[github.com/]<owner>/<repo>[/<path>][@<rev>]`. The revision can be a
branch, tag or full 40-hex SHA; omission selects remote HEAD. Other hosts,
empty/`.`/`..` path segments and invalid skill names fail. The last path segment
(or repository name for a root-level skill) is the name, and must match frontmatter.
Same-named branches and tags are ambiguous and fail; annotated tags use their peeled commit.

Import validates the document and writes the canonical full-SHA ref as one line in
`.niles/skills/<name>/SKILL.ref`, or `~/.niles/skills/<name>/SKILL.ref` with `--global`.
An existing local `SKILL.md` blocks import; both files in one directory are an error.
Reimport to update deliberately: it prints `old → new` SHAs. Remove the directory to
uninstall. Floating refs at rest fail. An invalid existing pointer is left unchanged;
delete it before reimporting rather than implicitly repairing invalid stored state.
Manifest loads parse the pin offline; launches fetch on a cache miss. Workspace pointers override global pointers or local documents
by the same name. Pinned skills can be primary, entry, menu, or injected through
`always`, `on_from`, `on_stage` and `--with`, exactly like local documents.

For one worker, use an inline ref in its menu:

```sh
niles spawn --skill worker task-id --menu anthropics/skills/skills/pdf@main "task"
```

This resolves once and snapshots the canonical pin in `WorkerMeta.imports`. It installs
no skill pointer. Discovery serves that pin ahead of the lookup chain throughout the
worker's lifetime, including after a branch moves or the cache is removed. Inline refs
are menu-only: `--with`, manifest lists and triggers accept names. A ref's name cannot
appear anywhere else in the spawn (including another inline ref), and counts toward
the 16-skill cap. The lead uses inline refs only when the operator names them.

Provenance shows `import github.com/owner/repo/path@<short-sha> <cache>/…/SKILL.md`.
The complete directory, including supporting scripts and reference files, is cached at
`~/.niles/cache/skills/github.com/<owner>/<repo>/<sha>/<path>/@skill/`. The reserved
`@skill` leaf gives root, ancestor and nested directories distinct cache keys; refs
cannot contain `@` in a path. Cache entries are complete directories installed by
atomic rename, with no readiness markers or recursive merging. Deleting an entire
entry is safe and triggers refetch. An incomplete populated entry must be removed
before retrying. Cache contents are immutable at a commit.
Git uses the operator's credentials for private repositories, with terminal prompting
disabled. Supporting files remain readable; Niles does not run imported scripts.
Git metadata is removed before caching. Each fetched skill directory is capped at
8 MiB of checked-out content; this is a disk-content limit after checkout, not a
network-transfer limit. A root-level skill includes the repository's content, so
that content counts toward its cap. `SKILL.md` keeps the usual document and prompt
limits. `compatibility` frontmatter is unchecked.
There is no search, auto-update, namespace, or separate pack root.

## Staging

`on_stage: [reviewer]` enables post-work skills. `gates: {rounds: 2}` sets the stage
limit (default 1, range 1–10) and enables staging even without post-work skills.
Spawn snapshots these settings and adds a standing stage instruction.
Write a nonempty report, run `niles agent status stage "summary"`, apply the printed
skills, then report `done` without lead approval. Bodies are capped at 32,768 bytes
at spawn and stage and printed on every round. No skills are injected at done;
`blocked`, `failed`, `needs-decision` and `working` remain ungated.
Stage wakes nobody and does not answer check-ins. Stage and configured done share
`stage.lock`; `gates.json` records completed rounds before the non-waking status.
Done starts a fresh round allowance for the next task. Stage/done require valid
worker metadata; unconfigured done creates no stage lock. `gates.post` adds sequential
`run:` argv gates; failures require another stage while rounds remain, or are annotated
on done at the round limit.
