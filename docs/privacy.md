# Privacy: what leaves your machine

Liberarr keeps everything in `LIBERARR_HOME` on your machine. Its database and web UI are the source of truth; no Liberarr server exists. Only the following leaves the machine.

| What | Where to | When |
|---|---|---|
| Your Plaud token, and your devices' serial numbers (already known to Plaud) to get their signatures; the device keys come back | Plaud's API (`api.plaud.ai` or your region's Plaud API host) | `link` only. |
| The audio of each recording, and your `LIBERARR_TRANSCRIBE_VOCABULARY` list when set | Your transcription provider | Transcription step. |
| The transcript of each recording | Your summary provider | Summary step. |
| Your API keys | The provider each key belongs to | Each request, and the key test in `init`. |

Nothing else is sent: no telemetry, no analytics. `pull` talks to the device over Bluetooth only.

## AI providers

**Every recording is sent to the providers you configure.** There is no per-recording choice yet. Do not use a provider you would not trust with a work meeting, and check its data-retention and training settings for your account.

With `claude`, transcripts go to Anthropic under your Claude account's privacy settings. With OpenRouter and no `ROUTING_ONLY` list, any provider OpenRouter picks may receive them.

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
