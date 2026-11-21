//! V8 引擎侧：每个插件一个线程 + 一个 Isolate + 一个 Context。
//!
//! 为什么不用线程池共享 isolate：V8 的 isolate 是单线程所有权模型；插件里出现死循环时，
//! 我们需要能单独 terminate/dispose 它，"一插件一线程"最简单也最安全。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use serde_json::Value;

use crate::bridge::{self, PluginShared};
use crate::ipc::{HostEvent, JsError, JsResult, PluginContext, PluginEvent};
use crate::prelude::PRELUDE;

/// V8 全进程只能初始化一次：rusty_v8 的 GLOBAL_STATE 是状态机，重复 initialize 会 panic。
static V8_INIT: std::sync::Once = std::sync::Once::new();

/// 初始化 V8 平台（幂等）。
pub fn init_v8() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
        tracing::info!("V8 已初始化");
    });
}

/// 从 TryCatch 作用域里取异常文本。用宏而非函数：这个版本的 scope 类型带 Pin 生命周期，
/// 写成函数签名既难读又容易和具体嵌套类型不匹配，就地展开让编译器自己推断。
macro_rules! tc_message {
    ($tc:expr) => {{
        let tc = &mut $tc;
        if tc.has_caught() {
            let via_message = tc
                .message()
                .map(|m| m.get(tc).to_rust_string_lossy(tc));
            let via_exception = tc
                .exception()
                .map(|e| e.to_rust_string_lossy(tc));
            Some(
                via_message
                    .or(via_exception)
                    .unwrap_or_else(|| "未知 JS 异常".to_string()),
            )
        } else {
            None
        }
    }};
}

/// 运行中的插件后端句柄。由宿主（app crate）持有。
pub struct PluginHandle {
    /// 插件 id
    pub plugin_id: String,
    /// 与宿主共享的运行状态
    pub shared: Arc<PluginShared>,
    inbound: Sender<HostEvent>,
    joiner: Mutex<Option<std::thread::JoinHandle<()>>>,
    next_async_id: Arc<AtomicU64>,
}

impl std::fmt::Debug for PluginHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginHandle")
            .field("plugin_id", &self.plugin_id)
            .field("running", &self.is_running())
            .finish()
    }
}

impl PluginHandle {
    /// isolate 线程是否仍在运行。
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    /// 投递事件（非阻塞）。
    /// 向插件线程投递一条事件。
    pub fn post(&self, event: HostEvent) -> JsResult<()> {
        if !self.is_running() {
            return Err(JsError::NotRunning(self.plugin_id.clone()));
        }
        self.inbound
            .send(event)
            .map_err(|_| JsError::Disconnected)
    }

    /// 分配异步能力调用 id。
    pub fn next_call_id(&self) -> u64 {
        self.next_async_id.fetch_add(1, Ordering::SeqCst)
    }

