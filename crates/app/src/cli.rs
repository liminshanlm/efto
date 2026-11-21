//! 无 GUI 的 CLI 通道：`efto cli <子命令>`。
//!
//! 两个目的：
//! 1. 让插件与应用索引能在没有窗口、没有 WebView 的情况下被端到端验证（CI/回归）；
//! 2. 证明 jsrt 的 `Host` 抽象与 Tauri 无关——这里就是第二个实现。
//!
//! UI 类能力（开窗、副标题、通知）在 CLI 下退化为 stdout 行，绝不静默失败。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use efto_core::apps::indexer::{Indexer, unix_now};
use efto_core::apps::launch;
use efto_core::config::Settings;
use efto_core::plugins::registry::{
    normalize_dynamic_items, DynamicItem, PluginRegistry,
};
use efto_core::search::HitPayload;
use efto_core::store::StoreManager;
use efto_core::usage::UsageStore;
use efto_core::ItemKind;
use efto_jsrt::{Host, HostEvent, PluginContext, PluginEvent, RuntimeEvent, StartRequest};

/// CLI 侧宿主能力。
pub struct CliHost {
    stores: Arc<std::sync::Mutex<StoreManager>>,
    /// plugin id -> 插件根目录：`module.resolve/load` 需要它才能与 GUI 宿主行为一致
    roots: std::sync::Mutex<BTreeMap<String, PathBuf>>,
    quiet: bool,
    /// 静音模式下仍需把回显留给 `--json`，避免 agent 只看到空事件数组
    lines: std::sync::Mutex<Vec<String>>,
}

