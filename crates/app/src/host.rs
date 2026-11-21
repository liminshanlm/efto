//! 宿主能力实现：把 jsrt 的 `Host` 契约落到 Tauri + Windows 上。
//!
//! 设计原则：
//! - 同步能力只做"快"的事（读写本地 JSON、启动进程、复制文本），绝不阻塞插件线程等 UI；
//! - 涉及窗口的能力一律异步（jsrt 已按模块分类），通过 `run_on_main_thread` 投递，不等待结果；
//! - 所有路径操作都限定在插件根目录内，越界直接拒绝。

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use efto_core::apps::launch;
use efto_core::fsutil;
use efto_core::plugins::registry::DynamicItem;
use efto_jsrt::{Host, ReplyTo};

use efto_core::exec;
use crate::state::Shared;
use crate::windows;

/// `efto.exec({ name, args, timeoutMs })` 的参数校验与落地。
///
/// GUI 与 CLI **共用这一个入口**（`cli.rs` 直接调用它）：两处各写一份参数校验，
/// 迟早会出现"CLI 能跑、GUI 拒绝"或反过来 —— 这个项目的 `EFTO_PLUGIN_DIRS`
/// 就已经踩过一次类似的分裂。
///
/// 这里刻意**不**接受 `path`/`cwd`/`shell`/`env` 之类的字段：能指定任意路径就等于
/// 绕过白名单。名字只认 [`exec::ALLOWLIST`]，实际文件由宿主自己按优先级发现，
/// 并且必须通过内置 sha256 校验（见 `exec::resolve_verified`）。
/// 参数数组原样交给 CreateProcess，不经过任何 shell。
pub(crate) fn exec_capability(
    plugin: &str,
    opts: &Value,
    es_hint: Option<&Path>,
) -> Result<Value, String> {
    let name = opts
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("exec 需要 name（白名单里的程序名，目前只有 \"es\"）")?;
    for forbidden in ["path", "program", "command", "shell", "cwd", "env", "file"] {
        if opts.get(forbidden).is_some() {
            return Err(format!("exec 不接受 {forbidden:?} 字段：只允许按白名单名字执行"));
        }
    }
    let arr = opts
        .get("args")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let arr = arr.as_array().ok_or("exec 的 args 必须是字符串数组")?;
    let mut args: Vec<String> = Vec::with_capacity(arr.len());
    for a in arr {
        args.push(
            a.as_str()
                .ok_or("exec 的 args 只能是字符串（数字请先转成字符串）")?
                .to_string(),
        );
    }
    let timeout = opts
        .get("timeoutMs")
        .and_then(|v| v.as_u64())
        .map(Duration::from_millis)
        .unwrap_or(exec::DEFAULT_TIMEOUT);

    // 阻塞执行：jsrt 把能力调用投递到独立线程池，不会卡住插件 isolate 之外的东西
    match exec::run(name, &args, timeout, es_hint) {
        Ok(r) => {
            tracing::info!(
                target: "efto::exec",
                plugin,
                name,
                code = r.code,
                "受限执行完成（{} 个参数）",
                args.len()
            );
            Ok(json!({
                "ok": true,
                "code": r.code,
                "stdout": r.stdout,
                "stderr": r.stderr,
                "path": r.path,
                "truncated": r.truncated,
            }))
        }
        Err(e) => {
            // 拒绝也要留痕：白名单/摘要拦截是安全事件，不该只回到插件里
            tracing::warn!(target: "efto::exec", plugin, name, "拒绝执行: {e}");
            Err(e.to_string())
        }
    }
}

pub struct TauriHost {
    app: AppHandle,
}

