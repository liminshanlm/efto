//! 内置计算器：输入是算式时，在结果列表里直接给出计算结果。
//!
//! 设计取舍（按重要性排序）：
//! 1. **绝不劫持应用搜索**。判定条件很严：必须出现运算符/函数调用/括号之一，
//!    且整串能被完整消费（残留任何字符就放弃）。因此 `chrome`、`winrar`、
//!    `3.10 64-bit`、`C:\Program Files`、`1700000000`（纯数字）都不算算式。
//! 2. **不 panic、不栈溢出**。递归下降带深度上限，输入长度上限，
//!    数字字面量长度上限；除零/溢出/未定义函数一律返回 None（不显示），而不是报错。
//! 3. **中文输入法友好**：全角数字与运算符（１＋１＝、×÷（））先归一化成半角，
//!    千分位逗号（1,000 + 1）支持，末尾多余的 `=` 容忍。

use serde_json::json;

use super::{HitPayload, SearchHit};
use crate::ItemKind;

/// 输入长度上限：超过就当普通文本处理。
const MAX_INPUT_LEN: usize = 256;
/// 递归深度上限，防止 `((((...))))` 打爆栈。
const MAX_DEPTH: u32 = 64;

/// 计算结果。
#[derive(Debug, Clone, PartialEq)]
pub struct CalcResult {
    /// 规范化后的算式（半角、去多余空格），用于展示
    pub expression: String,
    /// 数值
    pub value: f64,
    /// 人类可读的结果（整数不带小数点，大数用科学计数）
    pub formatted: String,
}

/// 尝试把输入当算式求值。不是算式 / 求值失败都返回 None。
pub fn evaluate(input: &str) -> Option<CalcResult> {
    let normalized = normalize(input);
    if normalized.is_empty() || normalized.len() > MAX_INPUT_LEN {
        return None;
    }
    // 必须至少含一个运算符、括号或函数调用，否则纯数字/纯标识符会污染搜索
    if !normalized.contains(|c: char| matches!(c, '+' | '-' | '*' | '/' | '%' | '^' | '(' | '['))
        && !has_function(&normalized)
    {
        return None;
    }
    // 时间优先：`2026-11-16` 是日期，不是 "2026 减 11 减 16"。
    // 没有这道闸门，输入日期会顺手算出 1,999，把用户真正想要的那行挤成一堆噪音。
    // 只在整个串确实被时间解析器接受时才让位，`2-3`、`11/16` 这类仍归计算器。
    if super::timestamp::looks_like_time_input(&normalized) {
        return None;
    }
    let tokens = tokenize(&normalized)?;
    if tokens.is_empty() {
        return None;
    }
    let mut p = Parser::new(tokens);
    let value = p.parse_expr(0)?;
    // 必须消费到结尾，否则像 "1 + x" / "2 gb" 这种会被误算
    if !p.at_end() {
        return None;
    }
    if !value.is_finite() || value.abs() > 1e300 {
        return None;
    }
    // 纯数字（没有任何运算）不当成计算结果，避免输入 "3" 就冒出一行
    if !contains_operation(&normalized) {
        return None;
    }
    Some(CalcResult {
        expression: normalized.trim_end_matches('=').trim().to_string(),
        value,
        formatted: format_number(value),
    })
}

/// 生成一条搜索命中。
pub fn calc_hit(query: &str) -> Option<SearchHit> {
    let r = evaluate(query)?;
    let hit = SearchHit::new(
        ItemKind::Calc,
        format!("calc:{}", r.expression),
        r.formatted.clone(),
        680.0,
        HitPayload::CopyText {
            text: r.formatted.clone(),
            echo: json!({ "expression": r.expression, "value": r.value }),
        },
    )
    .subtitle(format!("计算结果 · {} =", r.expression))
    .reason(super::MatchReason::Exact)
    .source("calc");
    Some(hit)
}

