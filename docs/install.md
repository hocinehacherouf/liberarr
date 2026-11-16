# Install

Liberarr is one binary. Nothing else is needed to run it.

## Supported platforms

| Target | System |
|---|---|
| `aarch64-apple-darwin` | macOS, Apple silicon |
| `x86_64-apple-darwin` | macOS, Intel |
| `x86_64-unknown-linux-gnu` | Linux, x86-64 |
| `aarch64-unknown-linux-gnu` | Linux, ARM64 |

Linux builds need glibc 2.35 or newer (Ubuntu 22.04, Debian 12) and use the host's BlueZ service for Bluetooth (present on desktop distributions); `libdbus` is linked dynamically. Windows is not supported.

## Install script

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/hocinehacherouf/liberarr/releases/latest/download/install.sh | sh
```

The script picks the archive for your system, checks its SHA-256 and puts `liberarr` in `~/.local/bin`. When the [GitHub CLI](https://cli.github.com) is installed and signed in, it also checks the archive's attestation (built by this repository's release workflow); otherwise it prints the command to check it by hand. It stops without installing anything on an unknown system, a failed download, a checksum mismatch or a failed attestation check. To install a given version, set `LIBERARR_VERSION`:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/hocinehacherouf/liberarr/releases/latest/download/install.sh | LIBERARR_VERSION=0.2.0 sh
```

## Homebrew (from the first release)

```sh
brew install hocinehacherouf/tap/liberarr
```

Works on macOS and Linux. Installing by this full name trusts only the `liberarr` formula of the tap.

## Manual steps

Download `liberarr-<target>.tar.gz` and its `.sha256` file from the [releases page](https://github.com/hocinehacherouf/liberarr/releases), then:

```sh
shasum -a 256 -c liberarr-<target>.tar.gz.sha256
gh attestation verify liberarr-<target>.tar.gz --repo hocinehacherouf/liberarr   # optional: proves the archive was built by this repository's release workflow
tar -xzf liberarr-<target>.tar.gz
chmod +x liberarr
xattr -d com.apple.quarantine liberarr 2>/dev/null || true   # macOS: the binary is not notarized
mkdir -p ~/.local/bin && mv liberarr ~/.local/bin/           # then make sure ~/.local/bin is in your PATH
liberarr --version
```

On macOS, the first `link` or `pull` asks for Bluetooth access for your terminal app: allow it.

## Update

`brew upgrade liberarr`, or run the install script again, or replace the binary the same way. Your data in `~/.liberarr` is kept.

## Uninstall

`brew uninstall liberarr`, or delete `~/.local/bin/liberarr`. Your recordings, transcripts and settings stay in `~/.liberarr` (or `LIBERARR_HOME`) until you delete that folder.

## Container (web UI)

From the first release, the image `ghcr.io/hocinehacherouf/liberarr` (Linux, x86-64 and ARM64) runs `liberarr serve`. It runs as a non-root user (uid 1000) and keeps everything in the `/data` volume (`LIBERARR_HOME`). It holds only the binary and the [Wolfi](https://wolfi.dev) packages it needs (glibc, CA certificates, libdbus, time zones): no shell, no package manager. The image is attested: `gh attestation verify oci://ghcr.io/hocinehacherouf/liberarr:<version> --repo hocinehacherouf/liberarr`.

First start: write your AI provider settings in a file, for example `providers.env` (see [Configuration](configuration.md) for the variables):

```sh
LIBERARR_TRANSCRIBE_PROVIDER=mistral
LIBERARR_TRANSCRIBE_API_KEY=your-key
LIBERARR_LLM_PROVIDER=mistral
LIBERARR_LLM_API_KEY=your-key
LIBERARR_LLM_MODEL=mistral-small-2603
LIBERARR_CONTENT_LANGUAGE=en
```

Then create the `.env` in the volume, and start the web UI:

```sh
docker run --rm -v liberarr:/data --env-file providers.env ghcr.io/hocinehacherouf/liberarr init --non-interactive
docker run -d --init --name liberarr --restart unless-stopped -p 127.0.0.1:8484:8484 -v liberarr:/data ghcr.io/hocinehacherouf/liberarr
```

`init` prints the web UI password once (user `admin`). The keys are now in `/data/.env`: delete `providers.env`. Open http://127.0.0.1:8484.

Or with Docker Compose, this `compose.yaml`:

```yaml
services:
  liberarr:
    image: ghcr.io/hocinehacherouf/liberarr:latest
    init: true
    restart: unless-stopped
    ports:
      - "127.0.0.1:8484:8484"
    volumes:
      - liberarr:/data

volumes:
  liberarr:
```

```sh
docker compose run --rm --env-from-file providers.env liberarr init --non-interactive
docker compose up -d
```

**Keep the port private.** Inside the container, `serve` listens on all interfaces, so the check that refuses foreign `Host` names on a loopback address is off. Publishing on `127.0.0.1` (as above) keeps the web UI reachable from this machine only. `-p 8484:8484` makes it reachable from your network, behind the login only: put it behind an HTTPS reverse proxy and set `LIBERARR_COOKIE_SECURE=true` in `/data/.env`.

**A bind mount instead of a volume** (`-v /some/folder:/data`) must be writable by uid 1000: `sudo chown 1000:1000 /some/folder`. Keep it on a local disk (see [Configuration](configuration.md)).

**Bluetooth.** The container is for the web UI and for processing files. Pulling from the device inside it:

- Linux: possible in principle through the host's BlueZ, by sharing its D-Bus socket (`-v /run/dbus:/run/dbus:ro`). Untested; `pull` and `link` from the host binary are the supported way.
- macOS: not possible. Docker Desktop runs containers in a Linux virtual machine without access to the Mac's Bluetooth.

To update: `docker pull` the new image (or `docker compose pull`), then recreate the container. Your data stays in the volume.

Next: [First run](first-run.md).
