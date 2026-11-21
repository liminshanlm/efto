//! hosts 落盘：提权写入助手。
//!
//! hosts 在系统目录下（Windows `%SystemRoot%\System32\drivers\etc\`、
//! macOS `/etc/hosts`），普通进程没有写权限。这里刻意**不**要求 efto 整体以
//! 管理员身份运行，而是保存时临时拉起一个提权的 `efto hosts-apply <request>`：
//!
//! 1. 非提权侧把校验过的内容写成请求文件（`~/.efto/uac/request-*.json`）；
//! 2. 拉起同一 exe 并等它退出：Windows `ShellExecuteExW(verb="runas")`（UAC），
//!    macOS `osascript … do shell script … with administrator privileges`；
//! 3. 提权侧重新校验内容 → 备份到同目录 `hosts.efto.bak` → 临时文件 + 原子替换；
//! 4. 提权侧把结果写成 `<request>.resp.json`，非提权侧读回并清理。
//!
//! 两条安全边界：
//! - 提权侧只接受 `~/.efto/uac/` 下的请求文件，响应路径由请求路径推导，
//!   请求 JSON 里没有任何"写哪里"的字段 —— 否则 `efto.exe hosts-apply` 就成了
//!   一个"用户点一下 UAC 就能以管理员写任意文件"的工具；
//! - 目标文件固定为系统 hosts，内容必须通过 `efto_core::hosts::validate`。

use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use efto_core::fsutil;
use efto_core::hosts;

/// 提权子进程的入口参数：`efto.exe hosts-apply <request.json>`。
pub const APPLY_ARG: &str = "hosts-apply";

/// hosts 文件位置。
///
/// Windows 走 `efto_core::hosts::default_path()`（`%SystemRoot%` 拼出，不写死盘符）；
/// macOS 固定 `/etc/hosts`。其它平台的 read/save/path 都会报平台不支持，
/// 这里的返回值不会被用户看到。
pub fn target_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/etc/hosts")
    }
    #[cfg(not(target_os = "macos"))]
    {
        hosts::default_path()
    }
}

/// 写入前自动备份的位置（与 hosts 同目录，便于用户自己找回）。
pub fn backup_path() -> PathBuf {
    target_path().with_file_name("hosts.efto.bak")
}

fn uac_dir() -> PathBuf {
    fsutil::data_root().join("uac")
}

/// `efto.hosts` 的 path 查询。Windows / macOS 给真实路径，其它平台明确报平台不支持。
pub fn path_capability() -> Result<Value, String> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err("当前平台不支持：hosts".into())
    }
    #[cfg(any(windows, target_os = "macos"))]
    {
        Ok(Value::String(target_path().to_string_lossy().to_string()))
    }
}

/// `efto.hosts.read` 的实现。
///
/// Windows / macOS 走真流程；其它平台明确报平台不支持——不能拿
/// `C:\Windows\...` 的"文件不存在"糊弄人（硬性约定 3）。
#[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_variables, unreachable_code))]
pub fn read_capability() -> Result<Value, String> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        return Err("当前平台不支持：hosts".into());
    }
    let path = target_path();
    let bytes = std::fs::read(&path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let (text, enc) = hosts::decode(&bytes);
    Ok(json!({
        "path": path.to_string_lossy(),
        "backup": backup_path().to_string_lossy(),
        "text": text,
        "encoding": enc.key(),
        "bytes": bytes.len(),
    }))
}

/// `efto.hosts.save` 的实现：校验 → 编码 → 提权写入。
pub fn save_capability(opts: &Value) -> Result<Value, String> {
    let text = opts
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("hosts.save 需要 text（完整文件内容）")?;
    let enc = match opts.get("encoding").and_then(|v| v.as_str()) {
        Some(s) => hosts::Encoding::parse(s)
            .ok_or_else(|| format!("未知编码 {s:?}：只支持 utf8 / ansi"))?,
        None => hosts::Encoding::Utf8,
    };
    hosts::validate(text).map_err(|errs| errs.join("\n"))?;
    let bytes = hosts::encode(text, enc)?;
    save_bytes(&bytes)
}

