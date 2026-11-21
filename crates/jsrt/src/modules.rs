//! 插件内 `require()` 的路径解析与加载（含 TS 转译）。
//!
//! 放在 jsrt 而不是宿主里，是因为 GUI 宿主与 CLI 宿主必须**共用同一套**解析规则，
//! 否则"CLI 验证通过、GUI 里跑不起来"这种分裂会出现（本项目真的踩过）。
//!
//! 安全约束：解析结果必须落在插件根目录内，越界直接拒绝。

use std::path::{Path, PathBuf};

use crate::ipc::JsError;
use crate::ts;

/// require 解析顺序：显式扩展名 → 补常见扩展名 → 目录下的 index.*
const EXT_ORDER: &[&str] = &[".js", ".mjs", ".cjs", ".json", ".ts", ".mts", ".cts"];
const INDEX_FILES: &[&str] = &["index.js", "index.mjs", "index.ts", "index.cjs", "index.json"];

/// 加载并转译后的模块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedModule {
    /// 转译后的 JS 源码
    /// 转译后的 JS 源码
    pub js: String,
    /// 解析出的绝对文件路径
    pub filename: PathBuf,
    /// 供嵌套 require 使用的基准目录
    pub dirname: PathBuf,
}

/// 解析 require 目标，结果必须落在插件目录内。
pub fn resolve(root: &Path, request: &str, from_dir: &str) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("插件根目录不可访问: {e}"))?;
    let request = request.trim();
    if request.is_empty() {
        return Err("require: 空模块名".into());
    }
    if !request.starts_with('.') && !is_absolute(request) {
        return Err(format!(
            "require('{request}'): 本运行器不提供 node_modules 与内置模块解析，请用相对路径（如 './lib/x'）"
        ));
    }
    let base = if from_dir.is_empty() {
        root.clone()
    } else {
        PathBuf::from(from_dir)
    };
    let target = if request.starts_with('.') {
        base.join(request)
    } else {
        PathBuf::from(request)
    };

    let mut tried: Vec<PathBuf> = Vec::new();
    for candidate in candidates(&target) {
        let Ok(canonical) = candidate.canonicalize() else {
            tried.push(candidate);
            continue;
        };
        if !canonical.starts_with(&root) {
            return Err(format!(
                "require('{request}'): 解析结果 {} 越出插件目录",
                canonical.display()
            ));
        }
        if canonical.is_file() {
            return Ok(canonical);
        }
        tried.push(canonical);
    }
    Err(format!(
        "require('{request}') 未找到（相对 {}），已尝试 {} 个路径",
        base.display(),
        tried.len()
    ))
}

fn is_absolute(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
}

fn candidates(target: &Path) -> Vec<PathBuf> {
    let mut out = Vec::with_capacity(EXT_ORDER.len() + 2);
    // 1. 原样（作者写了扩展名）
    out.push(target.to_path_buf());
    // 2. 常见 js/ts 写法互转：作者写 './a' 而实际是 './a.js' / './a.ts'
    if target.extension().is_none() {
        for ext in EXT_ORDER {
            let mut p = target.as_os_str().to_os_string();
            p.push(*ext);
            out.push(PathBuf::from(p));
        }
    }
    // 3. 目录 → index.*
    for idx in INDEX_FILES {
        out.push(target.join(idx));
    }
    out
}

/// 读取模块源码：TS 走类型剥离，JSON 自动包装，ESM 语法给出可操作提示。
pub fn load(path: &Path) -> Result<LoadedModule, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    let is_json = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    let is_ts = path
        .extension()
        .map(|e| {
            let e = e.to_ascii_lowercase();
            e == "ts" || e == "mts" || e == "cts"
        })
        .unwrap_or(false);

    let js = if is_json {
        // JSON 模块包一层，保持 `module.exports = {...}` 语义
        format!("module.exports = {text};\n")
    } else if is_ts {
        ts::transpile_ts(&text, &path.to_string_lossy()).map_err(|e| e.to_string())?
    } else {
        text
    };

    if let Some(kw) = ts::detect_esm(&js) {
        return Err(ts::esm_hint(&path.to_string_lossy(), kw));
    }

    let dirname = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(LoadedModule {
        js: strip_shebang(&js),
        filename: path.to_path_buf(),
        dirname,
    })
}

fn strip_shebang(src: &str) -> String {
    if let Some(rest) = src.strip_prefix("#!") {
        return rest.find('\n').map(|i| rest[i + 1..].to_string()).unwrap_or_default();
    }
    src.to_string()
}

/// 转成宿主 module.load 能力约定的 JSON 结构。
pub fn to_json(m: &LoadedModule) -> serde_json::Value {
    serde_json::json!({
        "js": m.js,
        "filename": m.filename.to_string_lossy(),
        "dirname": m.dirname.to_string_lossy(),
    })
}

