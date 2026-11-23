//! efto 入口。
//!
//! 为什么不用 `windows_subsystem = "windows"`（实测踩过两次）：
//! 1. 从 cmd/PowerShell 启动 GUI 子系统的 exe 时，调用方**不会等待**，
//!    `exe cli ... > file` 立刻返回且文件是空的——CLI 验证通道直接失效；
//! 2. 失去 stdout，`cli` 子命令没有任何输出。
//!
//! 所以保持控制台子系统，只在"这个控制台是我们自己新建的"时把它隐藏：
//! 用 GetConsoleProcessList 判断，只有本进程一个使用者时才隐藏——
//! 从终端里启动时控制台属于用户的 shell，绝不能动。

#[cfg(windows)]
fn hide_own_console() {
    use windows_sys::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

    let hwnd = unsafe { GetConsoleWindow() };
    if hwnd.is_null() {
        return; // 没有控制台（例如已被重定向到文件），无需处理
    }
    let mut pids = [0u32; 8];
    let count = unsafe { GetConsoleProcessList(&mut pids[0], pids.len() as u32) };
    if count <= 1 {
        // 只有我们挂在控制台上 => 这是双击启动时系统新建的控制台，藏掉
        unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
}

#[cfg(not(windows))]
fn hide_own_console() {}

/// 把「允许设置前台窗口」的权限交给当前实例。
///
/// 资源管理器「打开方式」拉起的是一个新进程：它拿着用户的前台权限，但真正要
/// 显示的插件窗口在已运行的实例里。不转交这份权限，`SetForegroundWindow` 会被
/// 前台锁拦下，插件窗口只出现在后面，用户以为没打开。
/// `ASFW_ANY` = 0xFFFF_FFFF：允许任意进程接管这一次前台切换。
#[cfg(windows)]
fn allow_foreground() {
    use windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow;
    unsafe {
        AllowSetForegroundWindow(0xFFFF_FFFF);
    }
}

#[cfg(not(windows))]
fn allow_foreground() {}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // UAC 提权助手：由 hosts 保存流程拉起，写完后立即退出，不进入 GUI。
    if args.get(1).map(|s| s.as_str()) == Some(efto_app::hosts::APPLY_ARG) {
        std::process::exit(efto_app::hosts::apply_main(
            args.get(2).map(|s| s.as_str()).unwrap_or(""),
        ));
    }
    if args.get(1).map(|s| s.as_str()) == Some("cli") {
        // CLI 通道保持控制台可见，输出照常走 stdout
        // 传 args[2..]：cli::main 的第一个参数应该是子命令本身
        std::process::exit(efto_app::cli::main(&args[2..]));
    }
    // 资源管理器「打开方式」：`efto.exe --open-file "<路径>" [--plugin <id>]`
    if let Some((path, plugin)) = efto_app::parse_open_file(&args) {
        hide_own_console();
        allow_foreground();
        efto_app::run_with_file(path, plugin);
        return;
    }
    hide_own_console();
    efto_app::run();
}