/// 全角转半角、统一符号、去掉无害空白。
fn normalize(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for ch in src.chars() {
        let mapped = match ch {
            // 全角数字与小数点
            '０'..='９' => char::from_u32(0x30 + (ch as u32 - 0xFF10)).unwrap_or(ch),
            '．' => '.',
            // 全角/中文运算符
            '＋' => '+',
            '－' | '—' | '–' => '-',
            '×' | '✕' | '∗' => '*',
            '÷' => '/',
            '＾' => '^',
            '％' => '%',
            '（' => '(',
            '）' => ')',
            '［' => '[',
            '］' => ']',
            '＝' => '=',
            // 千分位：中文逗号、不间断空格里的薄空格
            '，' | ',' => ',',
            '\u{2009}' | '\u{202F}' | '\u{00A0}' => ' ',
            c => c,
        };
        out.push(mapped);
    }
    // 千分位逗号在数字之间去掉（1,000 -> 1000），其余逗号留给函数参数
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if *c == ',' {
            let prev_digit = i > 0 && chars[i - 1].is_ascii_digit();
            let next_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
            // 只有形如 1,234（后面正好 3 位数字）才当千分位
            if prev_digit && next_digit && runs_three_digits(&chars, i + 1) {
                continue;
            }
        }
        cleaned.push(*c);
    }
    // 结尾的等号允许（"1+1=" 与 "1 + 1 = " 都要能用），所以先 trim 再判断
    cleaned.trim().to_string()
}

fn runs_three_digits(chars: &[char], start: usize) -> bool {
    let mut n = 0;
    let mut i = start;
    while i < chars.len() && chars[i].is_ascii_digit() {
        n += 1;
        i += 1;
    }
    // 允许 1,234,567 这种连续段
    n == 3 || (n > 0 && i < chars.len() && chars[i] == ',')
}

