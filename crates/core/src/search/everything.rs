//! 内联的 Everything 文件检索：直接输入关键词，文件结果补进下拉框。
//!
//! 与 `calc` / `clock` 同一类：结果就在主列表里，**不弹插件窗口**。
//! 交互沿用主列表约定 —— 文件行属于"执行类"：单击只选中，双击（或 Enter）打开。
//!
//! 边界：
//! 1. **至少 2 个字符**才查；单字符会命中海量文件，既没用又慢。
//! 2. **分数压在应用下面**：基础分 160，低于应用条目的最低分（≈234），
//!    文件只填空位，绝不把应用挤走（AGENTS.md 硬性约定 2）。
//! 3. **超时短**：es 实测 120~180ms，给 1.2s 上限；GUI 侧再放到异步线程里跑，
//!    不阻塞应用结果先上屏。
//! 4. **失败静默**：找不到 es.exe / Everything 没在跑 / 超时都返回空列表，
//!    不让每敲一个键都冒一条错误行（可用性看设置页的 Everything 状态）。
//! 5. 复用 [`crate::exec`] 的白名单 + 摘要校验 + 参数拦截，不另开一套执行路径。

use std::time::Duration;

use super::{HitPayload, SearchHit};
use crate::exec;
use crate::ItemKind;

/// 结果条数上限。文件只是补充，多了会把应用搜索结果挤下去。
pub const LIMIT: usize = 5;
/// es 单次调用超时。
pub const TIMEOUT: Duration = Duration::from_millis(1200);
/// 交给 es 自身的超时：索引没就绪时让它快速失败，别拖到进程级超时。
const ES_TIMEOUT_MS: u32 = 1000;
/// 向 es 多取一些：回收站里有成百上千个 `$I*.exe`，默认按名字排序时能占满整个窗口；
/// 改成按修改时间倒序 + 取 200 条，过滤后仍能凑满 [`LIMIT`] 条，且先给最近动过的文件。
const FETCH: usize = 200;
/// 隐式结果的基础分，按名次递减；必须低于应用条目的最低分（300×0.78≈234）。
const BASE_SCORE: f32 = 160.0;
/// 隐式检索的最小输入长度（字符数）。
const MIN_CHARS: usize = 2;

/// 交给 es 的查询串；输入太短（或全空白）返回 None。
pub fn implicit_query(input: &str) -> Option<&str> {
    let q = input.trim();
    (q.chars().count() >= MIN_CHARS).then_some(q)
}

/// 生成内联结果行。不可用 / 没结果都返回空列表，让应用结果自己上屏。
pub fn hits(input: &str) -> Vec<SearchHit> {
    let Some(q) = implicit_query(input) else {
        return Vec::new();
    };
    let hint = crate::config::Settings::load().everything_es_path;
    let hint_path = hint.as_deref().map(std::path::Path::new);
    let Ok(out) = exec::run("es", &es_args(q), TIMEOUT, hint_path) else {
        return Vec::new();
    };
    if out.code != 0 {
        return Vec::new();
    }
    parse_rows(&out.stdout)
        .into_iter()
        .take(LIMIT)
        .enumerate()
        .map(|(i, r)| file_hit(i, r, BASE_SCORE))
        .collect()
}

fn es_args(query: &str) -> Vec<String> {
    [
        "-n",
        &FETCH.to_string(),
        "-csv",
        "-no-header",
        "-full-path-and-name",
        "-size",
        "-date-modified",
        "-date-format",
        "1",
        // 不按名字排：回收站条目名字以 `$` 开头，会占满整个结果窗口
        "-sort",
        "date-modified-descending",
        "-timeout",
        &ES_TIMEOUT_MS.to_string(),
        query,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    path: String,
    size: String,
    mtime: String,
}

/// 解析 es 的 `-csv -no-header -full-path-and-name -size -date-modified` 输出。
fn parse_rows(stdout: &str) -> Vec<Row> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols = parse_csv_line(line);
        let path = cols.first().cloned().unwrap_or_default();
        if path.is_empty() || is_noise(&path) {
            continue;
        }
        out.push(Row {
            path,
            size: cols.get(1).cloned().unwrap_or_default(),
            mtime: cols.get(2).cloned().unwrap_or_default(),
        });
    }
    out
}

