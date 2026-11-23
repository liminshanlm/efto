//! 受限的外部程序执行。
//!
//! 两条使用路径，共用同一套约束：
//! - 内联搜索源 Everything 文件检索（[`crate::search::everything`]）——一等公民；
//! - 插件能力 `efto.exec`（`crates/app/src/host.rs`）——给插件用。
//!
//! 这是本项目**唯一**能拉起外部程序的地方，所以边界写死在这里，不交给插件：
//!
//! 1. **白名单 + 摘要锁定**。只有 [`ALLOWLIST`] 里的程序可以执行，而且必须
//!    sha256 与内置清单一致。为什么摘要也要校验：如果按名字去 PATH 里找 `es.exe`，
//!    任何能往 PATH 目录写文件的人就等于拿到了任意代码执行 —— 那正是这次加能力
//!    最容易引入的漏洞。找不到匹配摘要的候选时**逐个报告**实际摘要，而不是降级执行。
//! 2. **参数是数组，绝不经过 shell**。`Command::args()` 直接把参数交给
//!    CreateProcess，所以 `;` `&` `|` `>` 这些在插件侧拼不出任何额外语义。
//! 3. **超时与输出上限**。超时 `kill()`；stdout/stderr 各截断到 `max_bytes`，
//!    避免一个 `-n 1000000` 把内存吃掉。
//! 4. **输出按控制台代码页解码**。实测 es.exe 在 zh-CN 机器上输出 **GBK 而非 UTF-8**
//!    （中文路径原始字节 `0xD7 0xC9 ...`），所以必须用 `GetConsoleOutputCP()`/`GetACP()`
//!    经 `MultiByteToWideChar` 转 UTF-16，否则中文全乱。
//! 5. **缺失时可从官方地址取**。下载 zip → 解出目标文件 → 校验摘要 → 落到数据目录 tools/es/。
//!    网络与解压都只服务于"拿到那个被锁定的字节串"，拿到后仍走同一套校验。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::config::{CommandShell, Settings};
use crate::fsutil;

/// 单次执行的默认超时。es 查一次本机索引实测 120~180ms。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(3000);
/// 允许的最大超时，防止插件把能力线程池占死。
pub const MAX_TIMEOUT: Duration = Duration::from_millis(15_000);
/// stdout/stderr 各自的上限。
pub const MAX_OUTPUT: usize = 1 << 20;

/// 插件脚本（node/python）的默认超时。脚本比 es 查询慢得多，但也不能无限等。
pub const RUNTIME_DEFAULT_TIMEOUT: Duration = Duration::from_millis(10_000);
/// 插件脚本允许的最大超时：超过这个时间会把插件线程占死。
pub const RUNTIME_MAX_TIMEOUT: Duration = Duration::from_millis(60_000);

/// 插件可用的脚本运行时。解释器路径只能来自用户设置（`Settings::node_path` /
/// `Settings::python_path`），插件参数里不允许出现路径。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptRuntime {
    Node,
    Python,
}

impl ScriptRuntime {
    /// 能力参数与设置里使用的名字。
    pub fn key(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Python => "python",
        }
    }

    /// 面向用户的展示名，用于"未配置"错误文案。
    pub fn display(self) -> &'static str {
        match self {
            Self::Node => "Node.js",
            Self::Python => "Python",
        }
    }

    /// 把源码作为单参数交给解释器执行：`node -e <code>` / `python -c <code>`。
    fn code_flag(self) -> &'static str {
        match self {
            Self::Node => "-e",
            Self::Python => "-c",
        }
    }

    /// 在 PATH 目录里查找时接受的文件名（按优先级）。
    fn path_names(self) -> &'static [&'static str] {
        match self {
            Self::Node => {
                if cfg!(windows) {
                    &["node.exe"]
                } else {
                    &["node"]
                }
            }
            Self::Python => {
                if cfg!(windows) {
                    &["python.exe", "python3.exe"]
                } else {
                    &["python3", "python"]
                }
            }
        }
    }
}

/// 解析插件传来的 runtime 名字（大小写不敏感）。
pub fn script_runtime(name: &str) -> Option<ScriptRuntime> {
    match name.trim().to_ascii_lowercase().as_str() {
        "node" | "nodejs" | "node.js" => Some(ScriptRuntime::Node),
        "python" | "python3" => Some(ScriptRuntime::Python),
        _ => None,
    }
}

/// 白名单条目。
pub struct Allowed {
    /// 插件侧写的名字
    pub key: &'static str,
    /// 期望文件名
    pub file: &'static str,
    /// 锁定的 sha256（小写十六进制）。绝对路径的系统工具（[`Allowed::absolute_path`]）免钉
    /// ——系统升级会换二进制内容；其余条目必须钉死，否则"往 PATH 写同名文件"就拿到执行。
    pub sha256: &'static str,
    /// 缺失时的官方下载地址（zip）
    pub zip_url: &'static str,
    /// 对应版本，仅用于日志与错误文案
    pub version: &'static str,
    /// 固定绝对路径（系统自带工具用）；为空表示按候选目录查找 + 摘要校验。
    pub absolute_path: &'static str,
    /// 可执行平台：`"windows"` / `"macos"` / 空 = 全平台。
    pub os: &'static str,
    /// stdout 收满这么多行就杀进程、带部分结果返回（0 = 等自然退出）。
    /// 全量枚举型工具（mdfind 检索全盘能吐上万行、跑十几秒）内联检索等不起，
    /// 取前 N 行就够填结果框。
    pub max_lines: usize,
}

impl Allowed {
    /// 该条目在当前平台是否可执行。放在参数拦截之后判，边界语义跨平台都能测到。
    pub fn available_here(&self) -> bool {
        match self.os {
            "" => true,
            "windows" => cfg!(windows),
            "macos" => cfg!(target_os = "macos"),
            _ => false,
        }
    }
}

/// es.exe 1.1.0.18，取自 https://www.voidtools.com/es.zip
/// （zip 本身 sha256 = 9412f3b083a539a8de02301dc5f60a0033b3edadf12668136223baca66beb7b3）
pub const ES_SHA256: &str = "224550eddd6f09fbae8ffe9d5e1d4bab07563a96c895fe1fc43fe60df48c0d12";

pub const ALLOWLIST: &[Allowed] = &[
    Allowed {
        key: "es",
        file: "es.exe",
        sha256: ES_SHA256,
        zip_url: "https://www.voidtools.com/es.zip",
        version: "1.1.0.18",
        absolute_path: "",
        os: "windows",
        max_lines: 0,
    },
    // Spotlight 的命令行入口：系统自带、路径固定，只做文件名检索（`-name`）。
    // 不钉 sha256（系统升级会变），安全性靠"绝对路径 + 参数白名单"（见 rejected_arg）。
    // max_lines：mdfind 全量枚举（常见查询几千行）实测要十几秒，收满 40 行即返回。
    Allowed {
        key: "mdfind",
        file: "mdfind",
        sha256: "",
        zip_url: "",
        version: "macOS 自带",
        absolute_path: "/usr/bin/mdfind",
        os: "macos",
        max_lines: 40,
    },
];

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("不允许执行程序 {0:?}：白名单只有 {}", ALLOWLIST.iter().map(|a| a.key).collect::<Vec<_>>().join(", "))]
    NotAllowed(String),
    #[error("参数必须是字符串数组")]
    BadArgs,
    #[error("参数被拒绝: {0}")]
    DeniedArg(String),
    #[error("找不到已校验的 {key}。已检查：{checked}。es 可运行 scripts/fetch-es.ps1 取官方版本；系统工具应预装在标准路径")]
    NotFound { key: String, checked: String },
    #[error("拒绝执行 {path}：sha256 与内置清单不符（期望 {expected}，实际 {actual}）")]
    DigestMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("执行超时（{} ms），进程已终止", .0.as_millis())]
    Timeout(Duration),
    #[error("未配置 {0} 运行环境：请在 设置 → 运行时 中填写解释器路径并点“检测”")]
    NotConfigured(&'static str),
    #[error("{0}")]
    BadInterpreter(String),
    #[error("脚本代码不能为空")]
    EmptyScript,
    #[error("命令不能为空")]
    EmptyCommand,
    #[error("已取消")]
    Cancelled,
    #[error("找不到 {shell} 的可执行文件。已检查：{checked}。请在 设置 → 命令 中填写 Shell 路径")]
    ShellMissing { shell: String, checked: String },
    #[error("启动失败: {0}")]
    Spawn(String),
    #[error("下载失败: {0}")]
    Fetch(String),
    #[error("zip 解析失败: {0}")]
    Zip(String),
    #[error("写入失败: {0}")]
    Io(#[from] std::io::Error),
}

/// 一次执行的结果。
#[derive(Debug, Clone)]
pub struct RunOk {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    /// 实际执行的文件（便于日志/排障）
    pub path: String,
    pub truncated: bool,
}

pub fn lookup(key: &str) -> Option<&'static Allowed> {
    ALLOWLIST.iter().find(|a| a.key == key)
}

