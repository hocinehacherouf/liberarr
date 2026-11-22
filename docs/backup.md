# Backup and restore

Everything Liberarr knows lives in `LIBERARR_HOME` (default `~/.liberarr`). `liberarr backup` copies it to a new folder, and your usual backup tool stores that folder where you want: a disk, a NAS, Google Drive, Azure, OVH, Amazon S3.

## Take a backup

```sh
liberarr backup ~/liberarr-backup
```

- It is safe while `serve`, `tui` or `pull` run: the database is copied as one consistent snapshot, then the audio and the other files.
- The folder must be new or empty, and outside the home.
- It needs free space about the size of the home (mostly the audio). On a full disk the backup fails and its folder is removed.
- The backup is built in `<folder>.incomplete` and gets its final name only when it is complete. If the backup fails, that folder is removed. If it is interrupted, the `.incomplete` folder stays: delete it and never restore from it.
- The command prints counts and sizes only, and exits with an error code on any failure.

What the backup contains:

| Included | Not included |
|---|---|
| The database (recordings, transcripts, summaries, jobs) | `.env`: your keys, the admin password, `LIBERARR_SECRET_KEY` |
| `audio/`: the original audio | Downloads still in progress (`device/partial`): the next pull resumes them |
| `device/`: the device keys, encrypted | Symbolic links: never followed, so nothing outside the home is read |
| `obsidian/`: the notes, when they are in the home | The notes, when `LIBERARR_OBSIDIAN_DIR` points outside the home: back up that folder yourself; the command reminds you |
| | `models/`: the local Whisper and speaker models (about 620 MB), downloaded again when missing |

**Keep the `.env` somewhere safe and separate**, for example in a password manager. Without it, a restored home keeps every recording, transcript and summary, but you must run `liberarr init` again (AI provider keys, new admin password) and `liberarr link` again (the stored device keys cannot be read without the old `LIBERARR_SECRET_KEY`).

Do not copy the live home with a file-by-file tool (rsync, rclone, restic, borg, Syncthing) while Liberarr runs. The database and its `-wal` file would be copied at different moments, and the copy can lose recent work or be corrupt. Time Machine and other tools that copy from a filesystem snapshot are fine. When in doubt, back up the folder made by `liberarr backup`.

## Send it off the machine, encrypted

Recordings are often work meetings. Use a tool that encrypts before upload.

[restic](https://restic.readthedocs.io/) encrypts everything and keeps a history. It works with a local disk, SFTP, S3-compatible storage (Amazon, OVH, others), Azure Blob, Google Cloud Storage, and anything rclone supports:

```sh
liberarr backup ~/liberarr-backup \
  && restic -r <your-repository> backup ~/liberarr-backup \
  && rm -rf ~/liberarr-backup
```

[rclone](https://rclone.org/) with a [crypt remote](https://rclone.org/crypt/) encrypts file contents and names. It works with Google Drive, OneDrive, Dropbox, Azure Blob, S3 and many others. Create the crypt remote once with `rclone config` (for example `secret:` on top of your Drive), then:

```sh
liberarr backup ~/liberarr-backup \
  && rclone sync ~/liberarr-backup secret:liberarr \
  && rm -rf ~/liberarr-backup
```

`rclone sync` keeps one copy, the latest; restic keeps every snapshot. Keep the restic password or the rclone crypt password safe: without it, the backup cannot be read. To run the backup every day, put the lines in cron or launchd.

## Restore

1. Stop `serve`, `tui` and any `pull`.
2. Choose an empty `LIBERARR_HOME`. Never restore over a home in use: move the old one away first.
3. Copy the backup into it:
   - from a folder: `cp -R ~/liberarr-backup/. ~/.liberarr/`
   - from restic: `restic -r <your-repository> restore latest:<absolute path of the backed-up folder> --target ~/.liberarr`
   - from rclone: `rclone copy secret:liberarr ~/.liberarr`
4. Put your `.env` back in it (`chmod 600 ~/.liberarr/.env`), or run `liberarr init` if you lost it.
5. Check with `liberarr status`, then start `serve` or `tui` as usual.

A backup made by an older Liberarr is upgraded when it opens. A backup made by a newer one is refused: install that version or a later one first.
