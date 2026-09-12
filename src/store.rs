//! Application state and its JSON persistence.
//!
//! Everything lives in a single file, written whole. The store is small by
//! design: a list of OpenAI-compatible providers and a list of sessions.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    System,
}

impl Role {
    /// The role name used on the wire.
    pub fn wire(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Msg {
    pub id: String,
    pub role: Role,
    pub content: String,
    /// Model reasoning / thinking trace, when the API emits one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Set when the response failed; rendered as the message's failure state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Msg {
    pub fn new(role: Role, content: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.into(),
            thinking: None,
            error: None,
        }
    }
}

/// Which request/response shape a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ApiFormat {
    /// `POST /chat/completions` — OpenAI's original API, and what almost every
    /// compatible vendor (DeepSeek, Moonshot, Ollama, vLLM, one-api) speaks.
    #[default]
    #[serde(rename = "openai-completions")]
    OpenAiCompletions,
    /// `POST /responses` — OpenAI's newer Responses API.
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    /// `POST /messages` — Anthropic's Messages API (Claude).
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
}

impl ApiFormat {
    pub const ALL: [ApiFormat; 3] = [
        ApiFormat::OpenAiCompletions,
        ApiFormat::OpenAiResponses,
        ApiFormat::AnthropicMessages,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ApiFormat::OpenAiCompletions => "OpenAI Chat Completions",
            ApiFormat::OpenAiResponses => "OpenAI Responses",
            ApiFormat::AnthropicMessages => "Anthropic Messages",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            ApiFormat::OpenAiCompletions => "POST /chat/completions，绝大多数兼容接口都用这个",
            ApiFormat::OpenAiResponses => "POST /responses，OpenAI 新接口",
            ApiFormat::AnthropicMessages => "POST /messages，Claude 接口（x-api-key）",
        }
    }
}

/// One model the provider exposes, plus optional limits from `/models`.
///
/// Older configs stored a bare id string; both shapes deserialize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelInfo {
    pub id: String,
    /// Completion budget for this model when the catalog reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Context window in tokens when the catalog reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

impl ModelInfo {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            max_tokens: None,
            context_window: None,
        }
    }

    pub fn with_limits(
        id: impl Into<String>,
        max_tokens: Option<u32>,
        context_window: Option<u32>,
    ) -> Self {
        Self {
            id: id.into(),
            max_tokens,
            context_window,
        }
    }
}

impl From<&str> for ModelInfo {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for ModelInfo {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl<'de> Deserialize<'de> for ModelInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;

        struct ModelVisitor;

        impl<'de> Visitor<'de> for ModelVisitor {
            type Value = ModelInfo;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a model id string or an object with an id field")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ModelInfo::new(value))
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut id = None;
                let mut max_tokens = None;
                let mut context_window = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "id" => id = Some(map.next_value::<String>()?),
                        "max_tokens" | "max_output_tokens" | "max_completion_tokens" => {
                            max_tokens = map.next_value::<Option<u32>>()?;
                        }
                        "context_window" | "context_length" => {
                            context_window = map.next_value::<Option<u32>>()?;
                        }
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                let id = id.ok_or_else(|| de::Error::missing_field("id"))?;
                Ok(ModelInfo {
                    id,
                    max_tokens,
                    context_window,
                })
            }
        }

        deserializer.deserialize_any(ModelVisitor)
    }
}

/// One OpenAI-compatible endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Base URL without a trailing slash, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// Models the user kept for this provider.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
    /// Request/response shape; older configs default to Chat Completions.
    #[serde(default)]
    pub api: ApiFormat,
    /// Fallback completion budget when a model does not report one.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_max_tokens() -> u32 {
    8192
}

impl Provider {
    pub fn new(name: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            base_url: base_url.into(),
            api_key: String::new(),
            models: Vec::new(),
            api: ApiFormat::default(),
            max_tokens: default_max_tokens(),
        }
    }

    /// The key as the HTTP layer should send it: stored form is encrypted.
    pub fn api_key_plain(&self) -> String {
        decode_api_key(&self.api_key)
    }

    pub fn set_api_key_plain(&mut self, key: impl Into<String>) {
        self.api_key = encode_api_key(&key.into());
    }

    pub fn has_model(&self, id: &str) -> bool {
        self.models.iter().any(|model| model.id == id)
    }

    /// Budget for a chat turn: the model's own limit, else the provider fallback.
    pub fn max_tokens_for(&self, model_id: Option<&str>) -> u32 {
        model_id
            .and_then(|id| self.models.iter().find(|model| model.id == id))
            .and_then(|model| model.max_tokens)
            .unwrap_or(self.max_tokens)
    }
}

/// At-rest encoding for API keys.
///
/// Threat model: a casually shared or backed-up `config.json` should not
/// contain readable keys. This is XOR against a per-install key file — not
/// multi-user OS isolation. The install key sits beside the config.
const API_KEY_PREFIX: &str = "enc:v1:";

