//! OpenAI-compatible HTTP: model discovery and streaming chat completions.
//!
//! Requests run on a dedicated thread that owns a current-thread tokio
//! runtime, because GPUI runs on smol while reqwest needs a tokio reactor.
//! Results cross back over an async channel, so the UI never blocks.

use futures_lite::StreamExt as _;
use std::error::Error as StdError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::store::{ApiFormat, Proxy, Role};

/// Events emitted while a chat completion streams.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// A content delta, in arrival order.
    Delta(String),
    /// A reasoning / thinking delta, in arrival order.
    Thinking(String),
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

/// The conversation in Chat Completions shape.
fn chat_messages(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|(role, content)| {
            serde_json::json!({
                "role": role.wire(),
                "content": content,
            })
        })
        .collect()
}

/// The conversation in Responses shape: an item list with typed messages.
fn responses_input(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|(role, _)| *role != Role::System)
        .map(|(role, content)| {
            let kind = if *role == Role::Assistant {
                "output_text"
            } else {
                "input_text"
            };
            serde_json::json!({
                "role": role.wire(),
                "content": [{ "type": kind, "text": content }],
            })
        })
        .collect()
}

/// The conversation in Messages shape: system turns are lifted out (they are a
/// top-level field), the rest keep their role and content.
fn anthropic_messages(messages: &[(Role, String)]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .filter(|(role, _)| *role != Role::System)
        .map(|(role, content)| {
            serde_json::json!({
                "role": role.wire(),
                "content": content,
            })
        })
        .collect()
}

fn system_prompt(messages: &[(Role, String)]) -> String {
    messages
        .iter()
        .filter(|(role, _)| *role == Role::System)
        .map(|(_, content)| content.clone())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The text a stream delta carries, per API shape.
fn delta_text(api: ApiFormat, value: &serde_json::Value) -> Option<String> {
    let text = match api {
        ApiFormat::OpenAiCompletions => value
            .pointer("/choices/0/delta/content")
            .and_then(|text| text.as_str()),
        ApiFormat::OpenAiResponses => match value.get("type").and_then(|kind| kind.as_str()) {
            // OpenAI emits one of these per chunk, plus a final `...done`.
            Some("response.output_text.delta") => {
                value.get("delta").and_then(|text| text.as_str())
            }
            _ => None,
        },
        ApiFormat::AnthropicMessages => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("content_block_delta") => value.pointer("/delta/text").and_then(|text| text.as_str()),
            _ => None,
        },
    }?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Reasoning / thinking text a stream delta carries, per API shape.
///
/// DeepSeek and GLM use `reasoning_content` on the Chat Completions delta;
/// Anthropic streams `thinking` inside a `content_block_delta`.
fn thinking_text(api: ApiFormat, value: &serde_json::Value) -> Option<String> {
    let text = match api {
        ApiFormat::OpenAiCompletions => value
            .pointer("/choices/0/delta/reasoning_content")
            .or_else(|| value.pointer("/choices/0/delta/reasoning"))
            .and_then(|text| text.as_str()),
        ApiFormat::OpenAiResponses => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("response.reasoning_summary_text.delta")
            | Some("response.reasoning_text.delta") => {
                value.get("delta").and_then(|text| text.as_str())
            }
            _ => None,
        },
        ApiFormat::AnthropicMessages => match value.get("type").and_then(|kind| kind.as_str()) {
            Some("content_block_delta") => {
                let is_thinking = value
                    .pointer("/delta/type")
                    .and_then(|kind| kind.as_str())
                    == Some("thinking_delta");
                is_thinking
                    .then(|| {
                        value
                            .pointer("/delta/thinking")
                            .and_then(|text| text.as_str())
                    })
                    .flatten()
            }
            _ => None,
        },
    }?;
    (!text.is_empty()).then(|| text.to_string())
}