fn has_function(s: &str) -> bool {
    // 形如 sqrt( / round(
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            let mut j = i as isize - 1;
            while j >= 0 && (bytes[j as usize].is_ascii_alphabetic() || bytes[j as usize] == b'_') {
                j -= 1;
            }
            let name = &s[(j + 1) as usize..i];
            if !name.is_empty() {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// 是否真的发生了运算。用来把 `(5)`、`1,000` 这类"只是被括号/千分位包住的数字"
/// 排除掉——它们没有计算意义，出现在下拉框里只是噪音。
fn contains_operation(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let prev = s[..i].trim_end().chars().next_back();
        match c {
            '+' | '*' | '/' | '%' | '^' => return true,
            '-' => {
                // 前面是数字/右括号/常量结尾才算二元减法，否则是一元负号
                if matches!(prev, Some(p) if p.is_ascii_digit() || p == ')') {
                    return true;
                }
            }
            '(' => {
                // 函数调用：左括号紧贴标识符
                if matches!(prev, Some(p) if p.is_ascii_alphabetic() || p == '_') {
                    return true;
                }
            }
            ',' => {
                // 函数参数分隔的逗号（千分位已在 normalize 里去掉）
                if matches!(prev, Some(p) if p.is_ascii_digit()) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    LParen,
    RParen,
    Comma,
    Ident(String),
}

fn tokenize(s: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = i;
            let mut dots = 0;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                if chars[i] == '.' {
                    dots += 1;
                }
                i += 1;
            }
            // 科学计数法 1e10 / 2.5E-3
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            if dots > 1 {
                return None; // "1.2.3" 不是算式
            }
            let raw: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            if raw == "." || raw.is_empty() {
                return None;
            }
            let v: f64 = raw.parse().ok()?;
            out.push(Token::Num(v));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Ident(chars[start..i].iter().collect::<String>().to_ascii_lowercase()));
            continue;
        }
        let t = match c {
            '+' => Token::Add,
            '-' => Token::Sub,
            '*' => Token::Mul,
            '/' => Token::Div,
            '%' => Token::Mod,
            '^' => Token::Pow,
            '(' | '[' => Token::LParen,
            ')' | ']' => Token::RParen,
            ',' => Token::Comma,
            '=' => {
                // 结尾的等号允许，中间的等号不是算式
                if s.trim_end().ends_with('=') && i + 1 == chars.len() {
                    break;
                }
                return None;
            }
            _ => return None,
        };
        out.push(t);
        i += 1;
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// expr := term (('+'|'-') term)*
    fn parse_expr(&mut self, depth: u32) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        let mut left = self.parse_term(depth)?;
        loop {
            match self.peek() {
                Some(Token::Add) => {
                    self.bump()?;
                    left += self.parse_term(depth)?;
                }
                Some(Token::Sub) => {
                    self.bump()?;
                    left -= self.parse_term(depth)?;
                }
                _ => return Some(left),
            }
        }
    }

    /// term := factor (('*'|'/'|'%') factor)*
    fn parse_term(&mut self, depth: u32) -> Option<f64> {
        let mut left = self.parse_factor(depth)?;
        loop {
            match self.peek() {
                Some(Token::Mul) => {
                    self.bump()?;
                    left *= self.parse_factor(depth)?;
                }
                Some(Token::Div) => {
                    self.bump()?;
                    let rhs = self.parse_factor(depth)?;
                    if rhs == 0.0 {
                        return None;
                    }
                    left /= rhs;
                }
                Some(Token::Mod) => {
                    self.bump()?;
                    let rhs = self.parse_factor(depth)?;
                    if rhs == 0.0 {
                        return None;
                    }
                    left %= rhs;
                }
                _ => return Some(left),
            }
        }
    }

    /// factor := ('-'|'+') factor | atom ('^' factor)?
    ///
    /// 一元负号与幂的优先级按数学惯例处理：`-2^2 = -(2^2) = -4`，
    /// 而 `2^-3` 的指数部分仍可带一元负号。实现方式是让一元符号递归调用**自己**，
    /// 于是 `-2^2` 里 `2^2` 先被算掉再取负。
    fn parse_factor(&mut self, depth: u32) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        if matches!(self.peek(), Some(Token::Sub)) {
            self.bump()?;
            return Some(-self.parse_factor(depth + 1)?);
        }
        if matches!(self.peek(), Some(Token::Add)) {
            self.bump()?;
            return self.parse_factor(depth + 1);
        }
        let base = self.parse_atom(depth)?;
        if matches!(self.peek(), Some(Token::Pow)) {
            self.bump()?;
            let exp = self.parse_factor(depth + 1)?;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    fn parse_atom(&mut self, depth: u32) -> Option<f64> {
        if depth > MAX_DEPTH {
            return None;
        }
        match self.bump()? {
            Token::Num(v) => Some(v),
            Token::LParen => {
                let v = self.parse_expr(depth + 1)?;
                match self.bump() {
                    Some(Token::RParen) => Some(v),
                    _ => None,
                }
            }
            Token::Ident(name) => {
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.bump()?;
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Token::RParen)) {
                        loop {
                            args.push(self.parse_expr(depth + 1)?);
                            match self.bump() {
                                Some(Token::Comma) => continue,
                                Some(Token::RParen) => break,
                                _ => return None,
                            }
                        }
                    } else {
                        self.bump()?;
                    }
                    return call(&name, &args);
                }
                constant(&name)
            }
            _ => None,
        }
    }
}

fn constant(name: &str) -> Option<f64> {
    match name {
        "pi" => Some(std::f64::consts::PI),
        "tau" => Some(std::f64::consts::TAU),
        "e" => Some(std::f64::consts::E),
        "inf" => None, // 无穷不当结果展示
        _ => None,
    }
}

/// 一元函数：参数个数不对就放弃（返回 None 表示"这不是算式"，而不是报错）。
fn one(args: &[f64], f: impl Fn(f64) -> f64) -> Option<f64> {
    if args.len() == 1 {
        Some(f(args[0]))
    } else {
        None
    }
}

