# Skill workflows

## Write a skill

1. Create `.niles/skills/<name>/SKILL.md` in the repository, or `~/.niles/skills/<name>/SKILL.md`
   for every project. A workspace skill overrides a global one with the same name. Names use
   ASCII letters, digits, `-` and `_`.
2. Start it with YAML frontmatter: `name` must match the directory, and `description` must be
   nonempty and should say when to use the skill. Other keys are allowed; the body follows.
3. Launch it with `niles spawn <id> --skill <name>` on a project tier. Add a
   [composition binding](configuration.md#manifest-keys) when needed; `niles skills` lists
   installed skills, entry first, with their sources and descriptions.

`niles skills` scans workspace and global skill directories, resolves workspace overrides,
and reads each installed skill for its source and description. Invalid discovered skills are
skipped with loud warnings on stderr; valid skills remain listed and the command succeeds.
Manifest and model configuration errors still fail. Launches resolve only their named skills
and fail when one is missing. A skill directory holds exactly one of `SKILL.md` or an import's
`SKILL.ref`. Workspace skill files and directories can't be symlinks; global ones can.

The skill's body reaches the agent as system instructions: `--append-system-prompt` for Claude
and pi, `--config developer_instructions` for Codex. The opening user turn carries the task and
paths. Niles supplies the text at launch and never resumes a worker, so it doesn't restore it
from saved history.

## Entry sessions

`entry:` names the skill you talk to. It needs no binding and can't be spawned. This repository's
[`skills/`](../skills/) has two: `lead` coordinates workers, and `solo` does the work itself,
spawning workers only when asked. Native sub-agents get no Niles status line, wake or usage
rollup.

```yaml
entry: solo
entry_agent: claude:sonnet:high
tiers:
  standard:
    primary: claude:sonnet:high
    secondaries: []
```

Entry skills can use `{workspace}`, `{agent}`, `{dir}`, `{manifest}` and `{startup_context}`;
unknown placeholders stay literal. Startup context is fixed for the session.

## Spawning

`niles spawn <id> --skill <name>` launches an installed skill on the project tier list. `--from <id>` passes a finished
worker's report path as `from_report:`; that worker's latest status must be `done:`.
`--mechanical <reason>` logs the spawn as mechanical work in the entry session's log and
conflicts with `--from`. How the shipped `lead` skill decides what to spawn is in its
[`SKILL.md`](../skills/lead/SKILL.md).

## Orchestration contract

Every spawned worker's opening brief directs it to `niles agent status --help`, which prints
the binary's orchestration contract without requiring a worker environment. It binds every
skill, including replacements, without an opt-in or a special skill name.

Submit outcomes with `niles agent status <kind> <detail>`, never by appending to `status.log`.
Unknown kinds, empty details and control characters are rejected before a status line is
appended. `done` also requires a nonempty `report.md` and completion of any configured staging
protocol. Correct a rejected submission and resubmit; a rejected command doesn't wake anyone.

`done`, `failed`, `blocked` and `needs-decision` wake the entry agent. `working` and `stage`
don't. `done` hands back the dispatch; the worker stays live for `niles send` until closed.
Niles owns `closed` and `gone`; `gone` signals exit or idle without a report.
`niles wait` reserves stdout for wake lines and stderr for diagnostics, and exits 0 for a wake
(including `gone`), 11 when superseded, or 22 on timeout.

Finding tags such as `code`, `design` and `policy`, role names, and routing between skills are
optional skill conventions. The binary doesn't interpret findings or choose their recipient.
The operator and coordinating skill decide those workflows.

## Composing skills

A binding can add skills around its primary one:

```yaml
worker:
  skills: [style-guide]
  always: [coding-rules]
  on_from: [follow-design]
  on_stage: [self-review]
```

- `skills` is a menu. The worker sees each name and description and fetches a body with
  `niles agent skill <name>`; `niles agent skills` lists the menu again. `--menu <name>` adds
  more. A description without the word "when" gets a warning.
- `always` bodies follow the primary in the system instructions, then `on_from` bodies when
  spawned with `--from`, then `--with <name>` bodies. A name appears once, at its first place.
- `on_stage` bodies are delivered at [staging](#staging).

Spawn prints where every skill came from. A running worker keeps the lists it started with.

## Staging

Staging runs post-work skills and command gates before a worker reports `done`. Enable it in a
binding with `on_stage: [reviewer]`, or with `gates: {rounds: 2}` alone; `rounds` limits stage
rounds (default 1, range 1–10). `gates.post` adds sequential `run:` argv gates.

The worker writes a nonempty report, runs `niles agent status stage "summary"`, applies the
printed skills, then reports `done` without approval from the entry agent. A failed gate requires
another stage while rounds remain; at the round limit, failures are annotated on `done`. Stage
wakes nobody and doesn't answer check-ins.

## Imports

Pin a skill from any GitHub repository:

```sh
niles skills import anthropics/skills/skills/pdf@main
niles skills import Rhovian/niles/skills/lead@<full-sha> --global
```

A ref is `[github.com/]<owner>/<repo>[/<path>][@<rev>]`, where the revision is a branch, tag or
full SHA, and remote HEAD when omitted. The last path segment, or the repository name, is the
skill's name and must match its frontmatter. Import writes the resolved SHA to
`.niles/skills/<name>/SKILL.ref`, or under `~/.niles/skills/` with `--global`, and the skill then
works like a local one. Reimport to update; it prints the old and new SHAs. Delete the directory
to uninstall.

For one worker only, pass a ref to `--menu`:

```sh
niles spawn --skill worker task-id --menu anthropics/skills/skills/pdf@main "task"
```

Private repositories use your Git credentials. Niles never runs an imported skill's scripts,
though agents may. Fetched skills are cached under `~/.niles/cache/skills/`; deleting a cache
entry fetches it again.

## Named setups

A named setup is a whole alternative manifest. Select it with `--setup tdd` or `NILES_SETUP=tdd`
(the flag wins). `.niles/setups/tdd.yaml` in the repository overrides `~/.niles/setups/tdd.yaml`;
`--setup global:tdd` or `--setup workspace:tdd` picks one. Setups are never merged, and the skill
picker saves to the selected file. To switch, start a new entry session; running sessions keep
theirs. Selected workspace setups run their commands with your permissions; see
[Named setups](security.md#named-setups) for execution details.

## Limits

| What | Limit |
| --- | --- |
| Description | 1,024 characters (shown clamped to 200) |
| `SKILL.md` file | 131,072 bytes, including frontmatter |
| Assembled system instructions | 65,536 bytes; Codex's encoded argument 131,071 bytes |
| Menu | 16 skills |
| `on_stage` bodies | 32,768 bytes in total |
| Imported directory | 8 MiB of checked-out content |