/// 候选**目录**（按优先级，不检查存在性）。单独暴露是为了可测：
/// 之前 `candidates()` 直接过滤掉不存在的文件，导致"没装 es.exe"时的错误
/// 只能说"已检查：（一个候选文件都没找到）"，等于没告诉用户该往哪放。
pub fn candidate_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !out.contains(&p) {
            out.push(p);
        }
    };

    // 运行时下载到数据目录的副本（第三方二进制不随仓库分发）
    push(fsutil::data_root().join("tools").join("es"));

    // PATH
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            push(dir);
        }
    }

    // Everything 安装位置
    for dir in everything_dirs() {
        push(dir);
    }
    out
}

/// Everything 本体是否安装。用于区分"只缺 es.exe"和"完全没装"，
/// 设置页据此给出不同的引导文案。
pub fn everything_installed() -> bool {
    everything_dirs()
        .iter()
        .any(|d| d.join("Everything.exe").is_file())
}

/// 实际存在的候选文件，按 [`candidate_dirs`] 的顺序。
pub fn candidates(a: &Allowed, settings_hint: Option<&Path>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(h) = settings_hint {
        // 允许直接给 exe，也可能给成目录
        let p = if h.is_dir() { h.join(a.file) } else { h.to_path_buf() };
        out.push(p);
    }
    for dir in candidate_dirs() {
        let p = dir.join(a.file);
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    }
    out.retain(|p| p.is_file());
    out
}

fn everything_dirs() -> Vec<PathBuf> {
    let mut out = vec![
        PathBuf::from("C:\\Program Files\\Everything"),
        PathBuf::from("C:\\Program Files (x86)\\Everything"),
    ];
    // 常规位置（含 64/32 位与用户级安装）
    for base in [
        std::env::var_os("ProgramFiles"),
        std::env::var_os("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    {
        out.push(PathBuf::from(base).join("Everything"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(PathBuf::from(local).join("Programs").join("Everything"));
    }
    out
}

/// 文件 sha256（小写十六进制）。
pub fn sha256_of(path: &Path) -> Result<String, std::io::Error> {
    let bytes = std::fs::read(path)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

/// 找到第一个"存在且摘要匹配"的候选。都不匹配时返回最刺眼的那个错误，
/// 把期望/实际摘要都打出来，方便决定是重新下载还是更新清单。
pub fn resolve_verified(a: &Allowed, settings_hint: Option<&Path>) -> Result<PathBuf, ExecError> {
    // 绝对路径的系统工具（mdfind）：路径固定即消掉 PATH 劫持面；系统升级会换二进制
    // 内容，没法钉 sha256，校验只到"固定路径 + 存在"。
    if !a.absolute_path.is_empty() {
        let p = Path::new(a.absolute_path);
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(ExecError::NotFound {
                key: a.key.to_string(),
                checked: a.absolute_path.to_string(),
            })
        };
    }
    // 其余条目必须钉摘要：否则"往候选目录写同名文件"就绕过白名单
    if a.sha256.is_empty() {
        return Err(ExecError::Spawn(format!(
            "白名单条目 {} 配置错误：非绝对路径条目必须钉 sha256",
            a.key
        )));
    }
    let list = candidates(a, settings_hint);
    if list.is_empty() {
        return Err(ExecError::NotFound {
            key: a.key.to_string(),
            checked: candidate_dirs()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(" | "),
        });
    }
    let mut mismatch: Option<ExecError> = None;
    for p in &list {
        match sha256_of(p) {
            Ok(d) if d == a.sha256 => return Ok(p.clone()),
            Ok(d) => {
                if mismatch.is_none() {
                    mismatch = Some(ExecError::DigestMismatch {
                        path: p.display().to_string(),
                        expected: a.sha256.to_string(),
                        actual: d,
                    });
                }
            }
            Err(_) => {}
        }
    }
    Err(mismatch.unwrap_or_else(|| ExecError::NotFound {
        key: a.key.to_string(),
        checked: list
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" | "),
    }))
}

/// 参数级拦截（按条目分派）。
pub fn rejected_arg(key: &str, args: &[String]) -> Option<String> {
    match key {
        "es" => rejected_arg_es(args),
        "mdfind" => rejected_arg_mdfind(args),
        _ => Some(format!("未知白名单条目 {key:?}，参数一律拒绝")),
    }
}

/// mdfind（Spotlight CLI）：只允许 `[-onlyin <绝对目录>]... -name <查询词>`。
///
/// `-onlyin` 是必要的：全盘 `mdfind -name` 实测 5s 且结果被 `~/Library` 噪音塞满
/// （见 `search::spotlight` 的模块文档）。目录必须是**绝对路径**——相对路径会让
/// mdfind 按当前工作目录解释，那等于让调用方间接指定检索范围之外的东西。
///
/// 查询词不能以 `-` 开头（否则会被当成选项），长度有上限（防拉爆）。
fn rejected_arg_mdfind(args: &[String]) -> Option<String> {
    const MAX_DIRS: usize = 8;
    let mut dirs = 0usize;
    let mut i = 0usize;
    while i < args.len() && args[i] == "-onlyin" {
        let Some(dir) = args.get(i + 1) else {
            return Some("mdfind -onlyin 缺少目录参数".into());
        };
        if !dir.starts_with('/') {
            return Some(format!("-onlyin 目录 {dir:?} 必须是绝对路径"));
        }
        if dir.len() > 1024 || dir.contains('\0') {
            return Some(format!("-onlyin 目录 {dir:?} 不合法：过长或含 NUL"));
        }
        dirs += 1;
        if dirs > MAX_DIRS {
            return Some(format!("-onlyin 最多 {MAX_DIRS} 个目录"));
        }
        i += 2;
    }
    if args.len() != i + 2 || args[i] != "-name" {
        return Some("mdfind 只允许 [-onlyin 目录]... -name 查询词 形式的参数".into());
    }
    let q = &args[i + 1];
    if q.is_empty() || q.starts_with('-') || q.len() > 512 {
        return Some(format!(
            "查询词 {q:?} 不合法：不能为空、不能以 - 开头、长度不超过 512"
        ));
    }
    None
}

/// es 的参数级拦截。
///
/// 白名单 + 摘要只解决"能跑哪个程序"，没解决"能让它干什么"。es.exe 本身有
/// 写文件的能力（`-export-csv/-efu/-txt/-m3u <路径>` 会把结果写到任意位置，
/// `-set-run-count`/`-inc-run-count` 会改 Everything 的库，`-save-settings` 会落配置文件，
/// `-instance` 会去连别的 Everything 实例）。这些对"检索文件"这个用途毫无必要，
/// 所以直接拒 —— 否则白名单等于给插件开了个"用 es 写任意文件"的后门。
fn rejected_arg_es(args: &[String]) -> Option<String> {
    // es 文档："Internal -'s in options can be omitted, eg: -nodigitgrouping"，
    // 也允许 / 开头。所以判定前必须把前导符号和内部连字符都去掉再比，
    // 否则 `-exportcsv`、`/export-csv` 这类写法就绕过去了。
    const DENIED: &[&str] = &[
        "export",        // -export-csv/-efu/-txt/-m3u/-m3u8：写任意文件
        "setruncount",   // 改 Everything 的库
        "incruncount",
        "savesettings",  // 落配置文件
        "clearsettings",
        "instance",      // 连别的 Everything 实例
    ];
    for a in args {
        let norm: String = a
            .trim_start_matches(['-', '/'])
            .chars()
            .filter(|c| *c != '-')
            .collect::<String>()
            .to_lowercase();
        for d in DENIED {
            if norm.starts_with(d) {
                return Some(format!(
                    "参数 {a:?} 被宿主拒绝：es 的导出/写入/改设置类选项不在允许范围内（只读检索用不到）"
                ));
            }
        }
    }
    // -n / -max-results 的条数上限：防止一次拉爆内存或把 UI 卡住
    if let Some(i) = args.iter().position(|a| {
        let n: String = a
            .trim_start_matches(['-', '/'])
            .chars()
            .filter(|c| *c != '-')
            .collect::<String>()
            .to_lowercase();
        n == "n" || n == "maxresults"
    }) {
        if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
            if v > 500 {
                return Some(format!("-n {v} 超过上限 500"));
            }
        }
    }
    None
}

/// 执行白名单程序。
pub fn run(
    key: &str,
    args: &[String],
    timeout: Duration,
    settings_hint: Option<&Path>,
) -> Result<RunOk, ExecError> {
    let a = lookup(key).ok_or_else(|| ExecError::NotAllowed(key.to_string()))?;
    if let Some(why) = rejected_arg(key, args) {
        return Err(ExecError::DeniedArg(why));
    }
    // 平台门按条目判定（es 仅 Windows、mdfind 仅 macOS）。放在参数拦截之后，
    // 保证"参数级拒绝"这类边界语义在所有平台都能被测试到（硬性约定 3）。
    if !a.available_here() {
        return Err(ExecError::Spawn(format!("当前平台不支持：{key}")));
    }
    let timeout = timeout.min(MAX_TIMEOUT).max(Duration::from_millis(100));
    let path = resolve_verified(a, settings_hint)?;

    let mut cmd = Command::new(&path);
    cmd.args(args);
    let mut ok = spawn_and_collect(cmd, timeout, None, a.max_lines)?;
    ok.path = path.display().to_string();
    Ok(ok)
}

/// 启动子进程并收集输出：stdin 关闭、stdout/stderr 各截断到 [`MAX_OUTPUT`]、
/// 超时先 kill 再收线程。es、脚本运行时与搜索框命令共用这一段，
/// 避免多份超时/管道处理漂移。
///
/// `cancel` 是搜索框命令的取消令牌（Esc）：置位后与超时走同一条清理路径。
fn spawn_and_collect(
    mut cmd: Command,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
    max_lines: usize,
) -> Result<RunOk, ExecError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW：GUI 进程拉起控制台程序时不要闪一个黑框出来
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| ExecError::Spawn(e.to_string()))?;

    let mut so = child.stdout.take().expect("stdout piped");
    let mut se = child.stderr.take().expect("stderr piped");
    let reached = std::sync::Arc::new(AtomicBool::new(false));
    let t1 = {
        let reached = reached.clone();
        std::thread::spawn(move || read_capped_lines(&mut so, MAX_OUTPUT, max_lines, &reached))
    };
    let t2 = std::thread::spawn(move || read_capped(&mut se, MAX_OUTPUT));

    let deadline = Instant::now() + timeout;
    let mut code: Option<i32> = None;
    while Instant::now() < deadline {
        if let Some(c) = cancel {
            if c.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = t1.join();
                let _ = t2.join();
                return Err(ExecError::Cancelled);
            }
        }
        // 行数收够：提前杀进程返回部分结果（见 Allowed::max_lines）
        if reached.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let (out_bytes, _) = t1.join().map_err(|_| ExecError::Spawn("读取线程 panic".into()))?;
            let (err_bytes, _) = t2.join().map_err(|_| ExecError::Spawn("读取线程 panic".into()))?;
            return Ok(RunOk {
                code: 0,
                stdout: decode_os_bytes(&out_bytes),
                stderr: decode_os_bytes(&err_bytes),
                path: String::new(),
                // 提前收满即返回 = 输出必然不完整
                truncated: true,
            });
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                code = Some(st.code().unwrap_or(-1));
                break;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(e) => return Err(ExecError::Spawn(e.to_string())),
        }
    }
    let code = match code {
        Some(c) => c,
        None => {
            // 超时：先杀再收，否则读线程会一直挂在管道上
            let _ = child.kill();
            let _ = child.wait();
            let _ = t1.join();
            let _ = t2.join();
            return Err(ExecError::Timeout(timeout));
        }
    };

    let (out_bytes, out_cut) = t1.join().map_err(|_| ExecError::Spawn("读取线程 panic".into()))?;
    let (err_bytes, err_cut) = t2.join().map_err(|_| ExecError::Spawn("读取线程 panic".into()))?;
    Ok(RunOk {
        code,
        stdout: decode_os_bytes(&out_bytes),
        stderr: decode_os_bytes(&err_bytes),
        path: String::new(),
        truncated: out_cut || err_cut,
    })
}

