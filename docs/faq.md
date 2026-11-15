# FAQ

**Is Liberarr affiliated with Plaud?**
No. It is an independent project, not affiliated with, endorsed by or sponsored by Plaud. "Plaud" only names the hardware it works with.

**Do I need a Plaud account?**
Once, for `link`: the device keys are only available from your account. After that, `pull` works without it.

**Can I keep using the Plaud phone app?**
Yes, with rules:

- Keep the app **only for firmware updates**.
- Turn **off "Sync in idle"** in the app, and do not sync recordings through the app.
- Apply a firmware update only after checking its release notes, then check that `liberarr pull` still works.
- Close the app on your phone before `pull` or `link`: the device talks to one Bluetooth client at a time.

**Does Liberarr delete recordings from the device?**
No. It downloads them and keeps track of what it already has.

**Can I import recordings I already have as files?**
Yes: `liberarr process <files…>`. See [First run](first-run.md#other-commands).

**Can it run without any AI provider, fully offline?**
Not yet. Local transcription is planned for a later version.

**Which languages does transcription support?**
Whatever your transcription model supports. No language is forced, so a recording mixing French and English works with a multilingual model.

**In which language are summaries written?**
`LIBERARR_CONTENT_LANGUAGE`, `en` or `fr`, asked by `init`. See [Configuration](configuration.md#language).

**Does Liberarr ever change a transcript?**
No. The original audio is stored untouched, and a failed step never deletes earlier outputs.

**Can I edit the exported notes?**
Yes, outside the `%% liberarr:start %%` / `%% liberarr:end %%` markers. Liberarr rewrites only the text between them.

**Can several people use it?**
One admin user for now.

**Where are my data, and how do I back them up?**
In `LIBERARR_HOME` (default `~/.liberarr`). `liberarr backup <folder>` copies it safely, even while Liberarr runs; keep the `.env` separately. See [Backup and restore](backup.md).

**How do I get help on a command?**
`liberarr --help` and `liberarr <command> --help`, in English or French.
