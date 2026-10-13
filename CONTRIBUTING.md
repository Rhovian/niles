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

Run `scripts/cli-versions.sh` with the GitHub CLI (`gh`) installed and authenticated to show
installed, tested, and latest stable agent CLI versions. Installed versions come from each CLI's
`--version`, tested versions from `src/agents/families.rs`, and latest versions from each project's
GitHub latest stable release, regardless of install method. The script checks all four projects
even when a CLI is absent. Exit codes are 0 when all releases are covered, 1 when a release is
newer than tested, and 2 when a release lookup, installed version, or tested version cannot be
read. A newer release is a cue to run compatibility tests before bumping the tested versions.

`scripts/cli-versions.sh --update` updates all four CLIs without confirmation. It identifies
global npm installs by resolving the executable into npm's actual global package directory,
and Hermes Git installs from its reported install method and official repository remote.
Unrecognized methods or missing CLIs fail loudly before any update runs. npm updates target
the GitHub latest stable version in the same global prefix; legacy pi installs migrate to
`@earendil-works/pi-coding-agent`. Hermes uses its native stable-channel updater. Update failures
stop with exit 2; the newer-than-tested exit 1 still applies after a successful update.
`scripts/cli-versions.sh --plan` shows detected methods and commands without updating. Updating
requires the detected installer tools; npm detection also uses the Unix `realpath` utility.

From 1.0, patch releases cover fixes and internal changes. Minor releases cover any user-visible
change, including adding, removing, or renaming commands or flags, changing `--json` output or
`niles wait`'s exit codes, and adding manifest keys. When a change breaks existing usage, such as
a removed or renamed flag or a manifest format change, the release notes say what to update.
Major releases are reserved for a fundamental change in what niles is; there is no fixed trigger.

1. Bump `version` in `Cargo.toml` through a normal PR, and update the tested agent CLI versions
   in `src/agents/families.rs` and the README from each CLI's `--version`.
2. After merge, tag `vX.Y.Z` on `main` and push the tag.
3. The release workflow builds the binaries, publishes the GitHub release, then publishes the crate
   to crates.io. It then adds the tested CLI versions from the README to the release notes.

## Legal and security

By intentionally submitting a contribution for inclusion in this project, you agree to the
[contribution terms in the README](README.md#license).

For suspected vulnerabilities, do not open a public issue. Follow the
[security policy](SECURITY.md) and consult the [threat model](docs/security.md).
