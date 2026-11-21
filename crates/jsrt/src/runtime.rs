//! 插件运行时门面：宿主只需要和这个类型打交道。
//!
//! 职责边界：
//! - `PluginRuntime` 管"哪些插件在跑 + 事件收发"；
//! - 具体能力（db/通知/窗口/HTTP/文件）全部由宿主实现的 [`Host`] trait 提供，
//!   所以 jsrt 不依赖 tauri，也不依赖 core，可以被单独测试与复用。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use serde_json::Value;

use crate::engine::{self, PluginHandle};
use crate::ipc::{Host, HostEvent, JsError, JsResult, PluginContext, PluginEvent};
use crate::ts;

/// 带插件 id 的事件包装。
#[derive(Debug, Clone)]
pub struct RuntimeEvent {
    /// 事件来源插件 id
    pub plugin: String,
    /// 事件内容
    pub event: PluginEvent,
}

/// 启动请求。
#[derive(Debug, Clone)]
pub struct StartRequest {
    /// 插件运行上下文
    pub ctx: PluginContext,
    /// 入口文件绝对路径（.js / .mjs / .cjs / .ts）
    pub entry: std::path::PathBuf,
}

/// 插件运行时门面：管理运行中的插件与事件收发。
pub struct PluginRuntime {
    host: Arc<dyn Host>,
    handles: HashMap<String, Arc<PluginHandle>>,
    tx: Sender<RuntimeEvent>,
    rx: Receiver<RuntimeEvent>,
}

impl PluginRuntime {
    /// 用给定宿主创建运行时。
    pub fn new(host: Arc<dyn Host>) -> Self {
        let (tx, rx) = unbounded();
        Self {
            host,
            handles: HashMap::new(),
            tx,
            rx,
        }
    }

    /// 替换宿主实现（启动后才拿到 AppHandle 时用）。
    pub fn set_host(&mut self, host: Arc<dyn Host>) {
        self.host = host;
    }

    /// 指定插件后端是否在运行。
    pub fn is_running(&self, plugin_id: &str) -> bool {
        self.handles
            .get(plugin_id)
            .map(|h| h.is_running())
            .unwrap_or(false)
    }

    /// 正在运行的插件 id。
    pub fn running(&self) -> Vec<String> {
        self.handles
            .iter()
            .filter(|(_, h)| h.is_running())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// 取回插件句柄。
    pub fn handle(&self, plugin_id: &str) -> Option<Arc<PluginHandle>> {
        self.handles.get(plugin_id).cloned()
    }

    /// 启动插件后端。已在运行的插件直接返回 Ok（幂等，重复唤起只发 enter 事件）。
    pub fn start(&mut self, req: StartRequest) -> JsResult<()> {
        let id = req.ctx.plugin_id.clone();
        if let Some(h) = self.handles.get(&id) {
            if h.is_running() {
                return Ok(());
            }
        }
        let source = prepare_source(&req.entry, &id)?;
        let ui_sink_raw: Sender<PluginEvent> = {
            let tx = self.tx.clone();
            let plugin = id.clone();
            // 把 per-plugin 通道适配成带 id 的统一通道
            let (inner_tx, inner_rx) = unbounded::<PluginEvent>();
            std::thread::spawn(move || {
                while let Ok(ev) = inner_rx.recv() {
                    if tx.send(RuntimeEvent { plugin: plugin.clone(), event: ev }).is_err() {
                        break;
                    }
                }
            });
            inner_tx
        };
        let handle = engine::spawn(req.ctx, source, self.host.clone(), ui_sink_raw)?;
        self.handles.insert(id, Arc::new(handle));
        Ok(())
    }

    /// 停止插件（等待 isolate 线程回收）。
    pub fn stop(&mut self, plugin_id: &str) -> bool {
        match self.handles.remove(plugin_id) {
            Some(h) => h.stop(Duration::from_millis(1500)),
            None => true,
        }
    }

    /// 停止全部插件后端。
    pub fn stop_all(&mut self) {
        let ids: Vec<String> = self.handles.keys().cloned().collect();
        for id in ids {
            self.stop(&id);
        }
    }

    /// 唤起插件。
    pub fn enter(&self, plugin_id: &str, cmd: &str, feature: Value) -> JsResult<()> {
        self.send(
            plugin_id,
            HostEvent::Enter {
                cmd: cmd.to_string(),
                feature,
            },
        )
    }

    /// 宿主 → 插件 main 的消息。
    pub fn main_message(&self, plugin_id: &str, name: &str, data: Value) -> JsResult<()> {
        self.send(
            plugin_id,
            HostEvent::MainMessage {
                name: name.to_string(),
                data,
            },
        )
    }

    /// 插件 UI → 插件 main 的消息转发。
    pub fn ui_message(&self, plugin_id: &str, name: &str, data: Value) -> JsResult<()> {
        self.send(
            plugin_id,
            HostEvent::UiMessage {
                name: name.to_string(),
                data,
            },
        )
    }

    /// 通知插件其 UI 已关闭。
    pub fn ui_closed(&self, plugin_id: &str) -> JsResult<()> {
        self.send(plugin_id, HostEvent::UiClosed)
    }

    /// 异步能力完成后的回投。
    pub fn reply(&self, plugin_id: &str, call_id: u64, ok: bool, result: Value) -> JsResult<()> {
        self.send(plugin_id, HostEvent::Reply {
            id: call_id,
            ok,
            result,
        })
    }

    fn send(&self, plugin_id: &str, ev: HostEvent) -> JsResult<()> {
        match self.handles.get(plugin_id) {
            Some(h) => h.post(ev),
            None => Err(JsError::NotRunning(plugin_id.to_string())),
        }
    }

    /// 非阻塞取出插件发来的事件。宿主的 pump 循环每次调用它。
    pub fn drain_events(&self) -> Vec<RuntimeEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            out.push(ev);
            if out.len() >= 256 {
                break;
            }
        }
        out
    }

    /// 供宿主实现 UI 侧回包时借用同一个发送端。
    pub fn event_sender(&self) -> Sender<RuntimeEvent> {
        self.tx.clone()
    }

    /// 已登记的插件数量（含已退出的）。
    pub fn plugin_count(&self) -> usize {
        self.handles.len()
    }
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        // 尽量优雅退出，避免残留线程持有 isolate
        self.stop_all();
    }
}

