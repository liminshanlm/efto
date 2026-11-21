//! Windows hosts 文件：解码、逐行校验、按原编码写回。
//!
//! 这里只做**纯逻辑**：不碰权限、不提权、不决定写哪个文件。真正落盘在
//! `crates/app/src/hosts.rs`（UAC 提权助手），插件侧只能通过宿主能力调用。
//!
//! 两个容易踩的点：
//! 1. **编码**。Windows 的 hosts 不是 UTF-8 文件，zh-CN 机器上用户加的注释
//!    往往是 GBK（系统 ANSI 代码页）。按 UTF-8 读会得到替换字符，按 UTF-8 写回
//!    会把原有中文注释毁掉。所以读取时探测：UTF-8 成功就用 UTF-8，否则按
//!    `GetACP()` 解码；保存时沿用读取时的编码，ANSI 无法表示的新字符直接报错。
//! 2. **什么算条目**。只有"行首 IP + 至少一个主机名"才算；`#` 开头的行是注释，
//!    但如果 `#` 后面紧跟一个**合法 IP**，就当作"已禁用条目"参与校验 ——
//!    这样插件把条目注释掉之后，用户再改坏它时不会静默写进一个无效文件；
//!    同时 `# 说明：10.0.0.256 是错的写法` 这类普通注释不会被误报。

use std::net::{Ipv4Addr, Ipv6Addr};

/// hosts 文件位置（由 `%SystemRoot%` 拼出，不写死盘符）。
pub fn default_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"));
    root.join("System32").join("drivers").join("etc").join("hosts")
}

/// 文件编码。插件读到什么，保存时就写回什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    /// 系统 ANSI 代码页（zh-CN 上是 GBK / 936）
    Ansi,
}

impl Encoding {
    pub fn key(self) -> &'static str {
        match self {
            Self::Utf8 => "utf8",
            Self::Ansi => "ansi",
        }
    }

    /// 解析插件传来的编码名；无法识别时返回 None，由调用方决定是否回退。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "utf8" | "utf-8" => Some(Self::Utf8),
            "ansi" | "gbk" | "cp936" => Some(Self::Ansi),
            _ => None,
        }
    }
}

/// 内容上限：hosts 是给人读的小文件，超过这个体量一定是搞错了。
pub const MAX_BYTES: usize = 1 << 20;

/// 解码文件内容：优先 UTF-8（去掉 BOM），失败按系统 ANSI 代码页解码。
pub fn decode(bytes: &[u8]) -> (String, Encoding) {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => (s.to_string(), Encoding::Utf8),
        Err(_) => (decode_ansi(body), Encoding::Ansi),
    }
}

/// 按指定编码编码。ANSI 下有无法表示的字符时明确报错，不写 `?` 进去。
pub fn encode(text: &str, enc: Encoding) -> Result<Vec<u8>, String> {
    match enc {
        Encoding::Utf8 => Ok(text.as_bytes().to_vec()),
        Encoding::Ansi => encode_ansi(text),
    }
}

#[cfg(windows)]
fn decode_ansi(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{GetACP, MultiByteToWideChar};
    if bytes.is_empty() {
        return String::new();
    }
    let cp = unsafe { GetACP() };
    unsafe {
        let n = MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if n <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; n as usize];
        let m = MultiByteToWideChar(cp, 0, bytes.as_ptr(), bytes.len() as i32, buf.as_mut_ptr(), n);
        if m <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        buf.truncate(m as usize);
        String::from_utf16_lossy(&buf)
    }
}

#[cfg(not(windows))]
fn decode_ansi(bytes: &[u8]) -> String {
    // 非 Windows 只做无损兜底：单字节映射，至少不会 panic
    bytes.iter().map(|&b| b as char).collect()
}