fn config_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("KiiChat"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn install_key_path() -> PathBuf {
    config_dir().join("install.key")
}

fn load_or_create_install_key() -> [u8; 32] {
    let path = install_key_path();
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() == 32
    {
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return key;
    }
    let mut key = [0u8; 32];
    // uuid v4 carries 16 random bytes; two of them fill the key. Good enough
    // for file obfuscation without pulling a CSPRNG crate.
    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();
    key[..16].copy_from_slice(a.as_bytes());
    key[16..].copy_from_slice(b.as_bytes());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, key);
    key
}

fn xor_bytes(data: &[u8], key: &[u8; 32]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, byte)| byte ^ key[i % key.len()])
        .collect()
}

fn encode_api_key(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    encode_api_key_with(plain, &load_or_create_install_key())
}

fn decode_api_key(stored: &str) -> String {
    decode_api_key_with(stored, &load_or_create_install_key())
}

fn encode_api_key_with(plain: &str, key: &[u8; 32]) -> String {
    let mixed = xor_bytes(plain.as_bytes(), key);
    format!("{API_KEY_PREFIX}{}", base64_encode(&mixed))
}

fn decode_api_key_with(stored: &str, key: &[u8; 32]) -> String {
    let Some(rest) = stored.strip_prefix(API_KEY_PREFIX) else {
        // Plaintext from an older config; migrate on the next save.
        return stored.to_string();
    };
    let Ok(mixed) = base64_decode(rest) else {
        return String::new();
    };
    String::from_utf8(xor_bytes(&mixed, key)).unwrap_or_default()
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64_decode(text: &str) -> Result<Vec<u8>, ()> {
    fn val(c: u8) -> Result<u8, ()> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(()),
        }
    }
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut n = [0u8; 4];
        let mut pad = 0;
        for (i, byte) in chunk.iter().enumerate() {
            if *byte == b'=' {
                pad += 1;
                n[i] = 0;
            } else {
                n[i] = val(*byte)?;
            }
        }
        let word = ((n[0] as u32) << 18) | ((n[1] as u32) << 12) | ((n[2] as u32) << 6) | n[3] as u32;
        out.push((word >> 16) as u8);
        if pad < 2 {
            out.push((word >> 8) as u8);
        }
        if pad < 1 {
            out.push(word as u8);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    /// Provider bound to this conversation; `None` means "use the selected one".
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub messages: Vec<Msg>,
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: "新对话".into(),
            provider_id: None,
            model: None,
            messages: Vec::new(),
        }
    }

    /// Messages sent to the API: system and completed turns only. An assistant
    /// message that is still streaming (or failed) has no content to send back.
    pub fn request_messages(&self) -> Vec<(Role, String)> {
        self.messages
            .iter()
            .filter(|msg| !msg.content.is_empty() && msg.error.is_none())
            .map(|msg| (msg.role, msg.content.clone()))
            .collect()
    }
}

/// The color scheme the window uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
}

impl Theme {
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }

    pub fn toggled(self) -> Self {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }
}

/// How outbound requests reach the network.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum Proxy {
    /// Environment variables and the system proxy configuration (reqwest's default).
    #[default]
    System,
    /// Direct connections, ignoring any system or environment proxy.
    None,
    /// One explicit proxy for every request.
    Custom { url: String },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    pub providers: Vec<Provider>,
    #[serde(default)]
    pub sessions: Vec<Session>,
    /// Provider a new conversation starts with.
    #[serde(default)]
    pub selected_provider: Option<String>,
    /// Color scheme, persisted across runs.
    #[serde(default)]
    pub theme: Theme,
    /// Network path for every request.
    #[serde(default)]
    pub proxy: Proxy,
    /// Whether the conversation sidebar is folded away.
    #[serde(default)]
    pub sidebar_collapsed: bool,
}

impl Store {
    pub fn path() -> PathBuf {
        directories::BaseDirs::new()
            .map(|dirs| dirs.config_dir().join("KiiChat"))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("config.json")
    }

