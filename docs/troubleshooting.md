# Troubleshooting

Error messages name the setting or the command to run; they never show a secret. Start with `liberarr status` to see the devices, the last pull and failed work.

## Configuration

| Message | What to do |
|---|---|
| `….env not found: run liberarr init first` | Run `liberarr init`. If you use `LIBERARR_HOME`, check it points to the right folder. |
| `LIBERARR_… is missing, empty or still an example` | Run `liberarr init`: it asks only for what is missing. |
| `LIBERARR_… is too short` | Delete that line from `.env` and run `liberarr init` again: it generates a new value. Never do this for `LIBERARR_SECRET_KEY` once a device is linked. |
| `…_PROVIDER is not recognized` / `this provider cannot transcribe` | Use a value from [AI providers](ai-providers.md). `claude`, `codex` and `copilot` can only summarize, and `anthropic` is no longer supported (use `claude` or `openrouter`). |
| `The claude CLI did not answer (missing or not signed in)` (or `codex`, `copilot`) | Install the CLI and sign in by running it once, then run `liberarr init` again. Under `serve` on a server, add `CLAUDE_CODE_OAUTH_TOKEN` to the `.env`, see [AI providers](ai-providers.md#subscription-clis). In Docker no host CLI is visible: use an API provider. |
| `Notes are written in …, the interface language` | Add `LIBERARR_CONTENT_LANGUAGE=en` (or `fr`) to `.env` so notes keep one language. |
| `Warning: …_ROUTING_ONLY is empty` | OpenRouter may use its own shared models. Set the list, see [OpenRouter](ai-providers.md#openrouter). |
| `Warning: LIBERARR_TRANSCRIBE_VOCABULARY is ignored` | Your transcription provider or model takes no vocabulary. Use `local` or `mistral`, or remove the line. |
| `No answer available (input closed)` | `init` has no terminal to ask in. Use `liberarr init --non-interactive` with the variables exported. |

## Device (`link`, `pull`)

The device talks to one Bluetooth client at a time, and sleeps when idle.

- **`No device detected` / `No known Plaud device in range`**: wake the device (press a button), close the Plaud app on your phone (force-quit it), stay close, run the command again.
- **`No linked device: run liberarr link first`**: run `liberarr link`.
- **`Unlock test failed`**: close the Plaud app on the phone and run `liberarr link` again. If the account was reset or the keys changed, use `liberarr link --force`.
- **Plaud token refused**: tokens expire. Log in again at web.plaud.ai and copy a fresh `pld_ut` cookie. A pasted `Bearer ` prefix is fine.
- **macOS**: allow Bluetooth for your terminal app in System Settings → Privacy & Security → Bluetooth.
- **Linux**: the BlueZ service must be running (`systemctl status bluetooth`).
- **A pull was interrupted**: run `pull` again; the download resumes. A session that failed is retried on the next pull.
- **A recording is missing**: one still in progress on the device is skipped; stop the recording and pull again.

## Processing

`process`, `pull`, the web UI and the terminal app show which step failed (`transcribe`, `summarize` or `export`) and why.

- **Retry**: `liberarr rerun <id>` queues the failed step again; prior outputs are kept until it succeeds. It runs in `serve`, `tui`, or with the next `pull` or `process`.
- **`is already being processed`**: wait for the current work to finish.
- **HTTP 401 / 403**: the API key is wrong or revoked. Run `liberarr init`, choose to reconfigure the providers, and enter the new key.
- **HTTP 429 or overloaded**: already retried 3 times. Wait, then `rerun`. Check your provider's quota or credits.
- **Model not found**: the model name is wrong or retired. Run `liberarr init` and pick one from the list.
- **File too large**: only Ogg files (the device format) are split for upload. Convert another format to Ogg, or keep it under about 24 MB.
- **`local transcription reads Ogg Opus only`**: the file is not Ogg Opus. Convert it to Ogg Opus, or use a cloud transcriber ([AI providers](ai-providers.md#local-transcription)).
- **Model download fails or is slow**: the first local transcription downloads 575 MB from `huggingface.co`. Check the connection and run `rerun` again; a download that fails its size or checksum check is discarded, never kept. For an offline machine, use a cloud transcriber or copy `<LIBERARR_HOME>/models` from another machine.
- **Local transcription is very slow**: expected on a CPU-only machine or a NAS, see [speed](ai-providers.md#local-transcription). Use a cloud transcriber for those recordings.
- **Crash at start on `local` (illegal instruction)**: the CPU lacks AVX2 (x86-64) or Armv8.2-A (ARM). Use a cloud transcriber.
- **No timestamps / playback not synchronized**: the transcription model returns text only. Use `local`, `whisper-1` or a `voxtral` model.
- **`done, no speech detected`**: the recording has no speech; no summary or note is made, on purpose.
- **Work stuck after a crash**: the next `serve`, `tui`, `pull` or `process` puts interrupted tasks back in the queue.

## Web UI

- **Cannot open http://127.0.0.1:8484**: is `liberarr serve` running? Another program on port 8484? Use `--port`.
- **Forgot the password**: it is `LIBERARR_ADMIN_PASSWORD` in `.env`.
- **Too many login attempts**: after 5 failures, login is blocked for up to 15 minutes.
- **Login does not stick over plain HTTP**: remove `LIBERARR_COOKIE_SECURE=true`; it needs HTTPS.

## Still stuck

Open an issue with the command, the exact message and `liberarr --version`. Never paste your `.env`, tokens or transcripts.
