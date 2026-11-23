# Correct transcripts and summaries

A transcript can have a misheard name or a line Whisper invented, and a summary title can be vague. Fix them once in the web UI: the note, MCP and the terminal app then use your version. The engine's own text is never changed, and you can always see it and go back to it.

## What you can edit

- **A transcript line** (web UI, Transcript tab): the pencil opens an editor (Enter saves, Shift+Enter adds a line, Esc cancels). **Remove** strikes the line through and it stays restorable. An edited line shows an "Edited" badge: open it to read the original and press **Revert**.
- **Find and replace** in one transcript: literal text, any case unless you tick "Match case". It shows how many lines will change and applies them all at once, or none. A line that would end up empty is skipped: use Remove for that.
- **The summary title and text** (Summary tab): **Edit** opens the title and the Markdown with a live preview. **Restore the generated version** puts the engine's title and text back. Images in the Markdown are never loaded.
- **A transcript without timestamps** (some cloud providers) is edited as one block, the whole text.

## What you cannot edit

The action texts and the mind map (ticking an action as done works as before), and anything in the terminal app: it shows your corrections but does not edit.

## Where your corrections apply

- **Everywhere the text is read:** the web UI, the terminal app, MCP, the summary step on a rerun, and the Obsidian note.
- **The note updates by itself** after each save, between its `%% liberarr:start %%` markers as usual.
- **The engine's text is kept.** Your corrections are stored next to it, never over it.
- **Saving is refused** while the transcription is being redone (a summary edit, while the transcription or the summary is), because the new text would replace what you edit. It is also refused when the page shows an older version than the saved one (another tab saved first): reload the page and edit again.

## Rerunning a step

A new transcript or summary replaces the engine's text. Your corrections are dropped only if the new text differs from the one you corrected; if it comes out the same, they stay. The web UI asks before it reruns such a step, and `liberarr rerun` and the terminal app print a warning. The summary step drops only the summary edit. The other steps never touch your corrections.

## Two known limits

- If you remove every line, the recordings list does not mark the recording "No speech": that flag comes from the engine's text.
- In that case the export finds no speech and leaves the previous note as it was.