impl CliHost {
    fn new(quiet: bool) -> Self {
        Self {
            stores: Arc::new(std::sync::Mutex::new(StoreManager::default())),
            roots: std::sync::Mutex::new(BTreeMap::new()),
            quiet,
            lines: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn add_root(&self, plugin: &str, root: PathBuf) {
        self.roots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(plugin.to_string(), root);
    }

    fn root_of(&self, plugin: &str) -> Result<PathBuf, String> {
        self.roots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(plugin)
            .cloned()
            .ok_or_else(|| format!("未知插件根目录: {plugin}"))
    }

    fn say(&self, text: &str) {
        self.lines
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(text.to_string());
        if !self.quiet {
            println!("{text}");
        }
    }

    fn take_lines(&self) -> Vec<String> {
        std::mem::take(&mut *self.lines.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// 插件数据默认只在内存里累积；CLI 进程退出前必须显式落盘。
    fn flush(&self) -> Result<(), String> {
        let mut guard = self.stores.lock().unwrap_or_else(|p| p.into_inner());
        guard.flush_all().map_err(|e| e.to_string())
    }

    fn with_store<T>(&self, plugin: &str, f: impl FnOnce(&mut efto_core::store::PluginStore) -> T) -> T {
        let mut guard = self.stores.lock().unwrap_or_else(|p| p.into_inner());
        let store = guard.get_or_open(plugin);
        f(store)
    }
}

impl Host for CliHost {
    fn call(&self, plugin: &str, module: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        match (module, method) {
            ("db", "get") => {
                let key = arg_str(args, 0)?;
                Ok(self.with_store(plugin, |s| s.get(&key).unwrap_or(Value::Null)))
            }
            ("db", "set") => {
                let key = arg_str(args, 0)?;
                let value = args.get(1).cloned().unwrap_or(Value::Null);
                self.with_store(plugin, |s| s.set(&key, value))
                    .map_err(|e| e.to_string())?;
                Ok(json!(true))
            }
            ("db", "remove") => {
                let key = arg_str(args, 0)?;
                Ok(json!(self.with_store(plugin, |s| s.remove(&key))))
            }
            ("db", "all") => Ok(self.with_store(plugin, |s| s.all())),
            ("db", "keys") => Ok(Value::Array(
                self.with_store(plugin, |s| s.keys())
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            )),
            ("db", "size") => Ok(json!({ "keys": self.with_store(plugin, |s| s.keys()).len(), "size": self.with_store(plugin, |s| s.size_bytes()) })),
            ("db", "clear") => {
                self.with_store(plugin, |s| s.clear());
                Ok(json!(true))
            }
            ("notify", "show") => {
                let text = first_str(args);
                let sub = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                self.say(&format!("[notify] {text}{}", if sub.is_empty() { String::new() } else { format!(" — {sub}") }));
                Ok(json!(true))
            }
            ("system", "copyText") => {
                let text = arg_str(args, 0)?;
                self.say(&format!("[copy] {}", shorten(&text, 60)));
                host_cli_copy(&text)?;
                Ok(json!(true))
            }
            ("system", "pasteText") => Err("cli 模式不提供 pasteText".into()),
            ("system", "pasteImage") => Err("cli 模式不提供 pasteImage".into()),
            ("system", "imagePreview") => Err("cli 模式不提供 imagePreview".into()),
            ("system", "quit") => Ok(json!(true)),
            ("system", "dbPath") => Ok(Value::String(
                efto_core::fsutil::data_root().to_string_lossy().to_string(),
            )),
            ("system", "getPath") => Ok(Value::String(
                efto_core::fsutil::data_root().to_string_lossy().to_string(),
            )),
            ("shell", "openPath") | ("shell", "openExternal") | ("shell", "showItemInFolder") => {
                let p = arg_str(args, 0)?;
                self.say(&format!("[shell] {p}"));
                launch::shell_open(&p, None).map_err(|e| e.to_string())?;
                Ok(json!(true))
            }
            ("ui", "setSubTitle") => {
                self.say(&format!("[subtitle] {}", arg_str(args, 0).unwrap_or_default()));
                Ok(json!(true))
            }
            ("ui", "postMessage") => {
                // main -> UI 页面；CLI 没有窗口，回显出来以便断言
                let name = arg_str(args, 0)?;
                self.say(&format!(
                    "[ui→page:{name}] {}",
                    compact(&args.get(1).cloned().unwrap_or(Value::Null))
                ));
                Ok(json!(true))
            }
            ("ui", "ipcMessage") => {
                // CLI 没有 UI 页面，消息直接回显，便于断言插件确实发了
                let name = arg_str(args, 0)?;
                self.say(&format!("[ui-msg:{name}] {}", compact(&args.get(1).cloned().unwrap_or(Value::Null))));
                Ok(json!(true))
            }
            ("ui", "showUI") | ("ui", "hideUI") | ("ui", "showMainWindow") | ("ui", "hideMainWindow") => {
                self.say("[ui] cli 模式无窗口，已忽略");
                Ok(json!({ "ok": false, "reason": "headless" }))
            }
            ("redirect", "to") => {
                self.say(&format!("[redirect] {}", arg_str(args, 0).unwrap_or_default()));
                Ok(json!({ "ok": true }))
            }
            ("log", level) => {
                let msg = arg_str(args, 0).unwrap_or_default();
                self.say(&format!("[plugin-log:{level}] {msg}"));
                Ok(json!(true))
            }
            ("ai", "models") => Ok(crate::ai::models(&Settings::load())),
            ("output", "set") => {
                // 内联输出在 CLI 里逐行回显，便于脚本断言插件确实产出了结果
                let rows = args.first().cloned().unwrap_or(Value::Null);
                let hits = efto_core::plugins::hits_from_rows(plugin, &rows)?;
                for h in &hits {
                    self.say(&format!("[output] {}", compact(&serde_json::to_value(h).unwrap_or_default())));
                }
                Ok(json!(true))
            }
            ("broadcast", "send") => Ok(json!(true)),
            ("efto", "exec") => {
                // 与 GUI 宿主共用同一个入口（host::exec_capability），避免两套校验漂移
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let settings = efto_core::config::Settings::load();
                let hint = settings.everything_es_path.clone();
                let hint_path = hint.as_deref().map(std::path::Path::new);
                let v = crate::host::exec_capability(plugin, &opts, hint_path)?;
                self.say(&format!(
                    "[exec] {} code={} stdout={}B",
                    opts.get("name").and_then(|n| n.as_str()).unwrap_or("?"),
                    v["code"].as_i64().unwrap_or(-1),
                    v["stdout"].as_str().unwrap_or("").len()
                ));
                Ok(v)
            }
            ("runtime", "run") => {
                // 与 GUI 宿主共用 runtime_capability：解释器同样只来自设置文件
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let settings = efto_core::config::Settings::load();
                let root = self.root_of(plugin).ok();
                let v = crate::host::runtime_capability(
                    &opts,
                    settings.node_path.as_deref(),
                    settings.python_path.as_deref(),
                    root.as_deref(),
                )?;
                self.say(&format!(
                    "[runtime] {} code={} stdout={}B",
                    v["runtime"].as_str().unwrap_or("?"),
                    v["code"].as_i64().unwrap_or(-1),
                    v["stdout"].as_str().unwrap_or("").len()
                ));
                Ok(v)
            }
            ("hosts", "path") => Ok(Value::String(
                crate::hosts::target_path().to_string_lossy().to_string(),
            )),
            ("hosts", "read") => crate::hosts::read_capability(),
            ("hosts", "save") => {
                // 与 GUI 共用同一套校验/编码/提权写入；CLI 下同样会弹 UAC
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let v = crate::hosts::save_capability(&opts)?;
                self.say(&format!(
                    "[hosts] 已保存，备份 {}",
                    v["backup"].as_str().unwrap_or("-")
                ));
                Ok(v)
            }
            ("search", "setItems") => {
                let items = args
                    .first()
                    .cloned()
                    .map(serde_json::from_value::<Vec<DynamicItem>>)
                    .transpose()
                    .map_err(|e| format!("setItems 参数格式错误: {e}"))?
                    .unwrap_or_default();
                let items = normalize_dynamic_items(items);
                crate::state::save_plugin_search_items_file(plugin, &items)?;
                Ok(Value::Null)
            }
            ("module", "resolve") | ("module", "load") => {
                // 与 GUI 宿主共用 jsrt::modules 的实现，避免"CLI 能跑、GUI 跑不通"
                let root = self.root_of(plugin)?;
                efto_jsrt::modules::handle(&root, method, args).map_err(|e| e.to_string())
            }
            (m, meth) => Err(format!("cli 宿主未实现的能力: {m}.{meth}")),
        }
    }

    fn call_async(
        &self,
        plugin: &str,
        module: &str,
        method: &str,
        args: &[Value],
        reply: efto_jsrt::ReplyTo,
        id: u64,
    ) {
        let outcome: Result<Value, String> = match (module, method) {
            // 与 GUI 共用同一份实现；CLI 每次现读 settings.json，保证改配置后立即可见
            ("ai", "chat") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let settings = Settings::load();
                if opts.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
                    let stream_reply = reply.clone();
                    let result = crate::ai::chat_stream(&settings, &opts, &mut |delta| {
                        let _ = stream_reply.send(HostEvent::Stream { id, delta });
                    });
                    let _ = reply.send(reply_event(id, result));
                    return;
                }
                crate::ai::chat(&settings, &opts)
            }
            ("ai", "ocr") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                crate::ai::ocr(&Settings::load(), &opts)
            }
            _ => self.call(plugin, module, method, args),
        };
        let _ = reply.send(reply_event(id, outcome));
    }

    fn send_to_ui(&self, plugin: &str, name: &str, data: &Value) {
        self.say(&format!("[to-ui:{plugin}/{name}] {}", compact(data)));
    }
}

fn reply_event(id: u64, outcome: Result<Value, String>) -> HostEvent {
    match outcome {
        Ok(v) => HostEvent::Reply { id, ok: true, result: v },
        Err(e) => HostEvent::Reply {
            id,
            ok: false,
            result: Value::String(e),
        },
    }
}

/// CLI 参数：位置参数 + 已识别 flag。
///
/// 只识别已知 flag：`-2^2`、`-5+3` 这类以减号开头的算式曾被当成选项整条丢掉，
/// 表现为 "cli search -2^2" 报"需要关键词"。
#[derive(Default)]
struct CliFlags {
    positional: Vec<String>,
    quiet: bool,
    as_json: bool,
    help: bool,
    exact: bool,
    limit: Option<usize>,
    kinds: Option<Vec<ItemKind>>,
    id: Option<String>,
    wait_ms: Option<u64>,
    timeout_ms: Option<u64>,
    /// 只报告将要做什么，不真的执行（launch / files / action）
    dry_run: bool,
    /// 不写使用记录（launch）
    no_track: bool,
    /// 在资源管理器中定位而不是打开（files）
    reveal: bool,
    /// 按进程号选窗口（focus）
    pid: Option<u32>,
    /// 窗口操作类型（tile-windows）：grid / left / right / min-others / min-all / undo
    mode: Option<String>,
    /// 待导入的 JSON 文件（import-shortcuts；省略时读 stdin）
    file: Option<String>,
    /// 覆盖目标 exe 文件名（import-shortcuts）
    exe: Option<String>,
}

fn parse_args(args: &[String]) -> Result<CliFlags, String> {
    let mut f = CliFlags::default();
    let mut i = 1; // args[0] 是子命令
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--quiet" => f.quiet = true,
            "--json" => f.as_json = true,
            "--exact" => f.exact = true,
            "--dry-run" => f.dry_run = true,
            "--no-track" => f.no_track = true,
            "--reveal" => f.reveal = true,
            "--help" | "-h" => f.help = true,
            "--limit" => f.limit = Some(positive_flag(flag_value(args, &mut i, "--limit")?)?),
            "--kind" => f.kinds = Some(parse_kinds(&flag_value(args, &mut i, "--kind")?)?),
            "--id" => f.id = Some(flag_value(args, &mut i, "--id")?),
            "--pid" => f.pid = Some(pid_flag(flag_value(args, &mut i, "--pid")?)?),
            "--mode" => f.mode = Some(flag_value(args, &mut i, "--mode")?),
            "--file" => f.file = Some(flag_value(args, &mut i, "--file")?),
            "--exe" => f.exe = Some(flag_value(args, &mut i, "--exe")?),
            "--wait-ms" => f.wait_ms = Some(u64_flag(flag_value(args, &mut i, "--wait-ms")?)?),
            "--timeout-ms" => f.timeout_ms = Some(u64_flag(flag_value(args, &mut i, "--timeout-ms")?)?),
            _ => {
                if let Some(v) = a.strip_prefix("--limit=") {
                    f.limit = Some(positive_flag(v.to_string())?);
                } else if let Some(v) = a.strip_prefix("--kind=") {
                    f.kinds = Some(parse_kinds(v)?);
                } else if let Some(v) = a.strip_prefix("--id=") {
                    f.id = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--pid=") {
                    f.pid = Some(pid_flag(v.to_string())?);
                } else if let Some(v) = a.strip_prefix("--mode=") {
                    f.mode = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--file=") {
                    f.file = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--exe=") {
                    f.exe = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--wait-ms=") {
                    f.wait_ms = Some(u64_flag(v.to_string())?);
                } else if let Some(v) = a.strip_prefix("--timeout-ms=") {
                    f.timeout_ms = Some(u64_flag(v.to_string())?);
                } else {
                    f.positional.push(args[i].clone());
                }
            }
        }
        i += 1;
    }
    Ok(f)
}

