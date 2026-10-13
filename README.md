# Bodo

A free, open-source, AI-native digital audio workstation written entirely in Rust.

Bodo is at the very start: milestone M0, the foundations, is in progress. [`docs/STATUS.md`](docs/STATUS.md) tracks what exists.

- [`project.md`](project.md): the specification.
- [`AGENTS.md`](AGENTS.md): the rules for anyone, human or coding agent, who changes code here.
- [`docs/adr/`](docs/adr/): architecture decisions.

## Building

Install [rustup](https://rustup.rs); the toolchain is pinned in `rust-toolchain.toml`. The license gate also needs cargo-deny:

```sh
cargo install cargo-deny --locked
cargo xtask ci    # everything CI runs
```

## License

Bodo Kit (`kit/`) is licensed under MIT OR Apache-2.0, at your option. Everything else is licensed under GPL-3.0-or-later. See [ADR 0002](docs/adr/0002-license-split.md).
