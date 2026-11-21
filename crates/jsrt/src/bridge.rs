//! Rust ↔ V8 的原生绑定层。
//!
//! 约定：
//! - 回调是普通 `fn`（rusty_v8 通过 `MapFnTo` 接受），不带捕获；当前插件上下文放在
//!   thread_local（一个插件一个线程，因此不需要按 isolate 指针查表，也没有 unsafe 取 isolate 的需要）。
//! - 所有跨边界数据一律走 JSON 文本：Rust 侧 serde_json，JS 侧 JSON.parse/stringify。
//!   避免为每种类型写一套 Handle 转换，也天然支持插件传复杂对象。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::Value;

use crate::ipc::{Host, HostEvent, PluginEvent};
use crate::prelude::ENTRY_WRAPPER;

/// 定时器表：`id -> (到期时间, 周期)`，None 表示一次性。
pub type TimerTable = HashMap<u32, (Instant, Option<Duration>)>;

/// 周期任务的最小间隔（毫秒），对齐浏览器的 setInterval 钳制。
pub const MIN_INTERVAL_MS: u64 = 4;

/// 一个插件后端的可共享状态。
pub struct PluginShared {
    /// 插件 id（清单里的 name）
    pub plugin_id: String,
    /// 插件根目录（沙箱边界）
    pub root: String,
    /// 能力实现，由宿主提供
    pub host: Arc<dyn Host>,
    /// 宿主 -> 本插件 isolate 线程的事件通道发送端
    pub inbound: Sender<HostEvent>,
    /// 本插件 -> 宿主的事件通道发送端
    pub ui: Sender<PluginEvent>,
    /// 后端是否存活；由 spawn 置真、线程退出时置假
    pub running: AtomicBool,
    timers: Mutex<TimerTable>,
    next_timer_id: AtomicU32,
}

impl PluginShared {
    /// 构造共享状态。
    pub fn new(
        plugin_id: String,
        root: String,
        host: Arc<dyn Host>,
        inbound: Sender<HostEvent>,
        ui: Sender<PluginEvent>,
    ) -> Self {
        Self {
            plugin_id,
            root,
            host,
            inbound,
            ui,
            running: AtomicBool::new(false),
            timers: Mutex::new(HashMap::new()),
            next_timer_id: AtomicU32::new(1),
        }
    }

    /// 登记一个定时器；周期任务的间隔会被钳制到下限。
    pub fn arm(&self, id: u32, delay_ms: u64, repeat: bool) {
        // 周期任务给一个下限：setInterval(fn, 0) 会让事件循环以 1ms 频率空转，
        // 把插件线程和 CPU 一起拖死。浏览器的钳制是 4ms，这里对齐。
        let clamped = if repeat {
            delay_ms.max(MIN_INTERVAL_MS)
        } else {
            delay_ms
        };
        let due = Instant::now() + Duration::from_millis(clamped);
        let mut t = self.timers.lock();
        t.insert(
            id,
            (due, if repeat { Some(Duration::from_millis(clamped)) } else { None }),
        );
    }

    /// 取消定时器。
    pub fn clear_timer(&self, id: u32) {
        self.timers.lock().remove(&id);
    }

    /// 分配一个定时器 id。
    pub fn new_timer_id(&self) -> u32 {
        self.next_timer_id.fetch_add(1, Ordering::SeqCst)
    }

    /// 到期的定时器 id；周期任务会自动顺延。
    pub fn drain_due_timers(&self) -> Vec<u32> {
        let now = Instant::now();
        let mut due = Vec::new();
        {
            let mut t = self.timers.lock();
            let ids: Vec<u32> = t.keys().copied().collect();
            for id in ids {
                let Some((dead, period)) = t.get(&id).copied() else { continue };
                if dead <= now {
                    due.push(id);
                    match period {
                        Some(p) => {
                            // 落后过多时按当前时间重排，避免风暴式补偿
                            let next = if dead + p <= now { now + p } else { dead + p };
                            if let Some(e) = t.get_mut(&id) {
                                e.0 = next;
                            }
                        }
                        None => {
                            t.remove(&id);
                        }
                    }
                }
            }
        }
        due
    }

    /// 距离最近一个定时任务的等待时长（无任务时给一个兜底值，保证通道事件及时）。
    pub fn next_timer_timeout(&self) -> Duration {
        let now = Instant::now();
        let guard = self.timers.lock();
        let min = guard
            .values()
            .map(|(due, _)| due.saturating_duration_since(now))
            .min();
        match min {
            Some(d) => d.max(Duration::from_millis(1)),
            None => Duration::from_secs(3600),
        }
    }