    /// Loads the store, reporting a config the app could not read.
    ///
    /// An unparseable file is moved aside rather than ignored: starting empty
    /// and then saving would otherwise destroy every provider and session.
    pub fn load() -> (Self, Option<String>) {
        let path = Self::path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (Self::default(), None);
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(mut store) => {
                store.migrate_api_keys();
                (store, None)
            }
            Err(err) => {
                let backup = path.with_extension("json.invalid");
                let kept = std::fs::rename(&path, &backup);
                eprintln!("KiiChat: {} cannot be read ({err})", path.display());
                let warning = match kept {
                    Ok(()) => format!(
                        "配置文件无法解析（{err}），已备份为 {} 并以空配置启动。",
                        backup.display()
                    ),
                    Err(_) => format!("配置文件无法解析（{err}），已以空配置启动。"),
                };
                (Self::default(), Some(warning))
            }
        }
    }

    /// Rewrites plaintext keys into the encoded form; next `save` persists them.
    fn migrate_api_keys(&mut self) {
        for provider in &mut self.providers {
            if provider.api_key.is_empty() || provider.api_key.starts_with(API_KEY_PREFIX) {
                continue;
            }
            let plain = provider.api_key.clone();
            provider.set_api_key_plain(plain);
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        // Write beside the target, then replace it, so a crash mid-write cannot
        // truncate the previous store.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    pub fn provider_mut(&mut self, id: &str) -> Option<&mut Provider> {
        self.providers.iter_mut().find(|provider| provider.id == id)
    }

    pub fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    pub fn session_mut(&mut self, id: &str) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    /// The provider a conversation should use: its own binding, else the
    /// selected one, else the first configured provider.
    pub fn provider_for_session(&self, session_id: &str) -> Option<&Provider> {
        let session = self.session(session_id)?;
        let id = session
            .provider_id
            .as_deref()
            .or(self.selected_provider.as_deref())?;
        self.provider(id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn provider(api: ApiFormat) -> Provider {
        let mut provider = Provider::new("Mock", "http://127.0.0.1:18080/v1");
        provider.api = api;
        provider.models = vec![ModelInfo::new("mock-mini")];
        provider
    }

    /// The API format names are part of the config file's contract: a spelling
    /// the app writes but cannot read resets a provider to Chat Completions.
    #[test]
    fn api_formats_round_trip() {
        for api in ApiFormat::ALL {
            let store = Store {
                providers: vec![provider(api)],
                ..Store::default()
            };
            let text = serde_json::to_string(&store).expect("serialize");
            let parsed: Store = serde_json::from_str(&text).expect("parse");
            assert_eq!(parsed.providers[0].api, api, "round trip: {text}");
        }
    }

    /// A config written by an earlier release has no `api` field at all.
    #[test]
    fn config_without_api_defaults_to_chat_completions() {
        let text = r#"{
            "providers": [{
                "id": "p", "name": "Mock", "base_url": "http://127.0.0.1:18080/v1",
                "api_key": "k", "models": ["mock-mini"]
            }],
            "sessions": [],
            "selected_provider": "p"
        }"#;
        let store: Store = serde_json::from_str(text).expect("parse");
        assert_eq!(store.providers[0].api, ApiFormat::OpenAiCompletions);
        assert_eq!(store.theme, Theme::Light);
        assert_eq!(store.proxy, Proxy::System);
    }

    /// The names on the wire are the ones the settings buttons promise.
    #[test]
    fn api_format_names_are_stable() {
        assert_eq!(
            serde_json::to_string(&ApiFormat::OpenAiCompletions).unwrap(),
            "\"openai-completions\""
        );
        assert_eq!(
            serde_json::to_string(&ApiFormat::OpenAiResponses).unwrap(),
            "\"openai-responses\""
        );
        assert_eq!(
            serde_json::to_string(&ApiFormat::AnthropicMessages).unwrap(),
            "\"anthropic-messages\""
        );
    }

    #[test]
    fn api_key_round_trips_through_encoding() {
        let key = [7u8; 32];
        let plain = "sk-test-中文-and-ascii";
        let encoded = encode_api_key_with(plain, &key);
        assert!(encoded.starts_with(API_KEY_PREFIX));
        assert!(!encoded.contains(plain));
        assert_eq!(decode_api_key_with(&encoded, &key), plain);
    }

    #[test]
    fn plaintext_api_key_still_reads() {
        assert_eq!(decode_api_key_with("sk-legacy", &[1u8; 32]), "sk-legacy");
    }

    #[test]
    fn provider_defaults_max_tokens() {
        let text = r#"{
            "id": "p", "name": "Mock", "base_url": "http://127.0.0.1:18080/v1",
            "api_key": "k", "models": ["mock-mini"]
        }"#;
        let provider: Provider = serde_json::from_str(text).expect("parse");
        assert_eq!(provider.max_tokens, 8192);
    }

    #[test]
    fn thinking_field_is_optional() {
        let text = r#"{
            "id": "m", "role": "assistant", "content": "hi"
        }"#;
        let msg: Msg = serde_json::from_str(text).expect("parse");
        assert!(msg.thinking.is_none());
    }

    #[test]
    fn model_list_accepts_plain_ids_and_objects() {
        let text = r#"{
            "id": "p", "name": "Mock", "base_url": "http://127.0.0.1:18080/v1",
            "api_key": "k",
            "models": [
                "legacy-id",
                { "id": "rich", "max_tokens": 4096, "context_window": 128000 }
            ]
        }"#;
        let provider: Provider = serde_json::from_str(text).expect("parse");
        assert_eq!(provider.models[0].id, "legacy-id");
        assert_eq!(provider.models[0].max_tokens, None);
        assert_eq!(provider.models[1].max_tokens, Some(4096));
        assert_eq!(provider.models[1].context_window, Some(128000));
        assert_eq!(provider.max_tokens_for(Some("rich")), 4096);
        assert_eq!(provider.max_tokens_for(Some("legacy-id")), 8192);
    }
}