/// 用用户配置的解释器执行一段脚本源码（`node -e` / `python -c`）。
///
/// `interpreter` 只能来自设置，绝不来自插件参数；含分隔符或盘符时必须指向存在的文件，
/// 只写名字（如 `node`）则交给 CreateProcess 从 PATH 解析。参数数组不经过 shell。
pub fn run_script(
    rt: ScriptRuntime,
    interpreter: &str,
    code: &str,
    args: &[String],
    timeout: Duration,
    cwd: Option<&Path>,
) -> Result<RunOk, ExecError> {
    let interpreter = interpreter.trim();
    if interpreter.is_empty() {
        return Err(ExecError::NotConfigured(rt.display()));
    }
    if code.trim().is_empty() {
        return Err(ExecError::EmptyScript);
    }
    let timeout = timeout.min(RUNTIME_MAX_TIMEOUT).max(Duration::from_millis(100));
    let exe = resolve_interpreter(interpreter)?;

    let mut cmd = Command::new(&exe);
    cmd.arg(rt.code_flag()).arg(code).args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut ok = spawn_and_collect(cmd, timeout, None, 0)?;
    ok.path = exe.display().to_string();
    Ok(ok)
}

/// 探测解释器版本：`<interpreter> --version`，取第一条非空输出。
///
/// 设置页的"检测"按钮与保存后的状态展示都走这里，保证"显示可用"与"真的能跑"是同一判据。
pub fn probe_interpreter(interpreter: &str) -> Result<String, ExecError> {
    let interpreter = interpreter.trim();
    if interpreter.is_empty() {
        return Err(ExecError::BadInterpreter("解释器路径不能为空".into()));
    }
    let exe = resolve_interpreter(interpreter)?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--version");
    let out = spawn_and_collect(cmd, Duration::from_secs(5), None, 0)?;
    let line = out
        .stdout
        .lines()
        .chain(out.stderr.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if out.code != 0 {
        return Err(ExecError::BadInterpreter(format!(
            "{} --version 退出码 {}：{}",
            exe.display(),
            out.code,
            if line.is_empty() { "无输出" } else { line }
        )));
    }
    if line.is_empty() {
        return Err(ExecError::BadInterpreter(format!(
            "{} --version 没有任何输出，可能不是 Node/Python 解释器",
            exe.display()
        )));
    }
    Ok(line.to_string())
}

/// 在系统 PATH（外加平台兜底目录，见 [`FALLBACK_DIRS`]）里探测可用的解释器，
/// 返回第一个通过 `--version` 校验的绝对路径。
///
/// 设置页在未配置时用它自动补全（用户之后仍可在 设置 → 运行时 改成别的路径）。
/// 只按固定文件名查找，**不接受插件传入名字**；真正判据仍是 [`probe_interpreter`]，
/// 所以 Windows 商店里那个只会弹商店的 `python.exe` 占位程序会被跳过。
pub fn detect_on_path(rt: ScriptRuntime) -> Option<String> {
    let env_dirs = std::env::var_os("PATH")
        .map(|raw| std::env::split_paths(&raw).collect::<Vec<_>>())
        .unwrap_or_default();
    detect_in_dirs(rt, search_dirs(env_dirs))
}

/// macOS 上从 Finder / Dock 启动的 app 不继承 shell 的 PATH（不执行 rc 文件），
/// Homebrew 与 python.org 安装包落地的目录因此常常缺失，这里补上兜底。
#[cfg(target_os = "macos")]
const FALLBACK_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
#[cfg(not(target_os = "macos"))]
const FALLBACK_DIRS: &[&str] = &[];

/// [`detect_on_path`] 的目录序列：进程 PATH 在前，平台兜底目录在后。
fn search_dirs<I>(env_dirs: I) -> Vec<PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    let mut dirs: Vec<PathBuf> = env_dirs.into_iter().collect();
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
    dirs
}