/// 单行 CSV：字段用 `"` 包起来，内部引号写成 `""`。
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    cur.push('"');
                    it.next();
                } else {
                    quoted = false;
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// 用户永远不会想打开的路径。实测 `ext:exe` 的默认排序第一屏全是回收站条目。
fn is_noise(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [
        "\\$recycle.bin\\",
        "\\$windows.~bt",
        "\\node_modules\\",
        "\\winsxs\\",
        "\\temp\\",
        "\\.git\\",
        // zip 解出来的 macOS 元数据目录，在 Windows 上纯属垃圾
        "\\__macosx\\",
    ]
    .iter()
    .any(|k| p.contains(k))
}

fn file_hit(rank: usize, r: Row, base: f32) -> SearchHit {
    let name = r
        .path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(r.path.as_str())
        .to_string();
    let dir = r
        .path
        .rfind(['\\', '/'])
        .map(|i| r.path[..i].to_string())
        .unwrap_or_default();
    let mut sub = String::new();
    if !dir.is_empty() {
        sub.push_str(&dir);
    }
    let size = human_size(&r.size);
    let mtime = fmt_mtime(&r.mtime);
    if !size.is_empty() {
        sub.push_str(" · ");
        sub.push_str(&size);
    }
    if !mtime.is_empty() {
        sub.push_str(" · ");
        sub.push_str(&mtime);
    }
    SearchHit::new(
        ItemKind::File,
        format!("es:{}", r.path),
        name,
        // 按 es 的时间倒序递减；整体压在应用条目下面，只填空位
        base - rank as f32 * 4.0,
        HitPayload::Open {
            path: r.path.clone(),
        },
    )
    .subtitle(sub)
    .reason(super::MatchReason::Substring)
    .source("everything")
}

/// es 的 `-size` 是原始字节数；目录为空串。
fn human_size(raw: &str) -> String {
    let Ok(n) = raw.trim().parse::<u64>() else {
        return String::new();
    };
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// `2026-10-23T17:20:45` -> `2026-10-23 17:20`（分钟足够，省宽度）。
fn fmt_mtime(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 16 && t.as_bytes()[10] == b'T' {
        format!("{} {}", &t[..10], &t[11..16])
    } else {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_does_not_query() {
        assert_eq!(implicit_query("报告"), Some("报告"));
        assert_eq!(implicit_query("  ext:pdf "), Some("ext:pdf"));
        assert_eq!(implicit_query("a b"), Some("a b"), "查询串内部空格保留");
        for no in ["", " ", "\t", "a", "报"] {
            assert_eq!(implicit_query(no), None, "{no:?} 不该触发 Everything");
        }
    }

    #[test]
    fn csv_and_columns() {
        let sample = "\"C:\\a b\\x.exe\",182,2026-11-15T09:03:08\r\n\
                      \"E:\\咨询交付\\写报告\\SKILL.md\",4096,2026-10-23T17:20:45\r\n\
                      \"D:\\dir\\sub\",,2026-11-16T14:46:05\r\n";
        let rows = parse_rows(sample);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].path, "C:\\a b\\x.exe");
        assert_eq!(rows[1].path, "E:\\咨询交付\\写报告\\SKILL.md", "中文必须原样保留");
        assert_eq!(rows[2].size, "", "目录没有大小");
        // 引号转义（Windows 文件名不允许 "，这是防御性用例）
        assert_eq!(parse_csv_line("\"a\"\"b\",1,2")[0], "a\"b");
    }

    #[test]
    fn noise_is_filtered() {
        for bad in [
            r"C:\$RECYCLE.BIN\S-1-5\$I2UZ85Z.exe",
            r"C:\Users\x\AppData\Local\Temp\a\b.exe",
            r"C:\x\node_modules\webpack\cli.js",
            r"C:\Windows\WinSxS\amd64\a.exe",
            r"D:\repo\.git\hooks\pre-push.exe",
            r"C:\x\__MACOSX\skills\a.md",
        ] {
            assert!(is_noise(bad), "{bad} 应被过滤");
        }
        for good in [
            r"E:\Program Files\QwenWorkCN\咨询交付\写报告.md",
            r"C:\Users\DELL\Downloads\IsSetup_Agent.exe",
            r"D:\Tempero\bin\tempero.exe",
        ] {
            assert!(!is_noise(good), "{good} 不该被过滤");
        }
    }

    #[test]
    fn display_formatting() {
        assert_eq!(human_size("182"), "182 B");
        assert_eq!(human_size("4096"), "4.0 KB");
        assert_eq!(human_size("157286400"), "150 MB");
        assert_eq!(human_size(""), "");
        assert_eq!(human_size("abc"), "");
        assert_eq!(fmt_mtime("2026-10-23T17:20:45"), "2026-10-23 17:20");
        assert_eq!(fmt_mtime(""), "");
    }

    #[test]
    fn row_shape_opens_the_file() {
        let row = Row {
            path: r"E:\a\b\c.md".into(),
            size: "4096".into(),
            mtime: "2026-10-23T17:20:45".into(),
        };
        let h = file_hit(0, row, BASE_SCORE);
        assert_eq!(h.kind, ItemKind::File);
        assert_eq!(h.title, "c.md");
        assert_eq!(h.source, "everything");
        assert_eq!(h.score, BASE_SCORE);
        assert_eq!(h.reason, "substring");
        assert!(h.subtitle.contains(r"E:\a\b"), "{}", h.subtitle);
        assert!(h.subtitle.contains("4.0 KB"), "{}", h.subtitle);
        assert!(h.subtitle.contains("2026-10-23 17:20"), "{}", h.subtitle);
        match &h.payload {
            HitPayload::Open { path } => assert_eq!(path, r"E:\a\b\c.md"),
            other => panic!("文件行必须是 Open：{other:?}"),
        }
    }

    #[test]
    fn file_score_stays_below_apps() {
        let row = || Row {
            path: r"E:\a\b\c.md".into(),
            size: String::new(),
            mtime: String::new(),
        };
        let first = file_hit(0, row(), BASE_SCORE);
        let last = file_hit(LIMIT - 1, row(), BASE_SCORE);
        // 应用条目的最低分 ≈ 300×0.78 = 234（见 fuzzy 的分数分层），文件行必须更低
        assert!(first.score < 234.0, "文件行会抢走应用位次: {}", first.score);
        assert!(first.score > last.score, "按名次递减");
    }

    #[test]
    fn fetch_more_than_display_limit() {
        let args = es_args("ext:exe");
        let n = args.iter().position(|a| a == "-n").expect("-n 必须存在");
        assert_eq!(args[n + 1].parse::<usize>().unwrap(), FETCH);
        assert!(FETCH > LIMIT, "噪音占满时仍要能凑出结果");
        let t = args
            .iter()
            .position(|a| a == "-timeout")
            .expect("-timeout 必须存在");
        assert_eq!(args[t + 1], ES_TIMEOUT_MS.to_string());
    }

    #[test]
    fn end_to_end_against_the_live_index() {
        // 真实链路：es.exe（摘要校验过）→ 本机 Everything → 解析 → 行。
        // Everything 没装/没跑时打印原因并跳过，但**不静默通过**。
        let hits = hits("ext:exe");
        if hits.is_empty() {
            println!("skip: 没有结果（Everything 未运行或没装 es.exe）");
            return;
        }
        assert!(hits.len() <= LIMIT);
        assert_eq!(hits[0].source, "everything");
        assert!(hits.iter().all(|h| h.kind == ItemKind::File), "全是文件行");
        assert!(
            hits.iter().all(|h| !is_noise(&h.title)),
            "标题里不该出现噪音目录名"
        );
        assert!(hits[0].score < 234.0, "文件行必须排在应用下面");
        assert!(hits.windows(2).all(|w| w[0].score > w[1].score), "分数递减");
        println!("第一条: {} | {}", hits[0].title, hits[0].subtitle);
    }
}
