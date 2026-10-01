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

From 1.0, follow semantic versioning. Bump the major version for any breaking change, including
removing or renaming CLI commands or flags, changing `--json` output in a way that breaks
consumers, or changing `niles wait`'s exit codes. Bump the minor version when adding a command,
flag, manifest key, or `--json` field. All other changes bump the patch version. On-disk state,
including the manifest, is not a compatibility surface; document how to update manifests in the
release notes when their format changes.

1. Bump `version` in `Cargo.toml` through a normal PR.
2. After merge, tag `vX.Y.Z` on `main` and push the tag.
3. The release workflow builds the binaries and publishes the GitHub release.
4. Run `cargo publish`.

## Legal and security

By intentionally submitting a contribution for inclusion in this project, you agree to the
[contribution terms in the README](README.md#license).

For suspected vulnerabilities, do not open a public issue. Follow the
[security policy](SECURITY.md) and consult the [threat model](docs/security.md).
