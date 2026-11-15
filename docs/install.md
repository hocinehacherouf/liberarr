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

Next: [First run](first-run.md).
