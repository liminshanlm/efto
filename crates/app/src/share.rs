//! 局域网只读文件共享：一个最小的 HTTP/1.1 服务。
//!
//! 只暴露调用方显式传进来的路径；不支持上传、账号与权限。所有请求的路径都在
//! 共享根内解析（规范化 + 前缀校验），`..` / 编码绕过都会被拒绝。
//!
//! 移植自 https://github.com/cky-thinker/file-share 插件（Apache-2.0），
//! 后端由 Node/express 改写为 Rust 标准库实现。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Ipv6Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::{json, Value};

use crate::state::AppData;

/// 默认端口，与参考插件保持一致。
pub const DEFAULT_PORT: u16 = 5421;

#[derive(Clone, Debug)]
pub struct ShareItem {
    /// 展示名（默认取文件名）
    pub name: String,
    /// 用户传入的路径，用于回传（插件按它匹配删除），不是 canonicalize 后的根
    pub path: PathBuf,
    /// 规范化后的共享根，所有子路径都必须落在它下面
    pub root: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

pub struct ShareServer {
    port: u16,
    items: Arc<Mutex<Vec<ShareItem>>>,
    stop: Arc<AtomicBool>,
    /// IPv4 + 可选的 IPv6 两个监听线程；没有 IPv6 时只有一条。
    accept: Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// IPv6 监听是否真的建起来了：没建起来就不对外给 IPv6 地址。
    ipv6: bool,
}

impl ShareServer {
    pub fn start(port: u16, items: Vec<ShareItem>) -> Result<Self, String> {
        let listener = TcpListener::bind(("0.0.0.0", port))
            .map_err(|e| format!("端口 {port} 无法监听：{e}"))?;
        let actual = listener
            .local_addr()
            .map_err(|e| format!("读取监听端口失败：{e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("设置非阻塞失败：{e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let items = Arc::new(Mutex::new(items));
        let mut accept = vec![spawn_accept(listener, items.clone(), stop.clone())
            .map_err(|e| format!("启动共享线程失败：{e}"))?];
        // IPv6 是可选入口：Windows 默认 IPV6_V6ONLY=1，可以和 IPv4 监听同一端口；
        // 系统没开 IPv6 时绑定失败，直接跳过，不影响 IPv4。
        let mut ipv6 = false;
        if let Ok(l6) = TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, actual))) {
            let _ = l6.set_nonblocking(true);
            if let Ok(h) = spawn_accept(l6, items.clone(), stop.clone()) {
                accept.push(h);
                ipv6 = true;
            }
        }
        Ok(Self {
            port: actual,
            items,
            stop,
            accept: Mutex::new(accept),
            ipv6,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn set_items(&self, items: Vec<ShareItem>) {
        *self.items.lock() = items;
    }

    /// 停止监听并等 accept 线程退出：否则紧接着用同一端口重启会撞上旧监听。
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        for handle in self.accept.lock().drain(..) {
            let _ = handle.join();
        }
    }

    pub fn snapshot(&self) -> Value {
        let items = self.items.lock();
        json!({
            "running": true,
            "port": self.port,
            "urls": lan_urls(self.port),
            "urls6": if self.ipv6 { lan_urls6(self.port) } else { Vec::new() },
            "items": items.iter().map(|it| json!({
                "name": it.name,
                "path": it.path.to_string_lossy(),
                "is_dir": it.is_dir,
                "size": it.size,
            })).collect::<Vec<_>>(),
            "invalid": [],
        })
    }
}

/// 校验并规范化插件传来的条目；无法使用的路径单独返回，不阻塞其余条目。
pub fn parse_items(raw: &[Value]) -> (Vec<ShareItem>, Vec<Value>) {
    let mut items = Vec::new();
    let mut invalid = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for v in raw {
        let text = v.get("path").and_then(Value::as_str).unwrap_or("").trim();
        if text.is_empty() {
            invalid.push(json!({ "path": "", "reason": "路径为空" }));
            continue;
        }
        let meta = match std::fs::metadata(text) {
            Ok(m) => m,
            Err(e) => {
                invalid.push(json!({ "path": text, "reason": format!("无法读取：{e}") }));
                continue;
            }
        };
        let root = match Path::new(text).canonicalize() {
            Ok(p) => p,
            Err(e) => {
                invalid.push(json!({ "path": text, "reason": format!("路径无效：{e}") }));
                continue;
            }
        };
        if seen.iter().any(|p| p == &root) {
            continue;
        }
        seen.push(root.clone());
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.display().to_string())
            });
        items.push(ShareItem {
            name,
            // 回传用户传入的路径，不暴露 canonicalize 的 \\?\ 前缀：
            // 插件靠它做删除时的匹配，展示也不该看到前缀。
            path: PathBuf::from(text),
            root,
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
    }
    (items, invalid)
}

impl AppData {
    pub fn share_start(&self, opts: &Value) -> Result<Value, String> {
        start_into(&self.share, opts)
    }

