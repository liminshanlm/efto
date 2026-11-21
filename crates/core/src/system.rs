//! 系统命令：睡眠、重启、注销、清空回收站、屏保。
//!
//! 只接受**整串精确匹配**的关键词（忽略大小写），不做前缀/模糊匹配——
//! 这些动作一旦误触发代价很高，宁可让用户多打几个字。

use serde::{Deserialize, Serialize};

use crate::search::{MatchReason, SearchHit, HitPayload};
use crate::ItemKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemAction {
    Sleep,
    Restart,
    Logout,
    EmptyTrash,
    ScreenSaver,
}

impl SystemAction {
    pub fn id(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Restart => "restart",
            Self::Logout => "logout",
            Self::EmptyTrash => "empty_trash",
            Self::ScreenSaver => "screensaver",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Sleep => "睡眠",
            Self::Restart => "重启电脑",
            Self::Logout => "注销当前用户",
            Self::EmptyTrash => "清空回收站",
            Self::ScreenSaver => "启动屏幕保护",
        }
    }

    /// 从持久化/前端传回的 id 还原动作。
    pub fn from_id(id: &str) -> Option<Self> {
        [
            Self::Sleep,
            Self::Restart,
            Self::Logout,
            Self::EmptyTrash,
            Self::ScreenSaver,
        ]
        .into_iter()
        .find(|a| a.id() == id)
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Sleep => &["sleep", "睡眠"],
            Self::Restart => &["restart", "重启"],
            Self::Logout => &["logout", "logoff", "注销"],
            Self::EmptyTrash => &["emptytrash", "清空回收站"],
            Self::ScreenSaver => &["screensaver", "屏保"],
        }
    }

    /// 需要二次确认的动作（不可逆或会中断当前工作）。
    pub fn needs_confirm(self) -> bool {
        matches!(self, Self::Restart | Self::Logout | Self::EmptyTrash)
    }
}

/// 精确匹配系统命令关键词。
pub fn parse(query: &str) -> Option<SystemAction> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    [
        SystemAction::Sleep,
        SystemAction::Restart,
        SystemAction::Logout,
        SystemAction::EmptyTrash,
        SystemAction::ScreenSaver,
    ]
    .into_iter()
    .find(|a| a.keywords().iter().any(|k| k.eq_ignore_ascii_case(q)))
}

pub fn hit(action: SystemAction) -> SearchHit {
    SearchHit::new(
        ItemKind::External,
        format!("system:{}", action.id()),
        action.title().to_string(),
        690.0,
        HitPayload::SystemAction {
            action: action.id().to_string(),
        },
    )
    .subtitle(if action.needs_confirm() {
        "系统命令 · 执行前会二次确认".to_string()
    } else {
        "系统命令".to_string()
    })
    .reason(MatchReason::Exact)
    .source("system")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_keywords_only() {
        assert_eq!(parse("sleep"), Some(SystemAction::Sleep));
        assert_eq!(parse(" RESTART "), Some(SystemAction::Restart));
        assert_eq!(parse("重启"), Some(SystemAction::Restart));
        assert_eq!(parse("清空回收站"), Some(SystemAction::EmptyTrash));
        // 不该触发的反例
        assert_eq!(parse("sleep now"), None);
        assert_eq!(parse("restar"), None);
        assert_eq!(parse("重启电脑"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn confirm_flags() {
        assert!(SystemAction::Restart.needs_confirm());
        assert!(SystemAction::Logout.needs_confirm());
        assert!(SystemAction::EmptyTrash.needs_confirm());
        assert!(!SystemAction::Sleep.needs_confirm());
        assert!(!SystemAction::ScreenSaver.needs_confirm());
    }
}
