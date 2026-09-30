# Contributing

Before submitting a change, run these checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --no-fail-fast
```

Keep `--no-fail-fast`: plain `cargo test` stops at the first failing binary, so a failure in one
test file hides every later one.

By intentionally submitting a contribution for inclusion in this project, you agree to the
[contribution terms in the README](README.md#license).

For suspected vulnerabilities, do not open a public issue. Follow the
[security policy](SECURITY.md) and consult the [threat model](docs/security.md).