/// `efto.node.run` / `efto.python.run` 的落地。
///
/// 解释器**只能来自设置**（`Settings::node_path` / `python_path`）。插件参数里出现
/// `path`/`interpreter`/`program` 等字段一律拒绝 —— 否则就等于把 `efto.exec` 的
/// 白名单绕过成任意程序执行。脚本用 `-e`/`-c` 单参数传入，不落临时文件。
///
/// GUI 与 CLI 共用（和 `exec_capability` 一样），参数校验只写这一份。
pub(crate) fn runtime_capability(
    opts: &Value,
    node: Option<&str>,
    python: Option<&str>,
    cwd: Option<&Path>,
) -> Result<Value, String> {
    let kind = opts
        .get("runtime")
        .and_then(|v| v.as_str())
        .ok_or("runtime.run 需要 runtime（\"node\" 或 \"python\"）")?;
    let rt = exec::script_runtime(kind)
        .ok_or_else(|| format!("不支持的运行时 {kind:?}：只支持 node / python"))?;
    for forbidden in ["path", "interpreter", "program", "command", "shell", "cwd", "env", "file"] {
        if opts.get(forbidden).is_some() {
            return Err(format!(
                "runtime.run 不接受 {forbidden:?} 字段：解释器由 设置 → 运行时 决定"
            ));
        }
    }
    let code = opts
        .get("code")
        .and_then(|v| v.as_str())
        .ok_or("runtime.run 需要 code（脚本源码字符串）")?;
    let args: Vec<String> = match opts.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(xs)) => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                out.push(
                    x.as_str()
                        .ok_or("args 只能是字符串（数字请先转成字符串）")?
                        .to_string(),
                );
            }
            out
        }
        Some(_) => return Err("args 必须是字符串数组".into()),
    };
    let timeout = opts
        .get("timeoutMs")
        .and_then(|v| v.as_u64())
        .map(Duration::from_millis)
        .unwrap_or(exec::RUNTIME_DEFAULT_TIMEOUT);

    let interpreter = match rt {
        exec::ScriptRuntime::Node => node,
        exec::ScriptRuntime::Python => python,
    };
    let interpreter = interpreter
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "未配置 {} 运行环境：请在 设置 → 运行时 中填写解释器路径并点“检测”",
                rt.display()
            )
        })?;

    match exec::run_script(rt, interpreter, code, &args, timeout, cwd) {
        Ok(r) => Ok(json!({
            "runtime": rt.key(),
            "ok": true,
            "code": r.code,
            "stdout": r.stdout,
            "stderr": r.stderr,
            "path": r.path,
            "truncated": r.truncated,
        })),
        Err(e) => {
            tracing::warn!(target: "efto::runtime", runtime = rt.key(), "脚本执行失败: {e}");
            Err(e.to_string())
        }
    }
}

impl TauriHost {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn data(&self) -> Shared {
        self.app.state::<Shared>().inner().clone()
    }

    fn db(&self, plugin: &str, f: impl FnOnce(&mut efto_core::store::PluginStore) -> Result<Value, String>) -> Result<Value, String> {
        let data = self.data();
        let mut stores = data.stores.lock();
        let store = stores.get_or_open(plugin);
        f(store)
    }
}