    /// 请求退出并等待线程回收（带超时，插件卡死时不会拖住宿主）。
    pub fn stop(&self, wait: Duration) -> bool {
        let _ = self.inbound.send(HostEvent::Close);
        let mut guard = match self.joiner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let Some(t) = guard.take() else {
            return true;
        };
        let deadline = Instant::now() + wait;
        loop {
            if t.is_finished() {
                return true;
            }
            if Instant::now() >= deadline {
                tracing::warn!("插件 {} 线程未在 {wait:?} 内退出", self.plugin_id);
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for PluginHandle {
    fn drop(&mut self) {
        let _ = self.inbound.try_send(HostEvent::Close);
    }
}

/// 启动插件后端。`source` 必须是已经过 TS 转译的 JS。
pub fn spawn(
    ctx: PluginContext,
    source: String,
    host: Arc<dyn crate::ipc::Host>,
    ui_sink: Sender<PluginEvent>,
) -> JsResult<PluginHandle> {
    init_v8();

    let (inbound_tx, inbound_rx) = crossbeam_channel::unbounded::<HostEvent>();
    // ui_sink 的接收端由宿主持有，这里只放一个克隆用于回投 reply（异步能力）
    let shared = Arc::new(PluginShared::new(
        ctx.plugin_id.clone(),
        ctx.root.clone(),
        host,
        inbound_tx.clone(),
        ui_sink.clone(),
    ));
    bridge::register(shared.clone());

    let id = ctx.plugin_id.clone();
    let thread_id = id.clone();
    let shared_for_thread = shared.clone();
    let next_async_id = Arc::new(AtomicU64::new(1));
    // 先置为运行中再 spawn：否则调用方拿到 handle 后立刻 enter() 会看到 false
    // （is_running 由线程入口置 true 存在竞态），插件冷启动会被误判为未运行。
    shared.running.store(true, Ordering::SeqCst);

    let joiner = std::thread::Builder::new()
        .name(format!("efto-plugin-{id}"))
        .spawn(move || {
            bridge::attach(shared_for_thread.clone());
            let outcome = run_loop(&ctx, &source, &shared_for_thread, inbound_rx, &ui_sink);
            if let Err(e) = &outcome {
                let (stage, message) = match e {
                    JsError::Script { message, .. } => ("run", message.clone()),
                    JsError::Transpile { message, .. } => ("transpile", message.clone()),
                    other => ("fatal", other.to_string()),
                };
                let _ = ui_sink.send(PluginEvent::Error {
                    stage: stage.to_string(),
                    message,
                    stack: None,
                });
            }
            shared_for_thread.running.store(false, Ordering::SeqCst);
            bridge::detach();
            bridge::unregister(&thread_id);
            tracing::info!("插件 {thread_id} 后端线程退出");
        })
        .map_err(|e| JsError::HostCall {
            module: "runtime".into(),
            method: "spawn".into(),
            message: e.to_string(),
        })?;

    Ok(PluginHandle {
        plugin_id: id,
        shared,
        inbound: inbound_tx,
        joiner: Mutex::new(Some(joiner)),
        next_async_id,
    })
}

/// 插件线程主循环：建 context → 注入垫片与入口 → 事件驱动。
fn run_loop(
    ctx: &PluginContext,
    source: &str,
    shared: &Arc<PluginShared>,
    inbound: Receiver<HostEvent>,
    ui_sink: &Sender<PluginEvent>,
) -> JsResult<()> {
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let context = {
        v8::scope!(let hs, &mut isolate);
        let context = v8::Context::new(hs, Default::default());
        let scope = &mut v8::ContextScope::new(hs, context);
        bridge::install_globals(scope)
            .map_err(|e| JsError::Script { message: e, stack: None })?;
        v8::Global::new(scope, context)
    };

    // 1. 运行时垫片
    eval(&mut isolate, &context, PRELUDE, "efto-prelude.js").map_err(|e| JsError::Script {
        message: format!("运行时垫片加载失败: {e}"),
        stack: None,
    })?;

    // 2. 环境常量
    let bootstrap = format!(
        concat!(
            "globalThis.__PLUGIN_ROOT__={root};",
            "globalThis.__PLUGIN_ID__={pid};",
            "globalThis.__HOST_VERSION__={ver};",
            "globalThis.__LOCALE__={loc};",
            "globalThis.__THEME__={theme};"
        ),
        root = json_lit(&ctx.root),
        pid = json_lit(&ctx.plugin_id),
        ver = json_lit(&ctx.host_version),
        loc = json_lit(&ctx.locale),
        theme = json_lit(&ctx.theme),
    );
    eval(&mut isolate, &context, &bootstrap, "efto-env.js")?;

    // 3. 入口脚本（CommonJS 作用域，执行完把 exports 登记成钩子）
    let entry = format!(
        "(function(){{\n\
         \x20var __module={{id:{name},exports:{{}}}};\n\
         \x20var __factory=(function(module,exports,require,__filename,__dirname){{\n{body}\n}});\n\
         \x20__factory(__module,__module.exports,globalThis.__require,{name},{dir});\n\
         \x20globalThis.__efto_set_exports(__module.exports);\n\
         }})();",
        name = json_lit(&ctx.root),
        body = source,
        dir = json_lit(&dirname_of(&ctx.root)),
    );
    eval(&mut isolate, &context, &entry, &format!("{}:main", ctx.plugin_id)).map_err(|e| {
        JsError::Script {
            message: format!("插件入口执行失败: {e}"),
            stack: None,
        }
    })?;

    // 4. 事件循环：定时器到期时间作为 recv 超时，避免额外调度线程
    let mut alive = true;
    while alive {
        let timeout = shared.next_timer_timeout();
        match inbound.recv_timeout(timeout) {
            Ok(ev) => match ev {
                HostEvent::Close => alive = false,
                other => {
                    if let Err(e) = dispatch(&mut isolate, &context, &other) {
                        let _ = ui_sink.send(PluginEvent::Error {
                            stage: "dispatch".into(),
                            message: e.to_string(),
                            stack: None,
                        });
                    }
                }
            },
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                for id in shared.drain_due_timers() {
                    if let Err(e) = dispatch(&mut isolate, &context, &HostEvent::Timer { id }) {
                        let _ = ui_sink.send(PluginEvent::Error {
                            stage: "timer".into(),
                            message: e.to_string(),
                            stack: None,
                        });
                    }
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                tracing::debug!("插件 {} 事件通道断开", ctx.plugin_id);
                alive = false;
            }
        }
        isolate.perform_microtask_checkpoint();
    }

    drop(context);
    drop(isolate);
    Ok(())
}

fn json_lit(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn dirname_of(path: &str) -> String {
    std::path::Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 把宿主事件转成 `__efto_dispatch(kind, json)`。
fn dispatch(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    ev: &HostEvent,
) -> JsResult<()> {
    let (kind, payload) = match ev {
        HostEvent::Enter { cmd, feature } => (
            "enter",
            serde_json::json!({ "code": cmd, "type": "text", "payload": feature, "msg": feature }),
        ),
        HostEvent::MainMessage { name, data } => ("main_message", serde_json::json!({"name": name, "data": data})),
        HostEvent::UiMessage { name, data } => ("ui_message", serde_json::json!({"name": name, "data": data})),
        HostEvent::Timer { id } => ("timer", serde_json::json!({ "id": id })),
        HostEvent::Reply { id, ok, result } => (
            "reply",
            serde_json::json!({ "id": id, "ok": ok, "result": result }),
        ),
        HostEvent::Stream { id, delta } => (
            "stream",
            serde_json::json!({ "id": id, "delta": delta }),
        ),
        HostEvent::UiClosed => ("ui_closed", serde_json::json!({})),
        HostEvent::Close => return Ok(()),
    };
    let text = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".into());
    call_global(isolate, context, "__efto_dispatch", &[kind, &text])
}

/// 调全局函数，参数是字符串。
/// 调用一个全局函数（参数为字符串）。
pub fn call_global(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    name: &str,
    args: &[&str],
) -> JsResult<()> {
    let mut err: Option<String> = None;
    {
        v8::scope_with_context!(let scope, isolate, context);
        let global = scope.get_current_context().global(scope);
        let Some(key) = v8::String::new(scope, name) else {
            return Err(JsError::Script {
                message: format!("无法创建全局函数名 {name}"),
                stack: None,
            });
        };
        let Some(value) = global.get(scope, key.into()) else {
            return Err(JsError::Script {
                message: format!("全局函数 {name} 不存在"),
                stack: None,
            });
        };
        let Ok(func) = v8::Local::<v8::Function>::try_from(value) else {
            return Err(JsError::Script {
                message: format!("{name} 不是函数"),
                stack: None,
            });
        };
        let mut argv: Vec<v8::Local<v8::Value>> = Vec::with_capacity(args.len());
        for a in args {
            match v8::String::new(scope, a) {
                Some(s) => argv.push(s.into()),
                None => {
                    err = Some(format!("{name} 参数过大，无法转为 V8 字符串"));
                    break;
                }
            }
        }
        if err.is_none() {
            let recv = v8::undefined(scope).into();
            let tc = std::pin::pin!(v8::TryCatch::new(scope));
            let mut tc = tc.init();
            if func.call(&mut tc, recv, &argv).is_none() {
                err = Some(tc_message!(tc).unwrap_or_else(|| format!("{name} 调用失败")));
            }
        }
    }
    match err {
        Some(message) => Err(JsError::Script { message, stack: None }),
        None => Ok(()),
    }
}

/// 在当前 context 里执行脚本并返回结果（JSON 化）。
/// 在指定 context 中求值脚本并返回 JSON 化结果。
pub fn eval(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    code: &str,
    origin: &str,
) -> JsResult<Value> {
    // 用块表达式把结果带出来：块内借用 isolate，块结束后才能继续做微任务检查点，
    // 同时避免"先 None 占位再覆盖"这种多余赋值。
    let out = {
        v8::scope_with_context!(let scope, isolate, context);
        let tc = std::pin::pin!(v8::TryCatch::new(scope));
        let mut tc = tc.init();
        match v8::String::new(&mut tc, code) {
            Some(src) => {
                let compiled = v8::Script::compile(&mut tc, src, None);
                match compiled.and_then(|s| s.run(&mut tc)) {
                    Some(v) => Some(Ok(bridge::value_to_json(&mut tc, v))),
                    None => Some(Err(
                        tc_message!(tc).unwrap_or_else(|| format!("{origin}: 执行失败"))
                    )),
                }
            }
            None => Some(Err(format!(
                "{origin}: 源码过大或含非法字符，无法创建 V8 字符串"
            ))),
        }
    };
    match out {
        Some(Ok(v)) => Ok(v),
        Some(Err(message)) => Err(JsError::Script { message, stack: None }),
        None => Ok(Value::Null),
    }
}

/// 一次性的脚本求值器：给测试和"插件语法预检"用，不起插件线程。
#[derive(Default)]
/// 一次性脚本求值器（不起插件线程），供测试与语法预检使用。
pub struct ScratchEngine;

impl ScratchEngine {
    /// 构造求值器（确保 V8 已初始化）。
    pub fn new() -> Self {
        init_v8();
        Self
    }

    /// 在全新 context 中求值。
    pub fn eval(&self, code: &str) -> JsResult<Value> {
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        let context = {
            v8::scope!(let hs, &mut isolate);
            let context = v8::Context::new(hs, Default::default());
            v8::Global::new(hs, context)
        };
        let result = eval(&mut isolate, &context, code, "scratch.js");
        drop(context);
        // OwnedIsolate 必须在离开线程前 drop（rusty_v8 要求逆序）
        drop(isolate);
        result
    }
}
