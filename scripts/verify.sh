#!/usr/bin/env bash
# End-to-end verification without a GUI, driving `efto cli` on macOS/Linux.
#
# 对应 Windows 侧的 scripts/verify.ps1：两侧检查清单以 docs/verification.md 为
# 唯一事实源，改一侧必须对齐另一侧（含 expectFail 语义）。标注 [win] 的检查在
# 这边永远 SKIP（es.exe / hosts 提权 / 记事本是 Windows 专有），标注 [mac] 的
# 反之。
#
#   scripts/verify.sh
set -uo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
exe="${CARGO_TARGET_DIR:-$root/target}/debug/efto"
work="$root/target/verify-tmp"
mkdir -p "$work"
fail=0

# 让启动器发现仓库里的示例插件（与 verify.ps1 一致）
export EFTO_PLUGIN_DIRS="${EFTO_PLUGIN_DIRS:-$root/plugins}"

# 插件冒烟用临时夹具而不是仓库插件：仓库插件可能被用户在 ~/.efto/settings.json
# 里停用，注册表会过滤停用项，那会让这条检查因与构建无关的原因变红。
smoke="$work/smoke-plugin"
mkdir -p "$smoke"
cat > "$smoke/efto-plugin.json" <<'JSON'
{"name":"verify-smoke","version":"0.0.1","main":"main.js"}
JSON
cat > "$smoke/main.js" <<'JS'
efto.onPluginEnter(function (info) { var p = String(info.payload || ''); efto.output([{ title: 'verify-smoke', subtitle: p, copy: p }]); });
JS
export EFTO_PLUGIN_DIRS="$EFTO_PLUGIN_DIRS:$smoke"

Say() { echo "==> $*"; }
Step() { echo "--- $*"; }

# Run the CLI capturing stdout+stderr into a file and print the exit code.
Run() {
  local out="$1"; shift
  "$exe" "$@" >"$out" 2>&1
  echo $?
}

Check() {
  local name="$1"; local expect_fail="${2:-0}"; shift 2
  Step "$name"
  local f="$work/check.txt"
  local code
  code=$(Run "$f" "$@")
  local text
  text=$(cat "$f" 2>/dev/null || true)
  if [ -n "$text" ]; then echo "$text"; fi
  if [ "$expect_fail" = "1" ]; then
    if [ "$code" != "0" ]; then
      echo "  OK (exit $code, as expected)"
    else
      fail=$((fail + 1)); echo "  FAIL: expected a non-zero exit code"
    fi
  else
    if [ "$code" = "0" ]; then
      echo "  OK"
    else
      fail=$((fail + 1)); echo "  FAIL (exit $code)"
    fi
  fi
}

Say 'build (debug)'
"$root/scripts/cargo.sh" "build -p efto-app" >/dev/null || { echo 'build failed'; exit 1; }

if command -v node >/dev/null 2>&1; then
  Say 'frontend guard (node scripts/check-frontend.js)'
  if node "$root/scripts/check-frontend.js"; then echo '  OK'; else fail=$((fail + 1)); echo '  FAIL'; fi
  Say 'i18n guard (node scripts/check-i18n.js)'
  if node "$root/scripts/check-i18n.js"; then echo '  OK'; else fail=$((fail + 1)); echo '  FAIL'; fi
  Say 'time stamp cross-check (node scripts/check-time-probe.js)'
  if node "$root/scripts/check-time-probe.js"; then echo '  OK'; else fail=$((fail + 1)); echo '  FAIL'; fi
else
  Say 'node missing: frontend/i18n/time-probe guards SKIPPED (not counted as pass)'
fi

Check 'index built' 0 cli index
Step 'plugins discovered'
f="$work/plugins.txt"
code=$(Run "$f" cli plugins)
plugins_out=$(cat "$f")
echo "$plugins_out"
if [ "$code" = "0" ] && echo "$plugins_out" | grep -q 'translate'; then
  echo '  OK'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code): translate plugin not discovered"
fi

# 拼音首字母（fy → 翻译）应命中 translate 插件的 CJK 触发词。
# 与 verify.ps1 的 jsb→记事本 同一目的（拼音 e2e）；mac 没有 CJK 系统应用，
# 用插件触发词做靶子。
Step 'search by pinyin initials (fy) should find 翻译'
f="$work/search-pinyin.txt"
code=$(Run "$f" cli search fy --json)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q 'translate'; then
  echo '  OK'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code, matched=$(echo "$text" | grep -c translate))"
fi

Step 'search by CJK trigger word (翻译)'
f="$work/search-cjk.txt"
code=$(Run "$f" cli search 翻译 --json)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q 'translate'; then
  echo '  OK'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code)"
