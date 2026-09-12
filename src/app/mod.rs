//! The KiiChat window: a session sidebar, a conversation pane, and the
//! provider settings page.
//!
//! The view owns all durable state — providers, sessions, messages — and hands
//! `gpui-ai` snapshots to render. A completion runs on a worker thread; the UI
//! task only appends deltas and re-snapshots.
//!
//! Layout: `mod.rs` holds state and wiring; `chat.rs` the transcript shell;
//! `settings.rs` the settings pages. Palette lives in [`crate::theme`].

mod chat;
mod settings;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Entity,
    FollowMode, Hsla, IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    WindowControlArea, Pixels, Styled, Subscription, Window, div, list, prelude::*, px, relative,
};
use gpui_ai::orbs::Orbs;
use gpui_ai::prompt_bar::{PromptBar, PromptBarEvent, PromptModel};
use gpui_ai::stream::{ProgressState, StreamedContent};
use gpui_ai::streaming_text::StreamingText;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::checkbox::Checkbox;
use gpui_component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::text::TextView;

use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, h_flex, v_flex,
};

use crate::api::{self, StreamEvent};
use crate::icons;
use crate::store::{ApiFormat, Msg, Provider, Proxy, Role, Session, Store, Theme};
use crate::theme::{apply_theme, theme_mode};

const SIDEBAR_WIDTH: f32 = 232.;
/// Matches gpui-component's title bar height, so the window controls line up.
const TITLE_BAR_HEIGHT: Pixels = px(34.);
const DEFAULT_TITLE: &str = "新对话";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Chat,
    Settings,
}

/// The settings sub-page shown while [`Page::Settings`] is active.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Appearance,
    Network,
    Models,
}

impl Section {
    const ALL: [(Section, &'static str, IconName); 3] = [
        (Section::Appearance, "外观", IconName::Palette),
        (Section::Network, "网络", IconName::Globe),
        (Section::Models, "模型", IconName::Cpu),
    ];

    fn title(self) -> &'static str {
        match self {
            Section::Appearance => "外观",
            Section::Network => "网络",
            Section::Models => "模型",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Section::Appearance => "选择浅色或深色主题，设置会立即生效并保存。",
            Section::Network => "选择请求接口时使用的网络通道。",
            Section::Models => "添加任意 OpenAI 兼容的接口，点「获取模型」一键拉取模型列表。",
        }
    }
}

/// A user message being edited in place.
struct Editing {
    message_id: String,
    editor: Entity<TextareaState>,
}

/// One in-flight completion.
struct Stream {
    session_id: String,
    message_id: String,
    cancel: Arc<AtomicBool>,
}

/// The provider currently open in the settings editor.
#[derive(Default)]
struct Editor {
    /// `None` while composing a new provider.
    id: Option<String>,
    /// The models saved for this provider.
    models: Vec<String>,
    /// Everything the provider reported, while the user picks from it.
    fetched: Vec<String>,
    /// The models of `fetched` the user has ticked.
    picked: HashSet<String>,
    /// Request/response shape this provider speaks.
    api: ApiFormat,
    status: SharedString,
    fetching: bool,
}