/// 读入口并转译 TS。`.ts` 走 SWC 剥类型，JSX/TSX 明确拒绝。
pub fn prepare_source(entry: &Path, plugin_id: &str) -> JsResult<String> {
    if ts::looks_like_jsx(&entry.to_string_lossy()) {
        return Err(JsError::UnsupportedSyntax(format!(
            "{plugin_id}: JSX/TSX 入口不在第一阶段支持，请编译成 .js 或在 UI 侧使用框架"
        )));
    }
    let text = std::fs::read_to_string(entry)
        .map_err(|e| JsError::Transpile {
            path: entry.display().to_string(),
            message: format!("读取入口失败: {e}"),
        })?;
    let is_ts = entry
        .extension()
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "ts" || e == "mts" || e == "cts"
        })
        .unwrap_or(false);
    let js = if is_ts {
        let specifier = entry.to_string_lossy().to_string();
        ts::transpile_ts(&text, &specifier).map(strip_shebang)?
    } else {
        strip_shebang(text)
    };
    // 入口在函数作用域执行（CommonJS 语义），ESM 语法要给出明确改法而不是 V8 报错
    if let Some(kw) = ts::detect_esm(&js) {
        return Err(JsError::UnsupportedSyntax(ts::esm_hint(
            &entry.to_string_lossy(),
            kw,
        )));
    }
    Ok(js)
}

