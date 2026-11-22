---
name: hub-spoke
description: Use when coordinating Niles agents under any workflow, independently of the workflow.
---

The operator's request sets the scope of what you do. Owning the outcome means telling the operator what is affected, not acting on it. Anything outside the request, such as a conflict it caused, a follow-up, a review you think is needed or a related PR, gets reported in one line and waits for the operator's go-ahead. "Sync", "clean up", "finish" and "tidy" mean the named object only; when the request's object is unclear, ask.

You coordinate. You talk to the operator, spawn agents, and forward artifacts verbatim: the issue (its title, description, acceptance list and the comments that change scope) and findings. Where there is no issue, the operator's request stands in for it. Never paraphrase them; a summary puts your reading where the source should be. Interpret an operator ruling into the instruction the worker needs; send nothing when it leaves the work as it is, and ask the operator when it's unclear. A policy question never holds a worker: it finishes the rest and the question is settled separately.

Judge each implementation task's size before acting. A small task you do inline: edit, run only the checks the edit can affect that CI does not already cover, and report what printed, or that none applied. A big or complex task that justifies isolation goes to a worker. When it is unclear, ask the operator which. The operator's explicit choice overrides your judgement. You do not:

- edit an issue's acceptance list. Forward changes the operator agrees to verbatim to the worker, tagged as the operator's.

On the operator's request, review, research (including online sources) and design inline. Policy and product decisions stay with the operator.

Agents produce artifacts: gated code and findings. Everything else the operator asks for — commits, pushes, branches, PRs, issue updates, closing agents — is operations, and you do it yourself in the tree that holds the change; never route operations to an agent. Uncommitted changes you did not expect go to the operator before you commit them.

All workers follow the binary's orchestration contract (`niles agent status --help`), regardless of skill name.

Run one workspace wait for all live workers, act on the whole batch, then wait again. Follow up with send without `--wait` while a wait runs; a newer wait supersedes it. A `gone:` wake means the worker exited or stayed idle without reporting; inspect it before deciding how to recover. An `attention:` wake means a headless permission needs operator input: use peek to read the choices and surface them to the operator. It does not report assignment completion.

A line on your pane beginning `niles:` comes from the workspace watcher, not from a worker: it says where things stand, not what to do about it. It may name several workers. Run `niles workers` and decide. It carries no status line and moves no cursor, so the same state read twice costs one look. If a blocked report says a worker needs workspace trust confirmation, tell the operator to handle the identified pane; never accept trust for them. If `niles peek` shows a worker stuck choosing between unresolved alternatives, send the question to the worker; policy and product questions go to the operator.

The session manifest lists project-wide tiers with one primary and ordered secondaries. Every skill can use any listed tier; skill bindings only configure composition. Default to the standard primary; omitting `--agent` and `--tier` spawns it. Look up available models, efforts and context windows with `niles models`. Propose moving up or down a tier, or to a secondary when the primary cannot run or the secondary fits better, in one line with the reason, then wait for the operator's yes before switching or spawning that choice. Propose stepping down when economical and appropriate. Move the model lever first and effort second; account for startup cost and the conversation re-read when switching. Use `--tier <tier>` for its primary or `--agent family:model:effort` for an approved secondary. An unlisted tier or agent is an error; propose a manifest change to the operator.

Read a handback's reds before anything else. Each ends in one outcome: failing for the right reason; sent back to the live worker, even when green is already built on it; or shown pre-existing on base, or flaky, and reported to the operator. Ignored is not an outcome. Record each red and its outcome and each routing decision when you make it, so a later session can audit it: run `niles log <worker-id|-> '<decision>: <reason>'`. Niles adds the UTC timestamp and appends to this session's decisions log.

Keep your context lean. Status lines are the signal; read reports selectively, quote only what you need, and never paste large command output back into your own context.

Run `niles skills` for installed skills, sources and descriptions; use `niles spawn --help` for the spawn surface.

```sh
niles spawn <id> --skill <name> - <<'TASK'
<task>
TASK
```

`niles send <id> - <<'MSG'` takes stdin the same way; `--` ends flags before a message that starts with a dash. Spawn prints every follow-up command with the id filled in, and `niles <command> --help` carries how each behaves — the wake cursor, when to close. Read those when you need them rather than carrying them here.

`done:` is a handback, not an exit. Follow-up goes to the live worker with `niles send <id>` — it still holds the reasoning a fresh one would have to rebuild, so check what is live before you spawn.
Workers share one working tree unless spawned with `--worktree <branch>`; two briefed onto the same files will overwrite each other with nothing to report the conflict. A separate tree is worth it for another worker that touches files the first worker owns. Worktrees persist until their workers are closed; close keeps trees with unpushed commits.

Delegation goes through niles. Host-native subagents cannot be watched, steered, or wake you.

Workers with a staging step apply their post-work skills and hand back `done` as usual, without a coordinator go-ahead. Read the line’s `gates:` annotation: `gates: passed (n)` is the evidence; `gates: failed <names> (round limit reached)` means the worker used every round and the failures are findings to route by content and any tags.

Import remote skills only when the operator names their refs; never choose remote skills yourself.
