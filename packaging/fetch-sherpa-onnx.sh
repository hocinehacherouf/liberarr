#!/usr/bin/env bash
# Downloads the sherpa-onnx static library archive of one platform (osx-arm64, osx-x64, linux-x64 or
# linux-aarch64) into a directory, WITHOUT unpacking it, and checks it against packaging/sherpa-onnx.sha256.
# The build script of sherpa-onnx-sys downloads the same archive without any check: CI and release builds
# fetch it here instead and point SHERPA_ONNX_ARCHIVE_DIR at it (exported to $GITHUB_ENV when that is set).
# Usage: packaging/fetch-sherpa-onnx.sh <platform> <directory>
set -euo pipefail

platform="$1"
here="$(cd "$(dirname "$0")" && pwd)"
dir="$(mkdir -p "$2" && cd "$2" && pwd)"
# The version of the crate pin: a bump without new hashes fails below (no line for the new name).
version="$(sed -n 's/^sherpa-onnx = "=\(.*\)"$/\1/p' "$here/../Cargo.toml")"
# The name build.rs of sherpa-onnx-sys looks for in SHERPA_ONNX_ARCHIVE_DIR.
name="sherpa-onnx-v$version-$platform-static-lib.tar.bz2"

curl -fsSL --retry 3 -o "$dir/$name" "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$version/$name"
# grep fails when no line matches, so an unknown archive never passes.
grep " $name\$" "$here/sherpa-onnx.sha256" | (cd "$dir" && shasum -a 256 -c -) || { rm -f "$dir/$name"; exit 1; }

if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "SHERPA_ONNX_ARCHIVE_DIR=$dir" >> "$GITHUB_ENV"
fi