    /// 当前挂起的定时器数量。
    pub fn timer_count(&self) -> usize {
        self.timers.lock().len()
    }
}

// ------------------------------------------------------------------
// 注册表与线程上下文
// ------------------------------------------------------------------

static REGISTRY: Lazy<Mutex<HashMap<String, Arc<PluginShared>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

thread_local! {
    static CURRENT: RefCell<Option<Arc<PluginShared>>> = const { RefCell::new(None) };
}

/// 登记一个运行中的插件（诊断与统计用）。
pub fn register(shared: Arc<PluginShared>) {
    REGISTRY.lock().insert(shared.plugin_id.clone(), shared);
}

/// 注销插件登记。
pub fn unregister(plugin_id: &str) {
    REGISTRY.lock().remove(plugin_id);
}

/// 当前登记的插件数量。
pub fn registered_count() -> usize {
    REGISTRY.lock().len()
}

/// 按 id 取回已登记的插件状态。
pub fn get_registered(plugin_id: &str) -> Option<Arc<PluginShared>> {
    REGISTRY.lock().get(plugin_id).cloned()
}

/// 把当前插件绑定到本线程（插件线程入口处调用一次）。
/// 把插件状态绑定到当前线程（原生回调据此找到宿主）。
pub fn attach(shared: Arc<PluginShared>) {
    CURRENT.with(|c| *c.borrow_mut() = Some(shared));
}

/// 解绑当前线程（线程退出前调用）。
pub fn detach() {
    CURRENT.with(|c| *c.borrow_mut() = None);
}

fn current() -> Option<Arc<PluginShared>> {
    CURRENT.with(|c| c.borrow().clone())
}

// ------------------------------------------------------------------
// JS ↔ Rust 值转换
// ------------------------------------------------------------------

/// Value → JS（JSON 文本，由 JS 侧 JSON.parse）。
pub fn set_json_return<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<v8::Value>,
    value: &Value,
) {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    match v8::String::new(scope, &text) {
        Some(s) => rv.set(s.into()),
        None => throw(scope, "宿主返回值过大，无法创建 V8 字符串"),
    }
}

/// JS 值 → Value（走 JSON.stringify，一次覆盖对象/数组/标量）。
pub fn value_to_json<'s>(scope: &mut v8::PinScope<'s, '_>, v: v8::Local<v8::Value>) -> Value {
    if v.is_undefined() || v.is_null() || v.is_null_or_undefined() {
        return Value::Null;
    }
    if v.is_boolean() {
        return Value::Bool(v.boolean_value(scope));
    }
    if v.is_string() {
        return Value::String(v.to_rust_string_lossy(scope));
    }
    if v.is_number() {
        if let Some(n) = v.number_value(scope) {
            return serde_json::Number::from_f64(n)
                .map(Value::Number)
                .unwrap_or(Value::Null);
        }
        return Value::Null;
    }
    match v8::json::stringify(scope, v) {
        Some(s) => serde_json::from_str(&s.to_rust_string_lossy(scope)).unwrap_or(Value::Null),
        None => Value::Null,
    }
}

fn arg_string<'s>(scope: &mut v8::PinScope<'s, '_>, args: &v8::FunctionCallbackArguments<'_>, i: i32) -> String {
    let v = args.get(i);
    if v.is_undefined() || v.is_null() {
        return String::new();
    }
    v.to_rust_string_lossy(scope)
}

fn arg_json<'s>(scope: &mut v8::PinScope<'s, '_>, args: &v8::FunctionCallbackArguments<'_>, i: i32) -> Value {
    value_to_json(scope, args.get(i))
}

/// 读取能力调用的参数数组。
///
/// 兼容三种到达形式：JS 数组（正常路径）、被 JSON.stringify 过的字符串（旧插件或
/// 手工调用）、以及单个裸值。曾经这里只认数组，配合 JS 侧的 stringify 就把
/// `db.set('k', v)` 变成了 `db.set("[\"k\",v]")` —— 参数协议必须两边都对齐并留容错。
fn params_of<'s>(scope: &mut v8::PinScope<'s, '_>, args: &v8::FunctionCallbackArguments<'_>, i: i32) -> Vec<Value> {
    match arg_json(scope, args, i) {
        Value::Array(items) => items,
        Value::Null => vec![],
        Value::String(s) => serde_json::from_str::<Vec<Value>>(&s)
            .or_else(|_| serde_json::from_str::<Value>(&s).map(|v| vec![v]))
            .unwrap_or_else(|_| vec![Value::String(s)]),
        other => vec![other],
    }
}

