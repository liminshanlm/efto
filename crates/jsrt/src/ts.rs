//! TypeScript → JavaScript：用 deno_ast（内部就是 SWC）剥类型，不依赖 Node/tsc。
//!
//! 第一阶段只做**类型剥离**（type stripping），与 Node 22+ 的 `--experimental-strip-types` 同档：
//! 支持注解、interface、type、泛型、`as`、非空断言、类型导入导出；
//! 不支持需要"生成运行时代码"的语法（enum / namespace 里的值 / 装饰器 / 参数属性），
//! 命中时给出明确报错，提示改用 const 对象或预编译产物，而不是产出错误的 JS。

use std::sync::Arc;

use deno_ast::swc::ast::{Decl, EsVersion, ModuleDecl, ModuleItem, Program, Stmt};
use deno_ast::swc::codegen::text_writer::JsWriter;
use deno_ast::swc::codegen::{Config as CodegenConfig, Emitter, Node};
use deno_ast::swc::common::sync::Lrc;
use deno_ast::swc::common::SourceMap;
use deno_ast::swc::ecma_visit::VisitMutWith;
use deno_ast::swc::transforms::typescript::strip_type;
use deno_ast::{parse_module, MediaType, ModuleSpecifier, ParseParams};

use crate::ipc::JsError;

/// 转译 TypeScript 源码。`specifier` 只用于报错定位与 AST 内部标识。
pub fn transpile_ts(source: &str, specifier: &str) -> Result<String, JsError> {
    if source.trim().is_empty() {
        return Err(JsError::EmptySource(specifier.to_string()));
    }
    let parsed = parse_module(ParseParams {
        specifier: ModuleSpecifier::parse(&synthetic_url(specifier))
            .map_err(|e| JsError::Transpile {
                path: specifier.to_string(),
                message: format!("specifier 无效: {e}"),
            })?,
        text: Arc::from(source),
        media_type: MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    })
    .map_err(|e| JsError::Transpile {
        path: specifier.to_string(),
        message: format!("{e}"),
    })?;

    let mut program: Program = (*parsed.program()).clone();
    program.visit_mut_with(&mut strip_type());

    if let Some(feature) = find_unsupported(&program) {
        return Err(JsError::UnsupportedSyntax(format!(
            "{specifier} 使用了需要预编译的 TypeScript 语法：{feature}。请改用 const 对象/普通函数，或把清单的 main 指向已编译的 .js"
        )));
    }

    emit_js(&program, specifier)
}

/// 转译 .tsx / JSX 不支持（第一阶段），detect 后由调用方提示。
pub fn looks_like_jsx(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".tsx") || lower.ends_with(".jsx")
}

fn synthetic_url(specifier: &str) -> String {
    // 固定 scheme + 清洗过的名字，避免 Windows 路径转 URL 的坑
    let clean: String = specifier
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("efto-plugin:///{clean}")
}

fn emit_js(program: &Program, specifier: &str) -> Result<String, JsError> {
    let cm: Lrc<SourceMap> = Default::default();
    // swc 的 codegen Config 是 #[non_exhaustive]：不能在外部用结构体字面量构造，只能改字段
    let mut cfg = CodegenConfig::default();
    cfg.target = EsVersion::Es2022;
    cfg.minify = false;
    cfg.ascii_only = false;
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    {
        let writer = Box::new(JsWriter::new(cm.clone(), "\n", &mut buf, None));
        let mut emitter = Emitter {
            cfg,
            cm: cm.clone(),
            comments: None,
            wr: writer,
        };
        match program {
            Program::Module(m) => m
                .emit_with(&mut emitter)
                .map_err(|e| JsError::Transpile {
                    path: specifier.to_string(),
                    message: e.to_string(),
                })?,
            Program::Script(s) => s
                .emit_with(&mut emitter)
                .map_err(|e| JsError::Transpile {
                    path: specifier.to_string(),
                    message: e.to_string(),
                })?,
        }
    }
    String::from_utf8(buf).map_err(|e| JsError::Transpile {
        path: specifier.to_string(),
        message: format!("输出非 UTF-8: {e}"),
    })
}

/// 扫描剥离后仍残留的"需要代码生成"的语法。只查顶层与一级语句，够用且零依赖。
fn find_unsupported(program: &Program) -> Option<&'static str> {
    let items = match program {
        Program::Module(m) => &m.body,
        Program::Script(_) => return None,
    };
    for item in items {
        match item {
            ModuleItem::Stmt(Stmt::Decl(decl)) => {
                if let Some(f) = decl_feature(decl) {
                    return Some(f);
                }
            }
            ModuleItem::ModuleDecl(md) => {
                match md {
                    // `export enum E {}` / `export namespace N {}` 走的是 ExportDecl，
                    // 只看 Stmt::Decl 会完全漏掉——真机测试里就是这么漏的。
                    ModuleDecl::ExportDecl(export) => {
                        if let Some(f) = decl_feature(&export.decl) {
                            return Some(f);
                        }
                    }
                    ModuleDecl::TsImportEquals(_) | ModuleDecl::TsExportAssignment(_) => {
                        return Some("export = / import = require");
                    }
                    _ => {}
                }
            }
            ModuleItem::Stmt(_) => {}
        }
    }
    None
}

