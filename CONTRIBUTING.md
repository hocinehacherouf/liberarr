# Contributing to Liberarr

Thanks for your interest. Liberarr is maintained by one person in their spare time, so please keep changes small and focused.

## Before you start

- **Bugs and ideas:** open an issue first, with the templates. For a larger change, wait for a reply before writing code.
- **Security issues:** never in a public issue. See [SECURITY.md](SECURITY.md).
- **Privacy:** never attach real recordings, transcripts, `.env` files, keys, tokens or device serials. Use synthetic data.

## Development

Set up your machine with [docs/development.md](docs/development.md). The architecture, code organisation and project rules are in [AGENTS.md](AGENTS.md); they apply to human contributors too.

Before opening a pull request, run what the CI runs:

```sh
npm --prefix web ci && npm --prefix web run build && npm --prefix web run lint   # first: the binary embeds web/dist
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

### Demo data

To try the terminal app or the web UI without a device, an account or a real key, and for every screenshot, use the fictional recordings of the `demo_home` example. It fills a new or empty directory and refuses any other:

```sh
cargo run -p liberarr --example demo_home -- /tmp/liberarr-demo   # keep it running
LIBERARR_HOME=/tmp/liberarr-demo cargo run -p liberarr -- tui     # in another terminal
```

The first command stays open on purpose: it stands in for a running worker, so the recording shown as processing stays in that state. Once it is stopped, the app takes that job and it fails, because the demo providers point at a closed local port. Each demo recording has a file of silence as long as the recording, made by the example (an Ogg Opus file that passes `verify_ogg`, about 200 bytes per second), so the player of the web UI works and a click on the transcript seeks.

The README demo is recorded from this data with [VHS](https://github.com/charmbracelet/vhs): `vhs docs/assets/demo.tape` (the build commands are at the top of the tape).

### Fuzzing

The parsers of untrusted bytes in `liberarr-core` (Ogg, BLE adverts and device replies) have [cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets in `fuzz/`, a separate workspace that needs a nightly toolchain:

```sh
rustup toolchain install nightly && cargo install cargo-fuzz --locked
cd fuzz && cargo +nightly fuzz list   # ogg, device_advert, device_reply
cargo +nightly fuzz run ogg -- -max_total_time=180
```

A crash is a bug: fix it in the core and add its minimised input as a unit test.

## Pull requests

- One topic per pull request. Code, comments, commit messages and docs are in English.
- User-facing text goes through the translation keys, in English and French.
- Add a test only for critical behaviour (data loss, security, parsing untrusted input, the core pipeline).
- New dependencies: explain why in the pull request; versions are pinned exactly.
- The `rust`, `web` and `zizmor` CI jobs only reach the hosts in their Harden-Runner `allowed-endpoints`. If a change needs a new host, the job fails: the StepSecurity insights link in the Harden-Runner step's log (and the agent log in its post step) shows the blocked call, and the pull request adds that host to the job's list.
- Pull requests are squash-merged: the title and description become the commit message, so write them carefully.

By contributing, you agree that your contribution is licensed under the [MIT License](LICENSE) and you follow the [Code of Conduct](CODE_OF_CONDUCT.md).