fn throw<'s>(scope: &mut v8::PinScope<'s, '_>, msg: &str) {
    if let Some(s) = v8::String::new(scope, msg) {
        let exc = v8::Exception::type_error(scope, s);
        scope.throw_exception(exc.into());
    }
}

// ------------------------------------------------------------------
// 能力路由
// ------------------------------------------------------------------

/// jsrt 自己处理的能力（不经过宿主），其余透传给 Host。
fn route(shared: &Arc<PluginShared>, module: &str, method: &str, params: &[Value]) -> Result<Value, String> {
    match (module, method) {
        ("timer", "arm") => {
            let id = params
                .first()
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "timer.arm 缺少 id".to_string())? as u32;
            let delay = params.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
            let repeat = params.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            shared.arm(id, delay, repeat);
            Ok(Value::Bool(true))
        }
        ("timer", "clear") => {
            let id = params
                .first()
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "timer.clear 缺少 id".to_string())? as u32;
            shared.clear_timer(id);
            Ok(Value::Bool(true))
        }
        ("log", lvl) => {
            let text = params.first().and_then(|v| v.as_str()).unwrap_or("");
            // 只写 tracing 时，CLI `plugin-run --json` 拿不到插件的 console 输出，
            // 面向 agent 的机器可读结果（如 `EFTO_API {...}`）会丢。改走事件通道：
            // GUI 由 pump 统一落到 tracing，CLI 由 report_events 收进 events / logs。
            let _ = shared.ui.send(PluginEvent::Log {
                level: lvl.to_string(),
                text: text.to_string(),
            });
            Ok(Value::Null)
        }
        ("report", "error") => {
            let stage = params.first().and_then(|v| v.as_str()).unwrap_or("unknown");
            let msg = params.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let stack = params.get(2).and_then(|v| v.as_str()).map(|s| s.to_string());
            let _ = shared.ui.send(PluginEvent::Error {
                stage: stage.to_string(),
                message: msg.to_string(),
                stack,
            });
            Ok(Value::Null)
        }
        ("broadcast", "sendToDefaultOutput") => {
            // 语义上等价于把内容回传给宿主 UI 层
            let payload = params.first().cloned().unwrap_or(Value::Null);
            let _ = shared.ui.send(PluginEvent::ToUI {
                name: "default_output".into(),
                data: payload,
            });
            Ok(Value::Bool(true))
        }
        _ => shared
            .host
            .call(&shared.plugin_id, module, method, params),
    }
}

/// 需要异步执行的能力模块（JS 侧 __host_call_async 进来）。
fn is_async_module(module: &str) -> bool {
    matches!(
        module,
        "ui" | "redirect" | "workflow" | "network" | "dbAsync" | "runtime" | "hosts" | "ai"
    )
}

// ------------------------------------------------------------------
// 原生函数
// ------------------------------------------------------------------
//
// 每个回调的外层都是 panic 屏障：Rust panic 无法穿过 V8 的 `extern "C"` 栈帧，
// 不拦住整个进程会直接 abort（实测过：宿主实现里一个 `args[1]` 越界就把测试进程打死，
// 退出码 0xc0000409 / STATUS_STACK_BUFFER_OVERRUN）。拦下来之后最坏只是插件收到一条异常。

/// `__host_call(module, method, argsJson) -> resultJson`
pub fn native_host_call(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    rv: v8::ReturnValue<v8::Value>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host_call_inner(scope, args, rv)
    }));
    if outcome.is_err() {
        tracing::error!("__host_call 内部 panic，已转为 JS 异常");
        throw(scope, "efto: 宿主能力调用内部错误（详见宿主日志 RUST_LOG=debug）");
    }
}

fn host_call_inner(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let Some(shared) = current() else {
        throw(scope, "efto: 宿主上下文未绑定");
        return;
    };
    let module = arg_string(scope, &args, 0);
    let method = arg_string(scope, &args, 1);
    if module.is_empty() || method.is_empty() {
        throw(scope, "efto: 能力调用缺少 module/method");
        return;
    }
    let params: Vec<Value> = params_of(scope, &args, 2);
    match route(&shared, &module, &method, &params) {
        Ok(v) => set_json_return(scope, &mut rv, &v),
        Err(e) => throw(scope, &format!("{module}.{method}: {e}")),
    }
}

