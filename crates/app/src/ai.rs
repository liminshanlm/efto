//! 大模型能力：OpenAI 兼容 `POST {base_url}/chat/completions`。
//!
//! 模型配置只来自 设置 → 模型。插件不能传 `base_url` / `api_key`：
//! 那等于把用户的密钥交给插件，也会让"设置页测试"和"插件调用"走两套地址。
//!
//! 消息的 `content` 既可以是字符串，也可以是内容数组（`text` / `image`）。
//! `image` 只接受本机路径，由宿主读盘并转成 OpenAI 的 `image_url` data URL ——
//! 插件仍然拿不到文件字节与密钥。
//!
//! HTTP 复用 [`crate::http::execute`]（GUI 与 CLI 共用），
//! 因此这里没有任何网络细节，只负责拼请求与解释响应。

use efto_core::config::{ModelConfig, Settings};
use serde_json::{json, Value};

/// 默认超时。翻译这类整段文本生成常常超过 30 秒。
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_TIMEOUT_SECS: u64 = 300;
/// 错误信息里最多带多少字符的原始响应，避免把整页 HTML 灌进插件事件。
const MAX_ERROR_CHARS: usize = 300;
/// 单张图片上限。base64 后约放大 4/3，避免一张图把请求体撑到十几 MB。
pub(crate) const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// OCR 单文件上限，与 DeepSeek 文档一致（100 MiB）。
const MAX_OCR_BYTES: u64 = 100 * 1024 * 1024;

/// `efto.ai.models()`：**绝不返回 api_key**，只告诉插件有哪些模型可选。
pub fn models(settings: &Settings) -> Value {
    json!({
        "default": settings.default_model,
        "models": settings
            .models
            .iter()
            .map(|m| json!({
                "id": m.id,
                "name": m.name,
                "model": m.model,
                "base_url": m.base_url,
                "has_api_key": !m.api_key.trim().is_empty(),
            }))
            .collect::<Vec<_>>(),
    })
}

/// `efto.ai.chat({ messages, model?, temperature?, maxTokens?, timeoutSecs? })`。
pub fn chat(settings: &Settings, opts: &Value) -> Result<Value, String> {
    let requested = opts.get("model").and_then(|v| v.as_str());
    let cfg = settings.resolve_model(requested)?;
    let req = build_request(cfg, opts)?;
    let (status, data) = send(cfg, &req)?;
    let mut out = parse_response(status, &data, cfg)?;
    out["modelId"] = json!(cfg.id.trim());
    out["modelName"] = json!(cfg.name.trim());
    Ok(out)
}

/// `efto.ai.chat({ messages, model?, ..., stream: true, onDelta })`。
///
/// 走 SSE（`stream: true`），`emit` 收到 `{ text?: string, reasoning?: string }` 增量；
/// 返回值与非流式一致，额外带完整的 `reasoning`。
pub fn chat_stream(
    settings: &Settings,
    opts: &Value,
    emit: &mut dyn FnMut(Value),
) -> Result<Value, String> {
    let requested = opts.get("model").and_then(|v| v.as_str());
    let cfg = settings.resolve_model(requested)?;
    let req = build_request_with_stream(cfg, opts, true)?;
    let task = request_task(cfg, &req, "text/event-stream");

    let mut state = StreamState::new();
    let (status, error_body) = {
        let mut on_line = |line: &str| {
            for delta in state.push_line(line) {
                emit(delta);
            }
        };
        crate::http::stream(&task, &mut on_line).map_err(map_http_error)?
    };
    for delta in state.flush() {
        emit(delta);
    }
    if let Some(e) = state.error.take() {
        return Err(e);
    }
    if !(200..300).contains(&status) {
        let data =
            serde_json::from_str(&error_body).unwrap_or_else(|_| Value::String(error_body));
        return Err(error_text(status, &data));
    }
    if state.content.trim().is_empty() && state.reasoning.trim().is_empty() {
        return Err("模型返回了空内容".into());
    }

    Ok(json!({
        "content": state.content,
        "reasoning": state.reasoning,
        "model": state.model.unwrap_or_else(|| cfg.model.trim().to_string()),
        "modelId": cfg.id.trim(),
        "modelName": cfg.name.trim(),
        "finishReason": state.finish_reason,
        "usage": state.usage,
        "serverToolCalls": state.tool_calls,
        "annotations": state.annotations,
    }))
}

