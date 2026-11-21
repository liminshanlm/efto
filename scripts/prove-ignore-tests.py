"""反向验证：新加的忽略规则测试必须真的能抓到这两个缺陷。

  1) 把 compile_glob 退回区分大小写  -> 期望测试失败（这就是原来那个静默失效的 bug）
  2) 把默认列表里的 **/assemblies/** 改回 "**/ assemblies/**" -> 期望测试失败（死规则）
"""
import io
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
START = ROOT + r"\crates\core\src\apps\startmenu.rs"
CONFIG = ROOT + r"\crates\core\src\config.rs"


def run_tests():
    # cwd 必须是项目根：cargo 找不到 Cargo.toml 也会以 101 退出，
    # 而本脚本用"退出码非 0"判定"缺陷被抓到"——那样三枪全是假阳性。
    p = subprocess.run(
        ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass",
         "-File", os.path.join(ROOT, "scripts", "cargo.ps1"), "-Cmd",
         "test -p efto-core --lib apps::startmenu"],
        capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=ROOT,
    )
    out = (p.stdout or "") + (p.stderr or "")
    if "could not find `Cargo.toml`" in out or "error: " in out and "test result" not in out:
        raise RuntimeError("cargo 没有真正跑起来，退出码不可信：\n" + out[-500:])
    failed = [l for l in out.splitlines() if "FAILED" in l or "test result: FAILED" in l]
    return p.returncode, failed[:3], out


def mutate(path, old, new):
    s = io.open(path, encoding="utf-8").read()
    assert old in s, "注入模式没匹配上: " + old[:60]
    io.open(path, "w", encoding="utf-8", newline="\n").write(s.replace(old, new, 1))


def restore(path, text):
    io.open(path, "w", encoding="utf-8", newline="\n").write(text)


ok = True

s1 = io.open(START, encoding="utf-8").read()
mutate(START, "GlobBuilder::new(p).case_insensitive(true).build()",
       "GlobBuilder::new(p).build()  // MUTATED: 退回区分大小写")
code, lines, out = run_tests()
print("mutation 1 (退回区分大小写) exit=%d %s" % (code, "OK 被抓到" if code != 0 else "BAD 没抓到"))
for l in lines:
    print("   ", l[:110])
ok &= code != 0
restore(START, s1)

s2 = io.open(CONFIG, encoding="utf-8").read()
mutate(CONFIG, '"**/assemblies/**".into(),', '"**/ assemblies/**".into(),  ')
code, lines, out = run_tests()
print("mutation 2 (改回带空格的死规则) exit=%d %s" % (code, "OK 被抓到" if code != 0 else "BAD 没抓到"))
for l in lines:
    print("   ", l[:110])
ok &= code != 0
restore(CONFIG, s2)

code, lines, out = run_tests()
print("restored exit=%d %s" % (code, "OK" if code == 0 else "BAD"))
if code != 0:
    tail = [l for l in out.splitlines() if l.strip()][-14:]
    print("--- 还原后那次运行的原始输出 ---")
    for l in tail:
        print("   ", l[:120])
ok &= code == 0

print("\nRESULT:", "新测试有牙" if ok else "新测试抓不到，等于没写")
sys.exit(0 if ok else 1)