/// `__host_call_async(id, module, method, argsJson)`，结果通过 __efto_dispatch('reply') 回来。
pub fn native_host_call_async(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    rv: v8::ReturnValue<v8::Value>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        host_call_async_inner(scope, args, rv)
    }));
    if outcome.is_err() {
        tracing::error!("__host_call_async 内部 panic，已转为 JS 异常");
        throw(scope, "efto: 异步能力派发内部错误（详见宿主日志）");
    }
}

fn host_call_async_inner(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue<v8::Value>,
) {
    let Some(shared) = current() else {
        throw(scope, "efto: 宿主上下文未绑定");
        return;
    };
    let id = args
        .get(0)
        .number_value(scope)
        .map(|n| n as u64)
        .unwrap_or(0);
    let module = arg_string(scope, &args, 1);
    let method = arg_string(scope, &args, 2);
    let params: Vec<Value> = params_of(scope, &args, 3);

    // 未列为异步的模块直接同步执行并立即回投 reply，保持 JS 侧 Promise 语义一致
    if !is_async_module(&module) {
        let event = match route(&shared, &module, &method, &params) {
            Ok(v) => HostEvent::Reply {
                id,
                ok: true,
                result: v,
            },
            Err(e) => HostEvent::Reply {
                id,
                ok: false,
                result: Value::String(e),
            },
        };
        let _ = shared.inbound.send(event);
        return;
    }

    // 异步能力交给工作线程池：插件线程立刻返回，完成后回投 HostEvent::Reply
    let host = shared.host.clone();
    let inbound = shared.inbound.clone();
    let plugin = shared.plugin_id.clone();
    crate::pool::bump();
    let queued = crate::pool::submit(move || {
        host.call_async(&plugin, &module, &method, &params, inbound, id);
    });
    if !queued {
        let _ = shared.inbound.send(HostEvent::Reply {
            id,
            ok: false,
            result: Value::String("能力调用线程池已关闭".into()),
        });
    }
}

/// `__run_module_source(module, path, req) -> boolean`
///
/// 宿主读文件并按需转译 TS，这里在同一个 isolate 里编译成函数并执行，
/// 因此嵌套 require 不需要 eval / new Function。
pub fn native_run_module_source(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue<v8::Value>,
) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_module_inner(scope, args)
    }));
    let ok = match outcome {
        Ok(v) => v,
        Err(_) => {
            tracing::error!("__run_module_source 内部 panic");
            false
        }
    };
    rv.set_bool(ok);
}

fn run_module_inner(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments) -> bool {
    let Some(shared) = current() else {
        throw(scope, "efto: 宿主上下文未绑定");
        return false;
    };
    let module_val = args.get(0);
    let path = arg_string(scope, &args, 1);
    let req = args.get(2);
    if path.is_empty() {
        throw(scope, "require: 缺少模块路径");
        return false;
    }
    let meta = match shared.host.call(&shared.plugin_id, "module", "load", &[Value::String(path.clone())]) {
        Ok(v) => v,
        Err(e) => {
            throw(scope, &format!("require('{path}') 失败: {e}"));
            return false;
        }
    };
    let js = meta.get("js").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let filename = meta
        .get("filename")
        .and_then(|v| v.as_str())
        .unwrap_or(&path)
        .to_string();
    let dirname = meta
        .get("dirname")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 取 module.exports 作为 factory 的第二个实参
    let Ok(module_obj) = v8::Local::<v8::Object>::try_from(module_val) else {
        throw(scope, "require: module 参数不是对象");
        return false;
    };
    let exports_key = match v8::String::new(scope, "exports") {
        Some(s) => s,
        None => {
            throw(scope, "require: 内部错误");
            return false;
        }
    };
    let exports = match module_obj.get(scope, exports_key.into()) {
        Some(v) => v,
        None => v8::undefined(scope).into(),
    };

    let factory_src = ENTRY_WRAPPER.replace("%SOURCE%", &js);
    let origin = format!("{}:module", filename);
    let fname = match v8::String::new(scope, &filename) {
        Some(s) => s.into(),
        None => {
            throw(scope, "require: 文件名无效");
            return false;
        }
    };
    let dname = match v8::String::new(scope, &dirname) {
        Some(s) => s.into(),
        None => v8::undefined(scope).into(),
    };
    let argv = [module_val, exports, req, fname, dname];
    match compile_and_call(scope, &factory_src, &origin, &argv) {
        Ok(()) => true,
        // 有挂起异常时不能再做 JS 操作：只回 false，让真实异常传播给调用方
        Err(e) => {
            tracing::debug!("require 失败: {e}");
            false
        }
    }
}

