//! efto 插件运行时：在 Rust 进程里内嵌 V8，给插件提供 `efto.*` API。
//!
//! ## 分层
//! - [`engine`]  一个插件一个线程一个 isolate：V8 初始化、脚本编译、事件循环、微任务推进
//! - [`bridge`]  JS ↔ Rust 边界：三个原生函数（同步调用 / 异步调用 / 模块执行）+ 定时器表
//! - [`prelude`] 注入的 JS 垫片：`efto.*` / CommonJS require / console / 定时器
//! - [`ts`]      TypeScript 类型剥离（SWC，经 deno_ast），不依赖 Node 与 tsc
//! - [`ipc`]     消息与 [`Host`] trait：能力面用 `module.method(args)` 透传，新增 API 不改绑定层
//! - [`pool`]    异步能力的工作线程池，避免插件线程或主线程被 IO 卡住
//! - [`runtime`] 宿主唯一需要使用的门面
//!
//! ## 与宿主的契约
//! 宿主实现 [`ipc::Host`]（读写 db、发通知、开窗口、发 HTTP、解析并加载被 require 的模块），
//! 本 crate 不关心它背后是 Tauri 还是 CLI，因此可以被单独测试。
//!
//! ## 已知边界（第一阶段）
//! - 不支持 node_modules 与 Node 内置模块（垫片会给出明确报错，而不是静默失败）
//! - TS 只做类型剥离：`enum` / 带值的 `namespace` / 装饰器 / JSX 会被拒绝并提示预编译
//! - 异步能力的超时、取消与崩溃隔离（V8 terminate）留到第二阶段

#![warn(missing_docs)]

pub mod bridge;
pub mod engine;
pub mod ipc;
pub mod modules;
pub mod pool;
pub mod prelude;
pub mod runtime;
pub mod ts;

pub use bridge::PluginShared;
pub use engine::{call_global, eval, init_v8, PluginHandle};
pub use ipc::{Host, HostEvent, JsError, JsResult, PluginContext, PluginEvent, ReplyTo};
pub use modules::{load as load_module, resolve as resolve_module, LoadedModule};
pub use runtime::{prepare_source, PluginRuntime, RuntimeEvent, StartRequest};
pub use ts::{looks_like_jsx, transpile_ts};
