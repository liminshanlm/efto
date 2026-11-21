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
exec "$root/scripts/cargo.sh" "run -p efto-app $*"