fn flag_value(args: &[String], i: &mut usize, name: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{name} 需要一个值"))
}

fn positive_flag(raw: String) -> Result<usize, String> {
    let n: usize = raw.parse().map_err(|_| format!("--limit 需要正整数，收到 {raw}"))?;
    if n == 0 {
        return Err("--limit 需要正整数".into());
    }
    Ok(n)
}

fn u64_flag(raw: String) -> Result<u64, String> {
    raw.parse().map_err(|_| format!("需要毫秒数，收到 {raw}"))
}

fn pid_flag(raw: String) -> Result<u32, String> {
    raw.parse().map_err(|_| format!("--pid 需要进程号，收到 {raw}"))
}

fn parse_kinds(raw: &str) -> Result<Vec<ItemKind>, String> {
    let mut out = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let kind = match part.to_ascii_lowercase().as_str() {
            "app" => ItemKind::App,
            "plugin" => ItemKind::Plugin,
            "output" => ItemKind::Output,
            "action" => ItemKind::Action,
            "calc" => ItemKind::Calc,
            "time" => ItemKind::Time,
            "command" => ItemKind::Command,
            "file" => ItemKind::File,
            "external" => ItemKind::External,
            "bookmark" => ItemKind::Bookmark,
            other => {
                return Err(format!(
                    "未知 --kind: {other}（可用 app/plugin/output/action/calc/time/command/file/external/bookmark）"
                ))
            }
        };
        out.push(kind);
    }
    if out.is_empty() {
        return Err("--kind 需要至少一个类型".into());
    }
    Ok(out)
}

/// 退出码约定：0 成功 / 1 执行失败 / 2 用法错误 / 3 查无结果。
/// 3 单独一条，调用方才能区分"没问题但没搜到"和"执行出错"。
const EXIT_NOT_FOUND: i32 = 3;

/// --json 模式下把失败写成可解析对象；非 json 仍走 stderr。
fn report_error(as_json: bool, code: &str, message: impl Into<String>, exit: i32) -> i32 {
    let message = message.into();
    if as_json {
        println!(
            "{}",
            json!({ "ok": false, "code": code, "message": message })
        );
    } else {
        eprintln!("{message}");
    }
    exit
}

/// CLI 入口。`args` 是 "cli" 之后的全部参数，args[0] 为子命令。
pub fn main(args: &[String]) -> i32 {
    let Some(sub) = args.first().map(|s| s.as_str()) else {
        eprintln!("{}", USAGE);
        return 2;
    };
    if sub == "version" || sub == "--version" || sub == "-V" {
        println!("efto {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if sub == "help" || sub == "--help" || sub == "-h" {
        println!("{}", USAGE);
        return 0;
    }
    let has_json = args.iter().any(|a| a == "--json");
    let flags = match parse_args(args) {
        Ok(f) => f,
        Err(e) => return report_error(has_json, "bad_args", e, 2),
    };
    if flags.help {
        println!("{}", USAGE);
        return 0;
    }
    // 让 RUST_LOG 在 CLI 通道同样生效（插件线程的死因只有日志能说清）
    crate::init_tracing();

    let settings = Settings::load();
    match sub {
        "search" => cmd_search(&settings, &flags),
        "launch" => cmd_launch(&settings, &flags),
        "apps" => cmd_apps(&settings, &flags),
        "index" => cmd_index(&settings, &flags),
        "plugins" => cmd_plugins(&settings, &flags),
        "plugin-run" => cmd_plugin_run(&settings, &flags),
        "run-command" => cmd_run_command(&settings, &flags),
        "files" => cmd_files(&flags),
        "action" => cmd_action(&settings, &flags),
        "ps" => cmd_ps(&flags),
        "focus" => cmd_focus(&flags),
        "tile-windows" => cmd_tile_windows(&flags),
        "import-shortcuts" => cmd_import_shortcuts(&flags),
        "capabilities" => cmd_capabilities(&settings, &flags),
        other => report_error(
            flags.as_json,
            "unknown_command",
            format!("未知子命令: {other}\n{USAGE}"),
            2,
        ),
    }
}

const USAGE: &str = r#"用法: efto cli <子命令> [参数]

  search <关键词> [--json] [--limit N] [--kind app,plugin,...] [--exact]
                                搜索应用/插件/书签/文件，打印命中项；--exact 只留完全同名
  launch (--id <id> | <关键词>) [--json] [--dry-run] [--no-track]
                                启动命中的应用（--id 优先；--dry-run 只报告，
                                --no-track 不写使用记录）
  apps [关键词] [--json] [--limit N]
                                列出索引内的应用，默认最多 100 条
  index [--json]                重建应用索引并输出统计
  plugins [--json]              列出已发现插件与各自问题
  plugin-run <id> <cmd> [负载] [--json] [--quiet]
                [--wait-ms N] [--timeout-ms N]
                                启动插件后端、投递 enter、等待事件（验证用）
  run-command <命令...> [--json] [--dry-run] [--timeout-ms N]
                                用设置里的 Shell 执行命令，回传退出码与输出
                                （--dry-run 只打印解析出的命令行）
  files <路径> [--reveal] [--json] [--dry-run]
                                打开文件/文件夹；--reveal 在资源管理器中定位
  action <名称> [--json] [--dry-run]
                                宿主动作: reindex / open_plugin_dir / open_data_dir /
                                lock_screen（settings/screenshot/clipboard/quit 需要 GUI）
  ps [--json]                   列出可见顶层窗口（pid / hwnd / 进程 / 标题）
  focus (--pid N | <标题关键词>) [--json] [--dry-run]
                                把匹配的窗口切到前台
  tile-windows [--mode 模式] [--json] [--dry-run]
                                窗口操作（默认 grid 网格平铺）
                                模式: grid / left / right / min-others / min-all / undo
                                （--dry-run 只打印候选与目标矩形）
  import-shortcuts [--file <路径>] [--exe <文件名>] [--json] [--dry-run]
                                把快捷键 JSON 导入 ~/.efto/shortcuts/<exe>.json
                                格式: {"app":{"exe":["x.exe"],"name":"X"},
                                "groups":[{"name":"分组","items":[{"keys":"Ctrl+S",
                                "action":"保存"}]}]}；--file 省略时从 stdin 读取，
                                与既有文件按同名分组/key 合并（幂等），--dry-run 不写盘
  capabilities [--json]         输出机器可读的能力与退出码约定
  version                       打印版本
  help | --help                 打印本说明
退出码:
  0 成功   1 执行失败   2 用法错误   3 查无结果
环境变量:
  EFTO_PLUGIN_DIRS   追加插件搜索目录（; 分隔）
  RUST_LOG             tracing 级别，例如 debug（日志固定走 stderr）
"#;

/// CLI 的索引来源：优先用缓存，和 GUI 冷启动一致。
/// 以前每次都全量重扫（~200ms），`cli search` 用起来像卡了一下；
/// 需要强制刷新时显式跑 `cli index`。
fn build_index(settings: &Settings) -> efto_core::apps::indexer::AppIndex {
    if let Some((index, _)) = Indexer::load_cache() {
        let usage = UsageStore::load();
        let mut index = index;
        for e in index.entries_mut() {
            if let Some(rec) = usage.get(&e.id) {
                e.use_count = rec.count;
                e.last_used_ts = rec.last_ts;
            }
        }
        return index;
    }
    let usage = UsageStore::load();
    let (index, report) = Indexer::build(settings, &usage);
    let _ = Indexer::save_cache(&index, &report);
    index
}

fn cmd_search(settings: &Settings, f: &CliFlags) -> i32 {
    let query = f.positional.join(" ");
    if query.trim().is_empty() {
        return report_error(f.as_json, "bad_args", "search 需要关键词", 2);
    }
    let index = build_index(settings);
    let registry = discover_plugins(settings);
    // 与 GUI 搜索保持同一套源开关：开启收藏夹检索时，cli search 也能搜到书签。
    let mut bookmarks = efto_core::bookmarks::BookmarkStore::default();
    if settings.enable_bookmarks {
        bookmarks.refresh();
    }
    let limit = f.limit.unwrap_or(settings.max_results);
    let snippets = efto_core::snippets::SnippetStore::load();
    let mut hits = efto_core::search::SearchService::query_with_sources(
        &query,
        &index,
        &registry,
        limit,
        None,
        if settings.enable_bookmarks { Some(&bookmarks) } else { None },
        Some(settings),
        if snippets.is_empty() { None } else { Some(&snippets) },
    );
    if let Some(kinds) = &f.kinds {
        hits.retain(|h| kinds.contains(&h.kind));
    }
    if f.exact {
        hits.retain(|h| h.reason == "exact");
    }
    // 命令前缀命中时是短路结果：不再补文件行，否则 `> ipconfig` 后面会跟一串文件
    let is_command = hits.first().map(|h| h.kind == ItemKind::Command).unwrap_or(false);
    // 文件检索不再是魔法前缀：Everything 结果补在应用条目之后。
    // 只填空位，绝不挤掉已有结果（`--kind file` 时应用条目已被过滤掉，文件自然填满 limit）。
    #[cfg(windows)]
    let win_settings_query = efto_core::search::win_settings::is_query(&query);
    #[cfg(not(windows))]
    let win_settings_query = false;
    let wants_files = !is_command
        && !f.exact
        && !win_settings_query
        && f.kinds
            .as_ref()
            .map_or(true, |k| k.contains(&ItemKind::File));
    if wants_files {
        let room = limit.saturating_sub(hits.len());
        // 文件检索源（Everything/es.exe）是 Windows-only；mac 的 mdfind 是 v1.5
        #[cfg(windows)]
        hits.extend(
            efto_core::search::everything::hits(&query)
                .into_iter()
                .take(room),
        );
        #[cfg(not(windows))]
        let _ = room;
    }
    if f.as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".into())
        );
        return if hits.is_empty() { EXIT_NOT_FOUND } else { 0 };
    }
    if hits.is_empty() {
        if f.kinds.is_some() {
            println!("无匹配: {query}（已按 --kind 过滤）");
        } else {
            println!("无匹配: {query}");
        }
        return EXIT_NOT_FOUND;
    }
    for (i, h) in hits.iter().enumerate() {
        println!(
            "{:>2}. {:<28} [{}] {}  ({})",
            i + 1,
            h.title,
            h.reason,
            h.subtitle,
            h.source
        );
    }
    0
}

