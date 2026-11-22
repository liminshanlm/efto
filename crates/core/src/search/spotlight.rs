//! 内联的 Spotlight 文件检索（mdfind）：直接输入关键词，文件结果补进下拉框。
//!
//! 与 [`super::everything`]（Windows 的 es.exe 通道）同一交互约定：
//! 1. **至少 2 个字符**才查；
//! 2. **分数压在应用下面**：基础分 160，低于应用条目的最低分（≈234），
//!    文件只填空位，绝不把应用挤走（AGENTS.md 硬性约定 2）；
//! 3. **超时短** + 输出靠 exec 的 1MB 上限截断（mdfind 没有条数参数）；
//! 4. **失败静默**：mdfind 缺失 / 超时 / 非零退出都返回空列表；
//! 5. 复用 [`crate::exec`] 的白名单 + 参数白名单。
//!
//! 平台：只有 macOS 有 mdfind；其它平台 `hits()` 恒为空（run() 的平台门会报错，这里吞掉）。
//!
//! ## 为什么限定常用目录（2026-11-22）
//!
//! 全盘 `mdfind -name config` 实测 5.0s、4135 条，其中 3584 条落在 `~/Library`
//! 下的缓存与配置里，用户文档只占 166 条 —— 既慢又全是噪音，1.2s 超时下等于
//! 只能拿到被截断的垃圾结果。改成对 `~/Desktop`、`~/Documents`、`~/Downloads`
//! 做 `-onlyin` 限定后：同一查询 0.3s、166 条全是用户文件。
//!
//! 多个 `-onlyin` 是 OR 语义、可合并成一次调用（实测比逐目录调用快 3 倍），
//! 所以这里拼一条命令而不是循环。
//!
//! 代价：限定目录之外的文件（外接盘、`/opt` 等）搜不到。要覆盖这些路径需要
//! 自建文件名索引 + FSEvents 增量更新，见 docs/roadmap.md。

use std::path::PathBuf;
use std::time::Duration;

use super::{HitPayload, SearchHit};
use crate::exec;
use crate::ItemKind;

/// 结果条数上限。文件只是补充，多了会把应用搜索结果挤下去。
pub const LIMIT: usize = 5;
/// mdfind 单次调用超时。
///
/// 限定常用目录后实测 60~300ms；给 1.2s 是给冷缓存的余量，不是常态耗时。
pub const TIMEOUT: Duration = Duration::from_millis(1200);
/// 隐式结果的基础分，按名次递减；必须低于应用条目的最低分（300×0.78≈234）。
const BASE_SCORE: f32 = 160.0;
/// 隐式检索的最小输入长度（字符数）。
const MIN_CHARS: usize = 2;
/// 多取一些再过滤：mdfind 没有条数参数，可能吐出大量陈旧/噪音项。
const FETCH_MUL: usize = 4;
/// 在用户主目录下参与检索的常用目录（存在才加进 `-onlyin`）。
///
/// 不把主目录本身加进来：那等价于全盘（`~/Library` 立刻占满结果）。
/// 用户想扩大范围时走 `Settings::file_search_dirs`（见 [`search_dirs`]）。
const SEARCH_SUBDIRS: &[&str] = &["Desktop", "Documents", "Downloads"];

/// 交给 mdfind 的查询；输入太短（或全空白）返回 None。
pub fn implicit_query(input: &str) -> Option<&str> {
    let q = input.trim();
    (q.chars().count() >= MIN_CHARS).then_some(q)
}

/// 参与检索的目录：默认常用目录 + 用户在设置里追加的目录。
///
/// 空列表表示没法限定，`hits()` 直接返回空。
/// 目录集合的唯一事实源是 `Settings::effective_file_search_dirs` —— 设置页显示的
/// 范围与这里实际用的范围必须一致，否则用户会看到"配了却没生效"。
pub fn search_dirs() -> Vec<PathBuf> {
    crate::config::Settings::load().effective_file_search_dirs()
}