fi

# Built-in calculator: expressions must produce a result row, and ordinary
# queries must NOT be hijacked by it.（与 verify.ps1 同一组用例）
while IFS='|' read -r q expect hijack; do
  Step "query \"$q\""
  f="$work/calc.txt"
  code=$(Run "$f" cli search "$q")
  text=$(cat "$f")
  if [ "$hijack" = "1" ]; then
    if echo "$text" | grep -qi 'calc'; then
      fail=$((fail + 1)); echo "  FAIL: query was hijacked by the calculator"
    else
      echo '  OK (not hijacked)'
    fi
  elif [ "$code" = "0" ] && echo "$text" | grep -qF -- "$expect"; then
    echo "  OK -> $expect"
  else
    fail=$((fail + 1)); echo "  FAIL: expected $expect, exit $code"
    echo "$text"
  fi
done <<'CASES'
6*7|42|0
sqrt(16)+2^3|12|0
1,000+1|1,001|0
-2^2|-4|0
chrome||1
1700000000||1
CASES

Check 'plugin-run: plain JS plugin' 0 cli plugin-run verify-smoke greet hello-efto

# timestamp-ts 双向转换：带偏移的字面量必须给出固定 epoch 对（与机器时区无关），
# 喂回 epoch 必须落回同一历日。（与 verify.ps1 同一契约）
Step 'plugin-run: timestamp-ts time -> stamp'
f="$work/ts-forward.txt"
code=$(Run "$f" cli plugin-run timestamp-ts ts 2026-11-17T14:30:00+08:00)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q '1794897000' && echo "$text" | grep -q '1794897000000'; then
  echo '  OK (SWC strip + nested require + sec/ms rows)'
else
  fail=$((fail + 1)); echo "  FAIL: expected the epoch pair of 2026-11-17T14:30:00+08:00"
  echo "$text"
fi

Step 'plugin-run: timestamp-ts stamp -> time'
f="$work/ts-reverse.txt"
code=$(Run "$f" cli plugin-run timestamp-ts ts 1794897000)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -qE '2026-11-1[67]' && echo "$text" | grep -q 'ISO 8601'; then
  echo '  OK (local time + ISO rows)'
else
  fail=$((fail + 1)); echo "  FAIL: expected a 2026-11-1x local time and an ISO 8601 row"
  echo "$text"
fi

# [win] hosts 插件读原生 hosts 能力（写入要 UAC 提权）——mac 上是 v1.5，SKIP
Check 'plugin-run: hosts plugin reads native capability' 0 cli plugin-run hosts hosts hosts

# translate：空载荷给出内联提示（证明插件加载 + efto.output 通路），不碰网络
Step 'plugin-run: translate with empty payload'
f="$work/translate-empty.txt"
code=$(Run "$f" cli plugin-run translate fy)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -qi 'hello world'; then
  echo '  OK (inline hint via efto.output)'
else
  fail=$((fail + 1)); echo "  FAIL: expected the inline 'input text' hint"
  echo "$text"
fi

Check 'plugin-run: unknown plugin must fail loudly' 1 cli plugin-run nope x y

Say 'spotlight inline source (real mdfind + Spotlight index)'
# 检索已限定在 ~/Desktop、~/Documents、~/Downloads（见 search/spotlight.rs 模块文档），
# 所以探针必须落在这三个目录里才搜得到。用唯一名字创建 → 等索引 → 搜索 → 清理，
# 不依赖用户桌面上恰好有什么文件。
probe_dir="$HOME/Documents/efto-verify-probe"
probe_name="efto-verify-probe-$RANDOM"
spotlight_ok=0
mkdir -p "$probe_dir" 2>/dev/null && printf 'x' > "$probe_dir/$probe_name.txt" 2>/dev/null && {
  # Spotlight 索引有几秒延迟：最多等 10s，每轮 1s
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if mdfind -onlyin "$HOME/Documents" -name "$probe_name" 2>/dev/null | grep -q "$probe_name"; then
      spotlight_ok=1
      break
    fi
    sleep 1
  done
}
if [ "$spotlight_ok" = "1" ]; then
  f="$work/mdfind.txt"
  code=$(Run "$f" cli search "$probe_name" --kind file)
  text=$(cat "$f")
  if echo "$text" | grep -q '(spotlight)' && echo "$text" | grep -q "$probe_name"; then
    if echo "$text" | grep -qE '/\.Trash/|/node_modules/|/\.git/'; then
      fail=$((fail + 1)); echo '  FAIL: noise (trash/node_modules/.git) leaked into inline rows'
    else
      echo '  OK (probe file found in scoped dirs, file rows merged below app results)'
    fi
  else
    fail=$((fail + 1)); echo "  FAIL (exit $code): scoped mdfind did not find the probe file"
    echo "$text"
  fi
