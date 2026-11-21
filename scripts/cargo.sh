#!/usr/bin/env bash
# Run a cargo command on macOS/Linux with the vendored V8 archive selected per target.
# Mirrors scripts/cargo.ps1 on Windows (which only wraps the MSVC environment).
#   scripts/cargo.sh "test --workspace"
#   scripts/cargo.sh "test -p efto-core"
set -euo pipefail

Cmd="${1:-check --workspace}"
root="$(cd "$(dirname "$0")/.." && pwd)"

# cargo may not be on PATH when rustup was installed with --no-modify-path.
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

# rusty_v8 prebuilt static lib: .cargo/config.toml pins the Windows archive
# (it cannot express per-target [env]), so override both vars here for mac.
# Keep these in sync with scripts/bootstrap.sh, which downloads the archive.
target="$(uname -m)"
case "$target" in
  arm64)  target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *) echo "unsupported arch: $target" >&2; exit 1 ;;
esac
archive="$root/vendor/rusty_v8/librusty_v8_release_${target}.a.gz"
if [ ! -f "$archive" ]; then
  echo "missing V8 archive: $archive" >&2
  echo "run: scripts/bootstrap.sh" >&2
  exit 1
fi
export RUSTY_V8_ARCHIVE="$archive"

# Pinned sha256 per archive (guards mirror downloads, same policy as .cargo/config.toml).
# Unknown targets fall back to a .sha256 sidecar written by bootstrap.sh.
case "$target" in
  aarch64-apple-darwin)
    export RUSTY_V8_ARCHIVE_SHA256="a4a5484444ad4b5f10dc7bed141236dd9dbeabf2006d14520f19b935d4db4227" ;;
  *)
    if [ -f "$archive.sha256" ]; then
      export RUSTY_V8_ARCHIVE_SHA256="$(awk '{print $1}' "$archive.sha256")"
    else
      export RUSTY_V8_ARCHIVE_SHA256=""
    fi ;;
esac

# Split the command string like the PowerShell wrapper does.
read -r -a argv <<<"$Cmd"
echo "cargo ${argv[*]}"
exec cargo "${argv[@]}"
