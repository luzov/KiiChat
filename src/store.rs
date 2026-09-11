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

/// One OpenAI-compatible endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// Base URL without a trailing slash, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    /// Fetched from the provider's `/models` endpoint.
    #[serde(default)]
    pub models: Vec<String>,
    /// Request/response shape; older configs default to Chat Completions.
    #[serde(default)]
    pub api: ApiFormat,
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
        }
    }
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
            Ok(store) => (store, None),
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