    pub fn share_stop(&self) -> Value {
        stop_into(&self.share)
    }

    pub fn share_status(&self) -> Value {
        status_in(&self.share)
    }
}

/// 能力 `share.start` 的实际入口：也直接接受一个空槽位，便于单测。
pub fn start_into(slot: &Mutex<Option<ShareServer>>, opts: &Value) -> Result<Value, String> {
    let port = opts
        .get("port")
        .and_then(Value::as_u64)
        .map(|p| p.min(65535) as u16)
        .unwrap_or(DEFAULT_PORT);
    let raw = opts
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (items, invalid) = parse_items(&raw);

    let mut guard = slot.lock();
    if items.is_empty() {
        if let Some(server) = guard.take() {
            server.stop();
        }
        return Ok(json!({
            "running": false,
            "port": port,
            "urls": [],
            "urls6": [],
            "items": [],
            "invalid": invalid,
        }));
    }
    let need_restart = match guard.as_ref() {
        Some(server) => port != 0 && server.port() != port,
        None => true,
    };
    if need_restart {
        if let Some(server) = guard.take() {
            server.stop();
        }
        *guard = Some(ShareServer::start(port, items)?);
    } else if let Some(server) = guard.as_ref() {
        server.set_items(items);
    }
    let mut out = guard.as_ref().expect("刚刚启动").snapshot();
    out["invalid"] = Value::Array(invalid);
    Ok(out)
}

pub fn stop_into(slot: &Mutex<Option<ShareServer>>) -> Value {
    if let Some(server) = slot.lock().take() {
        server.stop();
    }
    json!({ "running": false, "port": 0, "urls": [], "urls6": [], "items": [], "invalid": [] })
}

pub fn status_in(slot: &Mutex<Option<ShareServer>>) -> Value {
    match slot.lock().as_ref() {
        Some(server) => server.snapshot(),
        None => json!({ "running": false, "port": 0, "urls": [], "urls6": [], "items": [], "invalid": [] }),
    }
}

fn spawn_accept(
    listener: TcpListener,
    items: Arc<Mutex<Vec<ShareItem>>>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("efto-share".to_string())
        .spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => {
                        // Windows 上 accept 出来的流会继承监听 socket 的非阻塞
                        // 模式：不恢复阻塞的话，handle 里的 read 在对方数据还没
                        // 到达时立即返回 WSAEWOULDBLOCK(10035)，连接被掐断。
                        // 本机测试数据瞬时到达发现不了；跨网段客户端有 RTT，
                        // accept→read 的间隙数据必然未到，100% 失败。恢复阻塞后
                        // set_read_timeout(5s) 才真正生效。
                        let _ = stream.set_nonblocking(false);
                        let items = items.clone();
                        std::thread::spawn(move || {
                            if let Err(e) = handle(stream, &items) {
                                tracing::warn!("共享连接处理失败（{peer}）：{e}");
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(80));
                    }
                    Err(e) => {
                        // accept 的瞬时错误不能让监听线程退出：客户端在 accept 前
                        // 就断开（浏览器预连接被放弃、页面请求取消）在 Windows 上
                        // 表现为 WSAECONNRESET。一旦 break，端口还在监听、内核还在
                        // 排队 SYN，但没人 accept —— 已打开的页面看着正常，之后所有
                        // 请求都卡到"网络错误"，而 UI 仍显示"正在分享"。
                        tracing::warn!("共享 accept 失败，继续监听：{e}");
                        std::thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        })
}

fn handle(mut stream: TcpStream, items: &Mutex<Vec<ShareItem>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        if let Some(pos) = find_sub(&buf, b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 32 * 1024 {
            return respond_text(&mut stream, 431, "请求头过大");
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        // 浏览器把 http 升级成 https 时，明文端口上收到的是 TLS 握手
        // （0x16 = handshake，紧跟 0x03 = TLS）。这里没有证书可谈，只能立刻
        // 断开并明确记录；否则会被当成"请求头过大/读超时"，看起来像传输故障。
        if buf.len() >= 3 && buf[0] == 0x16 && buf[1] == 0x03 {
            tracing::warn!("共享端口收到 TLS 握手：对方在用 https 访问，请改用 http://<ip>:<port>/");
            return Ok(());
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");

    let mut range = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("range") {
                range = parse_range(v.trim());
            }
        }
    }

    if method != "GET" && method != "HEAD" {
        return respond_text(&mut stream, 405, "只支持 GET / HEAD");
    }

    let path = target.split('?').next().unwrap_or("/");
    let segs: Vec<String> = match path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| percent_decode_str(s).decode_utf8().ok().map(|c| c.to_string()))
        .collect::<Option<Vec<_>>>()
    {
        Some(v) => v,
        None => return respond_text(&mut stream, 400, "路径编码无效"),
    };

    if segs.is_empty() {
        return serve_index(&mut stream, method, items);
    }
    let idx = match segs.get(1).and_then(|s| s.parse::<usize>().ok()) {
        Some(i) => i,
        None => return respond_text(&mut stream, 404, "没有这个共享项"),
    };
    let item = {
        let items = items.lock();
        match items.get(idx) {
            Some(it) => it.clone(),
            None => return respond_text(&mut stream, 404, "没有这个共享项"),
        }
    };

    match segs[0].as_str() {
        "f" => {
            if item.is_dir {
                return redirect(&mut stream, &format!("/s/{idx}/"));
            }
            serve_file(&mut stream, method, &item.root, &item.name, range)
        }
        "s" => {
            if !item.is_dir {
                return respond_text(&mut stream, 404, "该项不是文件夹");
            }
            let rel = &segs[2..];
            let resolved = match resolve_under(&item.root, rel) {
                Some(p) => p,
                None => return respond_text(&mut stream, 404, "路径无效"),
            };
            let meta = match std::fs::metadata(&resolved) {
                Ok(m) => m,
                Err(_) => return respond_text(&mut stream, 404, "文件不存在"),
            };
            if meta.is_dir() {
                serve_dir(&mut stream, method, idx, rel, &resolved)
            } else {
                let name = resolved
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "download".to_string());
                serve_file(&mut stream, method, &resolved, &name, range)
            }
        }
        _ => respond_text(&mut stream, 404, "没有这个页面"),
    }
}

/// 把相对片段安全地接到共享根上：拒绝 `..`、分隔符、盘符，并用 canonicalize 复核。
fn resolve_under(root: &Path, segs: &[String]) -> Option<PathBuf> {
    let mut p = root.to_path_buf();
    for s in segs {
        if s.is_empty()
            || s == "."
            || s == ".."
            || s.contains('\\')
            || s.contains('/')
            || s.contains(':')
            || s.contains('\0')
        {
            return None;
        }
        p.push(s);
    }
    let canon = p.canonicalize().ok()?;
    canon.starts_with(root).then_some(canon)
}

fn serve_index(stream: &mut TcpStream, method: &str, items: &Mutex<Vec<ShareItem>>) -> std::io::Result<()> {
    let items = items.lock();
    let mut rows = String::new();
    for (i, it) in items.iter().enumerate() {
        let (href, kind) = if it.is_dir {
            (format!("/s/{i}/"), "文件夹".to_string())
        } else {
            (format!("/f/{i}"), human_size(it.size))
        };
        rows.push_str(&format!(
            "<li><a href=\"{href}\">{}</a><span class=\"meta\">{kind}</span></li>",
            escape(&it.name)
        ));
    }
    let body = format!(
        "<p class=\"hint\">共 {} 项 · 只读共享</p><ul class=\"list\">{rows}</ul>",
        items.len()
    );
    respond_html(stream, method, 200, page("FileShare", &body))
}

fn serve_dir(
    stream: &mut TcpStream,
    method: &str,
    idx: usize,
    rel: &[String],
    dir: &Path,
) -> std::io::Result<()> {
    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            match e.metadata() {
                Ok(m) => entries.push((name, m.is_dir(), m.len())),
                Err(_) => continue,
            }
        }
    }
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));

    let mut body = String::new();
    if !rel.is_empty() {
        let mut parent: Vec<String> = rel.to_vec();
        parent.pop();
        body.push_str(&format!(
            "<p><a class=\"up\" href=\"/s/{idx}/{}\">返回上级</a></p>",
            encode_path(&parent)
        ));
    }
    body.push_str("<ul class=\"list\">");
    for (name, is_dir, size) in &entries {
        let mut child: Vec<String> = rel.to_vec();
        child.push(name.clone());
        let href = format!("/s/{idx}/{}", encode_path(&child));
        let kind = if *is_dir { "文件夹".to_string() } else { human_size(*size) };
        body.push_str(&format!(
            "<li><a href=\"{href}\">{}</a><span class=\"meta\">{kind}</span></li>",
            escape(name)
        ));
    }
    body.push_str("</ul>");
    let title = rel.last().cloned().unwrap_or_else(|| "共享目录".to_string());
    respond_html(stream, method, 200, page(&title, &body))
}

