# Development setup

For contributors building Liberarr from source. Users only need the binary (see [Install](install.md)).

You need Rust (version pinned in `rust-toolchain.toml`) and Node (build-time only, for the web UI). On Linux also `libdbus-1-dev` and `pkg-config`.

## With mise (optional)

[mise](https://mise.jdx.dev) installs both versions from the repository's `mise.toml`:

```sh
mise trust      # once, to allow the repository's mise.toml
mise install    # Rust from rust-toolchain.toml, Node from .node-version
mise run build  # web UI, then the release binary
```

## Without mise

Install [rustup](https://rustup.rs) (it reads `rust-toolchain.toml` by itself) and Node, the version pinned in `.node-version`. Then:

```sh
npm --prefix web ci && npm --prefix web run build   # builds web/dist, embedded into the binary
cargo build --release
./target/release/liberarr --version
```

Contributors can also install the binary with `cargo install --path crates/liberarr --locked` after the web build.