fn cmd_launch(settings: &Settings, f: &CliFlags) -> i32 {
    if f.id.is_some() {
        let id = f.id.as_deref().unwrap_or_default();
        let index = build_index(settings);
        let Some(entry) = index.get(id) else {
            return report_error(
                f.as_json,
                "not_found",
                format!("索引里没有 id: {id}"),
                EXIT_NOT_FOUND,
            );
        };
        let (id, title, target) = (entry.id.clone(), entry.name.clone(), entry.target.clone());
        return do_launch(f, &id, &title, &target);
    }
    if f.positional.is_empty() {
        return report_error(f.as_json, "bad_args", "launch 需要关键词或 --id", 2);
    }
    if !f.as_json {
        // 文本模式保持原有体验：先打印搜索结果，再启动第一条
        let code = cmd_search(settings, f);
        if code != 0 {
            return code;
        }
    }
    let index = build_index(settings);
    let registry = discover_plugins(settings);
    let query = f.positional.join(" ");
    let hits = efto_core::search::SearchService::query(&query, &index, &registry, 3);
    let Some(hit) = hits.first() else {
        return report_error(f.as_json, "not_found", "没有可启动的结果", EXIT_NOT_FOUND);
    };
    match &hit.payload {
        HitPayload::Launch { target } => do_launch(f, &hit.id, &hit.title, target),
        other => report_error(
            f.as_json,
            "not_launchable",
            format!("第一项不是可启动的应用（{other:?}），请用 search 查看命中详情"),
            1,
        ),
    }
}

fn do_launch(
    f: &CliFlags,
    id: &str,
    title: &str,
    target: &efto_core::apps::model::LaunchTarget,
) -> i32 {
    if f.dry_run {
        if f.as_json {
            println!(
                "{}",
                json!({ "ok": true, "dry_run": true, "id": id, "title": title, "target": target })
            );
        } else {
            println!("将启动: {title} -> {}", target.path);
        }
        return 0;
    }
    if let Err(e) = launch::launch(target) {
        return report_error(f.as_json, "launch_failed", format!("启动失败: {e}"), 1);
    }
    if !f.no_track {
        let mut usage = UsageStore::load();
        usage.record(id, unix_now());
        if let Err(e) = usage.save() {
            eprintln!("使用记录写入失败: {e}");
        }
    }
    if f.as_json {
        println!(
            "{}",
            json!({ "ok": true, "id": id, "title": title, "target": target })
        );
    } else {
        println!("已启动: {title} -> {}", target.path);
    }
    0
}

fn cmd_apps(settings: &Settings, f: &CliFlags) -> i32 {
    let index = build_index(settings);
    let needle = f.positional.join(" ").trim().to_lowercase();
    let mut rows: Vec<&efto_core::apps::model::AppEntry> = index
        .entries()
        .iter()
        .filter(|e| {
            needle.is_empty()
                || e.name.to_lowercase().contains(&needle)
                || e.target.path.to_lowercase().contains(&needle)
                || e.aliases.iter().any(|a| a.to_lowercase().contains(&needle))
        })
        .collect();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let total = rows.len();
    let limit = f.limit.unwrap_or(100);
    rows.truncate(limit);
    if f.as_json {
        let items: Vec<Value> = rows
            .iter()
            .map(|e| {
                json!({
                    "id": e.id,
                    "name": e.name,
                    "source": e.source.label(),
                    "target": e.target.path,
                })
            })
            .collect();
        println!("{}", json!({ "ok": true, "total": total, "items": items }));
        return if total == 0 { EXIT_NOT_FOUND } else { 0 };
    }
    if rows.is_empty() {
        println!("没有匹配的应用");
        return EXIT_NOT_FOUND;
    }
    for e in &rows {
        println!("{}\t{}\t{}\t{}", e.id, e.name, e.source.label(), e.target.path);
    }
    if total > rows.len() {
        println!("… 共 {total} 条，已显示 {}（用 --limit 调整）", rows.len());
    }
    0
}

