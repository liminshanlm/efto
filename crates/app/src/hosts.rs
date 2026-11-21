//! hosts 落盘：UAC 提权助手。
//!
//! hosts 在 `%SystemRoot%\System32\drivers\etc\` 下，普通进程没有写权限。
//! 这里刻意**不**要求 efto 整体以管理员身份运行，而是保存时临时拉起一个提权的
//! `efto.exe hosts-apply <request>`：
//!
//! 1. 非提权侧把校验过的内容写成请求文件（`~/.efto/uac/request-*.json`）；
//! 2. `ShellExecuteExW(verb="runas")` 拉起同一 exe，等待它退出；
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
pub fn target_path() -> PathBuf {
    hosts::default_path()
}

/// 写入前自动备份的位置（与 hosts 同目录，便于用户自己找回）。
pub fn backup_path() -> PathBuf {
    target_path().with_file_name("hosts.efto.bak")
}

fn uac_dir() -> PathBuf {
    fsutil::data_root().join("uac")
}

/// `efto.hosts` 的 path 查询。非 Windows 明确报平台不支持（同 read/save）。
pub fn path_capability() -> Result<Value, String> {
    #[cfg(not(windows))]
    {
        Err("当前平台不支持：hosts".into())
    }
    #[cfg(windows)]
    {
        Ok(Value::String(target_path().to_string_lossy().to_string()))
    }
}

/// `efto.hosts.read` 的实现。
///
/// 非 Windows 明确报平台不支持（macOS 读 /etc/hosts 与 osascript 提权是 v1.5，
/// 见 docs/macos-support-plan.md）——不能拿 `C:\Windows\...` 的"文件不存在"糊弄人。
#[cfg_attr(not(windows), allow(unused_variables, unreachable_code))]
pub fn read_capability() -> Result<Value, String> {
    #[cfg(not(windows))]
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

/// 提权写入入口（阻塞到 UAC 走完）。GUI 与 CLI 共用。
#[cfg_attr(not(windows), allow(unused_variables, unreachable_code))]
pub fn save_bytes(bytes: &[u8]) -> Result<Value, String> {
    #[cfg(not(windows))]
    {
        return Err("当前平台不支持：hosts".into());
    }
    #[cfg(windows)]
    {
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
        let params = format!("\"{APPLY_ARG}\" \"{}\"", request.display());
        let outcome = shell_execute_runas(&exe, &params);
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

/// `runas` 拉起子进程并等它结束，返回退出码。
///
/// 用 ShellExecuteExW 而不是 ShellExecuteW：只有前者能拿到进程句柄，
/// 用户点"否"时也能立刻拿到 ERROR_CANCELLED（1223），不用靠超时去猜。
#[cfg(windows)]
fn shell_execute_runas(exe: &Path, params: &str) -> Result<Option<u32>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let file: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
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

#[cfg(not(windows))]
fn shell_execute_runas(_exe: &Path, _params: &str) -> Result<Option<u32>, String> {
    Err("hosts 编辑仅支持 Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 非 Windows：hosts 能力（path/read/save）必须报"当前平台不支持"，
    /// 不能拿 `C:\Windows\...` 的"文件不存在"或更糟的静默成功糊弄（硬性约定 3）。
    #[cfg(not(windows))]
    #[test]
    fn hosts_capability_errors_on_this_platform() {
        for r in [
            path_capability(),
            read_capability(),
            save_capability(&serde_json::json!({"text": "127.0.0.1 x"})),
        ] {
            let e = r.expect_err("非 Windows 上 hosts 能力必须报错");
            assert!(e.contains("当前平台不支持"), "{e}");
        }
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