/// 提权写入入口（阻塞到提权授权走完）。GUI 与 CLI 共用。
///
/// 请求文件写入、响应读回与清理是平台无关的；平台差异只收敛在
/// `shell_execute_runas`（Windows: ShellExecuteExW(verb="runas")；
/// macOS: osascript `do shell script … with administrator privileges`）。
#[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_variables, unreachable_code))]
pub fn save_bytes(bytes: &[u8]) -> Result<Value, String> {
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        return Err("当前平台不支持：hosts".into());
    }
    let dir = uac_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 {} 失败: {e}", dir.display()))?;
    let token = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0)
    );
    let request = dir.join(format!("request-{token}.json"));
    let response = response_path(&request);
    let payload = json!({ "content": STANDARD.encode(bytes) });
    std::fs::write(&request, serde_json::to_vec(&payload).map_err(|e| e.to_string())?)
        .map_err(|e| format!("写请求文件失败: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("定位 efto.exe 失败: {e}"))?;
    let request_arg = request.to_string_lossy().to_string();
    let outcome = shell_execute_runas(&exe, &[APPLY_ARG, &request_arg]);
    let resp = std::fs::read(&response).ok();
    let _ = std::fs::remove_file(&request);
    let _ = std::fs::remove_file(&response);

    let code = outcome?;
    let resp: Response = match resp {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("提权进程返回了无法解析的结果: {e}"))?,
        None => {
            return Err(format!(
                "提权进程未返回结果（退出码 {code:?}）。可能被安全软件拦截，或 hosts 目录不可写"
            ))
        }
    };
    if !resp.ok {
        return Err(if resp.error.is_empty() {
            "提权写入失败".into()
        } else {
            resp.error
        });
    }
    Ok(json!({
        "ok": true,
        "path": target_path().to_string_lossy(),
        "backup": resp.backup,
        "bytes": resp.bytes,
    }))
}