/// 供宿主统一实现模块解析与加载两个能力（GUI 与 CLI 共用一套规则）。
pub fn handle(
    root: &Path,
    method: &str,
    args: &[serde_json::Value],
) -> Result<serde_json::Value, JsError> {
    match method {
        "resolve" => {
            let request = args.first().and_then(|v| v.as_str()).ok_or_else(|| {
                JsError::HostCall {
                    module: "module".into(),
                    method: "resolve".into(),
                    message: "缺少模块名".into(),
                }
            })?;
            let from = args.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let path = resolve(root, request, from).map_err(|e| JsError::HostCall {
                module: "module".into(),
                method: "resolve".into(),
                message: e,
            })?;
            Ok(serde_json::json!({
                "path": path.to_string_lossy(),
                "dirname": path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
            }))
        }
        "load" => {
            let raw = args.first().and_then(|v| v.as_str()).ok_or_else(|| JsError::HostCall {
                module: "module".into(),
                method: "load".into(),
                message: "load 需要路径参数".into(),
            })?;
            let candidate = PathBuf::from(raw);
            let inside = candidate
                .canonicalize()
                .ok()
                .map(|p| root.canonicalize().map(|r| p.starts_with(r)).unwrap_or(false))
                .unwrap_or(false);
            if !inside {
                return Err(JsError::HostCall {
                    module: "module".into(),
                    method: "load".into(),
                    message: format!("拒绝读取插件目录之外的文件: {raw}"),
                });
            }
            let m = load(&candidate).map_err(|e| JsError::HostCall {
                module: "module".into(),
                method: "load".into(),
                message: e,
            })?;
            Ok(to_json(&m))
        }
        other => Err(JsError::HostCall {
            module: "module".into(),
            method: other.to_string(),
            message: "module 只支持 resolve / load".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn resolves_explicit_and_implicit_extensions() {
        let d = dir();
        std::fs::write(d.path().join("a.js"), "1").unwrap();
        std::fs::write(d.path().join("b.ts"), "2").unwrap();
        std::fs::create_dir_all(d.path().join("c")).unwrap();
        std::fs::write(d.path().join("c").join("index.js"), "3").unwrap();

        assert!(resolve(d.path(), "./a.js", "").unwrap().ends_with("a.js"));
        assert!(resolve(d.path(), "./a", "").unwrap().ends_with("a.js"));
        assert!(resolve(d.path(), "./b", "").unwrap().ends_with("b.ts"));
        assert!(resolve(d.path(), "./c", "").unwrap().ends_with("index.js"));
    }

    #[test]
    fn resolves_relative_to_importer_dir() {
        let d = dir();
        std::fs::create_dir_all(d.path().join("lib")).unwrap();
        std::fs::write(d.path().join("lib").join("util.js"), "1").unwrap();
        std::fs::create_dir_all(d.path().join("sub")).unwrap();
        // 从 sub/ 引用 ../lib/util
        let got = resolve(d.path(), "../lib/util", &d.path().join("sub").to_string_lossy());
        assert!(got.unwrap().ends_with("util.js"));
    }

    #[test]
    fn rejects_bare_specifiers_and_escapes() {
        let d = dir();
        let e = resolve(d.path(), "fs", "").unwrap_err();
        assert!(e.contains("node_modules"), "{e}");
        std::fs::write(d.path().join("x.js"), "1").unwrap();
        // 试图跳出插件根
        let e = resolve(d.path(), "../../etc/hosts", "").unwrap_err();
        assert!(e.contains("未找到") || e.contains("越界"), "{e}");
        assert!(resolve(d.path(), "", "").is_err());
    }

    #[test]
    fn load_transpiles_ts_and_wraps_json() {
        let d = dir();
        let ts_path = d.path().join("m.ts");
        std::fs::write(&ts_path, "const n: number = 1; exports.n = n;").unwrap();
        let m = load(&ts_path).unwrap();
        assert!(!m.js.contains(": number"), "{}", m.js);
        assert_eq!(m.dirname, d.path());

        let json_path = d.path().join("data.json");
        std::fs::write(&json_path, "{\"a\":1}").unwrap();
        let m = load(&json_path).unwrap();
        assert!(m.js.starts_with("module.exports ="), "{}", m.js);
    }

    #[test]
    fn load_rejects_esm_with_hint() {
        let d = dir();
        let p = d.path().join("e.js");
        std::fs::write(&p, "import x from 'y';\n").unwrap();
        let e = load(&p).unwrap_err();
        assert!(e.contains("require"), "{e}");
    }

    #[test]
    fn load_strips_shebang() {
        let d = dir();
        let p = d.path().join("s.js");
        std::fs::write(&p, "#!/usr/bin/env node\nexports.a=1;").unwrap();
        assert!(!load(&p).unwrap().js.starts_with("#!"));
    }

    #[test]
    fn handle_resolves_and_refuses_outside_files() {
        let d = dir();
        std::fs::write(d.path().join("k.js"), "1").unwrap();
        let v = handle(d.path(), "resolve", &[json!("./k"), json!("")]).unwrap();
        assert!(v["path"].as_str().unwrap().ends_with("k.js"));

        let outside = std::env::temp_dir().join("efto-outside-probe.js");
        std::fs::write(&outside, "1").unwrap();
        let e = handle(d.path(), "load", &[json!(outside.to_string_lossy())]).unwrap_err();
        assert!(format!("{e}").contains("插件目录之外"), "{e}");
        let _ = std::fs::remove_file(&outside);
    }

    #[test]
    fn handle_unknown_method_errors() {
        let d = dir();
        assert!(handle(d.path(), "bogus", &[]).is_err());
    }
}
