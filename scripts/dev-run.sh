#!/usr/bin/env bash
# Build + run the desktop app in debug, with the repo's example plugins registered.
# 对应 Windows 侧的 scripts/dev-run.ps1。
#   scripts/dev-run.sh            # 起 GUI
#   scripts/dev-run.sh --cli ...  # 参数原样传给 efto
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

# 让启动器直接发现仓库里的示例插件，cargo run 开箱即用（与 dev-run.ps1 一致）。
if [ -z "${EFTO_PLUGIN_DIRS:-}" ]; then
  export EFTO_PLUGIN_DIRS="$root/plugins"
fi
if [ -z "${RUST_LOG:-}" ]; then
  export RUST_LOG=info
fi

echo "plugin dirs: $EFTO_PLUGIN_DIRS"
"$root/scripts/cargo.sh" "build -p efto-app" >/dev/null || exit 1
# 用本地开发签名身份签 debug 产物：adhoc 签名每次重编译都换身份，TCC 授权
# （屏幕录制/辅助功能）会失效；签名后跨重编译保持（见 scripts/sign-dev.sh）
bin="$root/target/debug/efto"
if security find-identity -v -p codesigning 2>/dev/null | grep -q "efto-dev"; then
  codesign --force --sign efto-dev "$bin" 2>/dev/null && echo "signed: efto-dev"
fi
exec "$bin" "$@"
