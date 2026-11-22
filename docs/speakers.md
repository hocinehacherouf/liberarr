# Speaker labels

Liberarr tells who spoke when and labels each transcript line "Speaker 1", "Speaker 2"… It is the `diarize` step, after the summary and before the export. The transcript text is never changed: only labels are added next to it.

## How it works

- **Local only.** Two small models run inside the binary: pyannote segmentation 3.0 (6 MB, MIT) and NeMo TitaNet small (40 MB, Apache-2.0). No audio and no speaker data is sent anywhere, whatever your transcription provider is.
- **One-time download.** The first diarization downloads both models from Hugging Face into `<LIBERARR_HOME>/models`, checks their SHA-256 and keeps them. After that it works offline. `liberarr backup` skips this folder.
- **Optional step.** If it fails, the failure is shown on the recording but the transcript, the summary and the note are still produced (without labels). `liberarr rerun <id>` retries it. Turn it off with `LIBERARR_DIARIZE=off` in the `.env` (default `on`).
- **Automatic count.** Liberarr finds the number of speakers by itself, and merges a speaker with under 5 seconds of speech into the neighbouring speaker, so a cough or an "mm-hm" does not become a person.
- **Cost.** About 1 minute per 19 minutes of audio on an M4 Pro, and 0.5 to 1 GB of memory while it runs. The binary is about 16 MB bigger.

## Where labels appear

- **Web UI:** a label before each transcript line.
- **Terminal app:** the same labels (no renaming there).
- **Obsidian note:** `**Name** [mm:ss] text`.
- **MCP** (`liberarr mcp`): the transcript an agent reads carries the labels.

## Fix the speakers

In the web UI, on the recording page:

- **Number of speakers:** choose `Automatic` or 1 to 10. A fixed number reruns the diarization with it. Use it when the automatic count is wrong (see [Troubleshooting](troubleshooting.md#processing)).
- **Rename a speaker** ("Speaker 1" to "Alice"): the name applies to this recording only. Names are reset when the diarization runs again and finds different turns.

No voiceprint is stored: Liberarr does not recognize the same person across meetings. That is planned ([issue #13](https://github.com/hocinehacherouf/liberarr/issues/13)).
