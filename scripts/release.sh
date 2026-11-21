#!/usr/bin/env bash
# 构建 macOS release 版本（.app / .dmg），并把签名/公证步骤留给显式参数。
#
#   scripts/release.sh            # 未签名构建（本机直跑可用）
#   scripts/release.sh --debug    # 其余参数原样转给 tauri build
#
# 对应 Windows 侧的发布工程（见 docs/macos-support-plan.md 附录）。产物在
#   crates/app/target/release/bundle/  （workspace 布局时可能在根 target/）
# 未签名产物外部分发会被 Gatekeeper 拦：右键打开，或
#   xattr -dr com.apple.quarantine <app>。
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

# cargo 可能不在 PATH（rustup --no-modify-path 安装），与 cargo.sh 同样兜底
if ! command -v cargo >/dev/null 2>&1; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

# 与 scripts/cargo.sh 相同的 V8 环境（.cargo/config.toml 的 [env] 只钉了 Windows 产物）
target="$(uname -m)"
case "$target" in
  arm64)  target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *) echo "unsupported arch: $target" >&2; exit 1 ;;
esac
archive="$root/vendor/rusty_v8/librusty_v8_release_${target}.a.gz"
if [ ! -f "$archive" ]; then
  echo "missing V8 archive: $archive（先跑 scripts/bootstrap.sh）" >&2
  exit 1
fi
export RUSTY_V8_ARCHIVE="$archive"
case "$target" in
  aarch64-apple-darwin)
    export RUSTY_V8_ARCHIVE_SHA256="a4a5484444ad4b5f10dc7bed141236dd9dbeabf2006d14520f19b935d4db4227" ;;
  *)
    if [ -f "$archive.sha256" ]; then
      export RUSTY_V8_ARCHIVE_SHA256="$(awk '{print $1}' "$archive.sha256")"
    fi ;;
esac

# tauri CLI：优先 PATH / cargo 子命令，最后 npx（纯构建期工具，不进产品依赖）
if command -v tauri >/dev/null 2>&1; then
  TAURI=(tauri)
elif command -v cargo-tauri >/dev/null 2>&1; then
  TAURI=(cargo tauri)
else
  TAURI=(npx -y "@tauri-apps/cli@2")
fi

cd "$root/crates/app"
echo "==> ${TAURI[*]} build $*"
exec "${TAURI[@]}" build "$@"
