//! 搜索框命令执行（`>` 前缀）。
//!
//! 入口判定很严：只有设置开启、且输入**以配置的前缀开头**时才归这里。
//! 命中后 [`crate::search::SearchService::query_with_sources`] 直接短路其它所有源 ——
//! 用户明确输入了命令，不应再混入应用/计算器/文件结果。

use crate::config::{CommandRunMode, Settings};
use crate::search::{HitPayload, SearchHit};
use crate::ItemKind;

/// 命令源结果的统一来源标签。
pub const SOURCE: &str = "command";

/// 空命令提示行的动作名；宿主执行时只回一句提示，不启动任何进程。
pub const HINT_ACTION: &str = "command_hint";

/// 解析命令正文；`None` 表示这条输入不归命令源。
pub fn parse<'a>(query: &'a str, settings: &Settings) -> Option<&'a str> {
    if !settings.command_enabled {
        return None;
    }
    let prefix = settings.command_prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    // 只认行首前缀：`a > b` 之类的普通输入不能被劫持
    let q = query.trim_start();
    q.strip_prefix(prefix).map(str::trim)
}

/// 命令源的结果行。正文为空时只给提示行，绝不执行。
pub fn hits(query: &str, settings: &Settings) -> Vec<SearchHit> {
    let Some(command) = parse(query, settings) else {
        return Vec::new();
    };
    if command.trim().is_empty() {
        let prefix = &settings.command_prefix;
        return vec![SearchHit::new(
            ItemKind::Command,
            "command:hint",
            "输入要执行的命令",
            999.0,
            HitPayload::Action {
                name: HINT_ACTION.into(),
            },
        )
        .subtitle(format!("{prefix} 后接命令，例如 {prefix}ipconfig"))
        .source(SOURCE)];
    }
    let verb = match settings.command_run_mode {
        CommandRunMode::Inline => "执行并在下拉里显示输出",
        CommandRunMode::Terminal => "在终端窗口中执行",
    };
    vec![SearchHit::new(
        ItemKind::Command,
        format!("command:{command}"),
        command.to_string(),
        999.0,
        HitPayload::RunCommand {
            command: command.to_string(),
        },
    )
    .subtitle(format!("在 {} 中{verb}", settings.command_shell.display()))
    .source(SOURCE)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommandShell, Settings};

    fn settings() -> Settings {
        Settings::default()
    }

    #[test]
    fn only_line_start_prefix_enters_command_source() {
        let s = settings();
        assert!(parse(">", &s).is_some());
        assert_eq!(parse("> ipconfig", &s), Some("ipconfig"));
        assert_eq!(parse("  >   ipconfig", &s), Some("ipconfig"));
        // 反例：前缀不在行首、普通输入、其它前缀都不能进命令源
        assert_eq!(parse("a > b", &s), None);
        assert_eq!(parse("chrome", &s), None);
        assert_eq!(parse(" echo hi", &s), None);
        assert_eq!(parse(">> x", &s), Some("> x"));
    }

    #[test]
    fn disabled_or_blank_prefix_never_fires() {
        let mut s = settings();
        s.command_enabled = false;
        assert_eq!(parse("> ipconfig", &s), None);
        assert!(hits("> ipconfig", &s).is_empty());

        let mut s = settings();
        s.command_prefix = "   ".into();
        assert_eq!(parse("> x", &s), None);
    }

    #[test]
    fn empty_body_is_a_hint_not_an_execution() {
        let s = settings();
        let h = hits(">", &s);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].kind, ItemKind::Command);
        assert!(matches!(h[0].payload, HitPayload::Action { ref name } if name == HINT_ACTION));
        assert!(h[0].subtitle.contains('>'));
    }

    #[test]
    fn command_hit_carries_the_exact_command_text() {
        let mut s = settings();
        s.command_shell = CommandShell::Cmd;
        let h = hits(">  git status  ", &s);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].title, "git status");
        assert_eq!(h[0].source, SOURCE);
        assert!(
            matches!(h[0].payload, HitPayload::RunCommand { ref command } if command == "git status")
        );
        assert!(h[0].subtitle.contains("cmd"), "{}", h[0].subtitle);
    }
}
