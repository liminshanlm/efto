//! 宿主 ↔ 插件运行时之间的消息与能力调用协议。
//!
//! 设计要点：能力调用用 `module.method(args)` 的**通用透传**形式，而不是给每个 API 写一个枚举变体。
//! 新增一个插件 API 只需要在 prelude.js 里加一行、在宿主 Host trait 里加一个 match 分支，
//! 不必改动 V8 绑定层。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 宿主 → 插件线程。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostEvent {
    /// 插件被唤起
    Enter {
        /// 命中的关键字
        cmd: String,
        /// 关键字之后的输入负载
        feature: Value,
    },
    /// 宿主发给插件 main 的消息
    MainMessage {
        /// 消息名
        name: String,
        /// 消息体
        data: Value,
    },
    /// 插件 UI 页面发来的消息
    UiMessage {
        /// 消息名
        name: String,
        /// 消息体
        data: Value,
    },
    /// 定时器到期
    Timer {
        /// JS 侧登记的定时器 id
        id: u32,
    },
    /// 异步能力调用完成，交给 JS 侧 resolve/reject
    Reply {
        /// 对应的调用 id
        id: u64,
        /// 成功还是失败
        ok: bool,
        /// 结果或错误文本
        result: Value,
    },
    /// 流式能力的增量（如 AI 逐段输出），最终仍以 Reply 收尾
    Stream {
        /// 对应的调用 id
        id: u64,
        /// 增量数据，结构由能力自己约定
        delta: Value,
    },
    /// UI 窗口关闭
    UiClosed,
    /// 要求插件退出并释放 isolate
    Close,
}

/// 插件 → 宿主。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginEvent {
    /// 同步能力调用：宿主返回值直接作为 native 调用的结果
    Call {
        /// 能力模块名，如 db / system / shell
        module: String,
        /// 方法名，如 get / copyText
        method: String,
        /// 参数列表
        args: Vec<Value>,
    },
    /// 异步能力调用：宿主稍后用 HostEvent::Reply 回传
    CallAsync {
        /// 回投 Reply 时使用的关联 id
        id: u64,
        /// 能力模块名
        module: String,
        /// 方法名
        method: String,
        /// 参数列表
        args: Vec<Value>,
    },
    /// 把消息投递给插件 UI 窗口
    ToUI {
        /// 消息名
        name: String,
        /// 消息体
        data: Value,
    },
    /// console.* 转发
    Log {
        /// log/info/warn/error/debug
        level: String,
        /// 已拼接成一行
        text: String,
    },
    /// 未捕获异常 / 加载失败
    Error {
        /// 出错阶段：load/run/dispatch/timer/hook
        stage: String,
        /// 错误消息
        message: String,
        /// 尽力取到的堆栈，可能为 None
        stack: Option<String>,
    },
    /// 插件主动退出
    Exit {
        /// 插件自报的退出码
        code: i64,
    },
}

/// 宿主需要实现的能力面。app crate 实现它，jsrt 只依赖这个 trait，避免反向依赖。
/// 宿主回复通道。用 crossbeam 而不是 std::sync::mpsc：插件线程的事件接收端本来就是同一种通道，
/// 类型统一后不需要在两条通道之间做适配。
/// 宿主回复通道。用 crossbeam 而不是 std::sync::mpsc：插件线程的事件接收端本来就是同一种通道，
/// 类型统一后不需要在两条通道之间做适配。
pub type ReplyTo = crossbeam_channel::Sender<HostEvent>;

