//! HTTP 能力实现（ureq 2.12 + Windows schannel）。
//!
//! 同步阻塞请求，跑在 jsrt 的能力线程池里：既不在插件线程（会卡住 isolate），
//! 也不在主线程（会卡住 UI）。返回 `{ statusCode, headers, data, requestId }`。
//!
//! 第一阶段不做请求取消，不代理系统 PAC 配置，响应体上限 8MB。

use std::time::Duration;

use serde_json::{Map, Value, json};
use ureq::AgentBuilder;

const MAX_BODY: usize = 8 * 1024 * 1024;
/// OCR 的结果是整篇文字，8MB 不够；但也不该无上限地吃内存。
const MAX_UPLOAD_RESPONSE: usize = 32 * 1024 * 1024;
const DEFAULT_TIMEOUT: u64 = 30;
const USER_AGENT: &str = concat!("efto/", env!("CARGO_PKG_VERSION"));

/// task：`{ id, type|method, url, headers, body|data, timeout }`。
pub fn execute(task: &Value) -> Result<Value, String> {
    let (status, resp) = open(task)?;

    let mut headers = Map::new();
    for name in resp.headers_names() {
        if let Some(value) = resp.header(&name) {
            headers.insert(name, Value::String(value.to_string()));
        }
    }
    let content_type = resp.content_type().to_string();

    let text = resp
        .into_string()
        .map_err(|e| format!("HTTP 读取响应体失败: {e}"))?;
    let text = if text.len() > MAX_BODY {
        tracing::warn!("HTTP 响应体超过 {MAX_BODY} 字节，已截断");
        let mut t: String = text.chars().take(MAX_BODY).collect();
        t.push_str("…[truncated]");
        t
    } else {
        text
    };

    let data = if content_type.contains("json") {
        serde_json::from_str(&text).unwrap_or(Value::String(text.clone()))
    } else {
        Value::String(text.clone())
    };

    Ok(json!({
        "statusCode": status,
        "headers": headers,
        "contentType": content_type,
        "data": data,
        "text": text,
        "requestId": task.get("id").cloned().unwrap_or(Value::Null),
    }))
}

/// 流式请求（SSE）：2xx 时逐行读取响应体并交给 `on_line`；非 2xx 时读完整 body。
///
/// 返回 `(status, error_body)`；状态码 2xx 时 `error_body` 为空。
/// 目前只给 `efto.ai.chat` 的流式分支使用。
pub fn stream(task: &Value, on_line: &mut dyn FnMut(&str)) -> Result<(u16, String), String> {
    let (status, resp) = open(task)?;
    if !(200..300).contains(&status) {
        let body = resp.into_string().unwrap_or_default();
        return Ok((status, body));
    }

    use std::io::{BufRead, BufReader};
    let mut reader = BufReader::new(resp.into_reader());
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => on_line(line.trim_end_matches(|c| c == '\r' || c == '\n')),
            Err(e) => return Err(format!("HTTP 读取响应体失败: {e}")),
        }
    }
    Ok((status, String::new()))
}

/// 文件上传（multipart/form-data），目前只给 `efto.ai.ocr` 使用。
///
/// 不走 `execute` 是因为它的 body 只支持 JSON / 字符串，二进制文件放不进去。
/// 返回 `(status, body)`；非 2xx 也把响应体带回来，交给调用方解释。
pub fn post_multipart(
    url: &str,
    headers: &[(String, String)],
    fields: &[(&str, &str)],
    file_field: &str,
    file_name: &str,
    file: &[u8],
    timeout_secs: u64,
) -> Result<(u16, String), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("HTTP: url 必须以 http:// 或 https:// 开头".into());
    }
    let timeout = timeout_secs.clamp(1, 300);
    let boundary = format!(
        "----efto{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let mut body: Vec<u8> = Vec::with_capacity(file.len() + 512);
    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }
    // 文件名可能带中文：同时给 ASCII 兜底和 RFC 5987 的 UTF-8 形式。
    let ascii_name: String = file_name
        .chars()
        .map(|c| if c.is_ascii() && c != '"' && c != '\\' { c } else { '_' })
        .collect();
    let encoded_name = percent_encode(file_name);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{ascii_name}\"; filename*=UTF-8''{encoded_name}\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(file);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let agent = agent(timeout)?;
    let mut req = agent
        .request("POST", url)
        .set("content-type", &format!("multipart/form-data; boundary={boundary}"));
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let outcome = req.send_bytes(&body);
    let resp = match outcome {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            return Ok((code, text));
        }
        Err(e) => return Err(format!("HTTP 请求失败: {e}")),
    };
    let status = resp.status();
    let text = resp
        .into_string()
        .map_err(|e| format!("HTTP 读取响应体失败: {e}"))?;
    let text = if text.len() > MAX_UPLOAD_RESPONSE {
        let mut cut = MAX_UPLOAD_RESPONSE;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text[..cut].to_string()
    } else {
        text
    };
    Ok((status, text))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// ureq 2 的 native-tls feature 只是"适配器"，不会自动成为默认 TLS 后端：
