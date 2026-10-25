# Role workflows

## Design and implementation

`niles spawn --role` accepts `worker`, `design`, `reviewer`, `security`, and `research`.
Workers can spawn directly without either design flag. `--design <id>` uses a live designer's
report as the design record; name the record author, whose latest status must be `done:`.
`--mechanical <reason>` optionally logs mechanical work to `decisions.log` in the latest lead
session. The flags conflict. Reviewers and security agents may use `--design`; only workers
accept `--mechanical`. Design and research accept neither flag.

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

The role's primary instructions reach every worker and lead through the family's system-prompt
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