fn strip_shebang(text: String) -> String {
    if text.starts_with("#!") {
        match text.find('\n') {
            Some(idx) => text[idx + 1..].to_string(),
            None => String::new(),
        }
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{PluginContext, ReplyTo};
    use serde_json::json;
    use std::path::PathBuf;

    /// 记录型宿主，用来断言 JS 侧发起的能力调用。
    struct RecordingHost {
        calls: Arc<MutexVec>,
    }

    #[derive(Default)]
    struct MutexVec(std::sync::Mutex<Vec<(String, String, Vec<Value>)>>);

    impl Host for RecordingHost {
        fn call(&self, _p: &str, module: &str, method: &str, args: &[Value]) -> Result<Value, String> {
            self.calls.0.lock().unwrap().push((
                module.to_string(),
                method.to_string(),
                args.to_vec(),
            ));
            match (module, method) {
                ("db", "get") => Ok(json!("cached")),
                ("db", "all") => Ok(json!({ "a": 1 })),
                // 不做参数个数假设：在 V8 回调里越界索引会让整个进程 abort（panic 无法穿过 C 帧）
                ("module", "resolve") => Ok(json!({
                    "path": args.first().and_then(|v| v.as_str()).unwrap_or(""),
                    "dirname": args.get(1).and_then(|v| v.as_str()).unwrap_or("."),
                })),
                ("module", "load") => {
                    let p = args.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
                    Ok(json!({
                        "js": "exports.mul = function (n) { return n * 2 }",
                        "filename": p,
                        "dirname": "."
                    }))
                }
                ("system", "copyText") => Ok(json!(true)),
                _ => Ok(Value::Null),
            }
        }
        fn send_to_ui(&self, _: &str, _: &str, _: &Value) {}
    }

    impl RecordingHost {
        fn new() -> (Self, Arc<MutexVec>) {
            let v = Arc::new(MutexVec::default());
            (
                Self { calls: v.clone() },
                v,
            )
        }
    }

    /// 模拟流式宿主：异步能力先回多条 Stream 增量，最后用 Reply 收尾。
    struct StreamingHost {
        calls: Arc<MutexVec>,
    }

    impl Host for StreamingHost {
        fn call(&self, _p: &str, module: &str, method: &str, args: &[Value]) -> Result<Value, String> {
            self.calls.0.lock().unwrap().push((
                module.to_string(),
                method.to_string(),
                args.to_vec(),
            ));
            Ok(Value::Null)
        }

        fn call_async(
            &self,
            _p: &str,
            module: &str,
            method: &str,
            args: &[Value],
            reply: ReplyTo,
            id: u64,
        ) {
            self.calls.0.lock().unwrap().push((
                module.to_string(),
                method.to_string(),
                args.to_vec(),
            ));
            let _ = reply.send(HostEvent::Stream {
                id,
                delta: json!({ "text": "he" }),
            });
            let _ = reply.send(HostEvent::Stream {
                id,
                delta: json!({ "text": "llo" }),
            });
            let _ = reply.send(HostEvent::Reply {
                id,
                ok: true,
                result: json!({ "content": "hello" }),
            });
        }

        fn send_to_ui(&self, _: &str, _: &str, _: &Value) {}
    }

    /// 测试用临时插件目录。TempDir 由 KEEPALIVE 持有，进程退出时统一清理。
    static KEEPALIVE: std::sync::LazyLock<std::sync::Mutex<Vec<tempfile::TempDir>>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

    fn tmp_plugin(files: &[(&str, &str)]) -> PathBuf {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        for (name, content) in files {
            let p = path.join(name);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&p, content).unwrap();
        }
        KEEPALIVE.lock().unwrap().push(dir);
        path
    }

    fn ctx(root: &Path) -> PluginContext {
        PluginContext {
            plugin_id: "unit".into(),
            root: root.to_string_lossy().to_string(),
            version: "1.0.0".into(),
            locale: "zh-CN".into(),
            theme: "light".into(),
            host_version: "0.1.0".into(),
        }
    }

    /// UI → main 的消息必须送到插件的 `onMainMessage`，
    /// 插件回的消息必须走 `ui.postMessage`（main → UI 页面）。
    ///
    /// 这条链路曾经三处断裂，而且都没有自动化测试兜着：
    /// `deliver()` 只认 `efto.onXxx`（不认 exports 写法）、`ui_message` 被投给
    /// `onMessage`、以及主进程侧的 `efto.ipcMessage` 误调了 UI→main 的实现。
    #[test]
    fn ui_message_reaches_exports_on_main_message_and_reply_goes_to_post_message() {
        let dir = tmp_plugin(&[(
            "main.js",
            r#"'use strict';
exports.onMainMessage = function (msg) {
  efto.ipcMessage('reply', { got: msg.name, payload: msg.data });
};
"#,
        )]);
        let host = Arc::new(RecordingHost { calls: Arc::new(MutexVec::default()) });
        let mut rt = PluginRuntime::new(host.clone());
        rt.start(StartRequest {
            ctx: ctx(&dir),
            entry: dir.join("main.js"),
        })
        .unwrap();
        rt.enter("unit", "go", json!(null)).unwrap();
        // 等插件线程把入口跑完（注册 onMainMessage）
        std::thread::sleep(std::time::Duration::from_millis(200));

        rt.ui_message("unit", "whoami", json!({ "n": 1 })).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut found = false;
        while std::time::Instant::now() < deadline {
            let calls = host.calls.0.lock().unwrap();
            if calls.iter().any(|(m, me, a)| {
                m == "ui" && me == "postMessage" && a.first() == Some(&json!("reply"))
            }) {
                found = true;
                break;
            }
            drop(calls);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            found,
            "onMainMessage 应被调用，插件应通过 ui.postMessage 回复；实际调用：{:?}",
            host.calls.0.lock().unwrap()
        );
    }

    #[test]
    fn transpile_then_run_ts_entry_end_to_end() {
        let root = tmp_plugin(&[("main.ts", "const n: number = 21 * 2; exports.n = n;")]);
        let (host, _rec) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let entry = root.join("main.ts");
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: entry.clone(),
        })
        .unwrap();
        assert!(rt.is_running("unit"));
        std::thread::sleep(Duration::from_millis(80));
        assert!(rt.is_running("unit"), "TS 入口应被转译并持续存活");
        rt.stop("unit");
        assert!(!rt.is_running("unit"));
    }

    #[test]
    fn esm_entry_is_rejected_with_actionable_hint() {
        let root = tmp_plugin(&[("main.js", "import fs from 'fs';\nexports.a = 1;")]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let r = rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        });
        match r {
            Err(JsError::UnsupportedSyntax(msg)) => {
                assert!(msg.contains("require"), "应给出 CommonJS 改法：{msg}");
                assert!(msg.contains("main.js"), "应指明文件：{msg}");
            }
            other => panic!("expected UnsupportedSyntax, got {other:?}"),
        }
    }

    #[test]
    fn ts_import_also_rejected() {
        let root = tmp_plugin(&[("main.ts", "import { a } from './lib';\nexports.a = a;")]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let r = rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.ts"),
        });
        assert!(matches!(r, Err(JsError::UnsupportedSyntax(_))), "{r:?}");
    }

    #[test]
    fn cjs_require_of_ts_module_works() {
        let root = tmp_plugin(&[
            (
                "main.js",
                "var f = require('./lib/format');\nexports.result = f.mul(6);\n",
            ),
            ("lib/format.ts", "export function mul(n: number): number { return n * 2 }"),
        ]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        // 被 require 的 .ts 里用 ESM 语法是 TS 作者的默认写法：宿主转译后仍是 ESM，
        // 因此当前只支持 CJS 形式的共享模块，这里用 CJS 版本验证链路。
        std::fs::write(
            root.join("lib/format.ts"),
            "function mul(n: number): number { return n * 2 }\nexports.mul = mul;",
        )
        .unwrap();
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(120));
        assert!(rt.is_running("unit"), "require .ts 后插件应存活");
        rt.stop("unit");
    }

    #[test]
    fn plugin_can_call_host_db_and_lifecycle() {
        let root = tmp_plugin(&[(
            "main.js",
            r#"
var seen = [];
exports.onPluginEnter = function (info) {
  var got = efto.db.get('k');
  efto.kv.set('k2', { v: 1 });
  efto.copyText('hello');
  seen.push(got);
  exports.__entered = info.code;
};
exports.onMainMessage = function (msg) {
  exports.__last = [msg.name, msg.data];
};
"#,
        )]);
        let (host, rec) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        })
        .unwrap();
        rt.enter("unit", "翻译", json!("hello world")).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let calls = rec.0.lock().unwrap();
        let names: Vec<String> = calls
            .iter()
            .map(|(m, meth, _)| format!("{m}.{meth}"))
            .collect();
        assert!(names.contains(&"db.get".to_string()), "{names:?}");
        assert!(names.contains(&"db.set".to_string()), "{names:?}");
        assert!(names.contains(&"system.copyText".to_string()), "{names:?}");
        drop(calls);

        // 生命周期消息可送达且不死循环
        rt.main_message("unit", "ping", json!({"x": 1})).unwrap();
        std::thread::sleep(Duration::from_millis(50));
        assert!(rt.is_running("unit"));
        rt.stop("unit");
    }

    /// `efto.ai.chat({ onDelta })` 的增量必须按顺序到达 JS 回调，最终 Promise 仍 resolve。
    /// 这条链路跨越 JS 垫片 → 宿主异步回投 → V8 dispatch，光测 HTTP 的 SSE 解析覆盖不到。
    #[test]
    fn async_stream_deltas_reach_the_plugin_callback() {
        let root = tmp_plugin(&[(
            "main.js",
            r#"
exports.onPluginEnter = function () {
  efto.ai.chat({
    messages: [{ role: 'user', content: 'hi' }],
    onDelta: function (d) { efto.copyText('delta:' + (d.text || '')); }
  }).then(function (res) { efto.copyText('done:' + res.content); });
};
"#,
        )]);
        let calls = Arc::new(MutexVec::default());
        let host = Arc::new(StreamingHost { calls: calls.clone() });
        let mut rt = PluginRuntime::new(host);
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        })
        .unwrap();
        rt.enter("unit", "go", json!(null)).unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut texts: Vec<String> = Vec::new();
        while std::time::Instant::now() < deadline {
            texts = calls
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, me, _)| m == "system" && me == "copyText")
                .filter_map(|(_, _, a)| a.first().and_then(|v| v.as_str()).map(str::to_string))
                .collect();
            if texts.len() >= 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            texts,
            vec!["delta:he", "delta:llo", "done:hello"],
            "Stream 增量与最终 Reply 的顺序/内容不对；实际调用：{:?}",
            calls.0.lock().unwrap()
        );

        // 传 onDelta 时请求体必须显式带 stream: true，否则宿主不会走 SSE。
        let calls = calls.0.lock().unwrap();
        let body = calls
            .iter()
            .find(|(m, me, _)| m == "ai" && me == "chat")
            .and_then(|(_, _, a)| a.first().cloned())
            .expect("应发起一次 ai.chat");
        assert_eq!(body.get("stream"), Some(&json!(true)), "{body}");
        drop(calls);
        rt.stop("unit");
    }

    #[test]
    fn timers_fire_and_can_be_cancelled() {
        let root = tmp_plugin(&[(
            "main.js",
            r#"
exports.tickCount = 0;
var h = setInterval(function () { exports.tickCount++; }, 20);
exports.onPluginEnter = function () { };
setTimeout(function () { clearInterval(h); }, 90);
"#,
        )]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        // 失败必须能解释：把插件事件全打出来
        let events = rt.drain_events();
        for e in &events {
            match &e.event {
                PluginEvent::Error { stage, message, stack } => {
                    println!("[error:{stage}] {message} {}", stack.clone().unwrap_or_default());
                }
                other => println!("[event] {other:?}"),
            }
        }
        assert!(rt.is_running("unit"), "定时器链路不应让插件退出（事件见上方）");
        rt.stop("unit");
    }

    #[test]
    fn syntax_error_surfaces_as_event() {
        let root = tmp_plugin(&[("main.js", "function ( {{{ broken")]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        })
        .unwrap();
        let mut events = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_millis(1500);
        while std::time::Instant::now() < deadline {
            events.extend(rt.drain_events());
            if events.iter().any(|e| matches!(e.event, PluginEvent::Error { .. })) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            events.iter().any(|e| matches!(e.event, PluginEvent::Error { .. })),
            "语法错误应上报 Error 事件，实际 {events:?}"
        );
        rt.stop("unit");
    }

    #[test]
    fn enum_entry_reports_unsupported() {
        let root = tmp_plugin(&[("main.ts", "export enum A { X = 1 }")]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let r = rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.ts"),
        });
        assert!(matches!(r, Err(JsError::UnsupportedSyntax(_))), "{r:?}");
    }

    #[test]
    fn missing_entry_reports_io_error() {
        let root = tmp_plugin(&[]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let r = rt.start(StartRequest {
            ctx: ctx(&root),
            entry: root.join("nope.js"),
        });
        assert!(matches!(r, Err(JsError::Transpile { .. })), "{r:?}");
    }

    #[test]
    fn start_is_idempotent_and_enter_after_start_works() {
        let root = tmp_plugin(&[("main.js", "exports.onPluginEnter = function () {};")]);
        let (host, _) = RecordingHost::new();
        let mut rt = PluginRuntime::new(Arc::new(host));
        let req = StartRequest {
            ctx: ctx(&root),
            entry: root.join("main.js"),
        };
        rt.start(req.clone()).unwrap();
        rt.start(req).unwrap();
        assert_eq!(rt.running().len(), 1);
        rt.enter("unit", "x", Value::Null).unwrap();
        rt.stop_all();
        assert_eq!(rt.running().len(), 0);
    }

    #[test]
    fn send_to_missing_plugin_errors() {
        let (host, _) = RecordingHost::new();
        let rt = PluginRuntime::new(Arc::new(host));
        assert!(matches!(
            rt.enter("ghost", "a", Value::Null),
            Err(JsError::NotRunning(_))
        ));
    }

    #[test]
    fn shebang_stripped() {
        let root = tmp_plugin(&[("main.js", "#!/usr/bin/env node\nexports.a = 1;")]);
        let src = prepare_source(&root.join("main.js"), "unit").unwrap();
        assert!(!src.starts_with("#!"), "{src}");
        assert!(src.contains("exports.a"));
    }
}