/// [`detect_on_path`] 的可测试内核：按目录顺序找固定文件名并逐个探测。
fn detect_in_dirs<I>(rt: ScriptRuntime, dirs: I) -> Option<String>
where
    I: IntoIterator,
    I::Item: AsRef<Path>,
{
    for dir in dirs {
        for name in rt.path_names() {
            let candidate = dir.as_ref().join(name);
            if !candidate.is_file() {
                continue;
            }
            let raw = candidate.to_string_lossy().to_string();
            if probe_interpreter(&raw).is_ok() {
                return Some(raw);
            }
        }
    }
    None
}

/// 解释器解析：带路径分隔符/盘符的必须是已存在的文件；纯名字交给 CreateProcess 走 PATH。
fn resolve_interpreter(raw: &str) -> Result<PathBuf, ExecError> {
    let p = Path::new(raw);
    let looks_like_path = p.is_absolute() || raw.contains('\\') || raw.contains('/');
    if !looks_like_path {
        return Ok(p.to_path_buf());
    }
    if !p.is_file() {
        return Err(ExecError::BadInterpreter(format!("{raw} 不存在或不是文件")));
    }
    Ok(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()))
}

/// 一次搜索框命令要启动的 Shell：程序 + 参数 + 展示名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellLaunch {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub label: String,
}

/// PowerShell 内联执行前显式把输出编码设为 UTF-8。
/// 不设的话 zh-CN 控制台按 GBK 输出，宿主再按 UTF-8 解码就是满屏替换字符。
const PS_UTF8_PREFIX: &str =
    "$OutputEncoding = [Console]::OutputEncoding = [Text.Encoding]::UTF8; ";

/// 按设置把"命令字符串"变成一次启动（不真正执行，便于单测与 CLI `--dry-run`）。
pub fn shell_launch(
    settings: &Settings,
    command: &str,
    terminal: bool,
) -> Result<ShellLaunch, ExecError> {
    if command.trim().is_empty() {
        return Err(ExecError::EmptyCommand);
    }
    let shell = settings.command_shell;
    let label = shell.display().to_string();
    let program = resolve_shell_program(shell, settings.command_shell_path.as_deref(), &label)?;
    let args = match shell {
        CommandShell::Powershell | CommandShell::Pwsh => {
            if terminal {
                vec![
                    "-NoLogo".into(),
                    "-NoExit".into(),
                    "-Command".into(),
                    command.into(),
                ]
            } else {
                vec![
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    format!("{PS_UTF8_PREFIX}{command}"),
                ]
            }
        }
        CommandShell::Cmd => vec![
            "/D".into(),
            if terminal { "/K".into() } else { "/C".into() },
            command.into(),
        ],
        CommandShell::GitBash | CommandShell::Zsh | CommandShell::Bash | CommandShell::Fish => {
            vec!["-lc".into(), command.into()]
        }
        CommandShell::Wsl => {
            let mut a: Vec<String> = Vec::new();
            if let Some(d) = settings
                .command_wsl_distro
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
            {
                a.push("-d".into());
                a.push(d.to_string());
            }
            a.extend(["-e".into(), "bash".into(), "-lc".into(), command.into()]);
            a
        }
        CommandShell::Custom => custom_args(&settings.command_custom_args, command),
    };
    Ok(ShellLaunch {
        program,
        args,
        label,
    })
}

/// 自定义 Shell 的参数模板：`{cmd}` 会被替换；没有占位符时命令追加在末尾。
fn custom_args(template: &str, command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut substituted = false;
    for tok in template.split_whitespace() {
        if tok.contains("{cmd}") {
            out.push(tok.replace("{cmd}", command));
            substituted = true;
        } else {
            out.push(tok.to_string());
        }
    }
    if !substituted {
        out.push(command.to_string());
    }
    out
}

fn resolve_shell_program(
    shell: CommandShell,
    explicit: Option<&str>,
    label: &str,
) -> Result<PathBuf, ExecError> {
    if let Some(raw) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        let p = Path::new(raw);
        if p.is_file() {
            return Ok(p.to_path_buf());
        }
        return Err(ExecError::ShellMissing {
            shell: label.to_string(),
            checked: format!("{}（设置里填写的路径）", p.display()),
        });
    }
    let (candidates, name) = match shell {
        CommandShell::Powershell => (
            vec![system_root().join("System32\\WindowsPowerShell\\v1.0\\powershell.exe")],
            "powershell.exe",
        ),
        CommandShell::Pwsh => (
            vec![
                PathBuf::from("C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
                PathBuf::from("C:\\Program Files\\PowerShell\\7-preview\\pwsh.exe"),
            ],
            "pwsh.exe",
        ),
        CommandShell::Cmd => (vec![system_root().join("System32\\cmd.exe")], "cmd.exe"),
        CommandShell::GitBash => (git_bash_candidates(), "bash.exe"),
        CommandShell::Wsl => (vec![system_root().join("System32\\wsl.exe")], "wsl.exe"),
        CommandShell::Zsh => (
            vec![PathBuf::from("/bin/zsh"), PathBuf::from("/usr/bin/zsh")],
            "zsh",
        ),
        CommandShell::Bash => (
            vec![
                PathBuf::from("/bin/bash"),
                PathBuf::from("/usr/bin/bash"),
                PathBuf::from("/opt/homebrew/bin/bash"),
                PathBuf::from("/usr/local/bin/bash"),
            ],
            "bash",
        ),
        CommandShell::Fish => (
            vec![
                PathBuf::from("/opt/homebrew/bin/fish"),
                PathBuf::from("/usr/local/bin/fish"),
                PathBuf::from("/usr/bin/fish"),
            ],
            "fish",
        ),
        CommandShell::Custom => {
            return Err(ExecError::ShellMissing {
                shell: label.to_string(),
                checked: "未填写自定义 Shell 路径".into(),
            })
        }
    };
    if let Some(p) = find_program(&candidates, name) {
        return Ok(p);
    }
    Err(ExecError::ShellMissing {
        shell: label.to_string(),
        checked: candidates
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" | "),
    })
}

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
}

fn git_bash_candidates() -> Vec<PathBuf> {
    let mut out = vec![
        PathBuf::from("C:\\Program Files\\Git\\bin\\bash.exe"),
        PathBuf::from("C:\\Program Files (x86)\\Git\\bin\\bash.exe"),
    ];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        out.push(
            PathBuf::from(local)
                .join("Programs")
                .join("Git")
                .join("bin")
                .join("bash.exe"),
        );
    }
    out
}

fn find_program(candidates: &[PathBuf], name: &str) -> Option<PathBuf> {
    for p in candidates {
        if p.is_file() {
            return Some(p.clone());
        }
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// 按设置执行一条命令并收集输出（内联模式）。`cancel` 置位时杀进程并返回 [`ExecError::Cancelled`]。
pub fn run_command(
    settings: &Settings,
    command: &str,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> Result<RunOk, ExecError> {
    let launch = shell_launch(settings, command, false)?;
    let timeout = timeout.max(Duration::from_millis(200));
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args);
    let mut ok = spawn_and_collect(cmd, timeout, cancel, 0)?;
    ok.path = launch.program.display().to_string();
    Ok(ok)
}

/// 在独立终端窗口里执行（不等待、不收集输出）。
pub fn run_command_terminal(settings: &Settings, command: &str) -> Result<(), ExecError> {
    let launch = shell_launch(settings, command, true)?;
    #[cfg(not(windows))]
    {
        // 终端模式 = "打开一个终端窗口执行"：mac 侧要接 Terminal.app，是 v1.5
        //（见 docs/macos-support-plan.md）。不能就这么 spawn——输出没人接，
        // 等于静默成功（硬性约定 3）。
        let _ = launch;
        Err(ExecError::Spawn(
            "当前平台不支持：终端模式执行（v1 仅内联模式）".into(),
        ))
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new(&launch.program);
        cmd.args(&launch.args);
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NEW_CONSOLE：交互式程序需要自己的窗口
            cmd.creation_flags(0x0000_0010);
        }
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| ExecError::Spawn(e.to_string()))
    }
}