fn cmd_index(settings: &Settings, f: &CliFlags) -> i32 {
    // 这个子命令的语义就是"强制重建"，所以不走缓存
    let usage = UsageStore::load();
    let (index, report) = Indexer::build(settings, &usage);
    let _ = Indexer::save_cache(&index, &report);
    let registry = discover_plugins(settings);
    let raw = report.start_menu + report.desktop + report.registry + report.dir_scan + report.system;
    if f.as_json {
        let dirs: Vec<String> = effective_dirs(settings)
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        println!(
            "{}",
            json!({
                "ok": true,
                "apps": index.len(),
                "raw": raw,
                "dropped_duplicates": report.dropped_duplicates,
                "sources": {
                    "start_menu": report.start_menu,
                    "desktop": report.desktop,
                    "registry": report.registry,
                    "dir_scan": report.dir_scan,
                    "system": report.system,
                },
                "plugins": registry.len(),
                "elapsed_ms": report.elapsed_ms,
                "plugin_dirs": dirs,
            })
        );
        // 问题写 stderr，避免污染 JSON
        for p in registry.plugins() {
            if !p.problems.is_empty() {
                eprintln!("{}: {}", p.manifest.name, p.problems.join(" / "));
            }
        }
        return 0;
    }
    println!(
        "索引: 应用 {} 条（原始 {}，去重合并 {}），插件 {} 个，用时 {}ms，目录 {:?}",
        index.len(),
        raw,
        report.dropped_duplicates,
        registry.len(),
        report.elapsed_ms,
        effective_dirs(settings)
    );
    for p in registry.plugins() {
        if !p.problems.is_empty() {
            println!("  ! {}: {}", p.manifest.name, p.problems.join(" / "));
        }
    }
    0
}

fn cmd_plugins(settings: &Settings, f: &CliFlags) -> i32 {
    let registry = discover_plugins(settings);
    let rows: Vec<Value> = registry
        .plugins()
        .iter()
        .map(|p| {
            json!({
                "id": p.id(),
                "name": p.manifest.name,
                "version": p.manifest.version,
                "backend": p.main_entry.as_ref().map(|x| x.to_string_lossy().to_string()),
                "ui": p.ui_entry.as_ref().map(|x| x.to_string_lossy().to_string()),
                "cmds": p.manifest.cmds.iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
                "problems": p.problems,
                "enabled": p.enabled,
            })
        })
        .collect();
    if f.as_json {
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
        return if rows.is_empty() { EXIT_NOT_FOUND } else { 0 };
    }
    if rows.is_empty() {
        println!("未发现插件。放到 {:?} 或设置 EFTO_PLUGIN_DIRS", effective_dirs(settings).first());
        return EXIT_NOT_FOUND;
    }
    for r in rows {
        println!(
            "{} v{}  backend={} ui={} cmds={}",
            r["name"].as_str().unwrap_or("?"),
            r["version"].as_str().unwrap_or("?"),
            r["backend"].as_str().unwrap_or("-"),
            r["ui"].as_str().unwrap_or("-"),
            r["cmds"]
                .as_array()
                .map(|a| a
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("/"))
                .unwrap_or_default(),
        );
        if let Some(problems) = r["problems"].as_array() {
            for p in problems {
                println!("  ! {}", p.as_str().unwrap_or(""));
            }
        }
    }
    0
}