fn serve_file(
    stream: &mut TcpStream,
    method: &str,
    path: &Path,
    name: &str,
    range: Option<(u64, Option<u64>)>,
) -> std::io::Result<()> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return respond_text(stream, 404, "文件不存在"),
    };
    let len = file.metadata()?.len();
    let (status, start, end) = match range {
        Some((start, end)) if start < len => {
            let end = end.unwrap_or(len - 1).min(len - 1);
            if end < start {
                return respond_range_error(stream, len);
            }
            (206, start, end)
        }
        Some(_) => return respond_range_error(stream, len),
        None => (200, 0, len.saturating_sub(1)),
    };
    let content_len = if len == 0 { 0 } else { end - start + 1 };
    let mut headers = vec![
        ("Content-Type".to_string(), mime_for(path).to_string()),
        ("Content-Length".to_string(), content_len.to_string()),
        ("Content-Disposition".to_string(), content_disposition(name)),
        ("Accept-Ranges".to_string(), "bytes".to_string()),
    ];
    if status == 206 {
        headers.push(("Content-Range".to_string(), format!("bytes {start}-{end}/{len}")));
    }
    write_head(stream, status, &headers)?;
    if method == "HEAD" || content_len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = content_len;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        stream.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    Ok(())
}

fn parse_range(value: &str) -> Option<(u64, Option<u64>)> {
    let rest = value.strip_prefix("bytes=")?;
    let first = rest.split(',').next()?.trim();
    let (a, b) = first.split_once('-')?;
    let start = a.trim().parse::<u64>().ok()?;
    let end = if b.trim().is_empty() {
        None
    } else {
        Some(b.trim().parse::<u64>().ok()?)
    };
    Some((start, end))
}

fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn respond_html(stream: &mut TcpStream, method: &str, status: u16, body: String) -> std::io::Result<()> {
    let headers = vec![
        ("Content-Type".to_string(), "text/html; charset=utf-8".to_string()),
        ("Content-Length".to_string(), body.len().to_string()),
    ];
    write_head(stream, status, &headers)?;
    if method != "HEAD" {
        stream.write_all(body.as_bytes())?;
    }
    Ok(())
}

fn respond_text(stream: &mut TcpStream, status: u16, text: &str) -> std::io::Result<()> {
    let headers = vec![
        ("Content-Type".to_string(), "text/plain; charset=utf-8".to_string()),
        ("Content-Length".to_string(), text.len().to_string()),
    ];
    write_head(stream, status, &headers)?;
    stream.write_all(text.as_bytes())
}

fn respond_range_error(stream: &mut TcpStream, len: u64) -> std::io::Result<()> {
    let body = "请求范围无效";
    let headers = vec![
        ("Content-Type".to_string(), "text/plain; charset=utf-8".to_string()),
        ("Content-Length".to_string(), body.len().to_string()),
        ("Content-Range".to_string(), format!("bytes */{len}")),
    ];
    write_head(stream, 416, &headers)?;
    stream.write_all(body.as_bytes())
}

fn redirect(stream: &mut TcpStream, location: &str) -> std::io::Result<()> {
    let headers = vec![
        ("Location".to_string(), location.to_string()),
        ("Content-Length".to_string(), "0".to_string()),
    ];
    write_head(stream, 302, &headers)
}

