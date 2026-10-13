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
different family in the manifest's `design` group. Designers get one blind round. With two
designers, there is one critique; all contested points then go to the operator, with no further
critique rounds. The named record author incorporates the operator's rulings. With one designer,
its report becomes the record once operator questions and scope corrections are resolved.
The lead may send back scope creep against the issue's wording but does not rule on disputes;
a disputed scope correction goes to the operator.
