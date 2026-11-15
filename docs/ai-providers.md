# AI providers and costs

Liberarr uses two AI steps, each with its own provider (its own `LIBERARR_TRANSCRIBE_*` or `LIBERARR_LLM_*` settings):

1. **Transcription**: the audio is sent, a timestamped transcript comes back.
2. **Summary**: the transcript is sent, a summary, a mind map and an action table come back.

`init` asks for both, tests each key and proposes models. You can use one provider for both steps or two different ones.

## Which provider for which step

| Provider (`*_PROVIDER`) | Transcription | Summary | Notes |
|---|---|---|---|
| `mistral` | yes | yes | Transcription with `voxtral-mini-transcribe-3` (recommended, segment timestamps). Summary: for example `mistral-small-2603`. |
| `openai` | yes | yes | Transcription with `whisper-1` (segment timestamps). `gpt-transcribe` and the `gpt-4o-*-transcribe` models return text only: no timestamps, so playback is not synchronized with the text. |
| `anthropic` | no | yes | Claude through the Anthropic API key, for example `claude-sonnet-5-5`. |
| `claude` | no | yes | Your signed-in `claude` CLI and its subscription: no URL, no key. Tolerated but not guaranteed by Anthropic; the `anthropic` API key is the recommended way. |
| `openrouter` | yes | yes | See [OpenRouter](#openrouter). |
| `openai-compatible` | yes | yes | Any OpenAI-compatible endpoint, such as a LiteLLM gateway, Groq, or Gemini's OpenAI-compatible URL (summary only). Transcription needs `/audio/transcriptions` with `response_format=verbose_json` for timestamps. |

Claude cannot transcribe audio: with `anthropic` or `claude` for the summary, keep another provider for transcription.

### Tested providers

The maintainer runs `mistral` and `claude` against the real services. `openai` and `anthropic` are tested against the providers' documentation and a mock server, not yet against the real services ([issue #16](https://github.com/hocinehacherouf/liberarr/issues/16)). `openrouter` and `openai-compatible` depend on the endpoint behind them. If a provider fails for you, please [open an issue](https://github.com/hocinehacherouf/liberarr/issues) with the error text: it never contains your key.

Transcription forces no language: a recording mixing French and English works. Pick a multilingual model.

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
- **`claude`**: counts against your Claude subscription limits, not billed per token.
- **OpenRouter**: billed on your own provider key when the request is routed to it (BYOK), otherwise on your OpenRouter credits.
- A recording without speech is transcribed, but never summarized.

What can cost more than once:

- `rerun` runs the step again **and the steps after it**: `rerun --step transcribe` pays a transcription and a summary again.
- Rate limits (HTTP 429), overloads (503, 529) and connection failures are retried automatically, at most 3 attempts in all. A transcription that timed out is not retried (the provider may still be working on it, and billing it). When a provider asks to wait more than 60 s, the job fails at once; run `rerun` later.

Check prices on each provider's pricing page: they change often.

Next: [Privacy](privacy.md).