/// 宿主必须实现的能力面。app crate 实现它；jsrt 只依赖这个 trait，因此不反向依赖 Tauri。
pub trait Host: Send + Sync + 'static {
    /// 同步调用。返回 Err(text) 时 JS 侧抛异常。
    fn call(&self, plugin: &str, module: &str, method: &str, args: &[Value]) -> Result<Value, String>;

    /// 异步调用：实现方完成后通过 sender 回 HostEvent::Reply。
    /// 默认实现直接同步执行并立刻回复，够覆盖第一阶段的多数场景。
    fn call_async(
        &self,
        plugin: &str,
        module: &str,
        method: &str,
        args: &[Value],
        reply: ReplyTo,
        id: u64,
    ) {
        let result = self.call(plugin, module, method, args);
        let event = match result {
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
        let _ = reply.send(event);
    }

    /// 转发给插件 UI 窗口。
    fn send_to_ui(&self, plugin: &str, name: &str, data: &Value);

    /// 生命周期通知（Enter/Exit/Error 等），可用于日志与 UI 状态。
    fn on_plugin_event(&self, plugin: &str, event: &PluginEvent) {
        let _ = (plugin, event);
    }
}

/// 运行时错误。
#[derive(Debug, thiserror::Error)]
pub enum JsError {
    #[error("插件源码为空: {0}")]
    /// 入口源码为空
    EmptySource(String),

    #[error("TypeScript 转译失败 {path}: {message}")]
    /// TS 转译失败
    Transpile {
        /// 出错文件
        path: String,
        /// 原因
        message: String,
    },

    #[error("JS 执行出错: {message}")]
    /// JS 执行异常
    Script {
        /// 异常消息
        message: String,
        /// 堆栈
        stack: Option<String>,
    },

    #[error("宿主调用失败 {module}.{method}: {message}")]
    /// 宿主能力返回错误
    HostCall {
        /// 模块名
        module: String,
        /// 方法名
        method: String,
        /// 错误描述
        message: String,
    },

    #[error("插件未运行: {0}")]
    /// 目标插件没在运行
    NotRunning(String),

    #[error("运行时通道已关闭")]
    /// 与插件线程的通道已断开
    Disconnected,

    #[error("不支持的语法（需要预编译）: {0}")]
    /// 需要预编译才能支持的语法
    UnsupportedSyntax(String),
}

/// 运行时统一的结果别名。
pub type JsResult<T> = Result<T, JsError>;

/// 启动插件时传给运行时的上下文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginContext {
    /// 插件 id
    pub plugin_id: String,
    /// 插件根目录
    pub root: String,
    /// 插件版本（来自清单）
    pub version: String,
    /// 语言环境，注入给 JS 的 efto.locale
    pub locale: String,
    /// 主题：light / dark
    pub theme: String,
    /// 宿主版本号，注入给 efto.version
    pub host_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_event_roundtrip() {
        let e = HostEvent::Enter {
            cmd: "翻译".into(),
            feature: json!("hello"),
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("\"type\":\"enter\""), "{s}");
        let back: HostEvent = serde_json::from_str(&s).unwrap();
        match back {
            HostEvent::Enter { cmd, feature } => {
                assert_eq!(cmd, "翻译");
                assert_eq!(feature, json!("hello"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn plugin_event_call_shape() {
        let c = PluginEvent::Call {
            module: "db".into(),
            method: "get".into(),
            args: vec![json!("k")],
        };
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("\"call\""));
        let back: PluginEvent = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, PluginEvent::Call { module, .. } if module == "db"));
    }

    /// 默认 call_async 立刻回复，插件侧不会永远等下去。
    struct DummyHost;
    impl Host for DummyHost {
        fn call(&self, _: &str, _module: &str, method: &str, _: &[Value]) -> Result<Value, String> {
            Ok(json!({ "echo": method }))
        }
        fn send_to_ui(&self, _: &str, _: &str, _: &Value) {}
    }

    #[test]
    fn default_async_impl_replies() {
        let (tx, rx) = crossbeam_channel::unbounded::<HostEvent>();
        DummyHost.call_async("p", "m", "x", &[], tx, 7);
        match rx.recv().unwrap() {
            HostEvent::Reply { id, ok, result } => {
                assert_eq!(id, 7);
                assert!(ok);
                assert_eq!(result, json!({ "echo": "x" }));
            }
            other => panic!("{other:?}"),
        }
    }
}
