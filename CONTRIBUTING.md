# Contributing

## Checks

The gate is whatever [CI](.github/workflows/ci.yml) runs. Run the same commands locally before
submitting a change. Install `tmux` first; the integration tests start a private tmux server.

Keep `--no-fail-fast`: plain `cargo test` stops at the first failing binary, so a failure in one
test file hides every later one.

The lints deny `unwrap`, `expect`, wildcard match arms on enums, and silent defaults such as
`unwrap_or` and `Result::ok`. A default is a decision, so make it at the parse boundary or as a
named const. Where a site truly owns it, use `#[expect(…, reason = "…")]` and say why.

## Pull requests

- Branch from `main` as `<issue>-<slug>`, and link the issue with `Closes #N`.
- Title the PR as an imperative sentence; it becomes the squash-merge commit.
- Say what changed, why, and what the checks printed.

[AGENTS.md](AGENTS.md) holds the full coding standards that agents working on this repo follow.

## Releases

From 1.0, patch releases cover fixes and internal changes. Minor releases cover any user-visible
change, including adding, removing, or renaming commands or flags, changing `--json` output or
`niles wait`'s exit codes, and adding manifest keys. When a change breaks existing usage, such as
a removed or renamed flag or a manifest format change, the release notes say what to update.
Major releases are reserved for a fundamental change in what niles is; there is no fixed trigger.

1. Bump `version` in `Cargo.toml` through a normal PR, and update the README's tested agent CLI
   versions from each CLI's `--version`.
2. After merge, tag `vX.Y.Z` on `main` and push the tag.
3. The release workflow builds the binaries and publishes the GitHub release.
   Add the tested CLI versions to its notes.
4. Run `cargo publish`.

## Legal and security

By intentionally submitting a contribution for inclusion in this project, you agree to the
[contribution terms in the README](README.md#license).

For suspected vulnerabilities, do not open a public issue. Follow the
[security policy](SECURITY.md) and consult the [threat model](docs/security.md).
