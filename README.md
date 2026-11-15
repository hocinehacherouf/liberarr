# Liberarr

[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/hocinehacherouf/liberarr/badge)](https://scorecard.dev/viewer/?uri=github.com/hocinehacherouf/liberarr)

A single CLI that frees your Plaud recordings from the vendor. Audio comes straight off the device over Bluetooth (or from local files); Liberarr produces a transcript, a summary (English or French), a mind map and an action table with the AI provider of your choice, exports notes to Obsidian, and serves a local web UI. One binary, nothing else to install.

Liberarr is an independent project, **not affiliated with, endorsed by or sponsored by Plaud**. "Plaud" is used only to describe the hardware it works with.

## Install

Download the archive for your platform from the [releases page](../../releases), check it and put `liberarr` in your `PATH`: see [docs/install.md](docs/install.md). macOS (Apple silicon, Intel) and Linux (x86-64, ARM64).

## Quick start

```sh
liberarr init    # creates ~/.liberarr/.env: generated secrets, asks for your AI provider keys
liberarr link    # one-time: fetches the device keys from your Plaud account, then tests an unlock
liberarr pull    # pulls new recordings off the device, then processes them
liberarr tui     # full-screen terminal app
liberarr serve   # or the web UI on http://127.0.0.1:8484
```

`liberarr --help` and `liberarr <command> --help` describe every command and option, in English or French.

## Documentation

[docs/](docs/README.md), written for a tester starting alone:

- [Install](docs/install.md)
- [First run](docs/first-run.md): `init`, `link`, `pull`, `tui`, `serve`, `process`, `rerun`, `status`
- [Configuration](docs/configuration.md): `LIBERARR_HOME`, the `.env` file, languages
- [AI providers and costs](docs/ai-providers.md)
- [Privacy](docs/privacy.md): what leaves your machine
- [Troubleshooting](docs/troubleshooting.md)
- [FAQ](docs/faq.md), including how to keep the Plaud app alongside

## Development

Requirements: Rust (version pinned in `rust-toolchain.toml`) and Node (build-time only, for the web UI). On Linux also `libdbus-1-dev` and `pkg-config`.

```sh
npm --prefix web ci && npm --prefix web run build   # builds web/dist, embedded into the binary
cargo build --release
./target/release/liberarr --version
```

To get both versions in one command with [mise](https://mise.jdx.dev), see [Development setup](docs/development.md). Contributors can also use `cargo install --path crates/liberarr --locked` after the web build. Rules for contributors and AI agents: see [AGENTS.md](AGENTS.md).

## License

MIT, see [LICENSE](LICENSE). Protocol credits in [NOTICE](NOTICE).