/// 端到端跑一个插件：启动后端 → 投递 enter → 收集事件 → 停止。
fn cmd_plugin_run(settings: &Settings, f: &CliFlags) -> i32 {
    let args = &f.positional;
    let Some(id) = args.first().map(|s| s.as_str()) else {
        return report_error(f.as_json, "bad_args", "plugin-run 需要插件 id", 2);
    };
    let cmd = args.get(1).cloned().unwrap_or_default();
    let payload = args.get(2).cloned().unwrap_or_default();
    // --json 时 stdout 只留最终对象，宿主回显与事件打印一律静音
    let quiet = f.quiet || f.as_json;

    let registry = discover_plugins(settings);
    let Some(descriptor) = registry.get(id) else {
        return report_error(
            f.as_json,
            "unknown_plugin",
            format!("未知插件: {id}（先看 cli plugins）"),
            EXIT_NOT_FOUND,
        );
    };
    let Some(entry) = descriptor.main_entry.clone() else {
        return report_error(
            f.as_json,
            "no_backend",
            format!("插件 {} 没有后端入口（main）", descriptor.manifest.name),
            1,
        );
    };

    let cli = Arc::new(CliHost::new(quiet));
    cli.add_root(&descriptor.id(), descriptor.root.clone());
    let host: Arc<dyn Host> = cli.clone();
    let mut runtime = efto_jsrt::PluginRuntime::new(host);
    let ctx = PluginContext {
        plugin_id: descriptor.id(),
        root: descriptor.root.to_string_lossy().to_string(),
        version: descriptor.manifest.version.clone(),
        locale: crate::i18n::resolve(&efto_core::config::Settings::load()).to_string(),
        theme: "light".into(),
        host_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if let Err(e) = runtime.start(StartRequest {
        ctx,
        entry: entry.clone(),
    }) {
        return report_error(f.as_json, "start_failed", format!("插件启动失败: {e}"), 1);
    }
    let enter_error = runtime
        .enter(id, &cmd, Value::String(payload.clone()))
        .err()
        .map(|e| e.to_string());

    // 给定时器/异步能力一点时间，再收事件
    let wait_ms = f.wait_ms.unwrap_or(if payload == "wait" { 900 } else { 400 });
    std::thread::sleep(std::time::Duration::from_millis(wait_ms));
    let events = report_events(&mut runtime, quiet, f.timeout_ms.unwrap_or(200));
    let alive = runtime.is_running(id);
    let stopped = runtime.stop(id);
    let flush_error = cli.flush().err();
    let lines = cli.take_lines();
    let errors = events
        .iter()
        .filter(|e| matches!(e.event, PluginEvent::Error { .. }))
        .count();
    // 插件 console.* 走事件通道；--json 下单独列出，agent 才能拿到
    // `EFTO_API {...}` 这类机器可读结果（此前只写 tracing，JSON 模式里会丢）。
    let logs: Vec<Value> = events
        .iter()
        .filter_map(|e| match &e.event {
            PluginEvent::Log { level, text } => Some(json!({ "level": level, "text": text })),
            _ => None,
        })
        .collect();
    let ok = enter_error.is_none() && errors == 0 && alive && flush_error.is_none();
    if f.as_json {
        let ev: Vec<Value> = events
            .iter()
            .map(|e| {
                json!({
                    "plugin": e.plugin,
                    "event": serde_json::to_value(&e.event).unwrap_or(Value::Null),
                })
            })
            .collect();
        println!(
            "{}",
            json!({
                "ok": ok,
                "running": alive,
                "events": ev,
                "logs": logs,
                "lines": lines,
                "errors": errors,
                "stopped": stopped,
                "enter_error": enter_error,
                "flush_error": flush_error,
            })
        );
    } else {
        if let Some(e) = &enter_error {
            eprintln!("enter 投递失败: {e}");
        }
        if let Some(e) = &flush_error {
            eprintln!("插件数据落盘失败: {e}");
        }
        if !quiet {
            println!(
                "结果: running={alive} 事件 {} 条（错误 {errors}），线程回收 {stopped}",
                events.len()
            );
        }
    }
    if !ok {
        return 1;
    }
    0
}

/// 用设置里的 Shell 执行命令；供脚本/agent 复用 GUI 的同一套入口。
/// 显式子命令不受 `command_enabled` 限制（用户已经明确要执行），但尊重 Shell 与超时设置。
fn cmd_run_command(settings: &Settings, f: &CliFlags) -> i32 {
    let command = f.positional.join(" ");
    if command.trim().is_empty() {
        return report_error(f.as_json, "bad_args", "run-command 需要命令内容", 2);
    }
    let timeout = f
        .timeout_ms
        .map(std::time::Duration::from_millis)
        .unwrap_or_else(|| settings.command_timeout());
    if f.dry_run {
        return match efto_core::exec::shell_launch(settings, &command, false) {
            Ok(l) => {
                if f.as_json {
                    println!(
                        "{}",
                        json!({
                            "ok": true,
                            "dry_run": true,
                            "shell": l.label,
                            "program": l.program.display().to_string(),
                            "args": l.args,
                        })
                    );
                } else {
                    println!("{} {}", l.program.display(), l.args.join(" "));
                }
                0
            }
            Err(e) => report_error(f.as_json, "shell_missing", e.to_string(), 1),
        };
    }
    match efto_core::exec::run_command(settings, &command, timeout, None) {
        Ok(ok) => {
            if f.as_json {
                println!(
                    "{}",
                    json!({
                        "ok": ok.code == 0,
                        "shell": settings.command_shell.display(),
                        "program": ok.path,
                        "code": ok.code,
                        "stdout": ok.stdout,
                        "stderr": ok.stderr,
                        "truncated": ok.truncated,
                    })
                );
            } else {
                print!("{}", ok.stdout);
                if !ok.stderr.is_empty() {
                    eprint!("{}", ok.stderr);
                }
            }
            if ok.code == 0 {
                0
            } else {
                1
            }
        }
        Err(e) => report_error(f.as_json, "run_failed", e.to_string(), 1),
    }
}

/// 打开文件/文件夹；`--reveal` 改为在资源管理器中定位。
fn cmd_files(f: &CliFlags) -> i32 {
    let raw = f.positional.join(" ");
    let path = raw.trim();
    if path.is_empty() {
        return report_error(f.as_json, "bad_args", "files 需要文件或文件夹路径", 2);
    }
    if !PathBuf::from(path).exists() {
        return report_error(f.as_json, "not_found", format!("路径不存在: {path}"), EXIT_NOT_FOUND);
    }
    if f.dry_run {
        if f.as_json {
            println!(
                "{}",
                json!({ "ok": true, "dry_run": true, "reveal": f.reveal, "path": path })
            );
        } else {
            println!("将{}: {path}", if f.reveal { "定位" } else { "打开" });
        }
        return 0;
    }
    let opened = if f.reveal {
        launch::reveal_in_folder(path)
    } else {
        launch::shell_open(path, None)
    };
    if let Err(e) = opened {
        return report_error(f.as_json, "open_failed", format!("打开失败: {e}"), 1);
    }
    if f.as_json {
        println!("{}", json!({ "ok": true, "reveal": f.reveal, "path": path }));
    } else {
        println!("已{}: {path}", if f.reveal { "定位" } else { "打开" });
    }
    0
}

/// 宿主动作。只跑不依赖窗口/事件循环的那几个；GUI 专属动作明确报 needs_gui。
fn cmd_action(settings: &Settings, f: &CliFlags) -> i32 {
    let Some(name) = f.positional.first().map(|s| s.as_str()) else {
        return report_error(
            f.as_json,
            "bad_args",
            "action 需要动作名（reindex / open_plugin_dir / open_data_dir / lock_screen）",
            2,
        );
    };
    match name {
        "reindex" | "open_plugin_dir" | "open_data_dir" | "lock_screen" => {}
        "settings" | "screenshot" | "clipboard" | "quit" => {
            return report_error(
                f.as_json,
                "needs_gui",
                format!("action {name} 需要 GUI 运行中：CLI 没有窗口与事件循环，无法执行"),
                1,
            )
        }
        other => {
            return report_error(
                f.as_json,
                "bad_args",
                format!(
                    "未知 action: {other}（可用 reindex / open_plugin_dir / open_data_dir / lock_screen）"
                ),
                2,
            )
        }
    }

    match name {
        "reindex" => {
            if f.dry_run {
                return emit_action(f, name, "将重建应用索引");
            }
            cmd_index(settings, f)
        }
        "open_plugin_dir" | "open_data_dir" => {
            let dir = if name == "open_plugin_dir" {
                settings
                    .effective_plugin_dirs()
                    .first()
                    .cloned()
                    .unwrap_or_else(efto_core::fsutil::data_root)
            } else {
                efto_core::fsutil::data_root()
            };
            if f.dry_run {
                return emit_action(f, name, &format!("将打开目录 {}", dir.display()));
            }
            if name == "open_plugin_dir" && !dir.exists() {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    return report_error(f.as_json, "fs_error", format!("创建插件目录失败: {e}"), 1);
                }
            }
            if let Err(e) = launch::shell_open(&dir.to_string_lossy(), None) {
                return report_error(f.as_json, "open_failed", format!("打开失败: {e}"), 1);
            }
            emit_action(f, name, &format!("已打开目录 {}", dir.display()))
        }
        "lock_screen" => {
            if f.dry_run {
                return emit_action(f, name, "将锁定屏幕");
            }
            if let Err(e) = crate::commands::lock_workstation() {
                return report_error(f.as_json, "lock_failed", e, 1);
            }
            emit_action(f, name, "已锁屏")
        }
        _ => unreachable!("动作名已在前面校验"),
    }
}

fn emit_action(f: &CliFlags, name: &str, detail: &str) -> i32 {
    if f.as_json {
        let mut v = json!({ "ok": true, "action": name, "detail": detail });
        if f.dry_run {
            v["dry_run"] = json!(true);
        }
        println!("{v}");
    } else {
        println!("{detail}");
    }
    0
}

/// 列出可见顶层窗口：pid / hwnd / 进程 / 标题，制表符分隔。
fn cmd_ps(f: &CliFlags) -> i32 {
    let rows = crate::winproc::windows();
    if f.as_json {
        println!("{}", json!({ "ok": true, "total": rows.len(), "items": rows }));
        return if rows.is_empty() { EXIT_NOT_FOUND } else { 0 };
    }
    if rows.is_empty() {
        println!("没有可见的顶层窗口");
        return EXIT_NOT_FOUND;
    }
    for w in &rows {
        println!("{}\t{}\t{}\t{}", w.pid, w.hwnd, w.process, w.title);
    }
    0
}

/// 把匹配的窗口切到前台：`--pid N` 或标题/进程名关键词。
fn cmd_focus(f: &CliFlags) -> i32 {
    let needle = f.positional.join(" ");
    if f.pid.is_none() && needle.trim().is_empty() {
        return report_error(f.as_json, "bad_args", "focus 需要 --pid N 或标题关键词", 2);
    }
    let Some(w) = crate::winproc::find(f.pid, &needle) else {
        let what = match f.pid {
            Some(pid) => format!("pid {pid}"),
            None => format!("标题包含 {needle:?}"),
        };
        return report_error(
            f.as_json,
            "not_found",
            format!("没有匹配的窗口: {what}"),
            EXIT_NOT_FOUND,
        );
    };
    if !f.dry_run {
        if let Err(e) = crate::winproc::activate(&w) {
            return report_error(f.as_json, "focus_failed", e, 1);
        }
    }
    if f.as_json {
        println!(
            "{}",
            json!({
                "ok": true,
                "dry_run": f.dry_run,
                "pid": w.pid,
                "hwnd": w.hwnd,
                "process": w.process,
                "title": w.title,
            })
        );
    } else {
        println!(
            "{}{} (pid {})",
            if f.dry_run { "将聚焦: " } else { "已聚焦: " },
            w.title,
            w.pid
        );
    }
    0
}