/// 设置页“测试”：用表单里尚未保存的配置发一次最小请求。
/// 只要 HTTP 2xx 就算连通（不要求内容非空，推理模型在 max_tokens 很小时可能返回空）。
pub fn probe(cfg: &ModelConfig) -> Result<Value, String> {
    let opts = json!({
        "messages": [{ "role": "user", "content": "ping" }],
        "maxTokens": 8,
        "temperature": 0.0,
        "timeoutSecs": 20,
    });
    let req = build_request(cfg, &opts)?;
    let started = std::time::Instant::now();
    let (status, data) = send(cfg, &req)?;
    if !(200..300).contains(&status) {
        return Err(error_text(status, &data));
    }
    let reply = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok(json!({
        "ok": true,
        "latency_ms": started.elapsed().as_millis() as u64,
        "model": cfg.model.trim(),
        "reply": reply,
    }))
}

/// `efto.ai.ocr({ path, purpose?, model? })`：调用 DeepSeek 的 OCR 接口，
/// 把本机文件（PDF / Word / PowerPoint / 图片）识别成逐页文字。
///
/// 文件不会进入对话上下文，也不会被存到远端的 Files API。
pub fn ocr(settings: &Settings, opts: &Value) -> Result<Value, String> {
    let requested = opts.get("model").and_then(|v| v.as_str());
    let cfg = settings.resolve_model(requested)?;
    let path = opts
        .get("path")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "ai.ocr 需要 path（本机文件路径）".to_string())?;
    let purpose = opts
        .get("purpose")
        .and_then(|v| v.as_str())
        .unwrap_or("text_recognition");
    if !matches!(purpose, "text_recognition" | "rich_layout_recognition") {
        return Err(format!(
            "ai.ocr 的 purpose 只能是 text_recognition / rich_layout_recognition，收到 {purpose:?}"
        ));
    }

    let meta = std::fs::metadata(path).map_err(|e| format!("读取文件失败（{path}）：{e}"))?;
    if !meta.is_file() {
        return Err(format!("不是文件：{path}"));
    }
    if meta.len() == 0 {
        return Err(format!("文件是空的：{path}"));
    }
    if meta.len() > MAX_OCR_BYTES {
        return Err(format!(
            "文件过大（{} 字节，上限 {MAX_OCR_BYTES}）",
            meta.len()
        ));
    }
    let file_name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| format!("无法从路径取出文件名：{path}"))?;
    let bytes = std::fs::read(path).map_err(|e| format!("读取文件失败（{path}）：{e}"))?;

    let url = ocr_endpoint(&cfg.base_url);
    let mut headers: Vec<(String, String)> = Vec::new();
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        headers.push(("authorization".into(), format!("Bearer {key}")));
    }
    let (status, body) = crate::http::post_multipart(
        &url,
        &headers,
        &[("purpose", purpose)],
        "file",
        &file_name,
        &bytes,
        180,
    )
    .map_err(|e| e.replace("HTTP", "OCR"))?;

    // 识别开始后状态码就是 200；失败信息在响应体的 error 字段里。
    // 解析失败时优先报状态码：网关报错常常是 HTML，直接说"解析失败"会掩盖真正原因。
    let data: Value = match serde_json::from_str(body.trim()) {
        Ok(v) => v,
        Err(_) if !(200..300).contains(&status) => {
            return Err(format!("OCR 请求失败（HTTP {status}）：{}", shorten(&body)));
        }
        Err(_) => return Err(format!("OCR 响应解析失败：{}", shorten(&body))),
    };
    if data.get("error").is_some() {
        return Err(format!("OCR 识别失败：{}", api_error_message(&data)));
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "OCR 请求失败（HTTP {status}）：{}",
            api_error_message(&data)
        ));
    }
    let pages = data
        .get("pages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if pages.is_empty() {
        return Err("OCR 没有返回识别结果".into());
    }
    Ok(json!({
        "id": data.get("id").cloned().unwrap_or(Value::Null),
        "purpose": purpose,
        "pageCount": data.get("page_count").and_then(|v| v.as_u64()).unwrap_or(pages.len() as u64),
        "fileName": file_name,
        "pages": pages
            .iter()
            .map(|p| json!({
                "index": p.get("index").cloned().unwrap_or(Value::Null),
                "text": p.get("text").and_then(|v| v.as_str()).unwrap_or(""),
                "finishReason": p.get("finish_reason").cloned().unwrap_or(Value::Null),
            }))
            .collect::<Vec<_>>(),
    }))
}