/// 提权子进程入口。返回值即进程退出码。
pub fn apply_main(request: &str) -> i32 {
    if request.is_empty() {
        eprintln!("{APPLY_ARG} 需要请求文件路径");
        return 2;
    }
    let path = Path::new(request);
    let response = response_path(path);
    // 只接受数据目录 uac/ 下的请求：见文件头的安全边界
    let allowed = uac_dir();
    let ok_dir = match (path.parent().and_then(|p| p.canonicalize().ok()), allowed.canonicalize()) {
        (Some(parent), Ok(allowed)) => parent == allowed,
        _ => false,
    };
    if !ok_dir {
        eprintln!("拒绝请求：{request} 不在 {} 下", allowed.display());
        return 2;
    }
    let outcome = apply_request(path);
    let resp = match &outcome {
        Ok((backup, bytes)) => Response {
            ok: true,
            error: String::new(),
            backup: backup.to_string_lossy().to_string(),
            bytes: *bytes,
        },
        Err(e) => Response {
            ok: false,
            error: e.clone(),
            backup: String::new(),
            bytes: 0,
        },
    };
    if let Ok(bytes) = serde_json::to_vec(&resp) {
        if let Err(e) = std::fs::write(&response, bytes) {
            eprintln!("写响应文件失败: {e}");
            return 1;
        }
    }
    match outcome {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Request {
    content: String,
}

#[derive(Serialize, Deserialize)]
struct Response {
    ok: bool,
    #[serde(default)]
    error: String,
    #[serde(default)]
    backup: String,
    #[serde(default)]
    bytes: usize,
}

fn response_path(request: &Path) -> PathBuf {
    request.with_extension("resp.json")
}

fn apply_request(request: &Path) -> Result<(PathBuf, usize), String> {
    let raw = std::fs::read(request).map_err(|e| format!("读取请求文件失败: {e}"))?;
    let req: Request =
        serde_json::from_slice(&raw).map_err(|e| format!("请求文件格式错误: {e}"))?;
    let bytes = STANDARD
        .decode(req.content.as_bytes())
        .map_err(|e| format!("请求内容不是合法 base64: {e}"))?;
    // 提权侧再校验一遍：不能假设非提权侧的校验一定跑过
    let (text, _) = hosts::decode(&bytes);
    hosts::validate(&text).map_err(|errs| errs.join("\n"))?;
    let target = target_path();
    let backup = backup_path();
    write_bytes(&target, &backup, &bytes)?;
    Ok((backup, bytes.len()))
}

/// 备份 → 临时文件 → 原子替换。备份失败直接中止，绝不"先写坏再想备份"。
fn write_bytes(target: &Path, backup: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 {} 失败: {e}", parent.display()))?;
    }
    if target.exists() {
        std::fs::copy(target, backup)
            .map_err(|e| format!("备份到 {} 失败: {e}", backup.display()))?;
    }
    let tmp = target.with_file_name("hosts.efto.tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写临时文件失败: {e}"))?;
    replace_file(&tmp, target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (a, b) = (wide(from), wide(to));
    let ok = unsafe {
        MoveFileExW(
            a.as_ptr(),
            b.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(format!(
            "替换 {} 失败: {}",
            to.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::rename(from, to).map_err(|e| format!("替换 {} 失败: {e}", to.display()))
}

/// 提权拉起子进程并等它结束，返回退出码。平台差异只在"怎么拉起"：
/// Windows 走 `ShellExecuteExW(verb="runas")`，macOS 走 osascript
/// `do shell script … with administrator privileges`。
#[cfg(windows)]
fn shell_execute_runas(exe: &Path, args: &[&str]) -> Result<Option<u32>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    // 用 ShellExecuteExW 而不是 ShellExecuteW：只有前者能拿到进程句柄，
    // 用户点"否"时也能立刻拿到 ERROR_CANCELLED（1223），不用靠超时去猜。
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let params = args
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let parameters: Vec<u16> = params.encode_utf16().chain(Some(0)).collect();
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(if err == 1223 {
            "已取消：未授予管理员权限（UAC）".into()
        } else {
            format!(
                "启动提权进程失败: {}",
                std::io::Error::from_raw_os_error(err as i32)
            )
        });
    }
    if info.hProcess.is_null() {
        return Ok(None);
    }
    let waited = unsafe { WaitForSingleObject(info.hProcess, 120_000) };
    if waited == WAIT_TIMEOUT {
        unsafe { CloseHandle(info.hProcess) };
        return Err("提权进程超时（120 秒），已放弃等待".into());
    }
    if waited != WAIT_OBJECT_0 {
        unsafe { CloseHandle(info.hProcess) };
        return Err("等待提权进程失败".into());
    }
    let mut code = 0u32;
    let got = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
    unsafe { CloseHandle(info.hProcess) };
    if got == 0 {
        return Err(format!(
            "读取提权进程退出码失败: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(Some(code))
}

/// sh 单引号引用：空格、单引号、`$` 等一律安全。
#[cfg(target_os = "macos")]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// AppleScript 字符串字面量转义。
#[cfg(target_os = "macos")]
fn apple_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 组装 `do shell script … with administrator privileges` 脚本。
///
/// 显式注入 `HOME=`：提权后命令以 root 身份跑，HOME 会解析成 root 的家目录，
/// `apply_main` 的 `uac/` 边界校验会把合法请求拒掉；发起方的 HOME 跟着命令走，
/// `apply_main` 一个字不用改（硬性约定 4：校验只有一份）。
#[cfg(target_os = "macos")]
fn admin_shell_script(exe: &Path, args: &[&str], home: &Path) -> String {
    let mut cmd = format!(
        "HOME={} {}",
        sh_quote(&home.to_string_lossy()),
        sh_quote(&exe.to_string_lossy())
    );
    for a in args {
        cmd.push(' ');
        cmd.push_str(&sh_quote(a));
    }
    format!(
        "do shell script \"{}\" with administrator privileges",
        apple_string(&cmd)
    )
}

/// 解析 osascript 的失败输出。
///
/// osascript 自身退出码恒为 1，真正信息在 stderr 结尾的 `… (N)`：
/// N = -128 是用户取消授权框；N > 0 是 `do shell script` 里命令自己的退出码
/// （`apply_main` 此时已写完响应文件，交回调用方读响应拿具体错误）；
/// 其它是 AppleScript/Apple Events 错误（如 Automation 被拒的 -1743）。
#[cfg(target_os = "macos")]
fn classify_osascript_failure(stderr: &str) -> Result<Option<u32>, String> {
    match trailing_error_code(stderr) {
        Some(-128) => Err("已取消：未授予管理员权限（macOS 管理员授权）".into()),
        Some(c) if c > 0 => Ok(Some(c as u32)),
        _ => Err(format!("提权拉起失败: {}", stderr.trim())),
    }
}

/// 取 stderr 末尾 `… (N)` 里的 N。
#[cfg(target_os = "macos")]
fn trailing_error_code(stderr: &str) -> Option<i64> {
    let s = stderr.trim_end();
    let inner = s.strip_suffix(')')?;
    let open = inner.rfind('(')?;
    inner[open + 1..].parse().ok()
}

#[cfg(target_os = "macos")]
fn shell_execute_runas(exe: &Path, args: &[&str]) -> Result<Option<u32>, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法定位用户主目录".to_string())?;
    let script = admin_shell_script(exe, args, &home);
    let out = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|e| format!("调用 osascript 失败: {e}"))?;
    if out.status.success() {
        return Ok(Some(0));
    }
    // 不设超时：管理员密码框是交互的，超时反而会把用户正在填的框杀掉
    classify_osascript_failure(&String::from_utf8_lossy(&out.stderr))
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn shell_execute_runas(_exe: &Path, _args: &[&str]) -> Result<Option<u32>, String> {
    Err("当前平台不支持：hosts".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows / macOS 之外：hosts 能力（path/read/save）必须报"当前平台不支持"，
    /// 不能拿 `C:\Windows\...` 的"文件不存在"或更糟的静默成功糊弄（硬性约定 3）。
    #[cfg(all(not(windows), not(target_os = "macos")))]
    #[test]
    fn hosts_capability_errors_on_this_platform() {
        for r in [
            path_capability(),
            read_capability(),
            save_capability(&serde_json::json!({"text": "127.0.0.1 x"})),
        ] {
            let e = r.expect_err("此平台上 hosts 能力必须报错");
            assert!(e.contains("当前平台不支持"), "{e}");
        }
    }

    /// macOS：路径固定 /etc/hosts，读取走真流程（/etc/hosts 是 UTF-8，
    /// `decode` 先试 UTF-8 应直接命中）。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_target_path_and_capabilities() {
        assert_eq!(target_path(), PathBuf::from("/etc/hosts"));
        assert_eq!(
            backup_path(),
            PathBuf::from("/etc/hosts.efto.bak"),
            "备份与 hosts 同目录"
        );
        assert_eq!(
            path_capability().unwrap(),
            Value::String("/etc/hosts".into())
        );
        let v = read_capability().expect("macOS 上 read 应能读 /etc/hosts");
        assert_eq!(v["path"], "/etc/hosts");
        assert_eq!(v["encoding"], "utf8");
        assert!(v["text"].as_str().unwrap().contains("localhost"));
    }

    /// macOS：提权命令构造。路径含空格/单引号必须安全；HOME 显式注入
    /// （提权后 root 身份下 HOME 会解析成 root 的家目录）。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_admin_shell_script_quotes_paths_and_injects_home() {
        let exe = Path::new("/Applications/My App/efto's bin");
        let req = "/Users/a b/.efto/uac/request-1.json";
        let script = admin_shell_script(exe, &["hosts-apply", req], Path::new("/Users/a b"));
        // 注入 HOME、每个参数单引号引用；单引号在 sh 里以 '\'' 转义，
        // 该反斜杠再经 AppleScript 字面量转义变成 \\ （正文里就是两个反斜杠）
        assert_eq!(
            script,
            "do shell script \"HOME='/Users/a b' '/Applications/My App/efto'\\\\''s bin' 'hosts-apply' '/Users/a b/.efto/uac/request-1.json'\" with administrator privileges"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_sh_quote_is_safe_for_shell_metacharacters() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote(r#"a"b$c`d"#), r#"'a"b$c`d'"#);
    }

    /// macOS：osascript 失败分类。取消（-128）报"已取消"；正数是提权子进程
    /// 退出码（交给响应文件解析）；其它（如 Automation 被拒 -1743）明确报错。
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_classify_osascript_failure() {
        let cancel = "0:24: execution error: 用户已取消。 (-128)";
        let e = classify_osascript_failure(cancel).unwrap_err();
        assert!(e.contains("已取消"), "{e}");

        let apply_failed = "0:39: execution error: 第 1 行：… (1)";
        assert_eq!(classify_osascript_failure(apply_failed).unwrap(), Some(1));
        assert_eq!(
            classify_osascript_failure("execution error: x (255)").unwrap(),
            Some(255)
        );

        let no_automation = "execution error: Not authorized to send Apple events. (-1743)";
        let e = classify_osascript_failure(no_automation).unwrap_err();
        assert!(e.contains("提权拉起失败"), "{e}");
        assert!(e.contains("-1743"), "{e}");
        let e = classify_osascript_failure("").unwrap_err();
        assert!(e.contains("提权拉起失败"), "{e}");
    }

    #[test]
    fn write_bytes_backs_up_then_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hosts");
        let backup = dir.path().join("hosts.efto.bak");
        std::fs::write(&target, b"old\n").unwrap();

        write_bytes(&target, &backup, b"new\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new\n");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old\n");

        write_bytes(&target, &backup, b"newer\n").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"newer\n");
        assert_eq!(std::fs::read(&backup).unwrap(), b"new\n");
        assert!(!dir.path().join("hosts.efto.tmp").exists(), "临时文件应被替换掉");
    }

    #[test]
    fn request_with_invalid_content_is_refused_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let req = Request {
            content: STANDARD.encode(b"10.0.0.999 broken\r\n"),
        };
        let path = dir.path().join("request-x.json");
        std::fs::write(&path, serde_json::to_vec(&req).unwrap()).unwrap();
        // 内容校验发生在提权侧，绝不写系统 hosts；这里只断言校验本身
        let raw = std::fs::read(&path).unwrap();
        let req: Request = serde_json::from_slice(&raw).unwrap();
        let bytes = STANDARD.decode(req.content.as_bytes()).unwrap();
        let (text, _) = hosts::decode(&bytes);
        assert!(hosts::validate(&text).is_err());
    }

    #[test]
    fn apply_rejects_requests_outside_the_uac_directory() {
        let dir = tempfile::tempdir().unwrap();
        let req = dir.path().join("request-x.json");
        std::fs::write(&req, b"{}").unwrap();
        assert_ne!(apply_main(&req.to_string_lossy()), 0);
        assert_ne!(apply_main(""), 0);
    }

    #[test]
    fn save_capability_refuses_bad_text_and_encoding() {
        let e = save_capability(&json!({ "text": "10.0.0.999 x\r\n" })).unwrap_err();
        assert!(e.contains("第 1 行"), "{e}");
        let e = save_capability(&json!({ "text": "127.0.0.1 a\r\n", "encoding": "utf16" }))
            .unwrap_err();
        assert!(e.contains("未知编码"), "{e}");
        assert!(save_capability(&json!({})).unwrap_err().contains("text"));
    }

    #[test]
    fn response_path_is_derived_from_request() {
        let p = response_path(Path::new("request-1.json"));
        assert_eq!(p, PathBuf::from("request-1.resp.json"));
    }
}