#[cfg(windows)]
fn encode_ansi(text: &str) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Globalization::{GetACP, WideCharToMultiByte};
    let wide: Vec<u16> = text.encode_utf16().collect();
    if wide.is_empty() {
        return Ok(Vec::new());
    }
    let cp = unsafe { GetACP() };
    let mut used_default: i32 = 0;
    unsafe {
        let n = WideCharToMultiByte(
            cp,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            &mut used_default,
        );
        if n <= 0 {
            return Err("按系统 ANSI 代码页编码失败".into());
        }
        if used_default != 0 {
            return Err(format!(
                "内容里有系统 ANSI 代码页（{cp}）无法表示的字符。为避免损坏文件，已取消保存；请删掉这些字符后重试"
            ));
        }
        let mut buf = vec![0u8; n as usize];
        let m = WideCharToMultiByte(
            cp,
            0,
            wide.as_ptr(),
            wide.len() as i32,
            buf.as_mut_ptr(),
            n,
            std::ptr::null(),
            &mut used_default,
        );
        if m <= 0 || used_default != 0 {
            return Err("按系统 ANSI 代码页编码失败".into());
        }
        buf.truncate(m as usize);
        Ok(buf)
    }
}

#[cfg(not(windows))]
fn encode_ansi(text: &str) -> Result<Vec<u8>, String> {
    Ok(text.chars().map(|c| c as u8).collect())
}