/// 导入快捷键 JSON：`--file` 指定文件，省略时从 stdin 读；
/// 写盘格式与 `~/.efto/shortcuts/<exe>.json` 一致，方便 agent 生成后直接导入。
fn cmd_import_shortcuts(f: &CliFlags) -> i32 {
    let raw = match &f.file {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) => {
                return report_error(f.as_json, "io_error", format!("读取 {path} 失败: {e}"), 1)
            }
        },
        None => {
            let mut buf = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                return report_error(f.as_json, "io_error", format!("读取 stdin 失败: {e}"), 1);
            }
            buf
        }
    };
    if raw.trim().is_empty() {
        return report_error(
            f.as_json,
            "missing_input",
            "需要快捷键 JSON：用 --file <路径> 指定，或通过 stdin 传入".to_string(),
            2,
        );
    }
    let set: efto_core::shortcuts::ShortcutSet = match serde_json::from_str(&raw) {
        Ok(set) => set,
        Err(e) => return report_error(f.as_json, "bad_json", format!("JSON 解析失败: {e}"), 2),
    };
    match efto_core::shortcuts::import_set(set, f.exe.as_deref(), f.dry_run) {
        Ok(report) => {
            if f.as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".to_string())
                );
            } else {
                println!("{}", report.message);
            }
            0
        }
        Err(e) => report_error(f.as_json, "bad_shortcuts", e, 2),
    }
}

/// 平铺主显示器上的可见窗口；`--dry-run` 只输出候选与目标矩形。
fn cmd_tile_windows(f: &CliFlags) -> i32 {
    let op = match f.mode.as_deref() {
        None => crate::tile::Op::Grid,
        Some(name) => match crate::tile::Op::from_name(name) {
            Some(op) => op,
            None => {
                return report_error(
                    f.as_json,
                    "bad_args",
                    format!("未知 --mode: {name}（可用 grid / left / right / min-others / min-all / undo）"),
                    2,
                )
            }
        },
    };
    let report = match crate::tile::run_op(None, op, f.dry_run) {
        Ok(r) => r,
        Err(e) => return report_error(f.as_json, "tile_failed", e, 1),
    };
    if f.as_json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_else(|_| "{}".into())
        );
        return 0;
    }
    println!(
        "工作区: {}x{} @ ({}, {})",
        report.work.width, report.work.height, report.work.x, report.work.y
    );
    if report.candidates.is_empty() {
        println!("{}", report.message);
        return EXIT_NOT_FOUND;
    }
    if report.layout.is_empty() {
        // 最小化 / 还原没有目标矩形，只列窗口
        for c in &report.candidates {
            println!("{}\t{}\t{}", c.pid, c.process, c.title);
        }
    } else {
        for (c, r) in report.candidates.iter().zip(report.layout.iter()) {
            println!(
                "{}\t{}\t{}\t-> {},{} {}x{}",
                c.pid, c.process, c.title, r.x, r.y, r.width, r.height
            );
        }
    }
    println!("{}", report.message);
    0
}

/// 机器可读的能力清单：子命令、flag、退出码、文件检索是否可用。
fn cmd_capabilities(settings: &Settings, f: &CliFlags) -> i32 {
    let hint = settings.everything_es_path.clone();
    let es = crate::commands::everything_state(hint.as_deref().map(std::path::Path::new));
    let caps = json!({
        "ok": true,
        "name": "efto",
        "version": env!("CARGO_PKG_VERSION"),
        "subcommands": [
            "search", "launch", "apps", "index", "plugins", "plugin-run", "run-command",
            "files", "action", "ps", "focus", "tile-windows", "import-shortcuts",
            "capabilities", "version", "help"
        ],
        "exit_codes": { "ok": 0, "error": 1, "bad_args": 2, "not_found": 3 },
        "flags": {
            "--json": ["search", "launch", "apps", "index", "plugins", "plugin-run", "run-command", "files", "action", "ps", "focus", "tile-windows", "import-shortcuts", "capabilities"],
            "--dry-run": ["launch", "files", "action", "focus", "tile-windows", "run-command", "import-shortcuts"],
            "--no-track": ["launch"],
            "--reveal": ["files"],
            "--pid": ["focus"],
            "--mode": ["tile-windows"],
            "--file": ["import-shortcuts"],
            "--exe": ["import-shortcuts"],
            "--limit": ["search", "apps"],
            "--kind": ["search"],
            "--exact": ["search"],
            "--id": ["launch"],
            "--wait-ms": ["plugin-run"],
            "--timeout-ms": ["plugin-run", "run-command"],
            "--quiet": ["plugin-run"],
        },
        "actions": {
            "headless": ["reindex", "open_plugin_dir", "open_data_dir", "lock_screen"],
            "gui_only": ["settings", "screenshot", "clipboard", "quit"],
        },
        "file_search": {
            "state": es.state,
            "es_path": es.es_path,
            "message": es.message,
        },
        "shortcuts_import": {
            "command": "import-shortcuts",
            "target": "~/.efto/shortcuts/<exe>.json",
            "format": {
                "app": { "exe": ["trae cn.exe"], "name": "Trae" },
                "groups": [{
                    "name": "文本编辑",
                    "items": [{ "keys": "Ctrl+Enter", "action": "新建一行进行编辑", "note": "可选" }]
                }]
            }
        },
    });
    if f.as_json {
        println!("{}", serde_json::to_string_pretty(&caps).unwrap());
        return 0;
    }
    println!("efto {} CLI", env!("CARGO_PKG_VERSION"));
    println!("子命令: search launch apps index plugins plugin-run run-command files action ps focus tile-windows import-shortcuts capabilities version help");
    println!("退出码: 0 成功 / 1 执行失败 / 2 用法错误 / 3 查无结果");
    println!("文件检索: {}（{}）", es.state, es.message);
    0
}

/// 收集并打印插件事件；返回全部事件供调用方判定。
fn report_events(
    runtime: &mut efto_jsrt::PluginRuntime,
    quiet: bool,
    budget_ms: u64,
) -> Vec<RuntimeEvent> {
    let mut events: Vec<RuntimeEvent> = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget_ms);
    while std::time::Instant::now() < deadline {
        let got = runtime.drain_events();
        if got.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        } else {
            events.extend(got);
        }
    }
    if !quiet {
        for e in &events {
            match &e.event {
                PluginEvent::Error { stage, message, stack } => {
                    println!("[error:{stage}] {message}{}", stack_suffix(stack));
                }
                PluginEvent::Log { level, text } => println!("[log:{level}] {text}"),
                PluginEvent::ToUI { name, data } => println!("[to-ui] {name} {}", compact(data)),
                other => println!("[event] {other:?}"),
            }
        }
    }
    events
}

fn stack_suffix(stack: &Option<String>) -> String {
    match stack {
        Some(s) => format!("\n{}", shorten(s, 400)),
        None => String::new(),
    }
}

/// 插件搜索目录。env 解析已统一到 core（`Settings::effective_plugin_dirs`），
/// 这里只是把 env 值注入进去，保证 GUI 与 CLI 完全同一套规则。
fn effective_dirs(settings: &Settings) -> Vec<PathBuf> {
    settings.effective_plugin_dirs()
}