/// 编译 factory 表达式并用 (module, exports, require, __filename, __dirname) 调用。
///
/// 注意：编译/执行失败时 V8 已经"挂起"了一个异常，此时**不能**再调用任何 JS 操作
/// （包括再 throw 一个友好消息），只能立刻返回。因此这里返回 Err 由调用方转成
/// `false`，让真实的 JS 异常自然传播到 `__require` 的 try/catch。
fn compile_and_call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    factory_src: &str,
    origin: &str,
    argv: &[v8::Local<v8::Value>],
) -> Result<(), String> {
    let src = match v8::String::new(scope, factory_src) {
        Some(s) => s,
        None => return Err(format!("{origin}: 模块源码过大，无法创建 V8 字符串")),
    };
    let Some(script) = v8::Script::compile(scope, src, None) else {
        return Err(format!("{origin}: 模块语法错误"));
    };
    let Some(value) = script.run(scope) else {
        return Err(format!("{origin}: 模块顶层执行抛出异常"));
    };
    let Ok(func) = v8::Local::<v8::Function>::try_from(value) else {
        return Err(format!("{origin}: factory 未求值为函数"));
    };
    let this = v8::undefined(scope).into();
    if func.call(scope, this, argv).is_none() {
        return Err(format!("{origin}: 模块执行抛出异常"));
    }
    Ok(())
}

/// 在 context 上注册全部原生函数。
///
/// 注意：不要把回调写成 `fn(&mut v8::PinScope, ...)` 的显式数组类型——rusty_v8 用
/// `MapFnTo` + const 断言挑选受支持的回调形状，手写类型会规约成另一种指针类型，
/// 触发 `UnitValue::SELF` 的 const-eval 失败（index out of bounds）。逐个传函数项即可。
/// 把三个原生函数注册到 context 的全局对象上。
pub fn install_globals<'s>(scope: &mut v8::PinScope<'s, '_>) -> Result<(), String> {
    let global = scope.get_current_context().global(scope);
    bind(scope, &global, "__host_call", native_host_call)?;
    bind(scope, &global, "__host_call_async", native_host_call_async)?;
    bind(scope, &global, "__run_module_source", native_run_module_source)?;
    Ok(())
}