/// `https://api.deepseek.com` / `https://api.deepseek.com/v1`
/// -> `https://api.deepseek.com/v1/tools/ocr`
fn ocr_endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    if base.ends_with("/v1") {
        format!("{base}/tools/ocr")
    } else {
        format!("{base}/v1/tools/ocr")
    }
}

fn api_error_message(data: &Value) -> String {
    if let Some(msg) = data.pointer("/error/message").and_then(|v| v.as_str()) {
        return msg.to_string();
    }
    if let Some(msg) = data.get("error").and_then(|v| v.as_str()) {
        return msg.to_string();
    }
    shorten(&data.to_string())
}

/// 拼好的请求，拆出来是为了单测不碰网络。
pub struct ChatRequest {
    pub url: String,
    pub body: Value,
    pub timeout_secs: u64,
}

pub fn build_request(cfg: &ModelConfig, opts: &Value) -> Result<ChatRequest, String> {
    build_request_with_stream(cfg, opts, false)
}

fn build_request_with_stream(
    cfg: &ModelConfig,
    opts: &Value,
    stream: bool,
) -> Result<ChatRequest, String> {
    let messages = parse_messages(opts)?;
    let model = cfg.model.trim();
    if model.is_empty() {
        return Err("模型配置缺少模型名（例如 gpt-4o-mini）".into());
    }
    let mut body = json!({ "model": model, "messages": messages, "stream": stream });

    if let Some(v) = opts.get("temperature") {
        let t = v
            .as_f64()
            .ok_or_else(|| "ai.chat 的 temperature 必须是数字".to_string())?;
        if !(0.0..=2.0).contains(&t) {
            return Err("ai.chat 的 temperature 必须在 0 ~ 2 之间".into());
        }
        body["temperature"] = json!(t);
    }
    if let Some(v) = opts.get("maxTokens") {
        let n = v
            .as_u64()
            .ok_or_else(|| "ai.chat 的 maxTokens 必须是正整数".to_string())?;
        if n == 0 || n > 32_768 {
            return Err("ai.chat 的 maxTokens 必须在 1 ~ 32768 之间".into());
        }
        body["max_tokens"] = json!(n);
    }
    // DeepSeek 的服务端联网工具：搜索与抓取都在服务端完成，客户端不执行任何工具。
    // 其他网关不认识这两个类型，由服务端返回错误，这里不做额外判断。
    let mut tools = Vec::new();
    if opts.get("webSearch").and_then(Value::as_bool).unwrap_or(false) {
        tools.push(json!({ "type": "deepseek_web_search" }));
    }
    if opts.get("readUrl").and_then(Value::as_bool).unwrap_or(false) {
        tools.push(json!({ "type": "deepseek_read_url" }));
    }
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }

    let timeout_secs = opts
        .get("timeoutSecs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(5, MAX_TIMEOUT_SECS);

    Ok(ChatRequest {
        url: endpoint(&cfg.base_url),
        body,
        timeout_secs,
    })
}

fn parse_messages(opts: &Value) -> Result<Vec<Value>, String> {
    let arr = opts
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "ai.chat 需要 messages 数组".to_string())?;
    if arr.is_empty() {
        return Err("ai.chat 的 messages 不能为空".into());
    }
    let mut out = Vec::with_capacity(arr.len());
    for (i, m) in arr.iter().enumerate() {
        let role = m
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("messages[{i}].role 必须是字符串"))?;
        if !matches!(role, "system" | "user" | "assistant") {
            return Err(format!(
                "messages[{i}].role 只能是 system / user / assistant，收到 {role:?}"
            ));
        }
        let content = m
            .get("content")
            .ok_or_else(|| format!("messages[{i}].content 不能为空"))?;
        out.push(json!({ "role": role, "content": parse_content(content, i)? }));
    }
    Ok(out)
}