pub struct KiiChat {
    store: Store,
    page: Page,
    section: Section,
    prompt: Entity<PromptBar>,
    /// Transcript list state: variable-height rows, tail-following.
    transcript: ListState,
    current: Option<String>,
    stream: Option<Stream>,
    editing: Option<Editing>,
    editor: Editor,
    notice: Option<SharedString>,
    name_input: Entity<InputState>,
    base_input: Entity<InputState>,
    key_input: Entity<InputState>,
    proxy_input: Entity<InputState>,
    /// Search boxes: the settings model list and the composer's picker.
    models_search: Entity<InputState>,
    picker_search: Entity<InputState>,
    /// Whether the composer's model picker is open.
    picker_open: bool,
    /// Message ids whose reasoning strip is expanded.
    thinking_open: HashSet<String>,
    max_tokens_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl KiiChat {

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prompt = cx.new(|cx| PromptBar::new("kiichat-prompt", window, cx));
        let transcript = ListState::new(0, ListAlignment::Bottom, px(256.));
        transcript.set_follow_mode(FollowMode::Tail);

        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("名称，例如 OpenAI"));
        let base_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Base URL，例如 https://api.openai.com/v1")
        });
        let key_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("API Key").masked(true));
        let proxy_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("http://127.0.0.1:7890"));
        let models_search = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索模型，例如 deepseek")
        });
        let picker_search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索模型"));
        let max_tokens_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("4096")
        });
        // Each search box re-renders the view as it is typed into.
        let mut subscriptions = vec![cx.subscribe_in(
            &prompt,
            window,
            |this: &mut Self, _, event: &PromptBarEvent, window, cx| {
                this.on_prompt_event(event, window, cx);
            },
        )];
        let mut live_inputs = vec![&models_search, &picker_search, &base_input, &max_tokens_input];
        for search in live_inputs.drain(..) {
            subscriptions.push(cx.subscribe_in(
                search,
                window,
                |_this: &mut Self, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                },
            ));
        }

        let (store, warning) = Store::load();
        let mut this = Self {
            store,
            page: Page::Chat,
            section: Section::Models,
            prompt,
            transcript,
            current: None,
            stream: None,
            editing: None,
            editor: Editor::default(),
            notice: None,
            name_input,
            base_input,
            key_input,
            proxy_input,
            models_search,
            picker_search,
            picker_open: false,
            thinking_open: HashSet::new(),
            max_tokens_input,
            _subscriptions: subscriptions,
        };
        if let Proxy::Custom { url } = this.store.proxy.clone() {
            this.proxy_input
                .update(cx, |input, cx| input.set_value(url, window, cx));
        }
        // The window exists by now, so the stored color scheme applies to the
        // first frame instead of flashing the default one.
        apply_theme(theme_mode(this.store.theme), window, cx);
        if let Some(warning) = warning {
            this.notice = Some(warning.into());
        }
        this
    }

    /// Selects the conversation to show, creating one when the store is empty.
    pub fn open(&mut self, cx: &mut Context<Self>) {
        if self.store.sessions.is_empty() {
            self.store.sessions.push(Session::new());
        }
        if self.current.is_none() {
            self.current = Some(self.store.sessions[0].id.clone());
        }
        self.sync_transcript(true);
        self.sync_prompt(cx);
    }

    // ------------------------------------------------------------ state sync

    fn current_session(&self) -> Option<&Session> {
        self.current
            .as_deref()
            .and_then(|id| self.store.session(id))
    }

    fn persist(&mut self, cx: &mut Context<Self>) {
        if let Err(err) = self.store.save() {
            self.warn(format!("保存配置失败: {err:#}"), cx);
        }
    }

    fn warn(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.notice = Some(text.into());
        cx.notify();
    }

    /// Re-syncs the transcript list with the current conversation.
    ///
    /// `force_follow` is for structural changes (switching conversations,
    /// sending); deltas only follow while the reader is already at the tail.
    fn sync_transcript(&mut self, force_follow: bool) {
        let count = self
            .current_session()
            .map_or(0, |session| session.messages.len());
        if self.transcript.item_count() != count {
            self.transcript.reset(count);
        }
        if force_follow || self.transcript.is_following_tail() {
            self.transcript.set_follow_mode(FollowMode::Tail);
            self.transcript.scroll_to_end();
        }
    }

    /// Points the composer's model picker at the conversation's provider.
    fn sync_prompt(&mut self, cx: &mut Context<Self>) {
        let provider = self
            .current
            .as_deref()
            .and_then(|id| self.store.provider_for_session(id));
        // Only the current model is handed to the composer: its own list cannot
        // be scrolled with the wheel (the popup layer swallows the event), so
        // selection happens in this app's searchable picker instead, and the
        // composer slot just shows what is selected.
        let models: Vec<PromptModel> = self
            .current_session()
            .and_then(|session| session.model.clone())
            .filter(|model| {
                provider.is_some_and(|provider| provider.models.contains(model))
            })
            .map(|model| {
                vec![PromptModel::new(model.clone(), model.clone()).provider(
                    provider.map(|provider| provider.name.clone()).unwrap_or_default(),
                )]
            })
            .unwrap_or_default();
        let selected = self
            .current_session()
            .and_then(|session| session.model.clone())
            .filter(|model| models.iter().any(|candidate| candidate.id() == model))
            .or_else(|| models.first().map(|model| model.id().to_string()));

        self.prompt.update(cx, |prompt, cx| {
            prompt.set_models(models, cx);
            if let Some(model) = selected {
                prompt.set_selected_model(model, cx);
            }
        });
    }

    fn set_progress(&mut self, progress: ProgressState, cx: &mut Context<Self>) {
        self.prompt
            .update(cx, |prompt, cx| prompt.set_progress(progress, cx));
    }

    // ----------------------------------------------------------------- theme

    fn set_theme(&mut self, theme: Theme, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.theme == theme {
            return;
        }
        self.store.theme = theme;
        apply_theme(theme_mode(theme), window, cx);
        self.persist(cx);
        cx.notify();
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_theme(self.store.theme.toggled(), window, cx);
    }

    // ---------------------------------------------------------------- events

    fn on_prompt_event(
        &mut self,
        event: &PromptBarEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PromptBarEvent::Submit { submission, .. } => {
                let text = submission.text().to_string();
                let model = submission.model_id().cloned();
                self.send(text, model, window, cx);
            }
            PromptBarEvent::ModelChanged { model_id, .. } => {
                if let Some(id) = self.current.clone() {
                    if let Some(session) = self.store.session_mut(&id) {
                        session.model = Some(model_id.to_string());
                    }
                    self.persist(cx);
                }
            }
            PromptBarEvent::CancelRequested { .. } => self.cancel_stream(cx),
            _ => {}
        }
    }

    /// Resolves the provider and model a completion in this conversation needs.
    ///
    /// Every missing prerequisite is reported through the notice banner rather
    /// than by mutating the transcript, so a rejected send leaves nothing behind.
    fn target(&mut self, cx: &mut Context<Self>) -> Option<(String, Provider, String)> {
        let session_id = self.current.clone()?;
        let Some(provider) = self.store.provider_for_session(&session_id).cloned() else {
            self.warn("还没有可用的供应商，请先打开「供应商设置」添加一个。", cx);
            return None;
        };
        let model = self
            .current_session()
            .and_then(|session| session.model.clone())
            .or_else(|| provider.models.first().cloned());
        let Some(model) = model else {
            self.warn(
                format!("供应商「{}」还没有模型列表，请到设置里获取模型。", provider.name),
                cx,
            );
            return None;
        };
        Some((session_id, provider, model))
    }

    fn send(
        &mut self,
        text: String,
        model: Option<SharedString>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.trim().to_string();
        if text.is_empty() || self.stream.is_some() {
            return;
        }
        let Some((session_id, provider, fallback_model)) = self.target(cx) else {
            return;
        };
        let model = model.map(|model| model.to_string()).unwrap_or(fallback_model);

        let placeholder = Msg::new(Role::Assistant, "");
        let message_id = placeholder.id.clone();
        let request = {
            let Some(session) = self.store.session_mut(&session_id) else {
                return;
            };
            if session.title == DEFAULT_TITLE {
                session.title = title_from(&text);
            }
            session.messages.push(Msg::new(Role::User, text));
            session.messages.push(placeholder);
            session.request_messages()
        };
        self.notice = None;
        self.launch(session_id, provider, model, message_id, request, cx);
    }

    /// Re-runs the completion that failed, dropping it and everything after it.
    fn retry(&mut self, message_id: &str, cx: &mut Context<Self>) {
        if self.stream.is_some() {
            return;
        }
        let Some((session_id, provider, model)) = self.target(cx) else {
            return;
        };
        let Some(session) = self.store.session_mut(&session_id) else {
            return;
        };
        let Some(index) = session.messages.iter().position(|msg| msg.id == message_id) else {
            return;
        };
        if index == 0 {
            return;
        }
        session.messages.truncate(index);
        let placeholder = Msg::new(Role::Assistant, "");
        let new_id = placeholder.id.clone();
        session.messages.push(placeholder);
        let request = session.request_messages();
        self.notice = None;
        self.launch(session_id, provider, model, new_id, request, cx);
    }

    /// Opens the in-place editor for a user message.
    fn begin_edit(&mut self, message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = self
            .current_session()
            .and_then(|session| session.messages.iter().find(|msg| msg.id == message_id))
            .map(|msg| msg.content.clone())
        else {
            return;
        };
        let editor =
            cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).default_value(text));
        self.editing = Some(Editing {
            message_id: message_id.to_string(),
            editor,
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            cx.notify();
        }
    }

    /// Applies an edit and answers the edited turn again, dropping the replies
    /// that followed it — an edited prompt invalidates them.
    fn submit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.take() else {
            return;
        };
        let text = editing.editor.read(cx).value().trim().to_string();
        if text.is_empty() {
            self.warn("消息内容不能为空。", cx);
            cx.notify();
            return;
        }
        let Some((session_id, provider, model)) = self.target(cx) else {
            cx.notify();
            return;
        };

        let placeholder = Msg::new(Role::Assistant, "");
        let message_id = placeholder.id.clone();
        let request = {
            let Some(session) = self.store.session_mut(&session_id) else {
                return;
            };
            let Some(index) = session
                .messages
                .iter()
                .position(|msg| msg.id == editing.message_id)
            else {
                return;
            };
            session.messages[index].content = text;
            session.messages[index].error = None;
            session.messages[index].error = None;
            session.messages.truncate(index + 1);
            session.messages.push(placeholder);
            session.request_messages()
        };
        self.notice = None;
        self.sync_transcript(true);
        self.launch(session_id, provider, model, message_id, request, cx);
    }

    /// Forks the conversation at a message into a new session.
    fn branch_from(&mut self, message_id: &str, cx: &mut Context<Self>) {
        let Some(session_id) = self.current.clone() else {
            return;
        };
        let (title, provider_id, model, messages) = {
            let Some(session) = self.store.session(&session_id) else {
                return;
            };
            let Some(index) = session
                .messages
                .iter()
                .position(|message| message.id == message_id)
            else {
                return;
            };
            (
                session.title.clone(),
                session.provider_id.clone(),
                session.model.clone(),
                session.messages[..=index].to_vec(),
            )
        };

        let mut branch = Session::new();
        branch.title = format!("{title} - 分支");
        branch.provider_id = provider_id;
        branch.model = model;
        branch.messages = messages;
        let id = branch.id.clone();
        let at = self
            .store
            .sessions
            .iter()
            .position(|session| session.id == session_id)
            .unwrap_or(0);
        self.store.sessions.insert(at, branch);
        self.current = Some(id);
        self.page = Page::Chat;
        self.sync_transcript(true);
        self.sync_prompt(cx);
        self.persist(cx);
        cx.notify();
    }

    fn launch(
        &mut self,
        session_id: String,
        provider: Provider,
        model: String,
        message_id: String,
        request: Vec<(Role, String)>,
        cx: &mut Context<Self>,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.stream = Some(Stream {
            session_id,
            message_id,
            cancel: cancel.clone(),
        });
        self.set_progress(ProgressState::Running, cx);
        self.sync_transcript(true);
        cx.notify();

        let receiver = api::stream_chat(
            provider.base_url.clone(),
            provider.api_key_plain(),
            provider.api,
            model,
            request,
            provider.max_tokens,
            self.store.proxy.clone(),
            cancel,
        );
        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                let terminal = matches!(event, StreamEvent::Done | StreamEvent::Error(_));
                if this
                    .update_in(cx, |this, window, cx| {
                        this.apply_stream_event(event, window, cx)
                    })
                    .is_err()
                {
                    return;
                }
                if terminal {
                    return;
                }
            }
        })
        .detach();
    }

    fn apply_stream_event(
        &mut self,
        event: StreamEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Content growth never yanks a reader who scrolled away from the tail;
        // `sync_transcript` follows only while the tail is in view.
        let Some(stream) = self.stream.as_ref() else {
            return;
        };
        let session_id = stream.session_id.clone();
        let message_id = stream.message_id.clone();

        match event {
            StreamEvent::Delta(text) => {
                if let Some(message) = self.message_mut(&session_id, &message_id) {
                    message.content.push_str(&text);
                    if let Some(index) = self.row_of(&session_id, &message_id) {
                        self.transcript.remeasure_items(index..index + 1);
                    }
                }
            }
            StreamEvent::Thinking(text) => {
                if let Some(message) = self.message_mut(&session_id, &message_id) {
                    let slot = message.thinking.get_or_insert_with(String::new);
                    slot.push_str(&text);
                    if let Some(index) = self.row_of(&session_id, &message_id) {
                        self.transcript.remeasure_items(index..index + 1);
                    }
                }
            }
            StreamEvent::Done => self.finish_stream(None, window, cx),
            StreamEvent::Error(error) => self.finish_stream(Some(error), window, cx),
        }
        self.sync_transcript(false);
        cx.notify();
    }

    fn finish_stream(
        &mut self,
        error: Option<String>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(stream) = self.stream.take() {
            let row = self.row_of(&stream.session_id, &stream.message_id);
            if let Some(message) = self.message_mut(&stream.session_id, &stream.message_id) {
                match error {
                    Some(error) => message.error = Some(error),
                    None if message.content.is_empty() => {
                        message.content = "（没有返回内容）".into()
                    }
                    None => {}
                }
            }
            if let Some(row) = row {
                self.transcript.remeasure_items(row..row + 1);
            }
        }
        self.set_progress(ProgressState::Pending, cx);
        self.sync_transcript(false);
        self.persist(cx);
        cx.notify();
    }

    fn cancel_stream(&mut self, cx: &mut Context<Self>) {
        if let Some(stream) = self.stream.as_ref() {
            stream.cancel.store(true, Ordering::Relaxed);
        }
        if self.stream.take().is_some() {
            self.set_progress(ProgressState::Pending, cx);
            self.persist(cx);
            cx.notify();
        }
    }

    /// Index of a message in a conversation, for list invalidation.
    fn row_of(&self, session_id: &str, message_id: &str) -> Option<usize> {
        self.store
            .session(session_id)?
            .messages
            .iter()
            .position(|message| message.id == message_id)
    }

    fn message_mut(&mut self, session_id: &str, message_id: &str) -> Option<&mut Msg> {
        self.store
            .session_mut(session_id)?
            .messages
            .iter_mut()
            .find(|msg| msg.id == message_id)
    }

    // -------------------------------------------------------------- sessions

    fn new_session(&mut self, cx: &mut Context<Self>) {
        let session = Session::new();
        let id = session.id.clone();
        self.store.sessions.insert(0, session);
        self.current = Some(id);
        self.page = Page::Chat;
        self.sync_transcript(true);
        self.sync_prompt(cx);
        self.persist(cx);
        cx.notify();
    }

    fn select_session(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.current.as_deref() == Some(id) {
            return;
        }
        self.cancel_stream(cx);
        self.current = Some(id.to_string());
        self.page = Page::Chat;
        self.sync_transcript(true);
        self.sync_prompt(cx);
        cx.notify();
    }

    fn delete_session(&mut self, id: &str, cx: &mut Context<Self>) {
        self.cancel_stream(cx);
        self.store.sessions.retain(|session| session.id != id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
            self.open(cx);
        }
        self.persist(cx);
        cx.notify();
    }

    // ---------------------------------------------------------------- proxy

    fn set_proxy(&mut self, proxy: Proxy, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.proxy == proxy {
            return;
        }
        self.store.proxy = proxy.clone();
        if let Proxy::Custom { url } = &proxy {
            self.proxy_input
                .update(cx, |input, cx| input.set_value(url.clone(), window, cx));
        }
        self.persist(cx);
        cx.notify();
    }

    /// Applies whatever address is typed in the proxy field.
    fn apply_custom_proxy(&mut self, cx: &mut Context<Self>) {
        let url = self.proxy_input.read(cx).value().trim().to_string();
        if url.is_empty() {
            self.warn("代理地址不能为空。", cx);
            return;
        }
        self.store.proxy = Proxy::Custom { url };
        self.notice = None;
        self.persist(cx);
        cx.notify();
    }

    // ------------------------------------------------------------- providers

    /// Opens the settings page, landing on the provider in use.
    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.id.is_none() {
            let selected = self
                .store
                .selected_provider
                .as_deref()
                .and_then(|id| self.store.provider(id))
                .cloned();
            self.edit_provider(selected.as_ref(), window, cx);
        }
        self.page = Page::Settings;
        cx.notify();
    }

    /// Loads a provider into the editor, or clears it for a new one.
    fn edit_provider(
        &mut self,
        provider: Option<&Provider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (id, name, base, key, models, api, max_tokens) = match provider {
            Some(provider) => (
                Some(provider.id.clone()),
                provider.name.clone(),
                provider.base_url.clone(),
                provider.api_key_plain(),
                provider.models.clone(),
                provider.api,
                provider.max_tokens.to_string(),
            ),
            None => (
                None,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
                ApiFormat::default(),
                "4096".to_string(),
            ),
        };
        self.editor = Editor {
            id,
            models,
            fetched: Vec::new(),
            picked: HashSet::new(),
            api,
            status: SharedString::default(),
            fetching: false,
        };
        self.models_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.name_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.base_input
            .update(cx, |input, cx| input.set_value(base, window, cx));
        self.key_input
            .update(cx, |input, cx| input.set_value(key, window, cx));
        self.max_tokens_input
            .update(cx, |input, cx| input.set_value(max_tokens, window, cx));
        cx.notify();
    }

    fn save_provider(&mut self, cx: &mut Context<Self>) {
        // A fetched list means the user has been choosing: save that choice.
        if !self.editor.fetched.is_empty() {
            let mut picked: Vec<String> = self
                .editor
                .fetched
                .iter()
                .filter(|model| self.editor.picked.contains(*model))
                .cloned()
                .collect();
            picked.sort();
            self.editor.models = picked;
        }
        let name = self.name_input.read(cx).value().trim().to_string();
        let base_url = self.base_input.read(cx).value().trim().to_string();
        let api_key = self.key_input.read(cx).value().trim().to_string();
        let max_tokens = self
            .max_tokens_input
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .unwrap_or(4096)
            .clamp(1, 2_000_000);
        if base_url.is_empty() {
            self.editor.status = "Base URL 不能为空。".into();
            cx.notify();
            return;
        }
        let name = if name.is_empty() {
            host_of(&base_url)
        } else {
            name
        };

        let id = match self.editor.id.clone() {
            Some(id) => {
                if let Some(provider) = self.store.provider_mut(&id) {
                    provider.name = name;
                    provider.base_url = base_url;
                    provider.set_api_key_plain(api_key);
                    provider.api = self.editor.api;
                    provider.models = self.editor.models.clone();
                    provider.max_tokens = max_tokens;
                }
                id
            }
            None => {
                let mut provider = Provider::new(name, base_url);
                provider.set_api_key_plain(api_key);
                provider.api = self.editor.api;
                provider.models = self.editor.models.clone();
                provider.max_tokens = max_tokens;
                let id = provider.id.clone();
                self.store.providers.push(provider);
                if self.store.selected_provider.is_none() {
                    self.store.selected_provider = Some(id.clone());
                }
                id
            }
        };
        self.editor.id = Some(id);
        self.editor.status = "已保存。".into();
        self.notice = None;
        self.persist(cx);
        self.sync_prompt(cx);
        cx.notify();
    }

    fn delete_provider(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.store.providers.retain(|provider| provider.id != id);
        if self.store.selected_provider.as_deref() == Some(id) {
            self.store.selected_provider = self.store.providers.first().map(|p| p.id.clone());
        }
        for session in &mut self.store.sessions {
            if session.provider_id.as_deref() == Some(id) {
                session.provider_id = None;
            }
        }
        if self.editor.id.as_deref() == Some(id) {
            self.edit_provider(None, window, cx);
        }
        self.persist(cx);
        self.sync_prompt(cx);
        cx.notify();
    }

    fn select_provider(&mut self, id: &str, cx: &mut Context<Self>) {
        self.store.selected_provider = Some(id.to_string());
        self.persist(cx);
        self.sync_prompt(cx);
        cx.notify();
    }

    /// Fetches `{base_url}/models` with the values currently in the editor.
    fn fetch_models(&mut self, cx: &mut Context<Self>) {
        if self.editor.fetching {
            return;
        }
        let base_url = self.base_input.read(cx).value().trim().to_string();
        let api_key = self.key_input.read(cx).value().trim().to_string();
        let api = self.editor.api;
        if base_url.is_empty() {
            self.editor.status = "先填写 Base URL。".into();
            cx.notify();
            return;
        }
        self.editor.fetching = true;
        self.editor.status = "正在获取模型列表…".into();
        cx.notify();

        let receiver =
            api::fetch_models(base_url, api_key, api, self.store.proxy.clone());
        cx.spawn(async move |this, cx| {
            let result = receiver
                .recv()
                .await
                .unwrap_or_else(|_| Err("任务被中断".into()));
            let _ = this.update(cx, |this, cx| this.apply_models(result, cx));
        })
        .detach();
    }

    fn apply_models(&mut self, result: Result<Vec<String>, String>, cx: &mut Context<Self>) {
        self.editor.fetching = false;
        match result {
            Ok(models) => {
                // Nothing is saved here: the fetched list is a menu to pick
                // from, and only 保存 writes the choice into the provider.
                self.editor.picked = models
                    .iter()
                    .filter(|model| self.editor.models.contains(model))
                    .cloned()
                    .collect();
                self.editor.status = format!(
                    "获取到 {} 个模型，已勾选 {} 个，确认后点「保存」。",
                    models.len(),
                    self.editor.picked.len()
                )
                .into();
                self.editor.fetched = models;
            }
            Err(error) => self.editor.status = format!("获取失败: {error}").into(),
        }
        cx.notify();
    }

    /// The fetched models matching the settings search box.
    fn fetched_matching(&self, cx: &App) -> Vec<String> {
        let query = self.models_search.read(cx).value().trim().to_lowercase();
        self.editor
            .fetched
            .iter()
            .filter(|model| query.is_empty() || model.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    /// The models the composer's picker offers, matching its search box.
    fn picker_matching(&self, cx: &App) -> Vec<String> {
        let query = self.picker_search.read(cx).value().trim().to_lowercase();
        let Some(provider) = self
            .current
            .as_deref()
            .and_then(|id| self.store.provider_for_session(id))
        else {
            return Vec::new();
        };
        provider
            .models
            .iter()
            .filter(|model| query.is_empty() || model.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    fn pick_model(&mut self, model: &str, cx: &mut Context<Self>) {
        let Some(id) = self.current.clone() else {
            return;
        };
        if let Some(session) = self.store.session_mut(&id) {
            session.model = Some(model.to_string());
        }
        self.picker_open = false;
        self.persist(cx);
        self.sync_prompt(cx);
        cx.notify();
    }

    // ---------------------------------------------------------------- render
}

impl Render for KiiChat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let dark = self.store.theme.is_dark();
        let collapsed = self.store.sidebar_collapsed;
        let title_bar = self.render_title_bar(window, dark, cx);
        let sidebar = self.render_sidebar(cx);
        let page = match self.page {
            Page::Chat => self.render_chat(cx),
            Page::Settings => self.render_settings(cx),
        };

        v_flex()
            .size_full()
            .min_w_0()
            .bg(background)
            .child(title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(!collapsed, |this| this.child(sidebar))
                    .child(div().flex_1().min_w_0().h_full().child(page)),
            )
    }
}

/// One system window control.
///
/// On Windows the platform hit-tests the registered area and performs the
/// action itself, so the handler is only wired where that is not the case.
#[allow(clippy::too_many_arguments)]
fn control_button(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    area: WindowControlArea,
    hover: Hsla,
    active: Hsla,
    ink: Hsla,
    hover_ink: Hsla,
    action: fn(&mut Window, &mut App),
) -> impl IntoElement {
    div()
        .id(id)
        .role(gpui::Role::Button)
        .aria_label(label)
        .flex()
        .w(TITLE_BAR_HEIGHT)
        .h_full()
        .flex_shrink_0()
        .justify_center()
        .items_center()
        .text_color(ink)
        .when(cfg!(target_os = "windows"), |this| {
            this.window_control_area(area)
        })
        .when(!cfg!(target_os = "windows"), |this| {
            this.on_click(move |_, window, cx| {
                cx.stop_propagation();
                action(window, cx);
            })
        })
        .hover(move |style| style.bg(hover).text_color(hover_ink))
        .active(move |style| style.bg(active).text_color(hover_ink))
        .child(Icon::new(icon).small())
}

/// The concrete URLs a Base URL resolves to, so a half-typed host is obvious.
///
/// Providers are usually entered as far as `/v1`; this shows what the app will
/// actually call, after the same normalization the HTTP layer applies.
fn endpoint_hint(base_url: &str, api: ApiFormat, muted: Hsla) -> AnyElement {
    let base = api::normalize_base(base_url);
    let completion = match api {
        ApiFormat::OpenAiCompletions => "chat/completions",
        ApiFormat::OpenAiResponses => "responses",
        ApiFormat::AnthropicMessages => "messages",
    };
    let dialogue = format!("对话：{base}/{completion}");
    let models = format!("模型列表：{base}/models");
    v_flex()
        .id("endpoint-hint")
        .gap(px(2.))
        .text_xs()
        .text_color(muted)
        .child(div().child(dialogue))
        .child(div().child(models))
        .into_any_element()
}

/// The three-format choice, with the selected format's shape spelled out.
fn format_row(selected: &ApiFormat, cx: &mut Context<KiiChat>) -> AnyElement {
    let theme = cx.theme();
    let buttons = ApiFormat::ALL
        .iter()
        .map(|format| {
            let format = *format;
            Button::new(SharedString::from(format!("api-{}", format.label())))
                .label(format.label())
                .small()
                .selected(selected == &format)
                .tooltip(format.description())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.editor.api = format;
                    cx.notify();
                }))
                .into_any_element()
        })
        .collect::<Vec<_>>();
    v_flex()
        .gap_1()
        .child(h_flex().gap_2().children(buttons))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(selected.description()),
        )
        .into_any_element()
}

fn title_from(text: &str) -> String {
    let line = text.lines().next().unwrap_or(text).trim();
    let title: String = line.chars().take(24).collect();
    if line.chars().count() > 24 {
        format!("{title}…")
    } else if title.is_empty() {
        DEFAULT_TITLE.into()
    } else {
        title
    }
}

fn host_of(base_url: &str) -> String {
    base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or(base_url)
        .to_string()
}