else
  echo '  SKIP: Spotlight did not index the probe file within 10s (index disabled/paused)'
fi
rm -rf "$probe_dir"

# 反例（硬性约定 2）：限定目录之外的文件不该被搜到 —— 同时验证"不越界到全盘"。
Say 'spotlight scope stays inside the allowed dirs'
outside_dir="/tmp/efto-verify-outside-$$"
outside_name="efto-outside-probe-$$"
if mkdir -p "$outside_dir" 2>/dev/null && printf 'x' > "$outside_dir/$outside_name.txt" 2>/dev/null; then
  sleep 2
  f="$work/mdfind-outside.txt"
  Run "$f" cli search "$outside_name" --kind file >/dev/null
  # 只看路径本身："无匹配: <查询词>" 这类提示也会包含探针名，不能拿名字当判据
  if grep -qF "$outside_dir" "$f"; then
    fail=$((fail + 1)); echo '  FAIL: file outside the scoped dirs was returned (scope leaked to full disk)'
  else
    echo '  OK (outside-scope path not returned)'
  fi
  rm -rf "$outside_dir"
else
  echo '  SKIP: cannot create probe under /tmp'
fi

Step 'macOS system settings source (x-apple.systempreferences:)'
f="$work/syssettings.txt"
code=$(Run "$f" cli search x-apple.systempreferences: --json)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q '"source": "settings"'; then
  echo '  OK'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code)"
fi
# 反例（硬性约定 2）：普通输入不许被劫持成设置页
f="$work/syssettings-neg.txt"
code=$(Run "$f" cli search 显示 --json)
text=$(cat "$f")
if echo "$text" | grep -q '"source": "settings"'; then
  fail=$((fail + 1)); echo '  FAIL: normal input hijacked by settings source'
else
  echo '  OK (not hijacked)'
fi

Say 'run-command: dry-run resolves a shell'
out=$(Check 'run-command: dry-run' 0 cli run-command 'echo efto-verify' --dry-run)
if echo "$out" | grep -qE 'powershell|pwsh|cmd|bash|wsl|zsh|fish'; then
  echo '  OK (resolves a shell)'
else
  fail=$((fail + 1)); echo '  FAIL: dry-run output does not look like a shell command'
fi

Say 'run-command: echo round-trip'
out=$(Check 'run-command: echo' 0 cli run-command 'echo efto-verify')
if echo "$out" | grep -q 'efto-verify'; then
  echo '  OK (stdout has the echo marker)'
else
  fail=$((fail + 1)); echo '  FAIL: stdout missing the echo marker'
fi

Say 'capabilities --json'
f="$work/caps.txt"
code=$(Run "$f" cli capabilities --json)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q '"not_found": 3' && echo "$text" | grep -q '"--dry-run"'; then
  echo '  OK (subcommands, exit codes and dry-run surface advertised)'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code)"
  echo "$text"
fi

Say 'files --dry-run'
probe="$work/probe.txt"
echo probe >"$probe"
f="$work/files.txt"
code=$(Run "$f" cli files "$probe" --dry-run --json)
text=$(cat "$f")
if [ "$code" = "0" ] && echo "$text" | grep -q '"dry_run":true'; then
  echo '  OK (reported without opening)'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code)"
  echo "$text"
fi

Check 'focus: unknown pid is not-found' 1 cli focus --pid 999999 --dry-run

Say 'launch + cleanup (mac: resolve + open TextEdit, then quit it)'
# 解析检查（不启动）：--dry-run 必须报出目标而不是报错
Check 'launch: dry-run resolves TextEdit' 0 cli launch textedit --dry-run
# 真启动一次再退出：mac 上 .app 走 `open`，pid 日志是 open 的 pid（立即退出），
# 清理只能靠 osascript quit——与 verify.ps1 的"杀掉那一个 pid"同义。
f="$work/launch.txt"
code=$(Run "$f" cli launch textedit)
text=$(cat "$f")
echo "$text"
if [ "$code" = "0" ]; then
  sleep 1
  osascript -e 'tell application "TextEdit" to quit' 2>/dev/null || true
  echo '  OK (launched and quit TextEdit)'
else
  fail=$((fail + 1)); echo "  FAIL (exit $code)"
fi

echo
if [ "$fail" -eq 0 ]; then echo 'ALL CHECKS PASSED'; else echo "$fail CHECK(S) FAILED"; fi
exit "$fail"
