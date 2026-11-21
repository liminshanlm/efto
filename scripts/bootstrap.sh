#!/usr/bin/env bash
# One-time toolchain bootstrap for efto on macOS (Linux 也能用，V8 产物按 target 取)。
# 对应 Windows 侧的 scripts/bootstrap.ps1；两者的产物清单以 docs/verification.md 为准。
#   scripts/bootstrap.sh
set -euo pipefail

Step() { echo "==> $*"; }
Have() { command -v "$1" >/dev/null 2>&1; }

root="$(cd "$(dirname "$0")/.." && pwd)"

target="$(uname -m)"
case "$target" in
  arm64)  target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *) echo "unsupported arch: $target" >&2; exit 1 ;;
esac

# --- 1. Rust via rustup，走 rsproxy 镜像（static.rust-lang.org 可能被墙），与 bootstrap.ps1 一致 ---
if [ ! -x "$HOME/.cargo/bin/cargo" ] && ! Have cargo; then
  Step 'Installing rustup (mirror: rsproxy.cn)'
  tmp="$TMPDIR/rustup-init-efto"
  curl -sSL -o "$tmp" "https://rsproxy.cn/rustup/dist/${target}/rustup-init"
  chmod +x "$tmp"
  RUSTUP_DIST_SERVER=https://rsproxy.cn \
    RUSTUP_UPDATE_ROOT=https://rsproxy.cn/rustup \
    "$tmp" -y --no-modify-path --profile minimal --default-toolchain stable
  rm -f "$tmp"
else
  Step 'rustup already present'
fi

# --- 2. V8 预编译静态库（GitHub 直连经常超时，走 gh-proxy 镜像，与 bootstrap.ps1 同一策略）---
# 文件名带 lib 前缀（Windows 侧没有），sha256 钉在 scripts/cargo.sh 里。
lib="$root/vendor/rusty_v8/librusty_v8_release_${target}.a.gz"
if [ ! -f "$lib" ]; then
  Step "Fetching rusty_v8 prebuilt static lib for ${target} (via mirror)"
  mkdir -p "$(dirname "$lib")"
  url="https://gh-proxy.com/https://github.com/denoland/rusty_v8/releases/download/v152.2.0/$(basename "$lib")"
  curl -sSL -C - -o "$lib" "$url"
  if [ "$(wc -c <"$lib")" -lt 1000000 ]; then
    rm -f "$lib"
    echo "V8 archive download failed. Retry or set RUSTY_V8_MIRROR (see .cargo/config.toml)." >&2
    exit 1
  fi
  shasum -a 256 "$lib" | tee "$lib.sha256"
  echo "pin this sha256 in scripts/cargo.sh if it is not there yet"
else
  Step 'V8 archive already vendored'
fi

# --- 3. 图标（已入库，缺失才重新生成）---
if [ ! -f "$root/crates/app/icons/icon.ico" ] && Have node; then
  Step 'Generating icons'
  node "$root/scripts/gen-icons.js"
fi

echo
echo 'next:'
echo '  scripts/cargo.sh "test --workspace"'
echo '  scripts/dev-run.sh'