/// `content` 可以是字符串，也可以是内容数组：
/// `{ "type": "text", "text": "..." }` / `{ "type": "image", "path": "D:\\a.png" }`。
/// 只接受这两种类型，其它一律明确报错。
fn parse_content(value: &Value, mi: usize) -> Result<Value, String> {
    match value {
        Value::String(s) => {
            if s.trim().is_empty() {
                return Err(format!("messages[{mi}].content 不能为空"));
            }
            Ok(json!(s))
        }
        Value::Array(parts) => {
            if parts.is_empty() {
                return Err(format!("messages[{mi}].content 不能为空"));
            }
            let mut out = Vec::with_capacity(parts.len());
            for (pi, part) in parts.iter().enumerate() {
                let ty = part
                    .get("type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("messages[{mi}].content[{pi}].type 必须是字符串"))?;
                match ty {
                    "text" => {
                        let text = part
                            .get("text")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| {
                                format!("messages[{mi}].content[{pi}].text 必须是字符串")
                            })?;
                        if text.trim().is_empty() {
                            return Err(format!("messages[{mi}].content[{pi}].text 不能为空"));
                        }
                        out.push(json!({ "type": "text", "text": text }));
                    }
                    "image" => {
                        let path = part
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .ok_or_else(|| {
                                format!("messages[{mi}].content[{pi}].path 必须是非空字符串")
                            })?;
                        let url = image_data_url(path)
                            .map_err(|e| format!("messages[{mi}].content[{pi}]: {e}"))?;
                        out.push(json!({
                            "type": "image_url",
                            "image_url": { "url": url },
                        }));
                    }
                    other => {
                        return Err(format!(
                            "messages[{mi}].content[{pi}] 不支持的类型 {other:?}（只支持 text / image）"
                        ))
                    }
                }
            }
            Ok(json!(out))
        }
        _ => Err(format!("messages[{mi}].content 必须是字符串或内容数组")),
    }
}

/// 图片扩展名 → MIME。预览等其它宿主能力也复用它。
pub(crate) fn image_mime(path: &str) -> Result<&'static str, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "webp" => Ok("image/webp"),
        "gif" => Ok("image/gif"),
        "bmp" => Ok("image/bmp"),
        _ => Err(format!(
            "不支持的图片格式 {ext:?}：只支持 png / jpg / jpeg / webp / gif / bmp"
        )),
    }
}

/// 字节 → `data:<mime>;base64,...`。
pub(crate) fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// 读本机图片并编码成 `data:<mime>;base64,...`。
/// 只认常见图片扩展名；文件不存在、太大、为空都给出可读原因。
fn image_data_url(path: &str) -> Result<String, String> {
    let mime = image_mime(path)?;
    let meta = std::fs::metadata(path).map_err(|e| format!("读取图片失败（{path}）：{e}"))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "图片过大（{} 字节，上限 {MAX_IMAGE_BYTES}）",
            meta.len()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("读取图片失败（{path}）：{e}"))?;
    if bytes.is_empty() {
        return Err(format!("图片是空文件：{path}"));
    }
    Ok(data_url(mime, &bytes))
}

/// `https://api.openai.com/v1` -> `https://api.openai.com/v1/chat/completions`；
/// 已经写成完整地址的原样使用。
fn endpoint(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn send(cfg: &ModelConfig, req: &ChatRequest) -> Result<(u16, Value), String> {
    let task = request_task(cfg, req, "application/json");
    let resp = crate::http::execute(&task).map_err(map_http_error)?;
    let status = resp.get("statusCode").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let mut data = resp.get("data").cloned().unwrap_or(Value::Null);
    // 部分网关把 JSON 标成 text/plain；能解析就解析，解析不了再按原文报错
    if let Value::String(s) = &data {
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            data = v;
        }
    }
    Ok((status, data))
}

