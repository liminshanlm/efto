//! 插件内联输出：把 `efto.output(rows)` 的 JSON 行转成搜索结果。
//!
//! 行格式：
//! ```json
//! { "title": "复制 v1.2.3", "subtitle": "包的版本号", "copy": "1.2.3" }
//! { "title": "打开项目", "subtitle": "D:\\work", "open": "D:\\work" }
//! ```

use serde_json::Value;

use crate::search::{HitPayload, SearchHit};
use crate::ItemKind;

/// 单次最多接受的行数（与垫片侧校验保持一致）。
pub const MAX_ROWS: usize = 50;

/// 把输出行转成 `SearchHit`。任何一行不合法都会整批报错，不做静默丢弃。
pub fn hits_from_rows(plugin_id: &str, rows: &Value) -> Result<Vec<SearchHit>, String> {
    let list = rows
        .as_array()
        .ok_or("efto.output 需要数组，行格式 { title, subtitle?, copy?, open? }")?;
    if list.len() > MAX_ROWS {
        return Err(format!("efto.output 单次最多 {MAX_ROWS} 条"));
    }
    let mut out = Vec::with_capacity(list.len());
    for (i, row) in list.iter().enumerate() {
        let title = row
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("efto.output 第 {} 行缺少非空 title", i + 1))?;
        let subtitle = row
            .get("subtitle")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let copy = row.get("copy").and_then(|v| v.as_str());
        let open = row.get("open").and_then(|v| v.as_str());
        let payload = match (copy, open) {
            (Some(text), _) => HitPayload::CopyText {
                text: text.to_string(),
                echo: row.clone(),
            },
            (None, Some(path)) if !path.trim().is_empty() => HitPayload::Open {
                path: path.to_string(),
            },
            _ => {
                return Err(format!(
                    "efto.output 第 {} 行需要 copy 或 open 字段",
                    i + 1
                ))
            }
        };
        let mut hit = SearchHit::new(
            ItemKind::Output,
            format!("plugin-output:{plugin_id}:{i}"),
            title,
            900.0 - i as f32,
            payload,
        )
        .subtitle(subtitle)
        .source(format!("plugin:{plugin_id}"));
        hit.reason = "plugin_output".into();
        out.push(hit);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_copy_and_open_rows() {
        let rows = json!([
            { "title": "复制版本号", "subtitle": "1.2.3", "copy": "1.2.3" },
            { "title": "打开项目", "open": "D:\\work" }
        ]);
        let hits = hits_from_rows("demo", &rows).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].kind, ItemKind::Output);
        assert!(matches!(hits[0].payload, HitPayload::CopyText { .. }));
        assert!(matches!(hits[1].payload, HitPayload::Open { .. }));
        assert_eq!(hits[0].source, "plugin:demo");
    }

    #[test]
    fn rejects_bad_rows_explicitly() {
        let no_action = json!([{ "title": "x" }]);
        assert!(hits_from_rows("demo", &no_action).is_err());
        let no_title = json!([{ "copy": "x" }]);
        assert!(hits_from_rows("demo", &no_title).is_err());
        assert!(hits_from_rows("demo", &json!({})).is_err());
    }
}