fn write_head(stream: &mut TcpStream, status: u16, headers: &[(String, String)]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        302 => "Found",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        416 => "Range Not Satisfiable",
        431 => "Request Header Fields Too Large",
        _ => "OK",
    };
    let mut out = format!("HTTP/1.1 {status} {reason}\r\n");
    for (k, v) in headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    stream.write_all(out.as_bytes())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn encode_path(segs: &[String]) -> String {
    segs.iter()
        .map(|s| utf8_percent_encode(s, NON_ALPHANUMERIC).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn content_disposition(name: &str) -> String {
    let fallback: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let encoded = utf8_percent_encode(name, NON_ALPHANUMERIC).to_string();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{encoded}",
        fallback.trim()
    )
}

fn mime_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" | "log" | "md" | "csv" => "text/plain; charset=utf-8",
        "json" => "application/json",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn human_size(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn lan_urls(port: u16) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ip) = primary_ipv4() {
        if !ip.is_loopback() {
            out.push(format!("http://{ip}:{port}/"));
        }
    }
    if out.is_empty() {
        out.push(format!("http://127.0.0.1:{port}/"));
    }
    out
}

fn lan_urls6(port: u16) -> Vec<String> {
    match primary_ipv6() {
        Some(ip) => vec![format!("http://[{ip}]:{port}/")],
        None => Vec::new(),
    }
}

fn primary_ipv6() -> Option<Ipv6Addr> {
    let sock = UdpSocket::bind("[::]:0").ok()?;
    sock.connect("[2001:4860:4860::8888]:80").ok()?;
    match sock.local_addr().ok()? {
        SocketAddr::V6(a) => {
            let ip = *a.ip();
            (!ip.is_loopback()).then_some(ip)
        }
        _ => None,
    }
}

fn primary_ipv4() -> Option<std::net::Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()? {
        std::net::SocketAddr::V4(a) => Some(*a.ip()),
        _ => None,
    }
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"zh-CN\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{}</title><style>{STYLE}</style></head><body><main><h1>{}</h1>{body}\
<p class=\"foot\">efto · 局域网只读共享</p></main></body></html>",
        escape(title),
        escape(title)
    )
}

