//! 事件泵：把插件线程发来的事件在主循环之外消费掉。
//!
//! 为什么不用 Tauri 的 async command 收事件：插件事件来自 jsrt 的 crossbeam 通道，
//! 与 tokio 无关；一个 60ms 轮询的守护线程最简单，也不会让插件线程等待宿主。

use std::time::Duration;

use tauri::{AppHandle, Emitter, EventTarget, Manager};
use efto_jsrt::{PluginEvent, RuntimeEvent};

use crate::state::Shared;
use crate::windows;

pub fn start(app: &AppHandle) {
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("efto-pump".into())
        .spawn(move || {
            let mut tick = 0u32;
            loop {
                // 取完事件立刻释放 State 与 runtime 锁，避免长时间占住插件运行时
                let events = {
                    let data = app.state::<Shared>();
                    let runtime = data.runtime.lock();
                    runtime.drain_events()
                };
                for ev in events {
                    route(&app, ev);
                }
                // 每 ~2 秒批量落盘一次脏数据（flush_all 内部跳过未变更的插件）
                tick = tick.wrapping_add(1);
                if tick % 32 == 0 {
                    let data = app.state::<Shared>();
                    data.flush_stores();
                }
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        });
}

/// 轮询间隔。60ms 足够让 UI 无感，又不会空转烧 CPU。
const POLL_MS: u64 = 60;

fn route(app: &AppHandle, ev: RuntimeEvent) {
    let data = app.state::<Shared>();
    match &ev.event {
        PluginEvent::Error { stage, message, stack } => {
            tracing::error!("插件 {} 错误[{stage}]: {message}", ev.plugin);
            if let Some(s) = stack {
                tracing::debug!("stack: {s}");
            }
            let payload = serde_json::json!({
                "plugin": ev.plugin,
                "stage": stage,
                "message": message,
                "stack": stack,
            });
            let _ = app.emit_to(EventTarget::window("main"), "plugin:error", payload);
        }
        PluginEvent::ToUI { name, data: payload } => {
            windows::emit_to_plugin_ui(app, &ev.plugin, name, payload);
        }
        PluginEvent::Log { level, text } => {
            match level.as_str() {
                "error" => tracing::error!("[{}] {text}", ev.plugin),
                "warn" => tracing::warn!("[{}] {text}", ev.plugin),
                _ => tracing::info!("[{}] {text}", ev.plugin),
            }
        }
        PluginEvent::Exit { code } => {
            tracing::info!("插件 {} 请求退出(code={code})", ev.plugin);
            data.runtime.lock().stop(&ev.plugin);
            windows::hide_plugin_ui(app, &ev.plugin);
        }
        PluginEvent::Call { .. } | PluginEvent::CallAsync { .. } => {
            // 正常路径下这两类事件不会到这里（bridge 已就地处理），出现说明协议走偏
            tracing::warn!("插件 {} 发来了意外的能力调用事件", ev.plugin);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn pump_polls_below_perceptible_delay() {
        assert!(super::POLL_MS <= 100, "轮询间隔过大会有输入卡顿感");
        assert!(super::POLL_MS >= 20, "轮询过快会白烧 CPU");
    }
}