/// 只有显式传 tls_connector 才会走系统证书库，否则 https 直接报
/// "cannot make HTTPS request because no TLS backend is configured"。
/// Windows 下 native-tls 用 schannel，能复用系统根证书与企业代理证书。
fn agent(timeout: u64) -> Result<ureq::Agent, String> {
    let tls = ureq::native_tls::TlsConnector::new()
        .map_err(|e| format!("HTTP: 初始化 TLS 失败: {e}"))?;
    Ok(AgentBuilder::new()
        .tls_connector(std::sync::Arc::new(tls))
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(timeout))
        .timeout_write(Duration::from_secs(timeout))
        .redirects(10)
        .user_agent(USER_AGENT)
        .build())
}

/// 校验参数、建立连接并发送请求，返回状态码与响应。
fn open(task: &Value) -> Result<(u16, ureq::Response), String> {
    let url = task
        .get("url")
        .and_then(|v| v.as_str())
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .ok_or_else(|| "HTTP: url 必须以 http:// 或 https:// 开头".to_string())?;

    let method = task
        .get("type")
        .and_then(|v| v.as_str())
        .or_else(|| task.get("method").and_then(|v| v.as_str()))
        .unwrap_or("get")
        .to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err(format!("HTTP: 不支持的方法 {method}"));
    }

    let timeout = task
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT)
        .clamp(1, 300);

    let agent = agent(timeout)?;
    let mut req = agent.request(&method, url);
    if let Some(headers) = task.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            let value = v
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| v.to_string());
            req = req.set(k, &value);
        }
    }

    let body = task.get("body").or_else(|| task.get("data"));
    let outcome = match body {
        None => req.call(),
        Some(v @ (Value::Object(_) | Value::Array(_))) => req.send_json(v.clone()),
        Some(Value::String(s)) => req.send_string(s),
        Some(v) => req.send_json(v.clone()),
    };

    // 非 2xx 也当成"有响应"：调用方自己判断 statusCode
    match outcome {
        Ok(r) => Ok((r.status(), r)),
        Err(ureq::Error::Status(code, r)) => Ok((code, r)),
        Err(e) => Err(format!("HTTP 请求失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_url_and_method() {
        let e = execute(&json!({ "url": "ftp://x/y" })).unwrap_err();
        assert!(e.contains("http://"), "{e}");
        let e2 = execute(&json!({ "url": "https://x/y", "type": "TRACE" })).unwrap_err();
        assert!(e2.contains("不支持的方法"), "{e2}");
    }

    #[test]
    fn missing_url_is_error() {
        assert!(execute(&json!({})).is_err());
    }

    /// 真实网络请求：只验证不会 panic、且能拿到结构化结果（离线环境允许失败）。
    #[test]
    fn execute_against_real_endpoint_or_fails_gracefully() {
        let task = json!({
            "id": "t1",
            "type": "get",
            "url": "https://index.crates.io/ur/eq/ureq",
            "timeout": 15,
        });
        match execute(&task) {
            Ok(v) => {
                assert!(v["statusCode"].is_number(), "{v}");
                assert!(v["headers"].is_object(), "{v}");
                assert_eq!(v["requestId"], json!("t1"));
            }
            Err(e) => {
                // 网络不可达不是实现缺陷，但要能给出可读原因
                assert!(e.contains("HTTP"), "{e}");
            }
        }
    }
}