/// 校验整份 hosts 文本。返回所有问题（带行号），一次让用户改完；
/// 只保留前 20 条，避免一个坏文件刷屏。
pub fn validate(text: &str) -> Result<(), Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    if text.len() > MAX_BYTES {
        errs.push(format!("内容超过 {} 字节上限", MAX_BYTES));
        return Err(errs);
    }
    if text.contains('\0') {
        errs.push("内容里含 NUL 字符".to_string());
    }

    for (i, raw) in text.lines().enumerate() {
        if errs.len() >= 20 {
            break;
        }
        let line_no = i + 1;
        if let Err(e) = check_line(raw) {
            errs.push(format!("第 {line_no} 行：{e}"));
        }
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn check_line(raw: &str) -> Result<(), String> {
    let line = raw.trim();
    if line.is_empty() {
        return Ok(());
    }
    let (enabled, body) = match line.strip_prefix('#') {
        Some(rest) => (false, rest.trim_start()),
        None => (true, line),
    };
    // 行内注释：`#` 之后不再校验
    let body = match body.find('#') {
        Some(i) => body[..i].trim_end(),
        None => body,
    };
    let tokens: Vec<&str> = body.split_whitespace().collect();
    if tokens.is_empty() {
        return Ok(()); // 纯注释
    }
    if !enabled && !is_ip(tokens[0]) {
        return Ok(()); // 普通注释，不当作条目
    }
    if !is_ip(tokens[0]) {
        return Err(format!("第一列 {:?} 不是合法的 IPv4/IPv6 地址", tokens[0]));
    }
    if tokens.len() < 2 {
        return Err(format!("IP {:?} 后面缺少主机名", tokens[0]));
    }
    for name in &tokens[1..] {
        if !valid_name(name) {
            return Err(format!("主机名 {name:?} 含非法字符（只允许字母、数字、`.`、`-`、`_`）"));
        }
    }
    Ok(())
}

/// 合法 IPv4 / IPv6。IPv6 允许 `%` 作用域后缀（如 `fe80::1%12`）。
pub fn is_ip(s: &str) -> bool {
    if s.parse::<Ipv4Addr>().is_ok() {
        return true;
    }
    let base = s.split('%').next().unwrap_or(s);
    !base.is_empty() && base.parse::<Ipv6Addr>().is_ok()
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_entries_and_comments() {
        let text = "# Copyright (c) 1993-2009 Microsoft Corp.\r\n\
                    \r\n\
                    127.0.0.1   localhost\r\n\
                    ::1         localhost\r\n\
                    10.0.0.5    dev.local dev\r\n\
                    # 8.8.8.8 dns.google\r\n\
                    # 只是说明文字，不是条目\r\n\
                    192.168.1.2  api.local # 行内备注\r\n";
        assert_eq!(validate(text), Ok(()));
    }

    #[test]
    fn rejects_bad_ip_and_missing_host() {
        let e = validate("10.0.0.999 broken\r\n").unwrap_err();
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("第 1 行"), "{e:?}");
        assert!(e[0].contains("IPv4") || e[0].contains("IPv6"), "{e:?}");

        let e = validate("127.0.0.1\r\n").unwrap_err();
        assert!(e[0].contains("缺少主机名"), "{e:?}");

        let e = validate("not-an-ip localhost\r\n").unwrap_err();
        assert!(e[0].contains("不是合法"), "{e:?}");
    }

    #[test]
    fn rejects_bad_hostname_characters() {
        let e = validate("127.0.0.1 bad/name\r\n").unwrap_err();
        assert!(e[0].contains("非法字符"), "{e:?}");
        let e = validate("127.0.0.1 a b c^d\r\n").unwrap_err();
        assert!(e[0].contains("非法字符"), "{e:?}");
    }

    #[test]
    fn disabled_entry_is_validated_but_plain_comment_is_not() {
        // 被注释掉的合法条目仍按条目校验：改坏主机名要报错
        let e = validate("#10.0.0.1 bad/name\r\n").unwrap_err();
        assert!(e[0].contains("非法字符"), "{e:?}");
        // 首个 token 不是合法 IP 时只是普通注释，不能误报
        assert_eq!(validate("#10.0.0.256 dev.local\r\n"), Ok(()));
        assert_eq!(validate("# 说明：10.0.0.256 是错的写法\r\n"), Ok(()));
        // 但同一行如果启用，就必须报错
        assert!(validate("10.0.0.256 dev.local\r\n").is_err());
    }

    #[test]
    fn reports_line_numbers_and_stops_at_20() {
        let mut text = String::new();
        for i in 1..=25 {
            text.push_str(&format!("bad-{i} host{i}\r\n"));
        }
        let e = validate(&text).unwrap_err();
        assert_eq!(e.len(), 20, "错误数量应封顶 20：{}", e.len());
        assert!(e[0].starts_with("第 1 行"), "{}", e[0]);
    }

    #[test]
    fn ip_parser_covers_scope_and_rejects_junk() {
        assert!(is_ip("127.0.0.1"));
        assert!(is_ip("::1"));
        assert!(is_ip("fe80::1%12"));
        assert!(!is_ip("10.0.0.999"));
        assert!(!is_ip("127.0.0.1.1"));
        assert!(!is_ip("localhost"));
        assert!(!is_ip(""));
    }

    #[test]
    fn utf8_roundtrip() {
        let text = "127.0.0.1 localhost # 本地\r\n";
        let bytes = encode(text, Encoding::Utf8).unwrap();
        let (back, enc) = decode(&bytes);
        assert_eq!(back, text);
        assert_eq!(enc, Encoding::Utf8);
        // BOM 会被剥掉
        let (no_bom, _) = decode(&[0xEF, 0xBB, 0xBF, b'x']);
        assert_eq!(no_bom, "x");
    }

    #[test]
    fn ansi_roundtrip_uses_the_system_code_page() {
        if !cfg!(windows) {
            return;
        }
        // 用中文注释验证：如果机器代码页不支持中文，编码阶段会明确报错，
        // 而不是写出 `?`。两边的期望都由同一个 ACP 推导，不写死 936。
        let text = "# 本地解析\r\n127.0.0.1 localhost\r\n";
        match encode(text, Encoding::Ansi) {
            Ok(bytes) => {
                let (back, enc) = decode(&bytes);
                assert_eq!(back, text);
                assert_eq!(enc, Encoding::Ansi);
            }
            Err(e) => {
                assert!(!e.is_empty());
            }
        }
    }

    #[test]
    fn encoding_names_are_strict() {
        assert_eq!(Encoding::parse("UTF-8"), Some(Encoding::Utf8));
        assert_eq!(Encoding::parse("ansi"), Some(Encoding::Ansi));
        assert_eq!(Encoding::parse("gbk"), Some(Encoding::Ansi));
        assert_eq!(Encoding::parse("utf16"), None);
    }

    #[test]
    fn nul_and_oversize_are_refused() {
        assert!(validate("127.0.0.1 a\0b\r\n").unwrap_err()[0].contains("NUL"));
        let huge = "x".repeat(MAX_BYTES + 1);
        assert!(validate(&huge).unwrap_err()[0].contains("上限"));
    }

    #[test]
    fn default_path_uses_system_root() {
        let p = default_path();
        assert_eq!(p.file_name().unwrap(), "hosts");
        assert!(p.to_string_lossy().contains("System32"));
    }
}