/// Whether an event closes the stream (beyond the `[DONE]` sentinel).
fn is_final_event(api: ApiFormat, value: &serde_json::Value) -> bool {
    let kind = value.get("type").and_then(|kind| kind.as_str());
    match api {
        ApiFormat::OpenAiCompletions => false,
        ApiFormat::OpenAiResponses => matches!(kind, Some("response.completed") | Some("response.failed")),
        ApiFormat::AnthropicMessages => matches!(kind, Some("message_stop")),
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

fn client(proxy: &Proxy, timeout: Option<Duration>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
    builder = match proxy {
        Proxy::System => builder,
        Proxy::None => builder.no_proxy(),
        Proxy::Custom { url } => {
            let url = url.trim();
            if url.is_empty() {
                return Err("自定义代理地址为空".into());
            }
            let proxy = reqwest::Proxy::all(url)
                .map_err(|err| format!("代理地址无效（{url}）: {err}"))?;
            builder.proxy(proxy)
        }
    };
    if let Some(timeout) = timeout {
        builder = builder.timeout(timeout);
    }
    builder.build().map_err(|err| format!("无法创建 HTTP 客户端: {err}"))
}

/// Flattens a request error into one line, source chain included.
///
/// reqwest reports "error sending request" for every transport failure, which
/// tells the user nothing; the cause (DNS, TLS, proxy, timeout) is one or more
/// `source()` hops down.
fn describe(err: &reqwest::Error) -> String {
    let mut text = err.to_string();
    let mut cause = StdError::source(err);
    while let Some(error) = cause {
        text.push_str(&format!(" → {error}"));
        cause = error.source();
    }
    text
}

fn truncate(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!("{head}…")
}

/// Applies a provider's authentication scheme to a request.
fn authorize(
    request: reqwest::RequestBuilder,
    api: ApiFormat,
    api_key: &str,
) -> reqwest::RequestBuilder {
    if api_key.is_empty() {
        return request;
    }
    match api {
        // Anthropic authenticates with a key header and an API version, and
        // rejects requests that carry a bearer token instead.
        ApiFormat::AnthropicMessages => request
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION),
        _ => request.bearer_auth(api_key),
    }
}

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Fetches the provider's model list from `{base_url}/models`.
///
/// The receiver yields exactly one result and then closes.
pub fn fetch_models(
    base_url: String,
    api_key: String,
    api: ApiFormat,
    proxy: Proxy,
) -> async_channel::Receiver<Result<Vec<String>, String>> {
    spawn_runtime(async move {
        let client = client(&proxy, Some(Duration::from_secs(30)))?;
        let url = format!("{}/models", normalize_base(&base_url));
        let request = authorize(client.get(&url), api, &api_key);
        let response = request
            .send()
            .await
            .map_err(|err| format!("请求 {url} 失败: {}", describe(&err)))?;
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
#[allow(clippy::too_many_arguments)]
pub fn stream_chat(
    base_url: String,
    api_key: String,
    api: ApiFormat,
    model: String,
    messages: Vec<(Role, String)>,
    max_tokens: u32,
    proxy: Proxy,
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

            let client = match client(&proxy, None) {
                Ok(client) => client,
                Err(err) => {
                    send(StreamEvent::Error(err)).await;
                    return;
                }
            };
            let base = normalize_base(&base_url);
            let (url, body) = match api {
                ApiFormat::OpenAiCompletions => (
                    format!("{base}/chat/completions"),
                    serde_json::json!({
                        "model": model,
                        "stream": true,
                        "messages": chat_messages(&messages),
                    }),
                ),
                ApiFormat::OpenAiResponses => (
                    format!("{base}/responses"),
                    serde_json::json!({
                        "model": model,
                        "stream": true,
                        "input": responses_input(&messages),
                    }),
                ),
                ApiFormat::AnthropicMessages => (
                    format!("{base}/messages"),
                    serde_json::json!({
                        "model": model,
                        "stream": true,
                        // The Messages API has no `system` role in `messages`
                        // and requires a token budget.
                        "max_tokens": max_tokens,
                        "system": system_prompt(&messages),
                        "messages": anthropic_messages(&messages),
                    }),
                ),
            };
            let request = authorize(client.post(&url).json(&body), api, &api_key);
            let response = match request.send().await {
                Ok(response) => response,
                Err(err) => {
                    send(StreamEvent::Error(format!(
                        "请求 {url} 失败: {}",
                        describe(&err)
                    )))
                    .await;
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
                        send(StreamEvent::Error(format!("连接中断: {}", describe(&err)))).await;
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
                        // Errors are shaped the same way in all three APIs: a
                        // message nested under `error`.
                        if let Some(message) = value
                            .pointer("/error/message")
                            .or_else(|| value.pointer("/error/type"))
                            .and_then(|message| message.as_str())
                        {
                            send(StreamEvent::Error(message.to_string())).await;
                            return;
                        }
                        if let Some(content) = delta_text(api, &value) {
                            send(StreamEvent::Delta(content)).await;
                        }
                        if let Some(thinking) = thinking_text(api, &value) {
                            send(StreamEvent::Thinking(thinking)).await;
                        }
                        if is_final_event(api, &value) {
                            send(StreamEvent::Done).await;
                            return;
                        }
                    }
                }
            }
            send(StreamEvent::Done).await;
        });
    });
    rx
}