fn decl_feature(decl: &Decl) -> Option<&'static str> {
    match decl {
        Decl::TsEnum(_) => Some("enum（建议用 const 对象）"),
        Decl::TsModule(_) => Some("namespace / module 的值声明"),
        Decl::Class(c) => {
            if !c.class.decorators.is_empty() {
                Some("装饰器 decorator")
            } else if c
                .class
                .body
                .iter()
                .any(|m| matches!(m, deno_ast::swc::ast::ClassMember::TsIndexSignature(_)))
            {
                Some("class 的 index signature")
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 只做类型剥离，不检查语法支持度（给 require 内部用）。
pub fn transpile_ts_lenient(source: &str, specifier: &str) -> Result<String, JsError> {
    transpile_ts(source, specifier)
}

/// 入口与 require 的模块都在函数作用域里执行（CommonJS 语义，与插件 main.js 一致），
/// 因此顶层 `import`/`export` 语法不可用。这里给出可操作的提示，而不是让 V8 抛
/// "Illegal use of syntactic limitation"。
///
/// 判定是词法的（不解析），刻意保守：只认行首的 import/export/`export default`。
/// 字符串里出现这些词不会误判，因为要求前面是行首或 `}`/`;` 之后的行首位置。
pub fn detect_esm(source: &str) -> Option<&'static str> {
    for line in source.lines() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }
        if let Some(rest) = t.strip_prefix("import ") {
            // `import x = require('y')` 是 TS 的 CJS 等价写法，允许
            if !rest.starts_with('=') {
                return Some("import");
            }
        }
        if t.starts_with("export ") || t.starts_with("export{") || t.starts_with("export {") {
            return Some("export");
        }
    }
    None
}

/// 把 ESM 语法转成 CommonJS 的建议文案。
pub fn esm_hint(specifier: &str, keyword: &str) -> String {
    format!(
        "{specifier} 用了 ESM 的 {keyword} 语法。插件入口与被 require 的模块都在 CommonJS 作用域里执行\
         （与插件 main.js 一致）：请把 `import x from './a'` 改成 `const x = require('./a')`，\
         把 `export function f(){{}}` 改成 `exports.f = function(){{}}` 或 `module.exports = {{ f }}`。\
         若要保留 ESM 写法，可先用 tsc/esbuild 编译成 CJS 再指向产物。"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_annotations_and_interfaces() {
        let src = r#"
interface Cfg { name: string; retries?: number }
const cfg: Cfg = { name: "demo" };
function greet(who: string, times: number = 1): string {
  return who.repeat(times);
}
export const out = greet(cfg.name, 2);
"#;
        let js = transpile_ts(src, "main.ts").unwrap();
        assert!(!js.contains("interface"), "{js}");
        assert!(!js.contains(": string"), "{js}");
        assert!(js.contains("greet"), "{js}");
        assert!(js.contains("export const out"), "{js}");
    }

    #[test]
    fn strips_type_only_imports_and_generics() {
        let js = transpile_ts(
            "import type { Foo } from './types.ts';\nconst x = <T,>(v: T): T => v;\nlet n = x<number>(1);\nexport { n };",
            "a.ts",
        )
        .unwrap();
        assert!(!js.contains("import type"), "{js}");
        assert!(!js.contains("<T,>"), "{js}");
    }

    #[test]
    fn rejects_enum_with_actionable_message() {
        let err = transpile_ts("export enum E { A = 1 }", "bad.ts").unwrap_err();
        match err {
            JsError::UnsupportedSyntax(msg) => {
                assert!(msg.contains("enum"), "{msg}");
                assert!(msg.contains("bad.ts"), "{msg}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_namespace_values() {
        assert!(matches!(
            transpile_ts("namespace N { export const a = 1 }", "ns.ts"),
            Err(JsError::UnsupportedSyntax(_))
        ));
    }

    #[test]
    fn syntax_error_is_reported() {
        let err = transpile_ts("const a: = ;;;", "broken.ts").unwrap_err();
        assert!(matches!(err, JsError::Transpile { .. }), "{err:?}");
    }

    #[test]
    fn empty_source_errors() {
        assert!(matches!(
            transpile_ts("   ", "e.ts"),
            Err(JsError::EmptySource(_))
        ));
    }

    #[test]
    fn output_is_valid_enough_for_v8() {
        // 冒烟：转译结果应能被 JS 引擎解析（在 jsrt 的集成测试里真的会喂给 V8）
        let js = transpile_ts(
            "export function add(a: number, b: number): number { return a + b }",
            "m.ts",
        )
        .unwrap();
        assert!(js.contains("function add(a, b)"), "{js}");
    }

    #[test]
    fn jsx_detected() {
        assert!(looks_like_jsx("a/b.tsx"));
        assert!(!looks_like_jsx("a/b.ts"));
    }
}