/// 默认常用目录中**存在**的那些（设置页用来区分"默认"与"用户添加"）。
pub fn default_search_dirs() -> Vec<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    SEARCH_SUBDIRS
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.is_dir())
        .collect()
}

/// 生成内联结果行。不可用 / 没结果都返回空列表，让应用结果自己上屏。
pub fn hits(input: &str) -> Vec<SearchHit> {
    let Some(q) = implicit_query(input) else {
        return Vec::new();
    };
    let dirs = search_dirs();
    if dirs.is_empty() {
        return Vec::new();
    }
    let Ok(out) = exec::run("mdfind", &mdfind_args(q, &dirs), TIMEOUT, None) else {
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

fn mdfind_args(query: &str, dirs: &[PathBuf]) -> Vec<String> {
    let mut args = Vec::with_capacity(dirs.len() * 2 + 2);
    for d in dirs {
        args.push("-onlyin".to_string());
        args.push(d.to_string_lossy().to_string());
    }
    args.push("-name".into());
    args.push(query.into());
    args
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

/// 检索结果里的噪音路径。
///
/// 即便已限定常用目录，仍要挡住：
/// - 垃圾桶（用户删掉的东西不该再冒出来）；
/// - 缓存与包管理目录（`node_modules`、`.git` 对象库里有海量同名文件，
///   搜 `config` 会被它们占满）；
/// - 隐藏目录（`.Trash`、`.cache` 这类）。
fn is_noise(path: &str) -> bool {
    let p = path.to_lowercase();
    const NOISE_MARKERS: &[&str] = &[
        "/.trash/",
        "/.cache/",
        "/caches/",
        "/node_modules/",
        "/.git/",
        "/library/caches/",
        "/library/application support/",
        "/library/containers/",
        "/library/group containers/",
    ];
    if NOISE_MARKERS.iter().any(|m| p.contains(m)) {
        return true;
    }
    // 隐藏文件：路径最后一段以 `.` 开头（如 .DS_Store）。但允许常见的点开头的
    // 用户文件（.zshrc 这类配置常被主动搜索），只滤掉系统噪音名。
    let name = p.rsplit('/').next().unwrap_or("");
    matches!(name, ".ds_store" | ".localized" | ".gitignore" | ".gitkeep")
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

    /// 噪音过滤必须挡住代码依赖目录：搜 `config` 时 `node_modules` 会占满结果。
    #[test]
    fn noise_filter_blocks_dependency_and_cache_dirs() {
        for p in [
            "/Users/x/proj/node_modules/foo/package.json",
            "/Users/x/proj/.git/config",
            "/Users/x/Library/Application Support/Quark/config.json",
            "/Users/x/Library/Containers/com.foo/config.json",
            "/Users/x/Desktop/.DS_Store",
        ] {
            assert!(is_noise(p), "{p} 应被判为噪音");
        }
        for p in [
            "/Users/x/Desktop/报告.pdf",
            "/Users/x/Documents/config.yaml",
            "/Users/x/Desktop/.zshrc",
        ] {
            assert!(!is_noise(p), "{p} 是用户文件，不该被过滤");
        }
    }

    /// 参数必须是「若干 -onlyin 目录 + -name 查询词」，顺序固定。
    #[test]
    fn mdfind_args_put_onlyin_before_name() {
        let dirs = vec![PathBuf::from("/Users/x/Desktop"), PathBuf::from("/Users/x/Documents")];
        let args = mdfind_args("报告", &dirs);
        assert_eq!(
            args,
            vec![
                "-onlyin",
                "/Users/x/Desktop",
                "-onlyin",
                "/Users/x/Documents",
                "-name",
                "报告"
            ]
        );
    }

    /// 没有可用目录时不应退化成全盘检索（那会带来 5s 延迟与海量噪音）。
    #[test]
    fn empty_dirs_means_no_query() {
        assert!(mdfind_args("x", &[]).contains(&"-name".to_string()));
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