impl Host for TauriHost {
    fn call(&self, plugin: &str, module: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        let data = self.data();
        match (module, method) {
            // ---------------- db（键值存储） ----------------
            ("db", "get") => {
                let key = arg_str(args, 0)?;
                self.db(plugin, |s| Ok(s.get(&key).unwrap_or(Value::Null)))
            }
            ("db", "set") => {
                let key = arg_str(args, 0)?;
                let value = args.get(1).cloned().unwrap_or(Value::Null);
                self.db(plugin, |s| s.set(&key, value).map(|_| Value::Bool(true)).map_err(|e| e.to_string()))
            }
            ("db", "remove") => {
                let key = arg_str(args, 0)?;
                self.db(plugin, |s| Ok(Value::Bool(s.remove(&key))))
            }
            ("db", "all") => self.db(plugin, |s| Ok(s.all())),
            ("db", "keys") => self.db(plugin, |s| {
                Ok(Value::Array(
                    s.keys().into_iter().map(Value::String).collect(),
                ))
            }),
            ("db", "size") => self.db(plugin, |s| Ok(json!({ "keys": s.keys().len(), "size": s.size_bytes() }))),
            ("db", "clear") => self.db(plugin, |s| {
                s.clear();
                Ok(Value::Bool(true))
            }),

            // ---------------- 局域网只读文件共享（原生插件 file-share） ----------------
            ("share", "start") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                data.share_start(&opts)
            }
            ("share", "stop") => Ok(data.share_stop()),
            ("share", "status") => Ok(data.share_status()),

            // ---------------- 动态搜索项（插件把自己的数据挂到启动器搜索） ----------------
            ("search", "setItems") => {
                let items = args
                    .first()
                    .cloned()
                    .map(serde_json::from_value::<Vec<DynamicItem>>)
                    .transpose()
                    .map_err(|e| format!("setItems 参数格式错误: {e}"))?
                    .unwrap_or_default();
                data.set_plugin_search_items(plugin, items)?;
                Ok(Value::Null)
            }

            // ---------------- Windows hosts（原生插件 hosts） ----------------
            ("hosts", "path") => crate::hosts::path_capability(),
            ("hosts", "read") => crate::hosts::read_capability(),

            // ---------------- 大模型（设置 → 模型，OpenAI 兼容） ----------------
            ("ai", "models") => Ok(crate::ai::models(&self.data().settings.lock())),

            // ---------------- 系统与外壳 ----------------
            ("system", "copyText") => {
                let text = arg_str(args, 0)?;
                copy_text(&text)
            }
            ("system", "pasteText") => paste_text(),
            ("system", "pasteImage") => paste_image(),
            ("system", "imagePreview") => image_preview(args),
            ("system", "dbPath") => Ok(Value::String(fsutil::data_root().to_string_lossy().to_string())),
            ("system", "getPath") => special_dir(&arg_str(args, 0)?).map(Value::String),
            ("system", "quit") => {
                let app = self.app.clone();
                std::thread::spawn(move || {
                    app.state::<Shared>().flush_stores();
                    app.state::<Shared>().runtime.lock().stop_all();
                    app.exit(0);
                });
                Ok(Value::Bool(true))
            }
            ("system", "relaunch") => self.app.restart(),
            ("system", "getDisplays") => Ok(json!([])),
            ("system", "createShortcut") | ("system", "removeShortcut") => Err(
                "第一阶段未实现桌面快捷方式（需要 Shell API），请改用 efto.shellOpenPath 打开插件目录".into(),
            ),
            ("system", "sendToFocusWindow") => {
                let text = args.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                let _ = self.app.emit_to(tauri::EventTarget::window("main"), "plugin:input", text);
                Ok(Value::Bool(true))
            }

            ("shell", "openPath") => {
                let p = arg_str(args, 0)?;
                launch::shell_open(&p, None).map(|_| Value::Bool(true)).map_err(|e| e.to_string())
            }
            ("shell", "openExternal") => {
                let url = arg_str(args, 0)?;
                if !(url.starts_with("http://") || url.starts_with("https://") || url.contains(':')) {
                    return Err("openExternal 只接受 URL 或协议串".into());
                }
                launch::shell_open(&url, None).map(|_| Value::Bool(true)).map_err(|e| e.to_string())
            }
            ("shell", "showItemInFolder") => {
                let p = arg_str(args, 0)?;
                let parent = Path::new(&p)
                    .parent()
                    .map(|x| x.to_string_lossy().to_string())
                    .unwrap_or(p.clone());
                launch::shell_open(&parent, None).map(|_| Value::Bool(true)).map_err(|e| e.to_string())
            }

            ("notify", "show") => {
                let title = args.first().and_then(|v| v.as_str()).unwrap_or("efto");
                let body = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
                use tauri_plugin_notification::NotificationExt;
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title(title)
                    .body(if body.is_empty() { title } else { body })
                    .show();
                Ok(Value::Bool(true))
            }
            ("notify", "hide") => Ok(Value::Bool(false)),

            // ---------------- UI 侧同步能力（不改窗口状态，只回传信息） ----------------
            ("ui", "setSubTitle") => {
                let text = args.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                let _ = self.app.emit_to(tauri::EventTarget::window("main"), "plugin:subtitle", text);
                Ok(Value::Bool(true))
            }
            ("ui", "closeWindow") => {
                let name = args.first().and_then(|v| v.as_str()).unwrap_or("");
                windows::close_plugin_ui(&self.app, plugin, name);
                Ok(Value::Bool(true))
            }
            ("ui", "ipcMessage") => {
                // UI → main 的显式转发（多数插件用 SDK 的 invoke 直接走命令）
                let name = arg_str(args, 0)?;
                let data = args.get(1).cloned().unwrap_or(Value::Null);
                self.app
                    .state::<Shared>()
                    .runtime
                    .lock()
                    .ui_message(plugin, &name, data)
                    .map_err(|e| e.to_string())?;
                Ok(Value::Bool(true))
            }

            // ---------------- require 的模块解析与读取（与 CLI 宿主共用 jsrt::modules） ----------------
            ("module", "resolve") | ("module", "load") => {
                let root = data
                    .plugin_root(plugin)
                    .ok_or_else(|| format!("未知插件: {plugin}"))?;
                efto_jsrt::modules::handle(&root, method, args).map_err(|e| e.to_string())
            }

            // ---------------- 插件内联输出（原生协议） ----------------
            // 没有 UI 的插件用 `efto.output(rows)` 把结果推到搜索框下方列表。
            // 这里只负责校验 + 广播；是否收起窗口由前端按"有没有收到输出"决定。
            ("output", "set") => {
                let rows = args.first().cloned().unwrap_or(Value::Null);
                let hits = efto_core::plugins::hits_from_rows(plugin, &rows)?;
                windows::show_main_quiet(&self.app);
                let _ = self.app.emit_to(
                    tauri::EventTarget::window("main"),
                    "plugin:output",
                    json!({ "plugin": plugin, "items": hits }),
                );
                Ok(Value::Bool(true))
            }

            // ---------------- 广播 ----------------
            // 插件 console.* 的落点。之前模块存在但没人实现，插件侧日志全丢，
            // 白屏/点了没反应时在宿主侧完全看不到线索。
            ("log", level) => {
                let msg = arg_str(args, 0).unwrap_or_default();
                match level {
                    "error" => tracing::error!("[plugin:{plugin}] {msg}"),
                    "warn" => tracing::warn!("[plugin:{plugin}] {msg}"),
                    "info" => tracing::info!("[plugin:{plugin}] {msg}"),
                    "debug" => tracing::debug!("[plugin:{plugin}] {msg}"),
                    "trace" => tracing::trace!("[plugin:{plugin}] {msg}"),
                    _ => tracing::info!("[plugin:{plugin}] {msg}"),
                }
                Ok(Value::Bool(true))
            }
            ("broadcast", "send") => {
                let channel = arg_str(args, 0)?;
                let data_arg = args.get(1).cloned().unwrap_or(Value::Null);
                let _ = self.app.emit("plugin:broadcast", json!({ "channel": channel, "data": data_arg, "from": plugin }));
                let rt = data.runtime.lock();
                for id in rt.running() {
                    if id != plugin {
                        let _ = rt.main_message(&id, &channel, data_arg.clone());
                    }
                }
                Ok(Value::Bool(true))
            }

            // ---------------- 明确未实现（诚实报错，不做静默成功） ----------------
            ("ui", "showUI") | ("ui", "hideUI") | ("ui", "showMainWindow") | ("ui", "hideMainWindow")
            | ("redirect", "to")
            | ("ui", "postMessage") | ("efto", "exec") | ("runtime", "run") | ("hosts", "save")
            | ("ai", "chat") | ("ai", "ocr") => {
                Err(format!("{module}.{method} 是异步能力，请通过 efto 的 Promise 接口调用"))
            }
            (m, meth) => Err(format!("宿主未实现的能力: {m}.{meth}")),
        }
    }

    fn call_async(
        &self,
        plugin: &str,
        module: &str,
        method: &str,
        args: &[Value],
        reply: ReplyTo,
        id: u64,
    ) {
        let outcome: Result<Value, String> = match (module, method) {
            ("ui", "showUI") => {
                let options = args.first().cloned().unwrap_or(Value::Object(Default::default()));
                self.show_plugin_ui(plugin, &options).map(|_| json!({ "ok": true }))
            }
            ("ui", "hideUI") => {
                windows::hide_plugin_ui(&self.app, plugin);
                Ok(json!({ "ok": true }))
            }
            ("ui", "showMainWindow") => {
                windows::show_main(&self.app);
                Ok(json!({ "ok": true }))
            }
            ("ui", "hideMainWindow") => {
                windows::hide_main(&self.app);
                Ok(json!({ "ok": true }))
            }
            ("redirect", "to") => {
                let label = arg_str(args, 0).unwrap_or_default();
                let options = args.get(1).cloned().unwrap_or(Value::Null);
                redirect_to(&self.app, &label, &options)
            }
            // main -> UI 页面（与上面的 ui.ipcMessage 方向相反，别合并）
            ("ui", "postMessage") => {
                // call_async 返回 ()，不能用 `?`；缺 name 就当空名继续（不至于 panic）
                let name = arg_str(args, 0).unwrap_or_default();
                let data = args.get(1).cloned().unwrap_or(Value::Null);
                self.send_to_ui(plugin, &name, &data);
                Ok(Value::Bool(true))
            }
            // ---------------- efto 扩展（见 efto_core::exec） ----------------
            ("efto", "exec") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let hint = self.data().settings.lock().everything_es_path.clone();
                // Option<String> -> Option<&Path>：借用必须先绑到局部变量，否则 guard 提前释放
                let hint_path = hint.as_deref().map(std::path::Path::new);
                exec_capability(plugin, &opts, hint_path)
            }
            // 脚本可能跑几十秒：和 exec 一样走异步，不占插件事件循环
            ("runtime", "run") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let data = self.data();
                let (node, python) = {
                    let s = data.settings.lock();
                    (s.node_path.clone(), s.python_path.clone())
                };
                let root = data.plugin_root(plugin);
                runtime_capability(&opts, node.as_deref(), python.as_deref(), root.as_deref())
            }
            // hosts 保存要等 UAC 走完（可能几秒到几十秒），必须异步
            ("hosts", "save") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                crate::hosts::save_capability(&opts)
            }
            // 大模型调用是网络请求（可能几十秒），必须异步
            ("ai", "chat") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let settings = self.data().settings.lock().clone();
                if opts.get("stream").and_then(|v| v.as_bool()).unwrap_or(false) {
                    // 流式：先逐段发 Stream，最后仍发一次 Reply 收尾
                    let stream_reply = reply.clone();
                    let result = crate::ai::chat_stream(&settings, &opts, &mut |delta| {
                        let _ =
                            stream_reply.send(efto_jsrt::HostEvent::Stream { id, delta });
                    });
                    let _ = reply.send(reply_event(id, result));
                    return;
                }
                crate::ai::chat(&settings, &opts)
            }
            // OCR 可能要上传几十 MB、识别上百秒，必须异步
            ("ai", "ocr") => {
                let opts = args.first().cloned().unwrap_or(Value::Null);
                let settings = self.data().settings.lock().clone();
                crate::ai::ocr(&settings, &opts)
            }
            _ => self.call(plugin, module, method, args),
        };
        let _ = reply.send(reply_event(id, outcome));
    }

    fn send_to_ui(&self, plugin: &str, name: &str, data: &Value) {
        windows::emit_to_plugin_ui(&self.app, plugin, name, data);
    }
}