fn bind<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: &v8::Local<v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<(), String> {
    let key = v8::String::new(scope, name).ok_or("绑定名无效")?;
    let func = v8::Function::builder(callback)
        .build(scope)
        .ok_or_else(|| format!("{name} 创建失败"))?;
    if global.set(scope, key.into(), func.into()).is_none() {
        return Err(format!("{name} 注册失败"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct NoopHost;
    impl Host for NoopHost {
        fn call(&self, _: &str, m: &str, meth: &str, args: &[Value]) -> Result<Value, String> {
            Ok(json!({ "module": m, "method": meth, "args": args }))
        }
        fn send_to_ui(&self, _: &str, _: &str, _: &Value) {}
    }

    fn shared() -> Arc<PluginShared> {
        let (_tx, _rx) = crossbeam_channel::unbounded::<HostEvent>();
        let (_utx, _urx) = crossbeam_channel::unbounded::<PluginEvent>();
        Arc::new(PluginShared::new(
            "test".into(),
            "C:/plug".into(),
            Arc::new(NoopHost),
            _tx,
            _utx,
        ))
    }

    #[test]
    fn timer_arm_drain_and_repeat() {
        let s = shared();
        s.arm(1, 0, false);
        s.arm(2, 10_000, true);
        assert_eq!(s.timer_count(), 2);
        let due = s.drain_due_timers();
        assert_eq!(due, vec![1], "只应触发已到期的 1");
        assert_eq!(s.timer_count(), 1);
        assert!(s.next_timer_timeout() > Duration::from_millis(0));
        s.clear_timer(2);
        assert_eq!(s.timer_count(), 0);
        assert_eq!(s.next_timer_timeout(), Duration::from_secs(3600));
    }

    #[test]
    fn repeated_timer_reschedules() {
        let s = shared();
        s.arm(7, 5_000, true);
        assert_eq!(s.drain_due_timers(), Vec::<u32>::new(), "5s 后到期，此刻不该触发");
        s.clear_timer(7);
        // 0 会被钳制到 MIN_INTERVAL_MS，所以第一次也要等过那个间隔
        s.arm(8, 0, true);
        assert_eq!(
            s.drain_due_timers(),
            Vec::<u32>::new(),
            "setInterval(0) 应被钳制，不该立刻到期"
        );
        std::thread::sleep(Duration::from_millis(MIN_INTERVAL_MS + 6));
        assert_eq!(s.drain_due_timers(), vec![8], "到点后应触发一次");
        assert_eq!(s.timer_count(), 1, "周期任务应保留");
        assert!(
            s.drain_due_timers().is_empty(),
            "刚顺延完不应立刻又可触发（至少间隔 {MIN_INTERVAL_MS}ms）"
        );
    }

    #[test]
    fn interval_zero_is_clamped_not_burned() {
        let s = shared();
        s.arm(1, 0, true);
        // 立刻 drain 拿不到任务：说明 0 被钳制，事件循环不会以 1ms 频率空转
        assert!(s.drain_due_timers().is_empty(), "setInterval(0) 被钳制到 >=4ms");
        std::thread::sleep(Duration::from_millis(MIN_INTERVAL_MS + 8));
        assert_eq!(s.drain_due_timers(), vec![1], "超过钳制间隔后应正常触发");
    }

    #[test]
    fn timeout_zero_is_not_clamped() {
        // setTimeout(0) 没有空转风险，不应被钳制
        let s = shared();
        s.arm(3, 0, false);
        assert_eq!(s.drain_due_timers(), vec![3]);
        assert_eq!(s.timer_count(), 0, "一次性任务触发后应移除");
    }

    #[test]
    fn route_passes_unknown_modules_to_host() {
        let s = shared();
        let v = route(&s, "db", "get", &[json!("k")]).unwrap();
        assert_eq!(v["module"], json!("db"));
        assert_eq!(v["args"][0], json!("k"));
    }

    #[test]
    fn route_log_and_report_do_not_hit_host() {
        let s = shared();
        assert_eq!(route(&s, "log", "info", &[json!("hi")]).unwrap(), Value::Null);
        let (u2t, u2r) = crossbeam_channel::unbounded::<PluginEvent>();
        // report 走 ui channel
        let s2 = {
            let (t, _r) = crossbeam_channel::unbounded::<HostEvent>();
            Arc::new(PluginShared::new(
                "p2".into(),
                String::new(),
                Arc::new(NoopHost),
                t,
                u2t,
            ))
        };
        route(&s2, "report", "error", &[json!("stage"), json!("boom"), Value::Null]).unwrap();
        match u2r.try_recv() {
            Ok(PluginEvent::Error { message, .. }) => assert_eq!(message, "boom"),
            other => panic!("expected error event, got {other:?}"),
        }
    }

    #[test]
    fn async_module_classification() {
        assert!(is_async_module("ui"));
        assert!(!is_async_module("db"));
        assert!(!is_async_module("system"));
    }

    /// 回归：JS 侧曾把参数 stringify 成字符串，Rust 侧只认数组，
    /// 导致 `db.set('k', v)` 变成一个 key 为 `["k",v]` 的写入。
    #[test]
    fn params_are_parsed_from_array_or_stringified_array() {
        // params_of 需要 V8 scope，这里直接测它的字符串分支等价逻辑
        let as_items = |v: Value| match v {
            Value::Array(items) => items,
            Value::Null => vec![],
            Value::String(s) => serde_json::from_str::<Vec<Value>>(&s)
                .or_else(|_| serde_json::from_str::<Value>(&s).map(|v| vec![v]))
                .unwrap_or_else(|_| vec![Value::String(s)]),
            other => vec![other],
        };
        assert_eq!(as_items(json!(["k", 1])), vec![json!("k"), json!(1)]);
        assert_eq!(
            as_items(json!("[\"k\",{\"a\":1}]")),
            vec![json!("k"), json!({"a": 1})],
            "stringified 数组也要还原成多个参数"
        );
        assert_eq!(as_items(Value::Null), Vec::<Value>::new());
        assert_eq!(as_items(json!("plain")), vec![json!("plain")]);
        assert_eq!(as_items(json!("\"quoted\"")), vec![json!("quoted")]);
    }

    #[test]
    fn registry_tracks_plugins() {
        let before = registered_count();
        let s = shared();
        let id = s.plugin_id.clone();
        register(s.clone());
        assert_eq!(registered_count(), before + 1);
        assert!(get_registered(&id).is_some());
        detach();
        assert!(current().is_none());
        attach(s);
        assert!(current().is_some());
        detach();
        unregister(&id);
        assert_eq!(registered_count(), before);
    }
}