fn request_task(cfg: &ModelConfig, req: &ChatRequest, accept: &str) -> Value {
    let mut headers = serde_json::Map::new();
    headers.insert("content-type".into(), json!("application/json"));
    headers.insert("accept".into(), json!(accept));
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        headers.insert("authorization".into(), json!(format!("Bearer {key}")));
    }
    json!({
        "type": "post",
        "url": req.url,
        "headers": headers,
        "body": req.body,
        "timeout": req.timeout_secs,
    })
}

/// http 层文案换成插件/用户能看懂的"模型"说法。
fn map_http_error(e: String) -> String {
    for (from, to) in [
        ("HTTP 请求失败: ", "模型请求失败: "),
        ("HTTP 读取响应体失败: ", "模型响应读取失败: "),
        ("HTTP: ", "模型: "),
    ] {
        if let Some(rest) = e.strip_prefix(from) {
            return format!("{to}{rest}");
        }
    }
    e
}

/// SSE 增量累积器。拆成独立结构便于单测，不碰网络。
#[derive(Default)]
struct StreamState {
    content: String,
    reasoning: String,
    data_buf: String,
    finish_reason: Value,
    usage: Value,
    model: Option<String>,
    /// DeepSeek 联网工具：完整的调用记录与引用只在最后一个 chunk 里发一次。
    tool_calls: Value,
    annotations: Value,
    done: bool,
    error: Option<String>,
}

impl StreamState {
    fn new() -> Self {
        Self::default()
    }

    /// 吃一行 SSE；空白行表示一个事件结束。返回需要发给插件的增量。
    fn push_line(&mut self, line: &str) -> Vec<Value> {
        if self.done || self.error.is_some() {
            return Vec::new();
        }
        if line.is_empty() {
            if self.data_buf.is_empty() {
                return Vec::new();
            }
            let data = std::mem::take(&mut self.data_buf);
            return self.process_data(&data);
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let chunk = rest.strip_prefix(' ').unwrap_or(rest);
            if !self.data_buf.is_empty() {
                self.data_buf.push('\n');
            }
            self.data_buf.push_str(chunk);
        } else if self.data_buf.is_empty() && line.trim_start().starts_with('{') {
            // 网关不支持流式时可能直接回一段 JSON（常见于错误体），也当成一个事件处理
            let data = line.to_string();
            return self.process_data(&data);
        }
        // `event:` / `id:` / `:` 注释行都忽略
        Vec::new()
    }

    /// 流结束时可能没有末尾空行，补处理一次残留。
    fn flush(&mut self) -> Vec<Value> {
        if self.data_buf.is_empty() {
            return Vec::new();
        }
        let data = std::mem::take(&mut self.data_buf);
        self.process_data(&data)
    }

    fn process_data(&mut self, data: &str) -> Vec<Value> {
        let data = data.trim();
        if data.is_empty() {
            return Vec::new();
        }
        if data == "[DONE]" {
            self.done = true;
            return Vec::new();
        }
        let value: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                self.error = Some(format!("模型流式响应解析失败: {e}"));
                return Vec::new();
            }
        };
        if value.get("error").is_some() {
            let msg = value
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| shorten(&value.to_string()));
            self.error = Some(format!("模型流式响应错误：{msg}"));
            return Vec::new();
        }
        if let Some(m) = value.get("model").and_then(|v| v.as_str()) {
            self.model = Some(m.to_string());
        }
        if let Some(u) = value.get("usage").filter(|v| !v.is_null()) {
            self.usage = u.clone();
        }
        if let Some(fr) = value.pointer("/choices/0/finish_reason").filter(|v| !v.is_null()) {
            self.finish_reason = fr.clone();
        }
        let mut out = Vec::new();
        // 搜索/抓取的进度事件挂在 choice 上，与 delta 并列；转给 UI 显示"正在搜索"。
        if let Some(ev) = value
            .pointer("/choices/0/server_tool_event")
            .filter(|v| !v.is_null())
        {
            out.push(json!({ "toolEvent": ev.clone() }));
        }
        let Some(delta) = value.pointer("/choices/0/delta") else {
            return out;
        };
        if let Some(calls) = delta.get("server_tool_calls").filter(|v| !v.is_null()) {
            self.tool_calls = calls.clone();
        }
        if let Some(ann) = delta.get("annotations").filter(|v| !v.is_null()) {
            self.annotations = ann.clone();
        }
        if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
            if !r.is_empty() {
                self.reasoning.push_str(r);
                out.push(json!({ "reasoning": r }));
            }
        }
        if let Some(t) = delta.get("content").and_then(|v| v.as_str()) {
            if !t.is_empty() {
                self.content.push_str(t);
                out.push(json!({ "text": t }));
            }
        }
        out
    }
}