impl TauriHost {
    fn show_plugin_ui(&self, plugin: &str, options: &Value) -> Result<(), String> {
        // State 是临时借用，必须先绑定到局部变量，否则 guard 会借用一个已释放的临时值
        let data = self.data();
        let root = data
            .plugin_root(plugin)
            .ok_or_else(|| format!("未知插件 {plugin}"))?;
        let entry = {
            let reg = data.registry.read();
            let p = reg.get(plugin).ok_or_else(|| format!("未知插件 {plugin}"))?;
            p.ui_entry
                .clone()
                .ok_or_else(|| "该插件没有 UI 入口(index.html)".to_string())?
        };
        let rel = entry
            .strip_prefix(&root)
            .map_err(|_| "UI 入口不在插件目录内".to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let title = options
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| data.plugin_name(plugin));
        let width = options.get("width").and_then(|v| v.as_f64()).unwrap_or(760.0);
        let height = options.get("height").and_then(|v| v.as_f64()).unwrap_or(480.0);
        windows::open_plugin_ui(&self.app, plugin, &rel, &title, width, height)
    }
}

// ------------------------------------------------------------------
// 辅助
// ------------------------------------------------------------------

fn reply_event(id: u64, outcome: Result<Value, String>) -> efto_jsrt::HostEvent {
    match outcome {
        Ok(v) => efto_jsrt::HostEvent::Reply { id, ok: true, result: v },
        Err(e) => efto_jsrt::HostEvent::Reply {
            id,
            ok: false,
            result: Value::String(e),
        },
    }
}