fn call(name: &str, a: &[f64]) -> Option<f64> {
    match name {
        "sqrt" => one(a, |x| if x < 0.0 { f64::NAN } else { x.sqrt() }),
        "cbrt" => one(a, f64::cbrt),
        "abs" => one(a, f64::abs),
        "ln" => one(a, |x| if x <= 0.0 { f64::NAN } else { x.ln() }),
        "log" | "log10" => one(a, |x| if x <= 0.0 { f64::NAN } else { x.log10() }),
        "log2" => one(a, |x| if x <= 0.0 { f64::NAN } else { x.log2() }),
        "exp" => one(a, f64::exp),
        "round" => one(a, f64::round),
        "floor" => one(a, f64::floor),
        "ceil" => one(a, f64::ceil),
        "trunc" => one(a, f64::trunc),
        "sin" => one(a, f64::sin),
        "cos" => one(a, f64::cos),
        "tan" => one(a, f64::tan),
        "asin" => one(a, f64::asin),
        "acos" => one(a, f64::acos),
        "atan" => one(a, f64::atan),
        "pow" if a.len() == 2 => Some(a[0].powf(a[1])),
        "min" if a.len() == 2 => Some(a[0].min(a[1])),
        "max" if a.len() == 2 => Some(a[0].max(a[1])),
        "hypot" if a.len() == 2 => Some(a[0].hypot(a[1])),
        "atan2" if a.len() == 2 => Some(a[0].atan2(a[1])),
        "mod" if a.len() == 2 && a[1] != 0.0 => Some(a[0] % a[1]),
        // 单位换算：这是启动器里最常见的"算式"之一
        "kb" => one(a, |x| x * 1024.0),
        "mb" => one(a, |x| x * 1024.0 * 1024.0),
        "gb" => one(a, |x| x * 1024.0 * 1024.0 * 1024.0),
        "tb" => one(a, |x| x * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        _ => None,
    }
}

/// 结果格式化：整数不带小数点，普通小数最多 10 位有效并去掉尾零，
/// 极大/极小用科学计数；整数部分加千分位分隔。
pub fn format_number(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let av = v.abs();
    if av != 0.0 && (av >= 1e16 || av < 1e-9) {
        return format!("{v:e}");
    }
    if (v - v.round()).abs() < 1e-12 && av < 1e16 {
        return group_thousands(&format!("{}", v.round() as i64));
    }
    let mut s = format!("{v:.10}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string();
    if s.is_empty() || s == "-" {
        s = "0".to_string();
    }
    if let Some(dot) = s.find('.') {
        let (int, frac) = (s[..dot].to_string(), s[dot..].to_string());
        format!("{}{frac}", group_thousands(&int))
    } else {
        group_thousands(&s)
    }
}

fn group_thousands(int_part: &str) -> String {
    let (sign, digits) = match int_part.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", int_part),
    };
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    out.push_str(sign);
    let lead = digits.len() % 3;
    for (i, c) in digits.chars().enumerate() {
        // i < lead 是最高位的零头；曾经直接 i - lead 在 usize 上减法下溢 panic
        if i > 0 && i >= lead && (i - lead) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Option<f64> {
        evaluate(s).map(|r| r.value)
    }

    #[test]
    fn precedence_and_parens() {
        assert_eq!(ev("1+2*3"), Some(7.0));
        assert_eq!(ev("(1+2)*3"), Some(9.0));
        assert_eq!(ev("2^3^2"), Some(512.0), "^ 右结合");
        assert_eq!(ev("-2^2"), Some(-4.0), "一元负号优先级低于 ^");
        assert_eq!(ev("10%3"), Some(1.0));
        assert_eq!(ev("7/2"), Some(3.5));
        assert_eq!(ev(" 1 + 1 = "), Some(2.0), "容忍结尾等号与空白");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(ev("sqrt(16)"), Some(4.0));
        assert_eq!(ev("round(2.5)"), Some(3.0));
        assert_eq!(ev("max(3, 10)"), Some(10.0));
        assert_eq!(ev("min(3, 10)"), Some(3.0));
        assert_eq!(ev("pow(2,10)"), Some(1024.0));
        assert!((ev("pi*2").unwrap() - std::f64::consts::TAU).abs() < 1e-12);
        assert_eq!(ev("gb(2)"), Some(2147483648.0), "单位换算");
        assert_eq!(ev("log(1000)"), Some(3.0));
    }

    #[test]
    fn rejects_non_expressions() {
        // 这些必须是 None，否则会劫持应用搜索
        for s in [
            "chrome",
            "winrar",
            "3.10 64-bit",
            "C:\\Program Files",
            "www.example.com",
            "1700000000",
            "5",
            "notepad",
            "1 + x",
            "2 gb",
            "https://a.b/c?x=1",
            "e",
            "",
            "   ",
            "1.2.3",
            "50% off",
            "a/b",
        ] {
            assert!(evaluate(s).is_none(), "不应把 {s:?} 当算式");
        }
    }

    #[test]
    fn fullwidth_and_thousands() {
        assert_eq!(ev("１＋１"), Some(2.0));
        assert_eq!(ev("10×10"), Some(100.0));
        assert_eq!(ev("1,000 + 1"), Some(1001.0));
        assert_eq!(ev("1,234,567"), None, "纯千分位数字没有运算，不该出现计算行");
        assert_eq!(ev("1,234,567+1"), Some(1234568.0));
    }

    #[test]
    fn safe_against_bad_input() {
        assert_eq!(ev("1/0"), None, "除零不显示");
        assert_eq!(ev("sqrt(-1)"), None);
        // 深度必须真的超过上限才放弃：刚好在下限内应当正常算出来
        let just_under = "(".repeat(MAX_DEPTH as usize) + "1+1" + &")".repeat(MAX_DEPTH as usize);
        assert!(ev(&just_under).is_some(), "上限内不该误杀");
        let too_deep = "(".repeat(MAX_DEPTH as usize + 5) + "1+1" + &")".repeat(MAX_DEPTH as usize + 5);
        assert_eq!(ev(&too_deep), None, "超深度直接放弃而不是栈溢出");
        assert!(ev(&"1+".repeat(400)).is_none(), "超长输入放弃");
        assert_eq!(ev("1e999"), None, "溢出放弃");
        assert_eq!(ev("((1+2)"), None, "括号不闭合");
    }

    #[test]
    fn unary_minus_precedence() {
        assert_eq!(ev("-2^2"), Some(-4.0), "-2^2 = -(2^2)");
        assert_eq!(ev("2^-3"), Some(0.125));
        assert_eq!(ev("-(2^2)"), Some(-4.0));
        assert_eq!(ev("3 - -2"), Some(5.0));
        assert_eq!(ev("- 5 + 10"), Some(5.0));
    }

    /// 千分位分组：位数不同，零头位置就不同（这里曾经下溢 panic 且漏掉分隔符）
    #[test]
    fn thousands_grouping_all_widths() {
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(12.0), "12");
        assert_eq!(format_number(123.0), "123");
        assert_eq!(format_number(1234.0), "1,234");
        assert_eq!(format_number(12345.0), "12,345");
        assert_eq!(format_number(123456.0), "123,456");
        assert_eq!(format_number(1234567.0), "1,234,567");
        assert_eq!(format_number(-1234.0), "-1,234");
        assert_eq!(format_number(-12.0), "-12");
    }

    #[test]
    fn formatting_is_human_readable() {
        assert_eq!(format_number(2.0), "2");
        assert_eq!(format_number(-2.0), "-2");
        assert_eq!(format_number(1024.0), "1,024");
        assert_eq!(format_number(0.5), "0.5");
        assert_eq!(format_number(1.0 / 3.0), "0.3333333333");
        assert_eq!(format_number(123456.75), "123,456.75");
        assert!(format_number(1e20).contains('e'));
    }

    #[test]
    fn hit_shape_matches_contract() {
        let hit = calc_hit("6*7").unwrap();
        assert_eq!(hit.kind, ItemKind::Calc);
        assert_eq!(hit.title, "42");
        assert_eq!(hit.source, "calc");
        assert!(hit.subtitle.contains("6*7"));
        match &hit.payload {
            HitPayload::CopyText { text, echo } => {
                assert_eq!(text, "42");
                assert_eq!(echo["expression"], json!("6*7"));
                assert_eq!(echo["value"], json!(42.0));
            }
            other => panic!("wrong payload {other:?}"),
        }
    }

    #[test]
    fn no_hit_for_plain_app_name() {
        assert!(calc_hit("chrome").is_none());
        assert!(calc_hit("").is_none());
    }
}
