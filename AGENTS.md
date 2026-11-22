# AGENTS.md — rules for any AI agent working on Liberarr

These rules apply to every AI coding agent (Claude Code, Copilot, Codex, Cursor, …). They are binding. If a task conflicts with a rule, stop and ask the owner.

## What Liberarr is

A single Rust CLI, *arr-stack-inspired, that frees Plaud recordings from the vendor: audio in (pulled straight off the device over Bluetooth, or from local files) → reliable transcript, AI summaries and visuals (mind map, action table), with any AI provider. A local web UI is served by the same binary (`liberarr serve`). Users install one binary with zero host prerequisites.

## Architecture rules

1. **Liberarr is autonomous.** Its own DB and web UI are the source of truth. Obsidian, Plaud cloud, and any AI vendor are optional plug-ins, never hard dependencies.
2. **Sources are pluggable** (like *arr indexers): direct device pull, local files, later others. The pipeline never knows where audio came from.
3. **All AI calls go through provider adapters in `liberarr-infra`**, behind the core ports `Transcriber` and `Llm`. Each adapter speaks its provider's HTTP API directly. No vendor SDKs in app code. A generic OpenAI-compatible adapter stays available for gateways such as LiteLLM.
4. **Pipeline steps are resumable jobs.** A failed step never deletes prior outputs. The original audio is stored untouched.
5. **Never silently change a transcript.** LLM corrections are stored as `{original, corrected, reason}`; the raw transcript is always kept and viewable. *Applies when that step exists (v3+).*
6. **Voiceprints never leave the machine.** Diarization, speaker embeddings and matching run locally only. Never send embeddings to any API.
7. **Routing is per recording type**, and steps that run before auto-classification use the pre-set type or the owner's "unclassified" safe route. *Applies when that step exists (v3+).*
8. **Obsidian export is one-way** and writes only between `%% liberarr:start %%` / `%% liberarr:end %%` markers.
9. **Portability:** all state lives in `LIBERARR_HOME` (default `~/.liberarr`); secrets in `<LIBERARR_HOME>/.env`. Nothing machine-specific in the binary.
10. **Single admin user** for now. Do not build multi-user scaffolding until asked.

## Code organisation (DDD, enforced by crates)

Cargo workspace (`Cargo.toml` at the root), four crates under `crates/`:

- **`liberarr-core`** — domain + application: entities, value objects, pure rules, use cases and **ports** (traits). One module per bounded context: `recordings`, `device`, `transcription`, `summaries`, `exports`, `jobs`, `auth`. No I/O crates, no SQL, no network, no framework imports (exception: serde models for validating untrusted LLM output).
- **`liberarr-infra`** — adapters implementing the core ports: SQLite (rusqlite), BLE (btleplug), HTTP AI client (reqwest), Plaud cloud (link only), filesystem/Obsidian, key store, Ogg utilities.
- **`liberarr-web`** — axum JSON API (thin: calls use cases or read models) + the embedded static UI (rust-embed of `web/dist`).
- **`liberarr`** — the binary: CLI (clap), config/home, and the **composition root**. The only place that wires adapters to ports.

Dependency direction (checked by Cargo): `core` depends on no other workspace crate; `infra` → `core`; `web` → `core`; `liberarr` → all three. `infra` and `web` never depend on each other.

**Frontend**: `web/` (Vite + React + TypeScript), `web/src/features/<context>/` mirrors the core contexts (API hooks, types, components); shared code in `web/src/shared/`.

## Stack

- Rust: the version pinned in `rust-toolchain.toml`, edition 2024. Workspace lints: `unsafe_code = "forbid"`, clippy `unwrap_used = "warn"` (allowed in tests). CI runs `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`, `cargo deny check advisories licenses sources`.
- CLI: clap. HTTP: axum. DB: SQLite via rusqlite (bundled). Device: btleplug (BLE). AI: reqwest, one adapter per provider.
- UI: **Vite** + React + TypeScript, Tailwind, **shadcn/ui** (Base UI components, used as shipped), TanStack Query, Mermaid. Built to `web/dist` and embedded in the binary. Node is a contributor/CI build tool only, never a user prerequisite. Build order: `npm --prefix web ci && npm --prefix web run build`, then `cargo build --release`.
- Transcription must handle **French with mixed English** in the same recording (multilingual models only, no forced language).
- User-facing text (CLI, web UI, exported notes) is translatable. Supported languages: English (default) and French. The CLI language follows `LIBERARR_LANG`, then the OS locale; the web UI follows the browser language. The summary language and the note headings follow `LIBERARR_CONTENT_LANGUAGE` (default: the UI language). Text goes through message keys (`rust-i18n` in the binary, `i18next` in the web UI); `liberarr-core` takes a `Language` and has no i18n library; adapter and core error texts stay English.
- Local transcription and speaker identification (whisper-rs, sherpa-onnx) arrive in v3.

## Security & privacy

- Pin exact versions of every dependency (`Cargo.lock` and `web/package-lock.json` are committed; npm uses `save-exact` and `min-release-age=7`; GitHub Actions are pinned by commit SHA). Every crate/package/action must be at least 7 days old, enforced by Renovate `minimumReleaseAge` (`renovate.json`). Never `latest`.
- **One secrets source: `<LIBERARR_HOME>/.env`** (mode 0600, gitignored, generated by `liberarr init`). No secret value in any other file; code reads env only. Never derive a second copy of a secret by hand. The app refuses to start when a secret is missing or a placeholder, naming the variable but never its value.
- Secrets the app stores itself (device unlock material) encrypted at rest with a key from `.env`, not stored next to the data.
- No unauthenticated endpoints except health. Rate-limit login.
- Recordings include work meetings and biometric data: never log transcripts, audio content, tokens, keys, serials, or embeddings.
- Never commit secrets, `.env`, or real recordings. Test fixtures use synthetic or owner-approved audio only.

## Agent behaviour

- **Ask before**: committing, pushing, adding a dependency, changing a rule in this file, or anything touching the Plaud account.
- **Smallest complete change.** No speculative features, options or abstractions. Reuse what exists before adding.
- **Tests only for critical behaviour** (data loss, security, money, parsing untrusted output, core pipeline). No tests for trivial code or for the sake of coverage.
- **Before every push**: run the `ponytail-review` skill on the changes and fix the findings judged relevant; mention the skipped ones.
- Verify before claiming done: run the tests/build and report real output, including failures.
- Reverse engineering of the Plaud device/app is a **separate project**; do not mix it into app code. The device protocol is implemented only from documented facts (see `NOTICE`) and lands as a source.
- Design docs and plans live in `docs/superpowers/` and are **never versioned** (gitignored).
- Talk to the owner in French. Everything else is in English: code, comments, commit messages, docs, GitHub issues, pull requests and labels.
