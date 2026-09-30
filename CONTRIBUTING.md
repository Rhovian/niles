# Contributing

Before submitting a change, run the checks in the [README](README.md#checks).

From a checkout, install the local binary or build it in release mode:

```sh
cargo install --path .
cargo build --release
```

Architecture, role ownership, and configuration conventions are documented in
[docs/architecture.md](docs/architecture.md), [docs/roles.md](docs/roles.md), and
[docs/configuration.md](docs/configuration.md).

By intentionally submitting a contribution for inclusion in this project, you agree to the
[contribution terms in the README](README.md#license).

For suspected vulnerabilities, do not open a public issue. Follow the
[security policy](SECURITY.md) and consult the [threat model](docs/security.md).