/// 发现插件并带上用户自定义触发词，保证 `cli search` 与 GUI 搜索一致。
fn discover_plugins(settings: &Settings) -> PluginRegistry {
    let mut registry = PluginRegistry::discover(&effective_dirs(settings));
    registry.set_cmd_aliases(settings.cmd_aliases.clone());
    // 停用的插件与 GUI 一致：搜不到、唤起不了
    registry.set_disabled(&settings.disabled_plugins);
    // 与 GUI 搜索同源：插件用 efto.setSearchItems 登记的自定义项也要能搜到
    crate::state::load_dynamic_items(&mut registry);
    registry
}

fn arg_str(args: &[Value], i: usize) -> Result<String, String> {
    args.get(i)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("第 {} 个参数必须是字符串", i + 1))
}

/// 首个字符串参数的宽松读取（回显用，读不出就退回 JSON 形式）。
fn first_str(args: &[Value]) -> String {
    args.first()
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

fn compact(v: &Value) -> String {
    let s = v.to_string();
    shorten(&s, 160)
}

fn shorten(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// CLI 侧写剪贴板（与 GUI 宿主共用实现，避免行为分叉）。
fn host_cli_copy(text: &str) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(efto_core::fsutil::data_root().join("last-copy.txt"))
        .map_err(|e| format!("无法写 last-copy.txt: {e}"))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("写入失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_keeps_minus_leading_expressions() {
        // 回归：`-2^2` 曾被当作 flag 丢弃
        let f = parse_args(&[
            "search".to_string(),
            "-2^2".to_string(),
            "--json".to_string(),
        ])
        .unwrap();
        assert_eq!(f.positional, vec!["-2^2".to_string()]);
        assert!(!f.quiet);
        assert!(f.as_json);

        let f2 = parse_args(&["plugin-run".to_string(), "a".to_string(), "--quiet".to_string()])
            .unwrap();
        assert_eq!(f2.positional, vec!["a".to_string()]);
        assert!(f2.quiet);
        assert!(!f2.as_json);

        // 只有子命令时位置参数为空，不 panic
        let f3 = parse_args(&["index".to_string()]).unwrap();
        assert!(f3.positional.is_empty());
    }

    #[test]
    fn parse_args_reads_value_flags() {
        let f = parse_args(&[
            "search".to_string(),
            "chrome".to_string(),
            "--limit".to_string(),
            "5".to_string(),
            "--kind=app,plugin".to_string(),
        ])
        .unwrap();
        assert_eq!(f.positional, vec!["chrome".to_string()]);
        assert_eq!(f.limit, Some(5));
        assert_eq!(f.kinds, Some(vec![ItemKind::App, ItemKind::Plugin]));
        assert!(parse_args(&[
            "search".to_string(),
            "x".to_string(),
            "--exact".to_string(),
        ])
        .unwrap()
        .exact);

        let f2 = parse_args(&[
            "plugin-run".to_string(),
            "p".to_string(),
            "c".to_string(),
            "--wait-ms=10".to_string(),
            "--timeout-ms".to_string(),
            "20".to_string(),
        ])
        .unwrap();
        assert_eq!(f2.wait_ms, Some(10));
        assert_eq!(f2.timeout_ms, Some(20));

        let f3 = parse_args(&[
            "launch".to_string(),
            "--id".to_string(),
            "exe:c:/x.exe".to_string(),
        ])
        .unwrap();
        assert_eq!(f3.id.as_deref(), Some("exe:c:/x.exe"));
        assert!(f3.positional.is_empty());
    }

    #[test]
    fn parse_args_rejects_bad_values() {
        assert!(parse_args(&["search".to_string(), "--limit".to_string(), "0".to_string()]).is_err());
        assert!(parse_args(&["search".to_string(), "--kind".to_string(), "nope".to_string()]).is_err());
        assert!(parse_args(&["search".to_string(), "--limit".to_string()]).is_err());
    }

    #[test]
    fn parse_args_reads_security_flags() {
        let f = parse_args(&[
            "launch".to_string(),
            "chrome".to_string(),
            "--dry-run".to_string(),
            "--no-track".to_string(),
        ])
        .unwrap();
        assert!(f.dry_run && f.no_track && !f.reveal);

        let f2 = parse_args(&[
            "files".to_string(),
            "C:/x.txt".to_string(),
            "--reveal".to_string(),
        ])
        .unwrap();
        assert!(f2.reveal);

        let f3 = parse_args(&["focus".to_string(), "--pid".to_string(), "42".to_string()]).unwrap();
        assert_eq!(f3.pid, Some(42));
        assert_eq!(
            parse_args(&["focus".to_string(), "--pid=7".to_string()])
                .unwrap()
                .pid,
            Some(7)
        );
        assert!(parse_args(&["focus".to_string(), "--pid=abc".to_string()]).is_err());
    }

    #[test]
    fn usage_lists_every_subcommand() {
        for cmd in [
            "search",
            "launch",
            "apps",
            "index",
            "plugins",
            "plugin-run",
            "run-command",
            "files",
            "action",
            "ps",
            "focus",
            "tile-windows",
            "capabilities",
            "version",
            "help",
        ] {
            assert!(USAGE.contains(cmd), "USAGE 缺少 {cmd}");
        }
    }

    #[test]
    fn shorten_keeps_length() {
        assert_eq!(shorten("abcd", 10), "abcd");
        let s = shorten("一二三四五", 3);
        assert_eq!(s.chars().count(), 4, "{s}");
        assert!(s.ends_with('…'));
    }

    #[test]
    fn cli_host_db_roundtrip_is_isolated_per_plugin() {
        let host = CliHost::new(true);
        let key = "k";
        host.call("pA", "db", "set", &[json!(key), json!("from-A")])
            .unwrap();
        host.call("pB", "db", "set", &[json!(key), json!("from-B")])
            .unwrap();
        assert_eq!(
            host.call("pA", "db", "get", &[json!(key)]).unwrap(),
            json!("from-A")
        );
        assert_eq!(
            host.call("pB", "db", "get", &[json!(key)]).unwrap(),
            json!("from-B")
        );
        let missing = host.call("pC", "db", "get", &[json!(key)]).unwrap();
        assert_eq!(missing, Value::Null, "未写过的插件不该看到别人的数据");
    }

    #[test]
    fn cli_host_flush_persists_plugin_data() {
        let id = format!("ut-cli-flush-{}", std::process::id());
        let path = efto_core::store::PluginStore::file_path(&id);
        let _ = std::fs::remove_file(&path);

        let host = CliHost::new(true);
        host.call(&id, "db", "set", &[json!("k"), json!("v")])
            .unwrap();
        host.flush().unwrap();

        let reopened = efto_core::store::PluginStore::open(&id);
        assert_eq!(
            reopened.get("k"),
            Some(json!("v")),
            "CLI 进程退出前必须把插件数据落盘"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cli_host_rejects_unknown_capability() {
        let host = CliHost::new(true);
        let e = host.call("p", "nope", "x", &[]).unwrap_err();
        assert!(e.contains("未实现"), "{e}");
    }

    #[test]
    fn arg_str_errors_are_readable() {
        assert_eq!(arg_str(&[json!("a")], 0).unwrap(), "a");
        assert_eq!(arg_str(&[json!(1)], 0).unwrap_err(), "第 1 个参数必须是字符串");
    }
}