const STYLE: &str = r#"
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body { margin: 0; padding: 36px 18px; font: 15px/1.6 "Segoe UI", "Microsoft YaHei", system-ui, sans-serif; background: #f2f4f7; color: #1d2230; }
main { max-width: 760px; margin: 0 auto; background: #fff; border-radius: 14px; padding: 26px 28px 18px; box-shadow: 0 8px 28px rgba(0, 0, 0, .08); }
h1 { font-size: 20px; margin: 0 0 14px; word-break: break-all; }
.hint { margin: 0 0 10px; color: #6b7484; font-size: 13px; }
.list { list-style: none; margin: 0; padding: 0; }
.list li { display: flex; align-items: center; gap: 12px; padding: 9px 2px; border-bottom: 1px solid #eceef2; }
.list a { color: #2e5ce7; text-decoration: none; word-break: break-all; }
.list a:hover { text-decoration: underline; }
.list .meta { margin-left: auto; flex: 0 0 auto; color: #6b7484; font-size: 12px; }
.up { color: #2e5ce7; text-decoration: none; font-size: 13px; }
.foot { margin: 18px 0 0; color: #99a1b3; font-size: 11.5px; }
@media (prefers-color-scheme: dark) {
  body { background: #15171c; color: #e8ebf2; }
  main { background: #1d2027; box-shadow: none; }
  .list li { border-color: #2a2f39; }
  .list a, .up { color: #6f8dff; }
  .hint, .list .meta { color: #99a1b3; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn start_test_server(items: Vec<ShareItem>) -> (u16, Arc<AtomicBool>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        spawn_accept(listener, Arc::new(Mutex::new(items)), stop.clone()).unwrap();
        (port, stop)
    }

    fn request(port: u16, raw: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(raw.as_bytes()).unwrap();
        s.shutdown(std::net::Shutdown::Write).ok();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        String::from_utf8_lossy(&out).to_string()
    }

    fn temp_file(name: &str, content: &str) -> (tempfile::TempDir, ShareItem) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        let root = path.canonicalize().unwrap();
        let item = ShareItem {
            name: name.to_string(),
            path: root.clone(),
            root,
            is_dir: false,
            size: content.len() as u64,
        };
        (dir, item)
    }

    #[test]
    fn serves_index_and_file_with_range() {
        let (_dir, item) = temp_file("hello.txt", "hello world");
        let (port, stop) = start_test_server(vec![item]);

        let index = request(port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(index.starts_with("HTTP/1.1 200"), "{index}");
        assert!(index.contains("hello.txt"));

        let full = request(port, "GET /f/0 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(full.starts_with("HTTP/1.1 200"), "{full}");
        assert!(full.ends_with("hello world"), "{full}");

        let part = request(port, "GET /f/0 HTTP/1.1\r\nHost: x\r\nRange: bytes=6-10\r\n\r\n");
        assert!(part.starts_with("HTTP/1.1 206"), "{part}");
        assert!(part.contains("Content-Range: bytes 6-10/11"));
        assert!(part.ends_with("world"), "{part}");

        stop.store(true, Ordering::Relaxed);
    }

    /// 复现跨网段客户端的失败：accept 出来的流若继承监听 socket 的非阻塞
    /// 模式，客户端 connect 后稍等片刻才发请求（真实网络总有 RTT），服务端
    /// read 立即返回 WSAEWOULDBLOCK(10035)，连接被掐断。本机测试数据
    /// 瞬时到达，所以必须人为注入延迟才能暴露。
    #[test]
    fn serves_client_that_sends_request_after_a_delay() {
        let (_dir, item) = temp_file("slow.txt", "slow payload");
        let (port, stop) = start_test_server(vec![item]);

        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        s.write_all(b"GET /f/0 HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        s.shutdown(std::net::Shutdown::Write).ok();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let resp = String::from_utf8_lossy(&out).to_string();
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        assert!(resp.ends_with("slow payload"), "{resp}");

        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn rejects_path_escape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("shared")).unwrap();
        std::fs::write(dir.path().join("secret.txt"), "top secret").unwrap();
        let root = dir.path().join("shared").canonicalize().unwrap();
        let item = ShareItem {
            name: "shared".to_string(),
            path: root.clone(),
            root,
            is_dir: true,
            size: 0,
        };
        let (port, stop) = start_test_server(vec![item]);

        let plain = request(port, "GET /s/0/../secret.txt HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(plain.starts_with("HTTP/1.1 404"), "{plain}");

        let encoded = request(port, "GET /s/0/%2e%2e/secret.txt HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(encoded.starts_with("HTTP/1.1 404"), "{encoded}");

        stop.store(true, Ordering::Relaxed);
    }

    #[test]
    fn reports_invalid_items() {
        let raw = vec![
            json!({ "path": "Z:\\definitely-not-here\\x.txt" }),
            json!({ "path": "" }),
        ];
        let (items, invalid) = parse_items(&raw);
        assert!(items.is_empty());
        assert_eq!(invalid.len(), 2);
    }

    #[test]
    fn echoes_input_path_instead_of_canonical_root() {
        let (dir, _) = temp_file("kept.txt", "x");
        let input = dir.path().join("kept.txt");
        let raw = vec![json!({ "path": input.to_string_lossy() })];
        let (items, invalid) = parse_items(&raw);
        assert!(invalid.is_empty(), "{invalid:?}");
        assert_eq!(items.len(), 1);
        // 插件按回传的 path 匹配删除；不能是 canonicalize 出来的 \\?\ 形式
        assert_eq!(items[0].path, input);
        assert_eq!(items[0].root, input.canonicalize().unwrap());
    }

    #[test]
    fn range_parser_accepts_open_ended() {
        assert_eq!(parse_range("bytes=10-"), Some((10, None)));
        assert_eq!(parse_range("bytes=0-99"), Some((0, Some(99))));
        assert_eq!(parse_range("items=0-1"), None);
    }

    #[test]
    fn disposition_keeps_utf8_name() {
        let value = content_disposition("报告 v1.pdf");
        assert!(value.contains("filename*=UTF-8''"));
        assert!(value.contains("%E6%8A%A5%E5%91%8A"));
    }

    #[test]
    fn capability_contract_starts_updates_and_stops() {
        let (_dir, item) = temp_file("payload.txt", "payload");
        let slot = Mutex::new(None);
        let first = start_into(
            &slot,
            &json!({ "port": 0, "items": [{ "path": item.path.to_string_lossy() }] }),
        )
        .unwrap();
        assert_eq!(first["running"], true);
        let port = first["port"].as_u64().unwrap() as u16;

        let body = request(port, "GET /f/0 HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(body.ends_with("payload"), "{body}");

        // 同端口再调用只更新条目，不重启监听
        let second = start_into(
            &slot,
            &json!({ "port": port, "items": [{ "path": item.path.to_string_lossy() }] }),
        )
        .unwrap();
        assert_eq!(second["port"], port);
        assert_eq!(second["running"], true);

        let stopped = stop_into(&slot);
        assert_eq!(stopped["running"], false);
        assert_eq!(status_in(&slot)["running"], false);
    }
}
