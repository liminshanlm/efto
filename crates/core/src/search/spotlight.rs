//! 内联的 Spotlight 文件检索（mdfind）：直接输入关键词，文件结果补进下拉框。
//!
//! 与 [`super::everything`]（Windows 的 es.exe 通道）同一交互约定：
//! 1. **至少 2 个字符**才查；
//! 2. **分数压在应用下面**：基础分 160，低于应用条目的最低分（≈234），
//!    文件只填空位，绝不把应用挤走（AGENTS.md 硬性约定 2）；
//! 3. **超时短** + 输出靠 exec 的 1MB 上限截断（mdfind 没有条数参数）；
//! 4. **失败静默**：mdfind 缺失 / 超时 / 非零退出都返回空列表；
//! 5. 复用 [`crate::exec`] 的白名单 + 参数白名单（`mdfind` 只允许 `[-name, 查询词]`）。
//!
//! 平台：只有 macOS 有 mdfind；其它平台 `hits()` 恒为空（run() 的平台门会报错，这里吞掉）。

use std::time::Duration;

use super::{HitPayload, SearchHit};
use crate::exec;
use crate::ItemKind;

/// 结果条数上限。文件只是补充，多了会把应用搜索结果挤下去。
pub const LIMIT: usize = 5;
/// mdfind 单次调用超时。
pub const TIMEOUT: Duration = Duration::from_millis(1200);
/// 隐式结果的基础分，按名次递减；必须低于应用条目的最低分（300×0.78≈234）。
const BASE_SCORE: f32 = 160.0;
/// 隐式检索的最小输入长度（字符数）。
const MIN_CHARS: usize = 2;
/// 多取一些再过滤：mdfind 没有条数参数，可能吐出大量陈旧/噪音项。
const FETCH_MUL: usize = 4;

/// 交给 mdfind 的查询；输入太短（或全空白）返回 None。
pub fn implicit_query(input: &str) -> Option<&str> {
    let q = input.trim();
    (q.chars().count() >= MIN_CHARS).then_some(q)
}

/// 生成内联结果行。不可用 / 没结果都返回空列表，让应用结果自己上屏。
pub fn hits(input: &str) -> Vec<SearchHit> {
    let Some(q) = implicit_query(input) else {
        return Vec::new();
    };
    let Ok(out) = exec::run("mdfind", &mdfind_args(q), TIMEOUT, None) else {
        return Vec::new();
    };
    if out.code != 0 {
        return Vec::new();
    }
    parse_rows(&out.stdout)
        .into_iter()
        // 陈旧索引项：Spotlight 偶尔返回已删除的路径
        .filter(|p| std::path::Path::new(p).exists())
        .take(LIMIT)
        .enumerate()
        .map(|(i, p)| file_hit(i, &p, BASE_SCORE))
        .collect()
}

fn mdfind_args(query: &str) -> Vec<String> {
    vec!["-name".into(), query.into()]
}

/// mdfind 输出：每行一个绝对路径。跳过噪音路径；只保留前 `LIMIT * FETCH_MUL` 条
/// （stat 有成本，放在 `hits` 里做，后面的本来也排不上）。
fn parse_rows(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !is_noise(l))
        .take(LIMIT * FETCH_MUL)
        .map(|s| s.to_string())
        .collect()
}

/// 垃圾桶与缓存目录里的文件对启动器没有意义（对齐 everything 的噪音过滤意图）。
fn is_noise(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("/.trash/") || p.contains("/library/caches/") || p.contains("/caches/")
}

fn file_hit(rank: usize, path: &str, base: f32) -> SearchHit {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let dir = path.rfind('/').map(|i| path[..i].to_string()).unwrap_or_default();
    SearchHit::new(
        ItemKind::File,
        format!("mdfind:{path}"),
        name,
        // 按名次递减；整体压在应用条目下面，只填空位
        base - rank as f32 * 4.0,
        HitPayload::Open {
            path: path.to_string(),
        },
    )
    .subtitle(dir)
    .reason(super::MatchReason::Substring)
    .source("spotlight")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_score_stays_below_apps() {
        // 应用条目的最低分 ≈ 300×0.78 = 234（见 fuzzy 的分数分层），文件行必须更低
        let first = file_hit(0, "/a/b.txt", BASE_SCORE);
        let last = file_hit(LIMIT - 1, "/a/c.txt", BASE_SCORE);
        assert!(first.score < 234.0, "文件行会抢走应用位次: {}", first.score);
        assert!(first.score > last.score, "按名次递减");
    }

    /// 反例（硬性约定 2）：单字符不该触发文件检索。
    #[test]
    fn single_char_does_not_trigger() {
        assert!(implicit_query("a").is_none());
        assert!(implicit_query(" ").is_none());
        assert!(implicit_query("").is_none());
        assert_eq!(implicit_query("ab"), Some("ab"));
    }

    #[test]
    fn parse_rows_skips_noise() {
        let stdout = "/Users/x/报告.pdf\n/.Trash/old.pdf\n/Library/Caches/tmp.bin\n/Users/x/ok.txt\n";
        let rows = parse_rows(stdout);
        assert_eq!(rows, vec!["/Users/x/报告.pdf".to_string(), "/Users/x/ok.txt".to_string()]);
    }

    #[test]
    fn hit_shape_is_a_file_row() {
        let h = file_hit(0, "/Users/x/报告.pdf", BASE_SCORE);
        assert_eq!(h.kind, ItemKind::File);
        assert_eq!(h.title, "报告.pdf");
        assert_eq!(h.source, "spotlight");
        assert_eq!(
            h.payload,
            HitPayload::Open {
                path: "/Users/x/报告.pdf".into()
            }
        );
        assert!(!h.subtitle.contains("报告.pdf"), "副标题只放目录");
    }
}