pub fn parse_response(status: u16, data: &Value, cfg: &ModelConfig) -> Result<Value, String> {
    if !(200..300).contains(&status) {
        return Err(error_text(status, data));
    }
    let content = data
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!(
                "模型响应缺少 choices[0].message.content：{}",
                shorten(&data.to_string())
            )
        })?;
    if content.trim().is_empty() {
        return Err("模型返回了空内容".into());
    }
    Ok(json!({
        "content": content,
        "reasoning": data
            .pointer("/choices/0/message/reasoning_content")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "model": cfg.model.trim(),
        "finishReason": data.pointer("/choices/0/finish_reason").cloned().unwrap_or(Value::Null),
        "usage": data.get("usage").cloned().unwrap_or(Value::Null),
        "serverToolCalls": data
            .pointer("/choices/0/message/server_tool_calls")
            .cloned()
            .unwrap_or(Value::Null),
        "annotations": data
            .pointer("/choices/0/message/annotations")
            .cloned()
            .unwrap_or(Value::Null),
    }))
}

/// 各家网关的错误体不统一：优先取 `error.message`，拿不到就截断原文。
fn error_text(status: u16, data: &Value) -> String {
    let msg = data
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("message").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| shorten(&data.to_string()));
    format!("模型请求失败（HTTP {status}）：{msg}")
}

