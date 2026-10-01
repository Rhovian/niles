# Niles

Rust CLI that coordinates coding agents from different model families in tmux. Single crate,
Unix only. Discover modules and commands from the source and `niles --help` rather than a list
here.

Machine-specific or private notes go in `AGENTS.local.md` (gitignored); read it if present.

## Git

- Branch for new work, named `<issue>-<slug>`. Don't commit to `main` unless told to.
- Commit and push only when explicitly asked. Finishing a change is not a cue to commit it.

## Checks

Run the checks in [CONTRIBUTING.md](CONTRIBUTING.md#checks) before reporting a change done, and
say what they printed. Never mark done without that proof.

## Standards

- Simple, idiomatic, first-principles. Find root causes; no shortcuts, temporary fixes, or
  "just in case" code.
- Minimal diffs: touch only what the change needs.
- APIs, internal ones included, are self-documenting and hard to misuse.
- Names and types carry meaning. Comment only what the code can't say: why, invariants,
  non-obvious tradeoffs. No comments that restate the code.
- For a non-trivial change, ask whether there is a more elegant way before presenting it. Don't
  over-engineer simple fixes.

## Fail loudly

- No silent defaults on failure: propagate with `?` or fail fast.
- A default is a decision. Make it at the parse boundary or as a named const, never inline at a
  use site. A site that truly owns the decision takes
  `#[expect(clippy::disallowed_methods, reason = "…")]`.
- No catch-and-continue; invalid state stops.
- No branches for impossible states; encode impossibility in types.

## Modularity

- Source files stay under ~500 lines; split by responsibility, one module per job.
- DRY by the rule of three; don't abstract at two occurrences.
- Never copy-paste-tweak; extract a function or generic.
- No pass-through functions unless they change visibility, satisfy a trait, or cross a module
  boundary under a name the caller's domain needs.

## Correctness

- Make invalid states unrepresentable with enums and newtypes. A newtype must carry weight: an
  invariant enforced at construction, a distinction between same-typed values, or impls the inner
  type can't have.
- Parse, don't validate: type raw input (manifests, status lines, tmux output) once at the
  boundary.
- `debug_assert!` invariants the types can't express.

## Lints

Mechanically checkable rules live in `[lints.clippy]` in `Cargo.toml` and in `clippy.toml`, not in
prose. The gate lints all targets. Suppress a lint with `#[expect(…, reason = "…")]`, not
`#[allow]`. The one exception is test code, which allows `unwrap_used` and `expect_used` at the
crate root; `tests/common` may use `#[allow(dead_code)]`, because each test binary uses a different
subset of it.

## Invariants

- Role prompts live in `src/templates/` and are compiled in with `include_str!`. They are the
  product's behavior, so edit them with the same care as code.
- Agent families and their launch mechanics are defined in `src/agents/families.rs`; models and
  effort levels in `src/agents/roster.yaml`.
- `niles wait` reserves stdout for wake lines; diagnostics go to stderr. Its exit codes (0, 10, 22)
  are a contract the lead relies on.
- Changes that move a trust boundary (agent execution, workspace state, pane text) update the
  [threat model](docs/security.md).