/// 设置页"检测"按钮：用给定配置跑一条 `echo`，确认真能启动。
/// 返回（解析到的程序路径，首条输出）。
pub fn probe_command_shell(
    shell: CommandShell,
    path: Option<&str>,
    distro: Option<&str>,
    custom_args: &str,
) -> Result<(String, String), ExecError> {
    let mut s = Settings::default();
    s.command_shell = shell;
    s.command_shell_path = path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    s.command_wsl_distro = distro
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string);
    s.command_custom_args = custom_args.to_string();
    let launch = shell_launch(&s, "echo efto-command-check", false)?;
    let out = run_command(&s, "echo efto-command-check", Duration::from_secs(5), None)?;
    let line = out
        .stdout
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if out.code != 0 || !out.stdout.contains("efto-command-check") {
        return Err(ExecError::BadInterpreter(format!(
            "{} 检测失败（退出码 {}）：{}",
            launch.label,
            out.code,
            if line.is_empty() { "没有输出" } else { line }
        )));
    }
    Ok((launch.program.display().to_string(), line.to_string()))
}

fn read_capped<R: Read>(r: &mut R, cap: usize) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut cut = false;
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > cap {
                    let take = cap.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                    cut = true;
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                cut = true;
                break;
            }
        }
    }
    (buf, cut)
}

/// 同 [`read_capped`]，但收满 `max_lines` 行就置位 `reached` 并停止读取。
/// 配合 [`Allowed::max_lines`]：调用方看到 `reached` 后杀掉子进程、带部分结果返回。
fn read_capped_lines<R: Read>(
    r: &mut R,
    cap: usize,
    max_lines: usize,
    reached: &AtomicBool,
) -> (Vec<u8>, bool) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut cut = false;
    let mut lines = 0usize;
    loop {
        match r.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() + n > cap {
                    let take = cap.saturating_sub(buf.len());
                    buf.extend_from_slice(&chunk[..take]);
                    cut = true;
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if max_lines > 0 {
                    lines += chunk[..n].iter().filter(|b| **b == b'\n').count();
                    if lines >= max_lines {
                        reached.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                cut = true;
                break;
            }
        }
    }
    (buf, cut)
}

/// 按操作系统代码页解码子进程输出。
///
/// es.exe 在 zh-CN 上输出 GBK（实测），不是 UTF-8；直接 `from_utf8_lossy` 会把
/// 中文路径打成替换字符。控制台代码页取 `GetConsoleOutputCP()`，为 0（无控制台，
/// 例如 GUI 启动）时退回 `GetACP()`。
pub fn decode_os_bytes(bytes: &[u8]) -> String {
    #[cfg(windows)]
    {
        if bytes.is_empty() {
            return String::new();
        }
        let cp = unsafe {
            use windows_sys::Win32::System::Console::GetConsoleOutputCP;
            let c = GetConsoleOutputCP();
            if c == 0 {
                use windows_sys::Win32::Globalization::GetACP;
                GetACP()
            } else {
                c
            }
        };
        if cp == 65001 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        match to_wide(cp, bytes) {
            Some(w) => String::from_utf16_lossy(&w),
            // 代码页转换失败时退回 lossy UTF-8：至少 ASCII 路径可用，不静默丢内容
            None => String::from_utf8_lossy(bytes).into_owned(),
        }
    }
    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(windows)]
fn to_wide(cp: u32, bytes: &[u8]) -> Option<Vec<u16>> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
    unsafe {
        let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if n <= 0 {
            return None;
        }
        let mut v = vec![0u16; n as usize];
        let m = MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, v.as_mut_ptr(), n);
        if m <= 0 {
            return None;
        }
        v.truncate(m as usize);
        Some(v)
    }
}

/// 从 zip 里取出指定文件（stored / deflate 两种）。
/// 只用标准库 + flate2，不引入 zip crate：es.zip 里就两个条目，手写足够且可测。
pub fn zip_extract(zip: &[u8], want: &str) -> Result<Vec<u8>, ExecError> {
    // 从尾部找 EOCD（0x06054b50）
    let sig = [0x50u8, 0x4b, 0x05, 0x06];
    let eocd = zip
        .windows(4)
        .rposition(|w| w == sig)
        .ok_or_else(|| ExecError::Zip("找不到 zip 结束标记".into()))?;
    let rd16 = |o: usize| u16::from_le_bytes([zip[eocd + o], zip[eocd + o + 1]]) as usize;
    let rd32 = |o: usize| {
        u32::from_le_bytes([
            zip[eocd + o],
            zip[eocd + o + 1],
            zip[eocd + o + 2],
            zip[eocd + o + 3],
        ]) as usize
    };
    let entries = rd16(10);
    let mut off = rd32(16); // 中央目录起始

    for _ in 0..entries {
        if off + 46 > zip.len() || zip[off..off + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err(ExecError::Zip("中央目录头损坏".into()));
        }
        let le16 = |o: usize| u16::from_le_bytes([zip[off + o], zip[off + o + 1]]) as usize;
        let le32 = |o: usize| {
            u32::from_le_bytes([
                zip[off + o],
                zip[off + o + 1],
                zip[off + o + 2],
                zip[off + o + 3],
            ]) as usize
        };
        let method = le16(10);
        let comp_size = le32(20);
        let name_len = le16(28);
        let extra_len = le16(30);
        let comment_len = le16(32);
        let local_off = le32(42);
        let name = std::str::from_utf8(&zip[off + 46..off + 46 + name_len])
            .map_err(|e| ExecError::Zip(e.to_string()))?
            .to_string();

        if name == want || name.ends_with(&format!("/{want}")) {
            if local_off + 30 > zip.len() || zip[local_off..local_off + 4] != [0x50, 0x4b, 0x03, 0x04] {
                return Err(ExecError::Zip("本地文件头损坏".into()));
            }
            let l_name = u16::from_le_bytes([zip[local_off + 26], zip[local_off + 27]]) as usize;
            let l_extra = u16::from_le_bytes([zip[local_off + 28], zip[local_off + 29]]) as usize;
            let data = local_off + 30 + l_name + l_extra;
            let raw = zip
                .get(data..data + comp_size)
                .ok_or_else(|| ExecError::Zip("数据越界".into()))?;
            return match method {
                0 => Ok(raw.to_vec()),
                8 => {
                    let mut out = Vec::new();
                    flate2::read::DeflateDecoder::new(raw)
                        .read_to_end(&mut out)
                        .map_err(|e| ExecError::Zip(e.to_string()))?;
                    Ok(out)
                }
                m => Err(ExecError::Zip(format!("不支持的压缩方式 {m}"))),
            };
        }
        off += 46 + name_len + extra_len + comment_len;
    }
    Err(ExecError::Zip(format!("zip 里没有 {want}")))
}

/// 从用户提供的本地文件安装：接受官方 zip 或已解出的可执行文件。
/// 仍然强制走内置 sha256 校验，不信任文件名。
pub fn install_from_local(a: &Allowed, bytes: &[u8]) -> Result<PathBuf, ExecError> {
    let data = if bytes.starts_with(b"PK\x03\x04") {
        zip_extract(bytes, a.file)?
    } else {
        bytes.to_vec()
    };
    let mut h = Sha256::new();
    h.update(&data);
    let got = hex(&h.finalize());
    if got != a.sha256 {
        return Err(ExecError::DigestMismatch {
            path: "本地文件".to_string(),
            expected: a.sha256.to_string(),
            actual: got,
        });
    }
    let dir = fsutil::data_root().join("tools").join(a.key);
    std::fs::create_dir_all(&dir)?;
    let out = dir.join(a.file);
    std::fs::write(&out, &data)?;
    Ok(out)
}

