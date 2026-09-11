//! OpenAI-compatible HTTP: model discovery and streaming chat completions.
//!
//! Requests run on a dedicated thread that owns a current-thread tokio
//! runtime, because GPUI runs on smol while reqwest needs a tokio reactor.
//! Results cross back over an async channel, so the UI never blocks.

use futures_lite::StreamExt as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::store::Role;

/// Events emitted while a chat completion streams.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A content delta, in arrival order.
    Delta(String),
    /// The stream ended normally.
    Done,
    /// The stream ended with a user-visible error.
    Error(String),
}

/// Normalizes a user-entered base URL.
///
/// Trailing slashes are dropped, and a bare host gets `/v1` appended, which is
/// what every OpenAI-compatible vendor expects.
pub fn normalize_base(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let after_scheme = trimmed.split_once("://").map_or(trimmed.as_str(), |(_, rest)| rest);
    if !after_scheme.contains('/') {
        format!("{trimmed}/v1")
    } else {
        trimmed
    }
}

/// Runs a fallible request on a worker thread with its own tokio runtime.
fn spawn_runtime<T: Send + 'static>(
    future: impl Future<Output = Result<T, String>> + Send + 'static,
) -> async_channel::Receiver<Result<T, String>> {
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime.block_on(future),
            Err(err) => Err(format!("无法启动网络运行时: {err}")),
        };
        let _ = tx.send_blocking(result);
    });
    rx
}

fn client(timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(|err| format!("无法创建 HTTP 客户端: {err}"))
}

fn truncate(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

/// Fetches the provider's model list from `{base_url}/models`.
///
/// The receiver yields exactly one result and then closes.
pub fn fetch_models(
    base_url: String,
    api_key: String,
) -> async_channel::Receiver<Result<Vec<String>, String>> {
    spawn_runtime(async move {
        let client = client(Some(Duration::from_secs(30)))?;
        let url = format!("{}/models", normalize_base(&base_url));
        let mut request = client.get(&url);
        if !api_key.is_empty() {
            request = request.bearer_auth(&api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|err| format!("请求 {url} 失败: {err}"))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| format!("读取响应失败: {err}"))?;
        if !status.is_success() {
            return Err(format!("HTTP {status}: {}", truncate(&body, 300)));
        }
        let value: serde_json::Value = serde_json::from_str(&body)
            .map_err(|err| format!("响应不是合法 JSON: {err}"))?;
        let mut models = Vec::new();
        if let Some(items) = value.get("data").and_then(|data| data.as_array()) {
            for item in items {
                let id = item
                    .as_str()
                    .or_else(|| item.get("id").and_then(|id| id.as_str()));
                if let Some(id) = id {
                    models.push(id.to_string());
                }
            }
        }
        models.sort();
        models.dedup();
        if models.is_empty() {
            Err(format!("接口没有返回模型: {}", truncate(&body, 200)))
        } else {
            Ok(models)
        }
    })
}

/// Starts a streaming chat completion. Set `cancel` to stop early.
pub fn stream_chat(
    base_url: String,
    api_key: String,
    model: String,
    messages: Vec<(Role, String)>,
    cancel: Arc<AtomicBool>,
) -> async_channel::Receiver<StreamEvent> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                let _ = tx.send_blocking(StreamEvent::Error(format!(
                    "无法启动网络运行时: {err}"
                )));
                return;
            }
        };
        runtime.block_on(async move {
            let send = |event: StreamEvent| {
                let tx = tx.clone();
                async move {
                    let _ = tx.send(event).await;
                }
            };

            let client = match client(None) {
                Ok(client) => client,
                Err(err) => {
                    send(StreamEvent::Error(err)).await;
                    return;
                }
            };
            let url = format!("{}/chat/completions", normalize_base(&base_url));
            let body = serde_json::json!({
                "model": model,
                "stream": true,
                "messages": messages
                    .iter()
                    .map(|(role, content)| serde_json::json!({
                        "role": role.wire(),
                        "content": content,
                    }))
                    .collect::<Vec<_>>(),
            });
            let mut request = client.post(&url).json(&body);
            if !api_key.is_empty() {
                request = request.bearer_auth(&api_key);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(err) => {
                    send(StreamEvent::Error(format!("请求 {url} 失败: {err}"))).await;
                    return;
                }
            };
            let status = response.status();
            if !status.is_success() {
                let detail = response.text().await.unwrap_or_default();
                send(StreamEvent::Error(format!(
                    "HTTP {status}: {}",
                    truncate(&detail, 300)
                )))
                .await;
                return;
            }

            let mut stream = Box::pin(response.bytes_stream());
            let mut buffer = String::new();
            while let Some(chunk) = stream.next().await {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        send(StreamEvent::Error(format!("连接中断: {err}"))).await;
                        return;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));

                while let Some(index) = buffer.find("\n\n") {
                    let event: String = buffer.drain(..index + 2).collect();
                    for line in event.lines() {
                        let Some(data) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let data = data.trim();
                        if data.is_empty() {
                            continue;
                        }
                        if data == "[DONE]" {
                            send(StreamEvent::Done).await;
                            return;
                        }
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
                            continue;
                        };
                        if let Some(message) = value
                            .pointer("/error/message")
                            .and_then(|message| message.as_str())
                        {
                            send(StreamEvent::Error(message.to_string())).await;
                            return;
                        }
                        if let Some(content) = value
                            .pointer("/choices/0/delta/content")
                            .and_then(|text| text.as_str())
                        {
                            if !content.is_empty() {
                                send(StreamEvent::Delta(content.to_string())).await;
                            }
                        }
                    }
                }
            }
            send(StreamEvent::Done).await;
        });
    });
    rx
}