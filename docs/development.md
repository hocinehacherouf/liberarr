# Development setup

For contributors building Liberarr from source. Users only need the binary (see [Install](install.md)).

You need Rust (version pinned in `rust-toolchain.toml`) and Node (build-time only, for the web UI). Local transcription builds whisper.cpp from source: you also need `cmake`, a C++ compiler and libclang (Linux: `cmake`, `g++`, `libclang-dev`; macOS: Xcode command line tools and `brew install cmake`). On Linux also `libdbus-1-dev` and `pkg-config`. The models are downloaded at run time.

## Speaker diarization (sherpa-onnx)

Local diarization links a prebuilt static archive of sherpa-onnx and onnxruntime (about 20 MB). The build script of the `sherpa-onnx-sys` crate downloads it from GitHub **without any checksum** and caches it under `target/sherpa-onnx-prebuilt/`. On your own machine you can let it do that. CI and release builds never do: they fetch the archive with `packaging/fetch-sherpa-onnx.sh`, which checks it against `packaging/sherpa-onnx.sha256` and sets `SHERPA_ONNX_ARCHIVE_DIR`. To build the same way (platform: `osx-arm64`, `osx-x64`, `linux-x64` or `linux-aarch64`):

```sh
packaging/fetch-sherpa-onnx.sh osx-arm64 target/sherpa-archives
SHERPA_ONNX_ARCHIVE_DIR="$PWD/target/sherpa-archives" cargo build --release
```

Never cache `target/` in a workflow: the build script reuses `target/sherpa-onnx-prebuilt/` without any check. Bumping `sherpa-onnx` needs the new archives' SHA-256 in `packaging/sherpa-onnx.sha256` (CI fails until it is there) and the licence texts in `NOTICE` and `packaging/THIRD-PARTY-onnxruntime.txt` re-checked.

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