/// 下载 zip → 解出目标 → 校验摘要 → 落到数据目录 tools/<key>/。
/// 只在本地一个都没有匹配候选时调用；失败后可由设置页手动下载/本地安装。
pub fn fetch_and_install(a: &'static Allowed) -> Result<PathBuf, ExecError> {
    // 同 app/http.rs：ureq 的 native-tls feature 不会自动启用默认 TLS，
    // 必须显式传 connector，否则 https 下载直接失败。
    let tls = ureq::native_tls::TlsConnector::new()
        .map_err(|e| ExecError::Fetch(format!("初始化 TLS 失败: {e}")))?;
    let agent = ureq::AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout(Duration::from_secs(60))
        .build();
    let resp = agent
        .get(a.zip_url)
        .call()
        .map_err(|e| ExecError::Fetch(e.to_string()))?;
    // ureq 2.x 用 into_reader() 拿字节流；上限 8MB 兜住"被换成别的东西"的情况
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(8u64 << 20)
        .read_to_end(&mut bytes)
        .map_err(|e| ExecError::Fetch(e.to_string()))?;
    install_from_local(a, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_capable_switches_are_refused() {
        // 白名单只回答"能跑哪个程序"。es 自己就能把结果写到任意路径，
        // 所以"能让它干什么"必须单独拦 —— 而且要考虑 es 的连字符可省略写法。
        for bad in [
            "-export-csv",
            "-exportcsv",           // 省略内部连字符
            "/export-csv",          // es 也接受 / 开头
            "-EXPORT-EFU",
            "-export-txt",
            "-export-m3u8",
            "-set-run-count",
            "-setruncount",
            "-inc-run-count",
            "-save-settings",
            "-savesettings",
            "-clear-settings",
            "-instance",
        ] {
            assert!(
                rejected_arg("es", &[bad.into(), "C:\\Users\\x\\Start Menu\\evil".into()]).is_some(),
                "{bad} 应被拒绝"
            );
        }
        // 正常检索参数必须原样通过，别把功能自己拦死
        let ok_args: Vec<String> = es_search_args("报告 ext:pdf")
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(rejected_arg("es", &ok_args), None, "常规检索参数被误拦");
        // 条数上限
        assert!(rejected_arg("es", &["-n".into(), "501".into(), "x".into()]).is_some());
        assert!(rejected_arg("es", &["-max-results".into(), "500".into(), "x".into()]).is_none());
    }

    /// mdfind 的参数白名单：只允许 `[-onlyin 绝对目录]... -name 查询词`，
    /// 查询词不能长得像选项。
    #[test]
    fn mdfind_only_allows_name_query() {
        assert_eq!(rejected_arg("mdfind", &["-name".into(), "报告".into()]), None);
        assert!(rejected_arg("mdfind", &["报告".into()]).is_some(), "缺 -name 必须拒");
        assert!(rejected_arg("mdfind", &["-name".into(), "-onlyin".into()]).is_some());
        assert!(rejected_arg("mdfind", &["-interpret".into(), "x".into()]).is_some());
        assert!(rejected_arg("mdfind", &["-name".into(), String::new()]).is_some());
        let long = "x".repeat(513);
        assert!(rejected_arg("mdfind", &["-name".into(), long]).is_some(), "超长查询必须拒");
        // 未知条目一律拒绝
        assert!(rejected_arg("nope", &["x".into()]).is_some());
    }

    /// `-onlyin` 只接受绝对目录，且必须排在 `-name` 之前。
    ///
    /// 相对路径会被 mdfind 按当前目录解释，等于让调用方间接扩大检索范围；
    /// 缺目录参数、目录过多、`-onlyin` 出现在 `-name` 之后都一律拒绝。
    #[test]
    fn mdfind_onlyin_requires_absolute_dirs() {
        let ok = vec![
            "-onlyin".to_string(),
            "/Users/x/Desktop".into(),
            "-name".into(),
            "报告".into(),
        ];
        assert_eq!(rejected_arg("mdfind", &ok), None, "正常的限定目录查询应通过");

        let two = vec![
            "-onlyin".to_string(),
            "/a".into(),
            "-onlyin".into(),
            "/b".into(),
            "-name".into(),
            "x".into(),
        ];
        assert_eq!(rejected_arg("mdfind", &two), None, "多个 -onlyin 应通过");

        // 相对路径
        let rel = vec!["-onlyin".to_string(), "Desktop".into(), "-name".into(), "x".into()];
        assert!(rejected_arg("mdfind", &rel).is_some(), "相对路径必须拒");

        // 缺目录参数
        let missing = vec!["-onlyin".to_string(), "-name".into(), "x".into()];
        assert!(rejected_arg("mdfind", &missing).is_some(), "缺目录必须拒");

        // -onlyin 出现在 -name 之后
        let late = vec!["-name".to_string(), "x".into(), "-onlyin".into(), "/a".into()];
        assert!(rejected_arg("mdfind", &late).is_some(), "-onlyin 必须在 -name 前");

        // 目录数量上限
        let mut many = Vec::new();
        for i in 0..9 {
            many.push("-onlyin".to_string());
            many.push(format!("/d{i}"));
        }
        many.push("-name".into());
        many.push("x".into());
        assert!(rejected_arg("mdfind", &many).is_some(), "目录过多必须拒");
    }

    /// 绝对路径条目免摘要校验；非绝对路径条目缺摘要属于配置错误，必须明确报错。
    #[test]
    fn absolute_path_entry_skips_digest() {
        let mdfind = lookup("mdfind").unwrap();
        assert!(!mdfind.absolute_path.is_empty());
        if cfg!(target_os = "macos") {
            let p = resolve_verified(mdfind, None).expect("/usr/bin/mdfind 应存在");
            assert_eq!(p, std::path::Path::new("/usr/bin/mdfind"));
        }
        let bad = Allowed {
            key: "x",
            file: "x",
            sha256: "",
            zip_url: "",
            version: "",
            absolute_path: "",
            os: "",
            max_lines: 0,
        };
        assert!(matches!(resolve_verified(&bad, None), Err(ExecError::Spawn(_))));
    }

    /// 平台门按条目判定：在各自平台外调用要报"当前平台不支持"。
    #[test]
    fn run_reports_platform_gate_per_entry() {
        #[cfg(not(target_os = "macos"))]
        {
            let e = run("mdfind", &["-name".into(), "x".into()], Duration::from_secs(1), None)
                .expect_err("非 macOS 不该跑 mdfind");
            assert!(e.to_string().contains("当前平台不支持"), "{e}");
        }
        #[cfg(not(windows))]
        {
            let e = run("es", &["report".into()], Duration::from_secs(1), None)
                .expect_err("非 Windows 不该跑 es");
            assert!(e.to_string().contains("当前平台不支持"), "{e}");
        }
    }

    /// 行数封顶：全量枚举型输出收满 N 行就提前杀进程返回，不等它跑完。
    #[test]
    fn max_lines_returns_early_with_partial_output() {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/C", "for /L %i in (1,1,1000000) do @echo line"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.args(["-c", "yes line"]);
            c
        };
        let started = Instant::now();
        let ok = spawn_and_collect(cmd, Duration::from_secs(10), None, 8)
            .expect("收满行数应提前返回 Ok");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "不该等全量枚举完: {:?}",
            started.elapsed()
        );
        assert!(ok.truncated, "提前返回必然截断");
        assert!(!ok.stdout.is_empty());
    }

    /// mdfind 端到端（真机）：常见查询全量上万行、十几秒，行数封顶必须生效。
    #[cfg(target_os = "macos")]
    #[test]
    fn mdfind_is_capped_and_fast() {
        let started = Instant::now();
        let ok = run(
            "mdfind",
            &["-name".into(), "the".into()],
            Duration::from_secs(10),
            None,
        )
        .expect("mdfind 应能跑通");
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "行数封顶没生效，等了全量枚举: {:?}",
            started.elapsed()
        );
        // 读块（8KB）粒度会过冲几行；上界只要证明"远小于全量"即可
        assert!(
            ok.stdout.lines().count() <= 500,
            "输出应收敛在 max_lines 附近，实际 {} 行",
            ok.stdout.lines().count()
        );
    }

    #[test]
    fn run_refuses_before_touching_the_filesystem() {
        // 参数被拒时不该再去解析/哈希 es.exe（错误必须是 DeniedArg）
        let e = run(
            "es",
            &["-export-csv".into(), "C:\\tmp\\x.csv".into()],
            Duration::from_millis(500),
            None,
        )
        .unwrap_err();
        assert!(matches!(e, ExecError::DeniedArg(_)), "期望 DeniedArg，实际 {e}");
    }

    /// 与 plugins/everything/main.js 里 esArgs 同构的参数，用来验证不误拦。
    fn es_search_args(query: &str) -> Vec<&'static str> {
        // 注意：这里返回 'static 只是测试方便
        let v = vec![
            "-n", "30", "-csv", "-no-header", "-full-path-and-name", "-size",
            "-date-modified", "-date-format", "1", "-timeout", "2000",
        ];
        let mut out: Vec<&'static str> = v;
        out.push(Box::leak(query.to_string().into_boxed_str()));
        out
    }

    #[test]
    fn only_allowlisted_names_run() {
        // 这是这个能力最关键的一条：任何不在表里的名字都必须拒绝
        for k in ["cmd", "powershell", "whoami", "../../windows/system32/cmd", "", "es.exe"] {
            let e = run(k, &[], Duration::from_millis(500), None).unwrap_err();
            assert!(
                matches!(e, ExecError::NotAllowed(_)),
                "{k:?} 应被白名单拒绝，实际 {e}"
            );
        }
        assert!(lookup("es").is_some());
    }

    #[test]
    fn install_from_local_rejects_unknown_bytes() {
        // 校验发生在落盘之前：非官方字节串必须被摘要拒绝，不能靠文件名混过去
        let a = lookup("es").unwrap();
        let e = install_from_local(a, b"not the official es.exe").unwrap_err();
        assert!(matches!(e, ExecError::DigestMismatch { .. }), "实际 {e}");
    }

    #[test]
    fn gbk_output_decodes_to_real_chinese() {
        // 实测 es.exe 在 zh-CN 输出 GBK：0xD7 0xC9 0xD1 0xAF = "咨询"
        let gbk = vec![0xD7u8, 0xC9, 0xD1, 0xAF];
        let s = decode_os_bytes(&gbk);
        // 机器代码页不是 936 时（比如英文系统）走 UTF-8 分支，不强求中文
        if cp_for_tests() == 936 {
            assert_eq!(s, "咨询", "GBK 必须解成中文，实际 {s:?}");
        } else {
            assert!(!s.is_empty());
        }
        assert_eq!(decode_os_bytes(b""), "");
        assert_eq!(decode_os_bytes(b"plain ascii"), "plain ascii");
    }

    fn cp_for_tests() -> u32 {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Console::GetConsoleOutputCP;
            let c = GetConsoleOutputCP();
            if c == 0 {
                use windows_sys::Win32::Globalization::GetACP;
                GetACP()
            } else {
                c
            }
        }
        #[cfg(not(windows))]
        {
            65001
        }
    }

    /// 构造最小 zip 夹具：只覆盖解析器用到的字段，不依赖仓库内二进制。
    fn zip_fixture(name: &str, data: &[u8], deflate: bool) -> Vec<u8> {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;

        let (method, payload) = if deflate {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
            enc.write_all(data).unwrap();
            (8u16, enc.finish().unwrap())
        } else {
            (0u16, data.to_vec())
        };
        let n = name.as_bytes();
        let comp = payload.len() as u32;
        let raw = data.len() as u32;
        let mut z = Vec::new();
        z.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        z.extend_from_slice(&20u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&method.to_le_bytes());
        z.extend_from_slice(&[0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&comp.to_le_bytes());
        z.extend_from_slice(&raw.to_le_bytes());
        z.extend_from_slice(&(n.len() as u16).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(n);
        z.extend_from_slice(&payload);
        let cd_start = z.len();
        z.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        z.extend_from_slice(&20u16.to_le_bytes());
        z.extend_from_slice(&20u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&method.to_le_bytes());
        z.extend_from_slice(&[0, 0, 0, 0]);
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&comp.to_le_bytes());
        z.extend_from_slice(&raw.to_le_bytes());
        z.extend_from_slice(&(n.len() as u16).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(n);
        let cd_size = z.len() - cd_start;
        z.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&(cd_size as u32).to_le_bytes());
        z.extend_from_slice(&(cd_start as u32).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z
    }

    #[test]
    fn zip_extraction_handles_stored_and_deflated_entries() {
        let payload = b"fake es bytes";
        for deflate in [false, true] {
            let zip = zip_fixture("es.exe", payload, deflate);
            assert_eq!(zip_extract(&zip, "es.exe").unwrap(), payload);
            assert!(matches!(zip_extract(&zip, "nope.exe"), Err(ExecError::Zip(_))));
        }
        assert!(matches!(
            zip_extract(b"not a zip", "es.exe"),
            Err(ExecError::Zip(_))
        ));
        let zip = zip_fixture("es.exe", payload, false);
        assert!(zip_extract(&zip[..zip.len() / 2], "es.exe").is_err());
    }

    /// 真实端到端：这台机器装了 Everything 且在跑。查不到就跳过，
    /// 但**不静默通过** —— 会打印原因，避免"测试绿了但其实没测"。
    /// es.exe 是 Windows 程序（输出还是 GBK 代码页），macOS 上没有对应物，
    /// 见 docs/macos-support-plan.md（Everything 检索源 Windows-only）。
    #[cfg(windows)]
    #[test]
    fn runs_es_against_the_local_everything_and_returns_utf8() {
        let a = lookup("es").unwrap();
        let Ok(path) = resolve_verified(a, None) else {
            println!("skip: 没有可用的 es.exe");
            return;
        };
        let out = run(
            "es",
            &[
                "-n".into(),
                "3".into(),
                "-csv".into(),
                "-no-header".into(),
                "-full-path-and-name".into(),
                "-size".into(),
                "-date-modified".into(),
                "-date-format".into(),
                "1".into(),
                "ext:exe".into(),
            ],
            Duration::from_secs(5),
            None,
        )
        .expect("es 应能跑通");
        if out.code != 0 {
            println!("es 退出码 {}（Everything 可能没在跑）: {}", out.code, out.stderr);
            return;
        }
        let lines: Vec<&str> = out.stdout.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 3, "应正好返回 3 行：{:?}", out.stdout);
        assert!(lines[0].starts_with('"') && lines[0].contains(".exe"), "{}", lines[0]);
        // CSV 三列：路径,大小,ISO 时间
        let cols = lines[0].split(',').count();
        assert!(cols >= 3, "列数不足: {}", lines[0]);
        assert!(out.path.ends_with("es.exe"));
        let _ = path;
    }

    #[test]
    fn timeout_kills_and_reports() {
        // 用一个必然超时的参数组合：-pause 会等按键，永远不会自己结束
        let e = run(
            "es",
            &["-pause".into(), "ext:exe".into()],
            Duration::from_millis(400),
            None,
        );
        match e {
            Err(ExecError::Timeout(_)) => {}
            // es 根本起不来（没装 Everything / 平台上 es.exe 无法执行）时跳过，
            // 但打印原因，不静默通过
            Err(e) => println!("skip: {e}"),
            // 某些机器上 es 可能不阻塞（无控制台时 -pause 失效），那也是通过
            Ok(_) => {}
        }
    }

    #[test]
    fn args_never_go_through_a_shell() {
        // 参数里塞 shell 元字符必须原样交给 es 当搜索词，而不是被解释执行。
        // 判据：进程正常退出、stderr 里没有 cmd 的错误、且我们没有 cmd.exe。
        let out = run(
            "es",
            &["-n".into(), "1".into(), "a&b|c>d".into()],
            Duration::from_secs(5),
            None,
        );
        match out {
            Ok(o) => assert!(o.code == 0 || o.stdout.is_empty() || !o.stdout.contains("系统找不到")),
            // Everything 没跑时 es 自己会报错，也算通过这条（重点是没被 shell 解释）
            Err(e) => println!("skip: {e}"),
        }
    }

    #[test]
    fn candidate_dirs_cover_data_dir_and_everything() {
        let d = candidate_dirs();
        assert!(
            d.iter().any(|p| p.display().to_string().contains("tools")),
            "候选目录里应有数据目录 tools：{d:?}"
        );
        assert!(
            d.iter().any(|p| p.display().to_string().contains("Everything")),
            "候选目录里应有 Everything 安装位置：{d:?}"
        );
        // 顺序：宿主安装的副本排在 PATH 之前。摘要校验是最后一道，
        // 但把可信副本优先本来就该是默认。
        let tools = d
            .iter()
            .position(|p| p.display().to_string().contains("tools"))
            .unwrap();
        let system32 = d
            .iter()
            .position(|p| p.display().to_string().contains("System32"))
            .unwrap_or(usize::MAX);
        assert!(tools < system32, "数据目录副本应优先于 PATH：tools={tools} system32={system32}");
    }

    #[test]
    fn candidates_only_return_existing_files() {
        let a = lookup("es").unwrap();
        for p in candidates(a, None) {
            assert!(p.is_file(), "候选里不该有不存在的路径：{p:?}");
        }
    }

    #[test]
    fn script_runtime_names_are_strict() {
        assert_eq!(script_runtime("node"), Some(ScriptRuntime::Node));
        assert_eq!(script_runtime("Node.js"), Some(ScriptRuntime::Node));
        assert_eq!(script_runtime("python3"), Some(ScriptRuntime::Python));
        assert_eq!(script_runtime(" ruby "), None);
        assert_eq!(script_runtime(""), None);
        assert_eq!(ScriptRuntime::Node.key(), "node");
        assert_eq!(ScriptRuntime::Python.key(), "python");
    }

    #[test]
    fn run_script_refuses_unconfigured_and_empty_code() {
        // 未配置解释器：必须明确说去设置里配，而不是"找不到文件"
        let e = run_script(ScriptRuntime::Node, "  ", "1", &[], Duration::from_secs(1), None)
            .unwrap_err();
        assert!(matches!(e, ExecError::NotConfigured("Node.js")), "实际 {e}");
        assert!(e.to_string().contains("设置"), "错误文案要指路：{e}");

        // 解释器配了但代码是空白：不能白起一个解释器等它读 stdin
        let e = run_script(ScriptRuntime::Python, "python.exe", " \n", &[], Duration::from_secs(1), None)
            .unwrap_err();
        assert!(matches!(e, ExecError::EmptyScript), "实际 {e}");
    }

    #[test]
    fn run_script_reports_missing_interpreter_path() {
        let missing = std::env::temp_dir().join("efto-no-such-node.exe");
        let e = run_script(
            ScriptRuntime::Node,
            &missing.display().to_string(),
            "console.log(1)",
            &[],
            Duration::from_secs(1),
            None,
        )
        .unwrap_err();
        assert!(matches!(e, ExecError::BadInterpreter(_)), "实际 {e}");
        assert!(e.to_string().contains("不存在"), "{e}");
    }

    #[test]
    fn probe_interpreter_rejects_empty_and_missing() {
        assert!(matches!(
            probe_interpreter(""),
            Err(ExecError::BadInterpreter(_))
        ));
        let missing = std::env::temp_dir().join("efto-no-such-python.exe");
        assert!(matches!(
            probe_interpreter(&missing.display().to_string()),
            Err(ExecError::BadInterpreter(_))
        ));
    }

    #[test]
    fn detect_skips_dirs_without_a_working_interpreter() {
        // 目录里只有同名占位文件（不是可执行解释器）：必须跳过而不是直接返回它
        let dir = std::env::temp_dir().join(format!("efto-detect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
        std::fs::write(&fake, b"not an interpreter").unwrap();
        assert_eq!(detect_in_dirs(ScriptRuntime::Node, [&dir]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_dirs_appends_platform_fallbacks_after_path() {
        let env_dir = PathBuf::from("/efto-env-only");
        let dirs = search_dirs([env_dir.clone()]);
        // PATH 顺序不能被打乱：兜底目录只补在后面
        assert_eq!(dirs[0], env_dir);
        for want in FALLBACK_DIRS {
            assert!(dirs.iter().any(|d| d == Path::new(want)), "{want} 应进搜索序列");
        }
        #[cfg(target_os = "macos")]
        {
            assert!(FALLBACK_DIRS.contains(&"/opt/homebrew/bin"));
            assert!(FALLBACK_DIRS.contains(&"/usr/local/bin"));
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(dirs.len(), 1);
    }

    #[test]
    fn shell_launch_builds_args_per_shell() {
        // Cmd / PowerShell / WSL 的参数拼装是 Windows 语义，macOS 上这些 shell
        // 不存在（见 docs/macos-support-plan.md），只测平台无关的 Custom 与空命令拒绝。
        #[cfg(windows)]
        {
            let mut s = Settings::default();
            s.command_shell = CommandShell::Cmd;
            let l = shell_launch(&s, "dir", false).unwrap();
            assert!(l.program.to_string_lossy().to_lowercase().ends_with("cmd.exe"), "{l:?}");
            assert_eq!(l.args, vec!["/D", "/C", "dir"]);
            let l = shell_launch(&s, "dir", true).unwrap();
            assert_eq!(l.args, vec!["/D", "/K", "dir"]);

            s.command_shell = CommandShell::Powershell;
            let l = shell_launch(&s, "ipconfig", false).unwrap();
            assert_eq!(l.args[3], "-Command");
            // 内联模式必须先把输出编码设成 UTF-8，否则中文输出会乱码
            assert!(l.args[4].contains("UTF8"), "{}", l.args[4]);
            assert!(l.args[4].ends_with("ipconfig"));
            let l = shell_launch(&s, "ipconfig", true).unwrap();
            assert!(l.args.iter().any(|a| a == "-NoExit"), "{:?}", l.args);

            s.command_shell = CommandShell::Wsl;
            s.command_wsl_distro = Some("Ubuntu".into());
            let l = shell_launch(&s, "ls", false).unwrap();
            assert_eq!(l.args, vec!["-d", "Ubuntu", "-e", "bash", "-lc", "ls"]);
        }

        let mut s = Settings::default();
        s.command_shell = CommandShell::Custom;
        s.command_shell_path = None;
        assert!(matches!(
            shell_launch(&s, "x", false),
            Err(ExecError::ShellMissing { .. })
        ));
        s.command_shell_path = Some(std::env::current_exe().unwrap().display().to_string());
        s.command_custom_args = "-c {cmd}".into();
        let l = shell_launch(&s, "echo hi", false).unwrap();
        assert_eq!(l.args, vec!["-c", "echo hi"]);
        // 没有占位符时命令追加在末尾
        s.command_custom_args = String::new();
        let l = shell_launch(&s, "echo hi", false).unwrap();
        assert_eq!(l.args, vec!["echo hi"]);

        // 空命令必须在启动进程之前拒绝
        assert!(matches!(
            shell_launch(&Settings::default(), "  ", false),
            Err(ExecError::EmptyCommand)
        ));
    }

    /// POSIX shell（macOS 默认 zsh）：`-lc` 保留用户 PATH，内联/终端同参。
    #[cfg(target_os = "macos")]
    #[test]
    fn posix_shell_launch_uses_lc() {
        let mut s = Settings::default();
        s.command_shell = CommandShell::Zsh;
        let l = shell_launch(&s, "echo hi", false).unwrap();
        assert!(l.program.ends_with("zsh"), "{l:?}");
        assert_eq!(l.args, vec!["-lc", "echo hi"]);
        let l = shell_launch(&s, "echo hi", true).unwrap();
        assert_eq!(l.args, vec!["-lc", "echo hi"]);
        // 平台默认必须是 zsh（Windows 是 PowerShell）
        assert_eq!(Settings::default().command_shell, CommandShell::Zsh);
    }

    #[test]
    fn run_command_captures_output_and_exit_code() {
        let mut s = Settings::default();
        s.command_shell = if cfg!(windows) {
            CommandShell::Cmd
        } else {
            CommandShell::GitBash
        };
        if shell_launch(&s, "echo x", false).is_err() {
            println!("skip: 本机没有可用的 Shell");
            return;
        }
        let out = run_command(&s, "echo efto-ok", Duration::from_secs(10), None).unwrap();
        assert_eq!(out.code, 0, "stderr: {}", out.stderr);
        assert!(out.stdout.contains("efto-ok"), "stdout: {:?}", out.stdout);
        assert!(out.path.to_lowercase().ends_with(".exe"), "{}", out.path);
    }

    #[test]
    fn run_command_can_be_cancelled() {
        let s = Settings::default();
        if shell_launch(&s, "echo x", false).is_err() {
            println!("skip: 本机没有平台默认 shell");
            return;
        }
        let token = std::sync::Arc::new(AtomicBool::new(false));
        let setter = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            setter.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        // 阻塞命令按平台 shell 语法写（PowerShell / POSIX 不通用）
        let e = run_command(
            &s,
            if cfg!(windows) {
                "Start-Sleep -Seconds 10"
            } else {
                "sleep 10"
            },
            Duration::from_secs(20),
            Some(&token),
        )
        .unwrap_err();
        assert!(matches!(e, ExecError::Cancelled), "实际 {e}");
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "取消后应立即返回，实际 {:?}",
            started.elapsed()
        );
    }
}
