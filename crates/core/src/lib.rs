//! efto core：与 UI / JS 引擎完全解耦的能力层。
//!
//! 分层：
//! - [`apps`]   应用发现（开始菜单 .lnk、注册表卸载项、常见安装目录）+ 索引 + 启动
//! - [`search`] 归一化、拼音、模糊打分、结果合并
//! - [`plugins`] 插件清单解析与插件目录发现
//! - [`store`]  插件键值存储
//! - [`usage`]  使用频次/最近使用，用于排序加权
//! - [`config`] 设置持久化
//!
//! 该 crate 不依赖 tauri / v8，可以被 CLI 或测试直接驱动。

pub mod apps;
pub mod bookmarks;
pub mod clipboard;
pub mod config;
pub mod error;
pub mod exec;
pub mod fsutil;
pub mod hosts;
pub mod plugins;
pub mod recent;
pub mod search;
pub mod shortcuts;
pub mod snippets;
pub mod store;
pub mod system;
pub mod usage;
pub mod websearch;

pub use error::{CoreError, Result};

/// 索引与搜索结果里统一使用的条目类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// 桌面应用（exe / 快捷方式）
    App,
    /// 插件唤起项
    Plugin,
    /// 插件通过 `efto.output()` 内联返回的结果
    Output,
    /// 内置指令，例如 "打开设置"
    Action,
    /// 内置计算器的结果
    Calc,
    /// 内置时间戳（秒级 / 毫秒级）的结果
    Time,
    /// 搜索框命令执行（`>` 前缀）的入口行
    Command,
    /// Everything 检索到的文件（搜索框直接输入即可），双击/回车打开
    File,
    /// 输入本身就是路径或 URL，直接给出"打开"项
    External,
    /// 浏览器收藏夹（Chrome / Edge）里的书签
    Bookmark,
    /// 用户配置的片段（Snippets），关键词精确命中
    Snippet,
}