fn arg_str(args: &[Value], i: usize) -> Result<String, String> {
    args.get(i)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("第 {} 个参数必须是字符串", i + 1))
}

fn special_dir(name: &str) -> Result<String, String> {
    let found = match name {
        "home" | "userHome" => dirs::home_dir(),
        "desktop" => dirs::desktop_dir(),
        "documents" => dirs::document_dir(),
        "downloads" => dirs::download_dir(),
        "data" | "appData" | "userData" => Some(fsutil::data_root()),
        "temp" => Some(std::env::temp_dir()),
        other => return Err(format!("未知目录名: {other}")),
    };
    let p = found.ok_or_else(|| "该平台无法定位此目录".to_string())?;
    Ok(p.to_string_lossy().to_string())
}

/// 剪贴板用 arboard（纯 Rust 绑定，Windows 走 user32/ole32，不引入 C 依赖，中文不会乱码）。
pub(crate) fn copy_text(text: &str) -> Result<Value, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    cb.set_text(text.to_string())
        .map_err(|e| format!("写入剪贴板失败: {e}"))?;
    Ok(Value::Bool(true))
}

fn paste_text() -> Result<Value, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    cb.get_text()
        .map(Value::String)
        .map_err(|e| format!("读取剪贴板失败: {e}"))
}

