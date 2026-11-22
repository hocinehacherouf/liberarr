# Privacy: what leaves your machine

Liberarr keeps everything in `LIBERARR_HOME` on your machine. Its database and web UI are the source of truth; no Liberarr server exists. Only the following leaves the machine.

| What | Where to | When |
|---|---|---|
| Your Plaud token, and your devices' serial numbers (already known to Plaud) to get their signatures; the device keys come back | Plaud's API (`api.plaud.ai` or your region's Plaud API host) | `link` only. |
| The audio of each recording, and your `LIBERARR_TRANSCRIBE_VOCABULARY` list when set | Your transcription provider | Transcription step, only with a cloud provider. Never with `local`. |
| Nothing about your recordings: a plain download of the two model files (Whisper, 575 MB, and the voice-activity model, 0.9 MB) | Hugging Face (`huggingface.co`) | Once, at the first local transcription, and again if `<LIBERARR_HOME>/models` is deleted. |
| The transcript of each recording | Your summary provider, or the vendor of the subscription CLI you chose | Summary step, only when a summary provider other than `none` is set. |
| Your API keys | The provider each key belongs to | Each request, and the key test in `init`. |
| The summaries, actions and transcripts your AI agent reads | The AI provider of that agent | Only if you connect an agent to `liberarr mcp` ([AI agents](agents.md)). |

Nothing else is sent: no telemetry, no analytics. `pull` talks to the device over Bluetooth only.

## AI providers

**Every recording is sent to the cloud providers you configure.** There is no per-recording choice yet. With the default `local` transcription, the audio never leaves your machine. Do not use a provider you would not trust with a work meeting, and check its data-retention and training settings for your account.

The default summary provider is `none`: no transcript leaves your machine at that step. With `local` transcription and a `none` summary, nothing about your recordings leaves the machine at all, after the one-time model download.

With `claude`, `codex` or `copilot`, the transcript text is sent by that tool to its vendor (Anthropic, OpenAI or GitHub), under your account's privacy settings there. `init` never selects one by itself. Liberarr runs these tools with their tools and file access turned off, and re-tests that lockdown with a canary probe on new versions of each CLI (see [AI providers](ai-providers.md#subscription-clis)). Local copies: `codex` keeps none (`--ephemeral`); `claude` keeps none (`--no-session-persistence`); `copilot` keeps a session copy of the run, transcript included, under `~/.copilot/session-state`, because no flag turns that off. Delete it yourself if that matters to you. With OpenRouter and no `ROUTING_ONLY` list, any provider OpenRouter picks may receive them.

## Plaud account

`link` fetches the device keys from your Plaud account once. Plaud keeps its own copy of the key pair; Liberarr cannot change that. The token you paste is used for that run only and never stored. Liberarr never uploads recordings to Plaud.

## On disk

- `.env` (secrets) is mode 0600, readable only by you.
- Device keys are encrypted at rest with a key from `.env`.
- Audio, transcripts and summaries are stored as is in `LIBERARR_HOME`: protect that folder like the recordings themselves (disk encryption, backups you trust).
- Transcripts, audio, tokens, keys and device serials are never written to logs.

## Web UI on the network

`serve` listens on `127.0.0.1:8484` by default: only your machine can reach it. It has one user (`admin`), a login rate limit, and an API key for scripts. On a loopback address it answers only requests addressed to `127.0.0.1`, `localhost` or `[::1]` with its port (421 otherwise), which blocks DNS-rebinding pages; a reverse proxy on the same machine must send that `Host`.

To reach it from your LAN or a NAS, pass `--host 0.0.0.0` and put it behind a reverse proxy with HTTPS, or a VPN. Liberarr prints a warning when the UI is reachable from the network. Under HTTPS, set `LIBERARR_COOKIE_SECURE=true`.

Next: [Troubleshooting](troubleshooting.md).
