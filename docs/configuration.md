# Configuration

## Where things live

Everything lives in `LIBERARR_HOME`, default `~/.liberarr`:

| Path | Content |
|---|---|
| `.env` | Settings and secrets (mode 0600, written by `init`). |
| `liberarr.db` | The database: recordings, transcripts, summaries, jobs. |
| `audio/` | The original audio, never modified. |
| `device/` | Device keys (encrypted) and partial downloads. |
| `models/` | The local Whisper model and voice-activity model, downloaded once (about 575 MB). Not in backups. |
| `obsidian/` | Exported notes, unless `LIBERARR_OBSIDIAN_DIR` is set. |

`LIBERARR_HOME` must be on a local disk: the job worker uses file locks, which are unreliable on network filesystems and some Docker bind mounts.

To move Liberarr to another machine, copy the whole folder while Liberarr is stopped, or use `liberarr backup` and restore it there (see [Backup and restore](backup.md)).

## The `.env` file

`<LIBERARR_HOME>/.env` is the only configuration source. Shell variables are ignored, except `LIBERARR_HOME` itself (it locates the file), the display preferences below, and the `init --non-interactive` input. Liberarr refuses to start when a required value is missing or still a placeholder (`change-me`), naming the variable but never its value.

Lines are `NAME=value`; `#` starts a comment.

### Written by `init`

| Variable | Meaning |
|---|---|
| `LIBERARR_SECRET_KEY` | Encrypts the stored device keys. At least 32 characters. **Never change it**: the stored device keys would become unreadable. |
| `LIBERARR_API_KEY` | Key for scripts calling the web API (`X-API-Key` header). At least 32 characters. |
| `LIBERARR_ADMIN_PASSWORD` | Web UI password of the `admin` user. At least 32 characters. |
| `LIBERARR_TRANSCRIBE_PROVIDER` | Transcription provider: `local` (default, Whisper on this machine, nothing to set), `openai`, `openai-compatible`, `mistral` or `openrouter`. With no transcription line at all it is `local`; a URL or a model without a provider means `openai-compatible`. |
| `LIBERARR_TRANSCRIBE_BASE_URL` | Base URL of its API (`http://` or `https://`). Not used with `local`. |
| `LIBERARR_TRANSCRIBE_API_KEY` | Its API key. Not used with `local`. |
| `LIBERARR_TRANSCRIBE_MODEL` | Transcription model. Not used with `local`. |
| `LIBERARR_LLM_PROVIDER` | Summary provider: `none` (default: transcript only), `claude`, `codex`, `copilot`, `openai-compatible`, `mistral` or `openrouter`. See [AI providers](ai-providers.md). |
| `LIBERARR_LLM_BASE_URL`, `LIBERARR_LLM_API_KEY`, `LIBERARR_LLM_MODEL` | Same, for the summary. With `claude`, `codex` or `copilot` there is no URL and no key, and the model is optional (empty: the CLI's own default). Not used with `none`. |
| `LIBERARR_LLM_PROGRAM` | `claude`, `codex` and `copilot` only: absolute path of the CLI, found by `init`. Empty: looked up on `PATH`. |
| `LIBERARR_TRANSCRIBE_ROUTING_ONLY`, `LIBERARR_LLM_ROUTING_ONLY` | OpenRouter only: providers allowed for that step. See [AI providers](ai-providers.md#openrouter). |
| `LIBERARR_CONTENT_LANGUAGE` | Language of summaries and note headings: `en` or `fr`. Without it, it follows the interface language and Liberarr prints a reminder. |
| `LIBERARR_OBSIDIAN_DIR` | Folder for the exported notes, for example inside an Obsidian vault. An absolute path (`~/` is expanded); `init` creates it when missing. Default `<LIBERARR_HOME>/obsidian`. |

### Optional, add them by hand

| Variable | Meaning |
|---|---|
| `CLAUDE_CODE_OAUTH_TOKEN` | `claude` summary provider, for a `serve` that cannot reach your `claude` login (a server): the token from `claude setup-token`. Passed to `claude` only. See [AI providers](ai-providers.md#subscription-clis). |
| `LIBERARR_LANG` | Language of the command line (messages and `--help`): `en` or `fr`. Default: the OS locale (`LC_ALL`, `LC_MESSAGES`, `LANG`), else English. |
| `LIBERARR_COOKIE_SECURE` | `true` to set the `Secure` flag on the login cookie, when the web UI is served over HTTPS. |
| `LIBERARR_TRANSCRIBE_VOCABULARY` | Names, acronyms and jargon the transcription should spell your way, comma-separated: `Liberarr, Plaud Note, HbA1c`. With `local` it is Whisper's initial prompt and stays on your machine. With `mistral` it is sent with every audio upload (see [Privacy](privacy.md)) as `context_bias`. Other providers ignore it, and Liberarr prints a warning at start. Keep the list short: Mistral counts it in the same context window as the audio, and a request over the limit fails. |

The note export is one-way. In an existing note, Liberarr writes only between `%% liberarr:start %%` and `%% liberarr:end %%`: your own text outside these markers is kept.

## Language

- Command line and `--help`: `LIBERARR_LANG` in `.env`, else the OS locale, else English.
- Web UI: the browser language.
- Summaries and note headings: `LIBERARR_CONTENT_LANGUAGE`.
- Transcription: no language is forced, so a recording mixing French and English works (see [local transcription](ai-providers.md#local-transcription) for the limits).

## Shell-only settings

These come from the shell, not from `.env`:

| Variable | Meaning |
|---|---|
| `LIBERARR_HOME` | Where everything lives (default `~/.liberarr`). |
| `LIBERARR_UI=tui` | `init` asks its questions full-screen. |
| `NO_COLOR` | Terminal app without colors or animation. |
| `LIBERARR_REDUCED_MOTION=1` | Terminal app without animation. |

Next: [AI providers and costs](ai-providers.md).