/// 读剪贴板里的位图，落成临时 PNG 并返回绝对路径；没有图片时返回空字符串。
///
/// 插件（尤其是 V8 主进程）拿不到剪贴板位图，也写不了文件，所以由宿主落盘，
/// 再配合 `ai.chat` 的 `{ type: 'image', path }` 使用。固定文件名会覆盖上一次粘贴。
fn paste_image() -> Result<Value, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用: {e}"))?;
    let img = match cb.get_image() {
        Ok(img) => img,
        // 剪贴板里是文本 / 文件 / 空，都按"没有图片"处理
        Err(_) => return Ok(Value::String(String::new())),
    };
    let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
        .ok_or_else(|| "剪贴板图片尺寸无效".to_string())?;
    let bytes = crate::screenshot::png_bytes(&rgba)?;
    let dir = std::env::temp_dir().join("efto");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败: {e}"))?;
    let path = dir.join("clipboard.png");
    std::fs::write(&path, bytes).map_err(|e| format!("写入临时图片失败: {e}"))?;
    Ok(Value::String(path.to_string_lossy().to_string()))
}

/// 本地图片预览：返回 data URL。能解码就缩到 640px 内再编码 PNG，
/// 避免几 MB 的原图塞进一条 IPC 消息；解码不了（webp/gif/bmp 等）就原样 base64。
fn image_preview(args: &[Value]) -> Result<Value, String> {
    let path = args.first().and_then(|v| v.as_str()).map(str::trim).unwrap_or("");
    if path.is_empty() {
        return Ok(Value::String(String::new()));
    }
    let mime = crate::ai::image_mime(path)?;
    let meta = std::fs::metadata(path).map_err(|e| format!("读取图片失败（{path}）：{e}"))?;
    if meta.len() > crate::ai::MAX_IMAGE_BYTES {
        return Err(format!("图片过大（{} 字节）", meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取图片失败（{path}）：{e}"))?;
    if let Ok(img) = image::load_from_memory(&bytes) {
        let thumb = img.thumbnail(640, 640);
        if let Ok(png) = crate::screenshot::png_bytes(&thumb.to_rgba8()) {
            return Ok(Value::String(crate::ai::data_url("image/png", &png)));
        }
    }
    Ok(Value::String(crate::ai::data_url(mime, &bytes)))
}

/// `redirect`：window 模式打开内置页面，url 模式交给系统浏览器。
pub(crate) fn redirect_to(app: &AppHandle, label: &str, options: &Value) -> Result<Value, String> {
    if let Some(url) = options.get("url").and_then(|v| v.as_str()) {
        launch::shell_open(url, None).map_err(|e| e.to_string())?;
        return Ok(json!({ "ok": true, "opened": url }));
    }
    if let Some(win) = options.get("window").and_then(|v| v.as_str()) {
        // window 形如 "index.html" 或插件内相对路径
        let plugin = options.get("plugin").and_then(|v| v.as_str()).unwrap_or("");
        let _ = win;
        windows::show_main(app);
        let _ = app.emit_to(tauri::EventTarget::window("main"), "plugin:redirect", json!({ "label": label, "from": plugin }));
        return Ok(json!({ "ok": true }));
    }
    windows::show_main(app);
    let _ = app.emit_to(tauri::EventTarget::window("main"), "plugin:redirect", json!({ "label": label }));
    Ok(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_str_requires_string() {
        assert_eq!(arg_str(&[json!("a")], 0).unwrap(), "a");
        assert!(arg_str(&[json!(1)], 0).is_err());
        assert!(arg_str(&[], 0).is_err());
    }

    #[test]
    fn special_dirs_known_names() {
        assert!(special_dir("data").is_ok());
        assert!(special_dir("home").is_ok());
        assert!(special_dir("nonsense").is_err());
    }

    /// exec 的 **JSON 边界**测试 —— 这一层直接面对不可信插件代码，
    /// 花样必须在到达 exec::run 之前就被挡掉。
    #[test]
    fn exec_boundary_refuses_every_way_to_escape_the_allowlist() {
        // 缺 name
        let e = exec_capability("p", &json!({ "args": [] }), None).unwrap_err();
        assert!(e.contains("name"), "{e}");
        // 非白名单名字
        for bad in ["cmd", "powershell", "whoami", "es.exe", "../tools/es/evil"] {
            let e = exec_capability("p", &json!({ "name": bad, "args": [] }), None).unwrap_err();
            assert!(e.contains("白名单"), "{bad} 应被拒绝：{e}");
        }
        // 试图绕过名字解析：给出路径/命令/shell/工作目录/环境
        for k in ["path", "program", "command", "shell", "cwd", "env", "file"] {
            let mut v = json!({ "name": "es", "args": [] });
            v.as_object_mut().unwrap().insert(k.into(), json!("C:\\Windows\\System32\\cmd.exe"));
            let e = exec_capability("p", &v, None).unwrap_err();
            assert!(e.contains(&format!("{k:?}")) || e.contains(k), "{k} 应被拒绝：{e}");
        }
        // args 形状
        let e = exec_capability("p", &json!({ "name": "es", "args": "-n 1" }), None).unwrap_err();
        assert!(e.contains("数组"), "{e}");
        let e = exec_capability("p", &json!({ "name": "es", "args": [1] }), None).unwrap_err();
        assert!(e.contains("字符串"), "{e}");
        // 参数级：es 能写文件的那些开关
        let e = exec_capability(
            "p",
            &json!({ "name": "es", "args": ["-export-csv", "C:\\Users\\x\\Start Menu\\s.csv"] }),
            None,
        )
        .unwrap_err();
        assert!(e.contains("参数"), "{e}");
    }

    /// exec 的 happy path 走完整链路（JSON -> 白名单 -> 摘要校验 -> 子进程 -> 代码页解码）。
    /// 依赖真实 es.exe（Windows 程序）与控制台代码页语义，macOS 上没有对应物，
    /// 见 docs/macos-support-plan.md（Everything 检索源 Windows-only）。
    #[cfg(windows)]
    #[test]
    fn exec_happy_path_returns_decoded_output() {
        // 走完整链路（JSON -> 白名单 -> 摘要校验 -> 子进程 -> 代码页解码）
        let v = exec_capability("p", &json!({ "name": "es", "args": ["-version"] }), None)
            .expect("es -version 应成功");
        assert_eq!(v["code"].as_i64(), Some(0), "{v}");
        let out = v["stdout"].as_str().unwrap_or("");
        assert!(
            out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false),
            "版本号应以数字开头，实际 {out:?}"
        );
        assert!(v["path"].as_str().unwrap_or("").ends_with("es.exe"));
    }

    /// runtime 的 JSON 边界：解释器必须来自设置，插件不能自己指路径。
    #[test]
    fn runtime_boundary_requires_settings_and_refuses_paths() {
        // 未配置：错误文案必须指向设置页，而不是含糊的"找不到"
        let e = runtime_capability(&json!({ "runtime": "node", "code": "1" }), None, None, None)
            .unwrap_err();
        assert!(e.contains("设置"), "{e}");

        // 未知运行时
        let e = runtime_capability(&json!({ "runtime": "ruby", "code": "1" }), Some("node.exe"), None, None)
            .unwrap_err();
        assert!(e.contains("只支持"), "{e}");

        // 插件试图指定解释器/工作目录/环境：一律拒绝
        for k in ["path", "interpreter", "program", "command", "shell", "cwd", "env", "file"] {
            let mut v = json!({ "runtime": "node", "code": "1" });
            v.as_object_mut().unwrap().insert(k.into(), json!("x"));
            let e = runtime_capability(&v, Some("node.exe"), None, None).unwrap_err();
            assert!(e.contains(&format!("{k:?}")), "{k} 应被拒绝：{e}");
        }

        // code 缺失 / args 形状
        let e = runtime_capability(&json!({ "runtime": "node" }), Some("node.exe"), None, None)
            .unwrap_err();
        assert!(e.contains("code"), "{e}");
        let e = runtime_capability(
            &json!({ "runtime": "node", "code": "1", "args": "a" }),
            Some("node.exe"),
            None,
            None,
        )
        .unwrap_err();
        assert!(e.contains("数组"), "{e}");
        let e = runtime_capability(
            &json!({ "runtime": "node", "code": "1", "args": [1] }),
            Some("node.exe"),
            None,
            None,
        )
        .unwrap_err();
        assert!(e.contains("字符串"), "{e}");
    }
}
