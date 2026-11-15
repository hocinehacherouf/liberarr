#!/bin/sh
# Installs the liberarr binary from a GitHub release into ~/.local/bin.
#   curl --proto '=https' --tlsv1.2 -LsSf https://github.com/hocinehacherouf/liberarr/releases/latest/download/install.sh | sh
# LIBERARR_VERSION=0.2.0 installs that version instead of the latest one.
# Stops without installing anything on an unknown system, a failed download, a checksum mismatch
# or a failed attestation check (run when a signed-in `gh` is available).
# Everything is in main, called on the last line: a download cut short runs nothing.
set -eu

repo=hocinehacherouf/liberarr

fail() {
  echo "liberarr install: $*" >&2
  exit 1
}

# Checks the archive's build provenance with gh: stops on a failed check, never blocks when the
# check cannot run (no gh, a gh without `attestation`, or not signed in).
verify() {
  hint="To check it by hand: gh attestation verify $2 --repo $repo --signer-workflow $repo/.github/workflows/release.yml"
  if ! command -v gh >/dev/null 2>&1 || ! gh attestation verify --help >/dev/null 2>&1; then
    echo "Attestation not checked (no GitHub CLI, or one too old for attestations). $hint"
    return
  fi
  status=0
  gh attestation verify "$1" --repo "$repo" --signer-workflow "$repo/.github/workflows/release.yml" >/dev/null 2>&1 || status=$?
  case $status in
    0) echo "Attestation checked: built by $repo's release workflow." ;;
    # gh's exit code when it needs a sign-in.
    4) echo "Attestation not checked (gh is not signed in). $hint" ;;
    *) fail "attestation check failed for $2: nothing installed. $hint" ;;
  esac
}

main() {
  # For tests only: a local folder laid out like the releases URL (file:///...).
  releases=${LIBERARR_RELEASES_URL:-https://github.com/$repo/releases}

  case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) target=aarch64-apple-darwin ;;
    Darwin/x86_64) target=x86_64-apple-darwin ;;
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux/aarch64 | Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    *) fail "no build for $(uname -s) $(uname -m). See https://github.com/$repo/blob/main/docs/install.md" ;;
  esac

  if [ -n "${LIBERARR_VERSION:-}" ]; then
    base="$releases/download/v${LIBERARR_VERSION#v}"
  else
    base="$releases/latest/download"
  fi

  command -v curl >/dev/null 2>&1 || fail "curl is required"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1"; }
  elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1"; }
  else
    fail "sha256sum or shasum is required"
  fi

  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT

  archive="liberarr-$target.tar.gz"
  for file in "$archive" "$archive.sha256"; do
    curl --proto '=https,file' --proto-redir '=https' --tlsv1.2 -fsSL -o "$tmp/$file" "$base/$file" ||
      fail "could not download $base/$file"
  done

  expected=$(cut -d ' ' -f 1 <"$tmp/$archive.sha256")
  actual=$(sha256 "$tmp/$archive" | cut -d ' ' -f 1)
  [ -n "$expected" ] && [ "$expected" = "$actual" ] || fail "checksum mismatch for $archive: nothing installed"
  verify "$tmp/$archive" "$archive"

  tar -xzf "$tmp/$archive" -C "$tmp" liberarr || fail "could not extract $archive"
  dir="$HOME/.local/bin"
  mkdir -p "$dir"
  # Copy then rename, so a running liberarr is never left half-written.
  cp "$tmp/liberarr" "$dir/.liberarr.new"
  chmod 755 "$dir/.liberarr.new"
  mv -f "$dir/.liberarr.new" "$dir/liberarr"

  echo "Installed $("$dir/liberarr" --version) in $dir"
  case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "Add $dir to your PATH to run liberarr." ;;
  esac
}

main "$@"
