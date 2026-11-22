# AI providers and costs

Liberarr uses two AI steps, each with its own provider (its own `LIBERARR_TRANSCRIBE_*` or `LIBERARR_LLM_*` settings):

1. **Transcription**: the audio is sent, a timestamped transcript comes back.
2. **Summary** (optional): the transcript is sent, a summary, a mind map and an action table come back. The default is `none`: you get the transcript and a note with the transcript only, and no transcript leaves your machine at this step.

`init` asks for both, tests each key (or each CLI) and proposes models. You can use one provider for both steps or two different ones.

## Which provider for which step

| Provider (`*_PROVIDER`) | Transcription | Summary | Notes |
|---|---|---|---|
| `none` | no | yes (default) | No summary: the transcript only. |
| `mistral` | yes | yes | Transcription with `voxtral-mini-transcribe-3` (recommended, segment timestamps). Summary: for example `mistral-small-2603`. |
| `openai-compatible` | yes | yes | Any OpenAI-compatible endpoint: OpenAI itself, a LiteLLM gateway, Groq, or Gemini's OpenAI-compatible URL (summary only). `openai` is accepted as another name for it. Transcription needs `/audio/transcriptions` with `response_format=verbose_json` for timestamps (`whisper-1` on OpenAI): a model that returns text only gives no timestamps, so playback is not synchronized with the text. |
| `openrouter` | yes | yes | See [OpenRouter](#openrouter). |
| `claude`, `codex`, `copilot` | no | yes | Your own signed-in command-line tool and its subscription: no URL, no key. See [Subscription CLIs](#subscription-clis). |

None of the CLIs can transcribe audio: with one of them for the summary, keep another provider for transcription.

`anthropic` (the Anthropic API) is no longer supported. A `.env` that still says `anthropic` is refused at start with a message naming the variable. Use `claude` (your Claude subscription) or `openrouter`, then run `liberarr init` again.

### Tested providers

The maintainer runs `mistral`, `claude`, `codex` and `copilot` against the real services. `openrouter` and `openai-compatible` depend on the endpoint behind them. If a provider fails for you, please [open an issue](https://github.com/hocinehacherouf/liberarr/issues) with the error text: it never contains your key.

Transcription forces no language: a recording mixing French and English works. Pick a multilingual model.

## Subscription CLIs

`claude` (Claude Code), `codex` (Codex CLI) and `copilot` (GitHub Copilot CLI) summarize through a tool you already pay for with a subscription: counted against its limits, not billed per token.

- **Detection.** `init` looks for these three programs on your `PATH` and offers the ones it finds. It never picks one for you: the transcript goes to that tool's vendor, so you confirm the choice. It then sends a trivial prompt; if that fails, it tells you to run the CLI once to sign in. The absolute path it found is stored as `LIBERARR_LLM_PROGRAM`, so `serve` still finds the tool when started by launchd or systemd with a short `PATH`.
- **Login.** Sign in with the tool itself, once, as your own user. `LIBERARR_LLM_MODEL` is optional: empty means the tool's own default model.
- **Servers.** A `serve` started outside your login session may not see the `claude` login. Run `claude setup-token` and put the result in `<LIBERARR_HOME>/.env` as `CLAUDE_CODE_OAUTH_TOKEN=...`: Liberarr passes it to `claude` only. Do not put it anywhere else.
- **Docker.** The container does not see the CLIs installed on your host, so `init` finds none there. For a container, use an API provider, or no summary.
- **Locked down.** Each CLI runs in an empty temporary directory that is removed afterwards. The transcript reaches it on standard input, never on the command line, and the reply is validated like any other. The CLI can only answer. How, per CLI:
  - `claude`: no tools, no user settings, no slash commands, only the MCP servers given on the command line (none), and no session file.
  - `codex`: your own `config.toml` (with its MCP servers) and rules are not loaded, no session file is written, the shell tool, hooks, web search, apps, plugins, browser and computer use, image viewing and generation, and multi-agent are turned off.
  - `copilot`: shell, write and URL tools are denied, built-in MCP servers are off, your custom instructions are not loaded, and it cannot ask you questions. Your own MCP servers still load, but non-interactive mode denies every call to them. Copilot keeps a local copy of the session (see [Privacy](privacy.md#ai-providers)).
- **Tested by a canary.** A preset is only shipped after `PROBE_CLI_CANARY=<cli> cargo run -p liberarr-infra --example ai_probe` prints `leaked=no wrote=no fetched=no summary_valid=yes`: a synthetic transcript asks the CLI to read a secret file, write a file, run `cat` and fetch a web page, and nothing must happen. Run it again on every new version of a CLI before trusting it; a CLI that fails is removed, not worked around.

## OpenRouter

OpenRouter's OpenAI-compatible API (`https://openrouter.ai/api/v1`), with an optional restriction to your own provider keys (BYOK), for example Mistral for transcription and Azure for summaries.

- Transcription models are `vendor/model` ids: `mistralai/voxtral-mini-transcribe-3-0` (proposed by `init`) and `openai/whisper-1` return timestamps.
- For summaries, type the model id (such as `openai/gpt-6-luna`).
- `LIBERARR_{TRANSCRIBE,LLM}_ROUTING_ONLY`: a comma-separated list of provider names (such as `mistral` or `azure`). With a list, every request of that step goes only to those providers, with no fallback. Without it (missing or empty), OpenRouter may serve requests with its own shared models, billed on your OpenRouter credits; Liberarr prints a warning at start.
- `init` lists the providers that serve the chosen model to pick the restriction from (or type a list; `-` for none).
- To be sure only your own key is used, also set each key's "Shared capacity fallback" to "Never use shared capacity for any model on this provider" in your OpenRouter BYOK settings.

## Costs

Liberarr itself is free. You pay your providers, on your own accounts:

- **Transcription**: usually billed per minute of audio. Each recording is transcribed once.
- **Summary**: billed per token (transcript in, summary out). One request per recording, plus one more when the first answer is malformed.
- **`claude`, `codex`, `copilot`**: count against your subscription limits, not billed per token.
- **`none`**: no summary, no cost.
- **OpenRouter**: billed on your own provider key when the request is routed to it (BYOK), otherwise on your OpenRouter credits.
- A recording without speech is transcribed, but never summarized.

What can cost more than once:

- `rerun` runs the step again **and the steps after it**: `rerun --step transcribe` pays a transcription and a summary again.
- Rate limits (HTTP 429), overloads (503, 529) and connection failures are retried automatically, at most 3 attempts in all. A transcription that timed out is not retried (the provider may still be working on it, and billing it). When a provider asks to wait more than 60 s, the job fails at once; run `rerun` later.

Check prices on each provider's pricing page: they change often.

Next: [Privacy](privacy.md).