fn shorten(s: &str) -> String {
    if s.chars().count() <= MAX_ERROR_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_ERROR_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ModelConfig {
        ModelConfig {
            id: "m1".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com/v1/".into(),
            api_key: "sk-test".into(),
            model: "deepseek-chat".into(),
        }
    }

    #[test]
    fn endpoint_joins_base_url_once() {
        assert_eq!(
            endpoint("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://x/chat/completions"),
            "https://x/chat/completions"
        );
    }

    #[test]
    fn build_request_carries_messages_and_options() {
        let opts = json!({
            "messages": [{ "role": "user", "content": "hello" }],
            "temperature": 0.2,
            "maxTokens": 64,
        });
        let req = build_request(&cfg(), &opts).unwrap();
        assert_eq!(req.url, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(req.body["model"], json!("deepseek-chat"));
        assert_eq!(req.body["messages"][0]["content"], json!("hello"));
        assert_eq!(req.body["temperature"], json!(0.2));
        assert_eq!(req.body["max_tokens"], json!(64));
        assert_eq!(req.timeout_secs, DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn build_request_rejects_bad_input() {
        for opts in [
            json!({}),
            json!({ "messages": [] }),
            json!({ "messages": [{ "role": "tool", "content": "x" }] }),
            json!({ "messages": [{ "role": "user", "content": "  " }] }),
            json!({ "messages": [{ "role": "user", "content": "x" }], "temperature": 5 }),
            json!({ "messages": [{ "role": "user", "content": "x" }], "maxTokens": 0 }),
        ] {
            assert!(build_request(&cfg(), &opts).is_err(), "{opts}");
        }
    }

    #[test]
    fn build_request_supports_image_content_parts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G']).unwrap();
        let opts = json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "识别这张图" },
                    { "type": "image", "path": path.to_string_lossy() }
                ]
            }]
        });
        let req = build_request(&cfg(), &opts).unwrap();
        let parts = req.body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], json!({ "type": "text", "text": "识别这张图" }));
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
    }

    #[test]
    fn build_request_rejects_bad_content_parts() {
        for content in [
            json!([{ "type": "video", "path": "x.mp4" }]),
            json!([{ "type": "text" }]),
            json!([{ "type": "text", "text": "  " }]),
            json!([{ "type": "image" }]),
            json!([]),
            json!(123),
        ] {
            let opts = json!({ "messages": [{ "role": "user", "content": content }] });
            assert!(build_request(&cfg(), &opts).is_err(), "{content}");
        }

        // 不存在 / 扩展名不支持 / 空文件都要有可读原因
        for path in ["C:\\no\\such\\file.png", "C:\\a\\file.txt"] {
            let opts = json!({
                "messages": [{ "role": "user", "content": [{ "type": "image", "path": path }] }]
            });
            let e = build_request(&cfg(), &opts).err().expect("应拒绝");
            assert!(e.contains("图片"), "{e}");
        }
    }

    #[test]
    fn build_request_can_enable_stream() {
        let opts = json!({ "messages": [{ "role": "user", "content": "hi" }] });
        let req = build_request_with_stream(&cfg(), &opts, true).unwrap();
        assert_eq!(req.body["stream"], json!(true));
        assert_eq!(build_request(&cfg(), &opts).unwrap().body["stream"], json!(false));
    }

    #[test]
    fn build_request_declares_deepseek_web_tools() {
        let opts = json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "webSearch": true,
            "readUrl": true,
        });
        let req = build_request(&cfg(), &opts).unwrap();
        assert_eq!(
            req.body["tools"],
            json!([
                { "type": "deepseek_web_search" },
                { "type": "deepseek_read_url" }
            ])
        );

        let plain = build_request(
            &cfg(),
            &json!({ "messages": [{ "role": "user", "content": "hi" }] }),
        )
        .unwrap();
        assert!(plain.body.get("tools").is_none());
    }

    #[test]
    fn stream_state_collects_tool_events_and_citations() {
        let mut s = StreamState::new();
        let mut deltas = Vec::new();
        for line in [
            r#"data: {"choices":[{"delta":{},"server_tool_event":{"type":"call_started","step":0,"call":{"action":{"type":"search","queries":["q"]}}}}]}"#,
            "",
            r#"data: {"choices":[{"delta":{"server_tool_calls":[{"index":0,"calls":[{"sources":[{"url":"https://a","title":"A"}]}]}],"annotations":[{"type":"url_citation","url_citation":{"url":"https://a","title":"A"}}]},"finish_reason":"stop"}]}"#,
            "",
            "data: [DONE]",
            "",
        ] {
            deltas.extend(s.push_line(line));
        }
        assert_eq!(
            deltas[0],
            json!({ "toolEvent": {
                "type": "call_started",
                "step": 0,
                "call": { "action": { "type": "search", "queries": ["q"] } }
            }})
        );
        assert_eq!(s.tool_calls[0]["calls"][0]["sources"][0]["url"], json!("https://a"));
        assert_eq!(s.annotations[0]["url_citation"]["title"], json!("A"));
    }

    #[test]
    fn ocr_endpoint_targets_v1_tools() {
        assert_eq!(
            ocr_endpoint("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/tools/ocr"
        );
        assert_eq!(
            ocr_endpoint("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/tools/ocr"
        );
        assert_eq!(
            ocr_endpoint("https://gateway.example.com/v1/chat/completions"),
            "https://gateway.example.com/v1/tools/ocr"
        );
    }

    #[test]
    fn stream_state_accumulates_text_and_reasoning() {
        let mut s = StreamState::new();
        let mut deltas = Vec::new();
        for line in [
            r#"data: {"model":"deepseek-chat","choices":[{"delta":{"reasoning_content":"想"}}]}"#,
            "",
            r#"data: {"choices":[{"delta":{"content":"你"}}]}"#,
            "",
            r#"data: {"choices":[{"delta":{"content":"好"},"finish_reason":"stop"}],"usage":{"total_tokens":3}}"#,
            "",
            "data: [DONE]",
            "",
        ] {
            deltas.extend(s.push_line(line));
        }
        assert_eq!(s.content, "你好");
        assert_eq!(s.reasoning, "想");
        assert_eq!(s.finish_reason, json!("stop"));
        assert_eq!(s.usage["total_tokens"], json!(3));
        assert_eq!(s.model.as_deref(), Some("deepseek-chat"));
        assert_eq!(
            deltas,
            vec![json!({ "reasoning": "想" }), json!({ "text": "你" }), json!({ "text": "好" })]
        );
    }

    #[test]
    fn stream_state_flushes_and_reports_errors() {
        let mut s = StreamState::new();
        s.push_line(r#"data: {"choices":[{"delta":{"content":"半"}}]}"#);
        assert_eq!(s.flush(), vec![json!({ "text": "半" })]);

        let mut bad = StreamState::new();
        let deltas = bad.push_line(r#"data: {"error":{"message":"invalid key"}}"#);
        assert!(deltas.is_empty());
        bad.push_line("");
        assert!(bad.error.unwrap().contains("invalid key"));
    }

    #[test]
    fn chat_stream_reads_sse_from_http() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // 必须读完整请求（头 + Content-Length 指定的 body）再回包：
            // 服务端带着未读数据关连接会触发 RST，客户端读响应体会拿到
            // EINVAL（macOS 实测 3 次挂 1 次，收干净后归零）。
            let mut buf = [0u8; 8192];
            let mut req = Vec::new();
            let header_end;
            loop {
                match sock.read(&mut buf) {
                    Ok(0) => return,
                    Ok(n) => {
                        req.extend_from_slice(&buf[..n]);
                        if let Some(pos) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = pos + 4;
                            break;
                        }
                    }
                    Err(_) => return,
                }
            }
            let headers = String::from_utf8_lossy(&req[..header_end]).to_ascii_lowercase();
            let content_length: usize = headers
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            while req.len() < header_end + content_length {
                match sock.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                    Err(_) => return,
                }
            }
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"你\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n"
            );
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });

        let mut settings = Settings::default();
        settings.models = vec![ModelConfig {
            id: "m1".into(),
            name: "Local".into(),
            base_url: format!("http://{addr}"),
            api_key: String::new(),
            model: "fake".into(),
        }];
        settings.default_model = Some("m1".into());

        let opts = json!({ "messages": [{ "role": "user", "content": "hi" }] });
        let mut deltas = Vec::new();
        let result = chat_stream(&settings, &opts, &mut |d| deltas.push(d)).unwrap();
        server.join().unwrap();

        assert_eq!(result["content"], json!("你好"));
        assert_eq!(result["finishReason"], json!("stop"));
        assert_eq!(deltas, vec![json!({ "text": "你" }), json!({ "text": "好" })]);
    }

    #[test]
    fn parse_response_extracts_content() {
        let data = json!({
            "choices": [{ "message": { "role": "assistant", "content": "你好" }, "finish_reason": "stop" }],
            "usage": { "total_tokens": 12 }
        });
        let out = parse_response(200, &data, &cfg()).unwrap();
        assert_eq!(out["content"], json!("你好"));
        assert_eq!(out["finishReason"], json!("stop"));
        assert_eq!(out["usage"]["total_tokens"], json!(12));
    }

    #[test]
    fn parse_response_reports_http_and_shape_errors() {
        let e = parse_response(401, &json!({ "error": { "message": "invalid key" } }), &cfg())
            .unwrap_err();
        assert!(e.contains("HTTP 401"), "{e}");
        assert!(e.contains("invalid key"), "{e}");

        let e2 = parse_response(200, &json!({ "choices": [] }), &cfg()).unwrap_err();
        assert!(e2.contains("choices[0].message.content"), "{e2}");

        let e3 = parse_response(
            200,
            &json!({ "choices": [{ "message": { "content": "   " } }] }),
            &cfg(),
        )
        .unwrap_err();
        assert!(e3.contains("空内容"), "{e3}");
    }

    #[test]
    fn chat_without_models_fails_loudly() {
        let e = chat(&Settings::default(), &json!({ "messages": [{ "role": "user", "content": "hi" }] }))
            .unwrap_err();
        assert!(e.contains("未配置模型"), "{e}");
    }

    #[test]
    fn models_never_expose_api_key() {
        let mut s = Settings::default();
        s.models = vec![cfg()];
        s.normalize_models();
        let v = models(&s);
        assert_eq!(v["default"], json!("m1"));
        assert_eq!(v["models"][0]["has_api_key"], json!(true));
        assert!(!v.to_string().contains("sk-test"), "{v}");
    }
}
