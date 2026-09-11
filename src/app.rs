//! The KiiChat window: a session sidebar, a conversation pane, and the
//! provider settings page.
//!
//! The view owns all durable state — providers, sessions, messages — and hands
//! `gpui-ai` snapshots to render. A completion runs on a worker thread; the UI
//! task only appends deltas and re-snapshots.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, ClipboardItem, Context, ElementId, Entity,
    FollowMode, Hsla, IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    WindowControlArea,
    Pixels, Styled, Subscription, Window, div, list, prelude::*, px, relative, rgb,
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
    StyledExt as _, Theme as ThemeGlobal, ThemeColor, ThemeMode, h_flex, v_flex,
};

use crate::api::{self, StreamEvent};
use crate::icons;
use crate::store::{ApiFormat, Msg, Provider, Proxy, Role, Session, Store, Theme};

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
        // Each search box re-renders the view as it is typed into.
        let mut subscriptions = vec![cx.subscribe_in(
            &prompt,
            window,
            |this: &mut Self, _, event: &PromptBarEvent, window, cx| {
                this.on_prompt_event(event, window, cx);
            },
        )];
        let mut live_inputs = vec![&models_search, &picker_search, &base_input];
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
            provider.base_url,
            provider.api_key,
            provider.api,
            model,
            request,
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
        let (id, name, base, key, models, api) = match provider {
            Some(provider) => (
                Some(provider.id.clone()),
                provider.name.clone(),
                provider.base_url.clone(),
                provider.api_key.clone(),
                provider.models.clone(),
                provider.api,
            ),
            None => (
                None,
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
                ApiFormat::default(),
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
                    provider.api_key = api_key;
                    provider.api = self.editor.api;
                    provider.models = self.editor.models.clone();
                }
                id
            }
            None => {
                let mut provider = Provider::new(name, base_url);
                provider.api_key = api_key;
                provider.api = self.editor.api;
                provider.models = self.editor.models.clone();
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

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let sidebar_bg = theme.sidebar;
        let sidebar_border = theme.sidebar_border;
        let sidebar_fg = theme.sidebar_foreground;

        let mut sidebar = v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(sidebar_border)
            .text_color(sidebar_fg);

        match self.page {
            Page::Chat => {
                sidebar = sidebar
                    .child(
                        div().p_2().child(
                            Button::new("new-session")
                                .label("新建对话")
                                .icon(IconName::Plus)
                                .small()
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| this.new_session(cx))),
                        ),
                    )
                    .child(
                        div()
                            .id("session-list")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(v_flex().px_2().pb_2().gap(px(2.)).children(self.session_rows(cx))),
                    );
            }
            Page::Settings => {
                let accent = theme.sidebar_accent;
                let accent_fg = theme.sidebar_accent_foreground;
                sidebar = sidebar.child(
                    div().flex_1().min_h_0().overflow_y_scrollbar().child(
                        v_flex()
                            .gap(px(2.))
                            .p_2()
                            .children(self.section_rows(accent, accent_fg, cx)),
                    ),
                );
            }
        }
        sidebar.into_any_element()
    }

    fn session_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let accent = theme.sidebar_accent;
        let accent_fg = theme.sidebar_accent_foreground;
        self.store
            .sessions
            .iter()
            .map(|session| {
                let active = self.current.as_deref() == Some(session.id.as_str());
                let select = cx.listener({
                    let id = session.id.clone();
                    move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.select_session(&id, cx);
                    }
                });
                let remove = cx.listener({
                    let id = session.id.clone();
                    move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.delete_session(&id, cx);
                    }
                });
                h_flex()
                    .id(SharedString::from(format!("session-{}", session.id)))
                    .group("session-row")
                    .role(gpui::Role::Button)
                    .aria_label(format!("会话：{}", session.title))
                    .gap_1()
                    .pl_2()
                    .pr_1()
                    .py_1()
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |this| this.bg(accent).text_color(accent_fg))
                    .on_click(select)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(session.title.clone()),
                    )
                    .child(
                        div().opacity(0.).group_hover("session-row", |this| this.opacity(1.)).child(
                            Button::new(SharedString::from(format!("delete-{}", session.id)))
                                .icon(IconName::Delete)
                                .ghost()
                                .small()
                                .accessibility_label("删除会话")
                                .on_click(remove),
                        ),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// Sidebar rows while settings are open: one per settings section.
    fn section_rows(&self, accent: Hsla, accent_fg: Hsla, cx: &mut Context<Self>) -> Vec<AnyElement> {
        Section::ALL
            .iter()
            .map(|(section, label, icon)| {
                let section = *section;
                let active = self.section == section;
                let select = cx.listener(move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                    this.section = section;
                    cx.notify();
                });
                h_flex()
                    .id(SharedString::from(format!("section-{}", section.title())))
                    .role(gpui::Role::Button)
                    .aria_label(format!("设置项：{label}"))
                    .gap_2()
                    .px_2()
                    .py_2()
                    .rounded(px(6.))
                    .text_sm()
                    .cursor_pointer()
                    .when(active, |this| this.bg(accent).text_color(accent_fg))
                    .on_click(select)
                    .child(Icon::new(icon.clone()).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(*label),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_chat(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut pane = v_flex().flex_1().min_w_0().h_full().gap_2().p_3();

        if let Some(notice) = self.notice.clone() {
            pane = pane.child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .border_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.secondary_foreground)
                            .child(notice),
                    )
                    .child(
                        Button::new("dismiss-notice")
                            .icon(IconName::Close)
                            .ghost()
                            .small()
                            .tooltip("关闭提示")
                            .accessibility_label("关闭提示")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }

        let empty = self
            .current_session()
            .is_none_or(|session| session.messages.is_empty());
        let body = if empty {
            self.render_welcome(cx)
        } else {
            let state = self.transcript.clone();
            // The list is a custom element: without an explicit size it lays
            // out to zero and paints nothing.
            div()
                .size_full()
                .child(list(state, cx.processor(Self::render_row)).size_full())
                .into_any_element()
        };

        pane.child(div().flex_1().min_h_0().child(body))
            .child(self.render_model_bar(cx))
            .child(div().flex_none().child(self.prompt.clone()))
            .into_any_element()
    }

    /// The model row above the composer, and its picker when open.
    ///
    /// The composer's own model menu cannot be scrolled with the wheel (the
    /// popup layer swallows the event), so selection lives here: an inline
    /// panel with a search box and a list that filters as you type.
    fn render_model_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let current = self
            .current_session()
            .and_then(|session| session.model.clone())
            .unwrap_or_else(|| "未选择模型".into());
        let provider = self
            .current
            .as_deref()
            .and_then(|id| self.store.provider_for_session(id))
            .map(|provider| provider.name.clone());

        let mut bar = h_flex()
            .flex_none()
            .justify_between()
            .gap_2()
            .px_1()
            .child(
                Button::new("open-picker")
                    .label(current.clone())
                    .icon(IconName::ChevronsUpDown)
                    .ghost()
                    .small()
                    .tooltip("选择模型")
                    .accessibility_label("选择模型")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.picker_open = !this.picker_open;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(provider.unwrap_or_default()),
            );

        if !self.picker_open {
            return bar.into_any_element();
        }

        let matching = self.picker_matching(cx);
        let rows: Vec<AnyElement> = matching
            .iter()
            .enumerate()
            .map(|(ix, model)| {
                let is_current = current == *model;
                let select = cx.listener({
                    let model = model.clone();
                    move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.pick_model(&model, cx);
                    }
                });
                h_flex()
                    .id(("pick-model", ix))
                    .role(gpui::Role::Button)
                    .aria_label(format!("模型：{model}"))
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(is_current, |this| {
                        this.bg(theme.accent).text_color(theme.accent_foreground)
                    })
                    .on_click(select)
                    .child(model.clone())
                    .into_any_element()
            })
            .collect();

        let empty = rows.is_empty();
        bar = bar.child(
            v_flex()
                .id("model-picker-panel")
                .gap_2()
                .p_2()
                .mb_2()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .child(Input::new(&self.picker_search))
                .child(
                    div()
                        .id("model-picker-list")
                        .max_h(px(220.))
                        .overflow_y_scrollbar()
                        .child(if empty {
                            v_flex()
                                .p_2()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(if self.store.providers.is_empty() {
                                    "还没有供应商，先去「设置 → 模型」添加。"
                                } else if self
                                    .current
                                    .as_deref()
                                    .and_then(|id| self.store.provider_for_session(id))
                                    .is_some_and(|provider| provider.models.is_empty())
                                {
                                    "这个供应商还没有模型，先去「设置 → 模型」获取。"
                                } else {
                                    "没有匹配的模型。"
                                })
                                .into_any_element()
                        } else {
                            v_flex().gap(px(2.)).children(rows).into_any_element()
                        }),
                ),
        );
        bar.into_any_element()
    }

    /// The window's own title bar: the title, the sidebar toggle, then the
    /// app's toggles and the system window controls.
    ///
    /// gpui-component's `TitleBar` cannot host controls, and neither can a
    /// drag region that sits in the same row: on Windows the platform reads a
    /// registered `WindowControlArea::Drag` for the whole row, so clicks on
    /// anything beside it become window drags. Only the title text carries the
    /// drag area; every control is a sibling of it.
    fn render_title_bar(
        &self,
        window: &mut Window,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let collapsed = self.store.sidebar_collapsed;
        let in_settings = self.page == Page::Settings;
        let controls = window.window_controls();

        let mut left = h_flex()
            .flex_none()
            .gap_2()
            .pl_3()
            .child(
                div()
                    .id("title-drag")
                    .text_sm()
                    .font_semibold()
                    .window_control_area(WindowControlArea::Drag)
                    .child("KiiChat"),
            );
        if !in_settings {
            left = left.child(
                Button::new("toggle-sidebar")
                    .icon(if collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .ghost()
                    .small()
                    .tooltip(if collapsed { "展开侧边栏" } else { "折叠侧边栏" })
                    .accessibility_label("折叠或展开侧边栏")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.sidebar_collapsed = !this.store.sidebar_collapsed;
                        this.persist(cx);
                        cx.notify();
                    })),
            );
        }

        let mut bar = h_flex()
            .flex_shrink_0()
            .w_full()
            .h(TITLE_BAR_HEIGHT)
            .items_center()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.title_bar_border)
            .text_color(theme.foreground)
            .child(left)
            .child(div().flex_1().min_w_0())
            .child(
                Button::new("toggle-theme")
                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                    .ghost()
                    .small()
                    .tooltip(if dark { "切换到浅色" } else { "切换到深色" })
                    .accessibility_label("切换深浅色")
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_theme(window, cx))),
            )
            .child(
                Button::new("toggle-page")
                    .icon(if in_settings {
                        IconName::ArrowLeft
                    } else {
                        IconName::Settings
                    })
                    .ghost()
                    .small()
                    .tooltip(if in_settings { "返回对话" } else { "设置" })
                    .accessibility_label(if in_settings { "返回对话" } else { "设置" })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.page == Page::Settings {
                            this.page = Page::Chat;
                            cx.notify();
                        } else {
                            this.show_settings(window, cx);
                        }
                    })),
            );

        // macOS draws its own controls; everywhere else the platform hit-tests
        // the areas below (Windows) or we drive them by hand.
        if !cfg!(target_os = "macos") {
            bar = bar
                .when(controls.minimize, |this| {
                    this.child(control_button(
                        "window-minimize",
                        "最小化",
                        IconName::WindowMinimize,
                        WindowControlArea::Min,
                        theme.secondary_hover,
                        theme.secondary_active,
                        theme.foreground,
                        |window, _| window.minimize_window(),
                    ))
                })
                .when(controls.maximize, |this| {
                    this.child(control_button(
                        "window-maximize",
                        if window.is_maximized() { "还原" } else { "最大化" },
                        if window.is_maximized() {
                            IconName::WindowRestore
                        } else {
                            IconName::WindowMaximize
                        },
                        WindowControlArea::Max,
                        theme.secondary_hover,
                        theme.secondary_active,
                        theme.foreground,
                        |window, _| window.zoom_window(),
                    ))
                })
                .child(control_button(
                    "window-close",
                    "关闭",
                    IconName::WindowClose,
                    WindowControlArea::Close,
                    theme.danger,
                    theme.danger_active,
                    theme.danger_foreground,
                    |window, _| window.remove_window(),
                ));
        }
        bar.into_any_element()
    }

    fn render_welcome(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .child(Orbs::new().diameter(px(56.)))
            .child(div().text_xl().font_semibold().child("开始新的对话"))
            .child(
                div()
                    .max_w(relative(0.7))
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .text_center()
                    .child("在左侧新建会话，在下方输入消息。首次使用请先打开「设置」，在「模型」里添加一个 OpenAI 兼容的接口。"),
            )
            .into_any_element()
    }

    /// One transcript row: a bubble plus the actions that apply to it.
    fn render_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self
            .current_session()
            .and_then(|session| session.messages.get(index))
            .cloned()
        else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let is_user = message.role == Role::User;
        let failed = message.error.as_deref().filter(|error| !error.is_empty());
        let streaming = self
            .stream
            .as_ref()
            .is_some_and(|stream| stream.message_id == message.id);
        let editing = self
            .editing
            .as_ref()
            .filter(|editing| editing.message_id == message.id)
            .map(|editing| editing.editor.clone());

        let content_id = ElementId::from((SharedString::from(message.id.clone()), index));
        let body: AnyElement = match &editing {
            Some(editor) => v_flex()
                .gap_2()
                .child(Textarea::new(editor).w_full())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(("save-edit", index))
                                .label("保存并重发")
                                .primary()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.submit_edit(cx))),
                        )
                        .child(
                            Button::new(("cancel-edit", index))
                                .label("取消")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                        ),
                )
                .into_any_element(),
            None => {
                let content = match failed {
                    Some(error) => StreamedContent::failed(message.content.clone(), error),
                    None if streaming => StreamedContent::running(message.content.clone()),
                    None => StreamedContent::done(message.content.clone()),
                };
                if is_user {
                    TextView::markdown(content_id, content.text())
                        .selectable(true)
                        .into_any_element()
                } else {
                    StreamingText::new(content_id, &content).into_any_element()
                }
            }
        };

        let bubble = v_flex()
            .id(ElementId::from((
                SharedString::from(message.id.clone()),
                index + 1_000_000,
            )))
            .min_w_0()
            .max_w(relative(0.82))
            // Room between the answer and the actions under it.
            .gap(px(12.))
            .px_3()
            .py_2()
            .rounded(theme.radius_lg)
            .when(is_user && editing.is_none(), |this| this.bg(theme.secondary))
            .when(editing.is_some(), |this| {
                this.border_1().border_color(theme.primary)
            })
            .when(failed.is_some(), |this| this.border_1().border_color(theme.danger))
            .child(body)
            .when(!streaming && editing.is_none(), |this| {
                this.child(self.render_actions(index, &message, failed.is_some(), window, cx))
            });

        v_flex()
            .id(ElementId::from((
                SharedString::from(message.id.clone()),
                index + 2_000_000,
            )))
            .w_full()
            .px_4()
            .py_2()
            .when(is_user, |this| this.items_end())
            .child(bubble)
            .into_any_element()
    }

    /// Icon-only actions for one message.
    ///
    /// A failed reply's retry grows a red label, so the way out of a failure
    /// sits where every other action lives.
    fn render_actions(
        &self,
        index: usize,
        message: &Msg,
        failed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let is_user = message.role == Role::User;
        let _ = window;

        let copy = cx.listener({
            let text = message.content.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                }
                this.notice = None;
                cx.notify();
            }
        });
        let branch = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                this.branch_from(&id, cx);
            }
        });
        let retry = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                this.retry(&id, cx);
            }
        });
        let edit = cx.listener({
            let id = message.id.clone();
            move |this: &mut Self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>| {
                this.begin_edit(&id, window, cx);
            }
        });

        let mut actions = h_flex()
            .w_full()
            .gap(px(2.))
            // User bubbles hug the right edge, replies the left.
            .when(is_user, |this| this.justify_end())
            .child(
                Button::new(("copy", index))
                    .icon(IconName::Copy)
                    .ghost()
                    .small()
                    .tooltip("复制")
                    .accessibility_label("复制")
                    .on_click(copy),
            )
            .child(
                Button::new(("branch", index))
                    .icon(Icon::empty().path(icons::GIT_BRANCH_PATH))
                    .ghost()
                    .small()
                    .tooltip("以此消息为起点新建会话")
                    .accessibility_label("分支")
                    .on_click(branch),
            );

        if is_user {
            actions = actions.child(
                Button::new(("edit", index))
                    .icon(Icon::empty().path(icons::SQUARE_PEN_PATH))
                    .ghost()
                    .small()
                    .tooltip("编辑并重发")
                    .accessibility_label("编辑")
                    .on_click(edit),
            );
        } else {
            let retry_button = Button::new(("retry", index))
                .icon(IconName::RotateCw)
                .small()
                .tooltip("重新生成")
                .accessibility_label("重试")
                .on_click(retry);
            actions = actions.child(if failed {
                // Expanded and inked, without a fill: the failure earns the
                // extra attention, the icon earns the color.
                retry_button
                    .label("重试")
                    .text()
                    .text_color(theme.danger)
            } else {
                retry_button.ghost()
            });
        }
        actions.into_any_element()
    }

    fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let surface = theme.secondary;
        let muted = theme.muted_foreground;
        let primary = theme.primary;
        let primary_fg = theme.primary_foreground;
        let radius = theme.radius;
        let radius_lg = theme.radius_lg;

        let card = |title: &str, body: AnyElement| {
            v_flex()
                .gap_3()
                .p_4()
                .rounded(radius_lg)
                .border_1()
                .border_color(border)
                .bg(surface)
                .child(div().text_sm().font_semibold().child(title.to_string()))
                .child(body)
        };

        let body: AnyElement = match self.section {
            Section::Appearance => {
                let dark = self.store.theme.is_dark();
                card(
                    "主题",
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("theme-light")
                                .label("浅色")
                                .icon(IconName::Sun)
                                .small()
                                .selected(!dark)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_theme(Theme::Light, window, cx);
                                })),
                        )
                        .child(
                            Button::new("theme-dark")
                                .label("深色")
                                .icon(IconName::Moon)
                                .small()
                                .selected(dark)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_theme(Theme::Dark, window, cx);
                                })),
                        )
                        .into_any_element(),
                )
                .into_any_element()
            }
            Section::Network => {
                let proxy = self.store.proxy.clone();
                let url_input = self.proxy_input.clone();
                card(
                    "代理",
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("proxy-system")
                                        .label("跟随系统")
                                        .small()
                                        .selected(matches!(proxy, Proxy::System))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_proxy(Proxy::System, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("proxy-none")
                                        .label("不使用代理")
                                        .small()
                                        .selected(matches!(proxy, Proxy::None))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.set_proxy(Proxy::None, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("proxy-custom")
                                        .label("自定义代理")
                                        .small()
                                        .selected(matches!(proxy, Proxy::Custom { .. }))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let current = match this.store.proxy.clone() {
                                                Proxy::Custom { url } => url,
                                                _ => String::new(),
                                            };
                                            this.set_proxy(Proxy::Custom { url: current }, window, cx);
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().min_w_0().child(Input::new(&url_input)))
                                .child(
                                    Button::new("apply-proxy")
                                        .label("应用")
                                        .small()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.apply_custom_proxy(cx);
                                        })),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .child("「跟随系统」读取系统与环境变量（HTTP_PROXY / HTTPS_PROXY）中的代理；自定义示例：http://127.0.0.1:7890"),
                        )
                        .into_any_element(),
                )
                .into_any_element()
            }
            Section::Models => {
                let selected = self.store.selected_provider.clone();
                let editing = self.editor.id.clone();
                let providers = self.store.providers.clone();

                let rows: Vec<AnyElement> = providers
                    .iter()
                    .enumerate()
                    .map(|(ix, provider)| {
                        let is_selected = selected.as_deref() == Some(provider.id.as_str());
                        let is_editing = editing.as_deref() == Some(provider.id.as_str());
                        let edit = cx.listener({
                            let provider = provider.clone();
                            move |this: &mut Self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>| {
                                this.edit_provider(Some(&provider), window, cx);
                            }
                        });

                        h_flex()
                            .id(("provider", ix))
                            .role(gpui::Role::Button)
                            .aria_label(if is_selected {
                                format!("供应商：{}（当前使用）", provider.name)
                            } else {
                                format!("供应商：{}", provider.name)
                            })
                            .gap_2()
                            .px_2()
                            .py_2()
                            .rounded(radius)
                            .cursor_pointer()
                            .when(is_editing, |this| {
                                this.bg(theme.accent).text_color(theme.accent_foreground)
                            })
                            .on_click(edit)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_sm()
                                    .child(provider.name.clone()),
                            )
                            .when(provider.models.is_empty(), |this| {
                                this.child(div().flex_none().text_xs().text_color(muted).child("无模型"))
                            })
                            .when(is_selected, |this| {
                                this.child(
                                    div()
                                        .flex_none()
                                        .px_2()
                                        .py(px(1.))
                                        .rounded(radius)
                                        .bg(primary)
                                        .text_color(primary_fg)
                                        .text_xs()
                                        .child("当前"),
                                )
                            })
                            .into_any_element()
                    })
                    .collect();

                // The fetched models, as a list to pick from.
                let mut selection: Option<AnyElement> = None;
                if !self.editor.fetched.is_empty() {
                    let matching = self.fetched_matching(cx);
                    let query = self.models_search.read(cx).value().trim().to_lowercase();
                    let rows: Vec<AnyElement> = matching
                        .iter()
                        .enumerate()
                        .map(|(ix, model)| {
                            let picked = self.editor.picked.contains(model);
                            let toggle = cx.listener({
                                let model = model.clone();
                                move |this: &mut Self, checked: &bool, _, cx: &mut Context<Self>| {
                                    if *checked {
                                        this.editor.picked.insert(model.clone());
                                    } else {
                                        this.editor.picked.remove(&model);
                                    }
                                    this.editor.status = format!(
                                        "已勾选 {} / {} 个模型，点「保存」生效。",
                                        this.editor.picked.len(),
                                        this.editor.fetched.len()
                                    )
                                    .into();
                                    cx.notify();
                                }
                            });
                            h_flex()
                                .id(("model-pick", ix))
                                .gap_2()
                                .px_2()
                                .py_1()
                                .rounded(radius)
                                .when(picked, |this| this.bg(theme.accent))
                                .child(
                                    Checkbox::new(("model-check", ix))
                                        .checked(picked)
                                        .label(model.clone())
                                        .on_click(toggle),
                                )
                                .into_any_element()
                        })
                        .collect();

                    let select_all = cx.listener({
                        let matching = matching.clone();
                        move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                            for model in &matching {
                                this.editor.picked.insert(model.clone());
                            }
                            this.editor.status =
                                format!("已勾选 {} 个模型，点「保存」生效。", this.editor.picked.len())
                                    .into();
                            cx.notify();
                        }
                    });
                    let clear_all = cx.listener(|this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.editor.picked.clear();
                        this.editor.status = "已清空勾选。".into();
                        cx.notify();
                    });

                    selection = Some(
                        v_flex()
                            .gap_2()
                            .p_2()
                            .rounded(radius)
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().flex_1().min_w_0().child(Input::new(&self.models_search)))
                                    .child(
                                        Button::new("pick-all")
                                            .label(if query.is_empty() {
                                                "全选".to_string()
                                            } else {
                                                format!("全选 {} 项", matching.len())
                                            })
                                            .small()
                                            .on_click(select_all),
                                    )
                                    .child(
                                        Button::new("pick-none")
                                            .label("清空")
                                            .small()
                                            .on_click(clear_all),
                                    ),
                            )
                            .child(
                                div()
                                    .id("fetched-models")
                                    .max_h(px(240.))
                                    .overflow_y_scrollbar()
                                    .child(v_flex().gap(px(2.)).children(rows)),
                            )
                            .into_any_element(),
                    );
                }

                let field = |label: &str, input: AnyElement| {
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().text_color(muted).child(label.to_string()))
                        .child(input)
                };

                let mut actions = h_flex()
                    .gap_2()
                    .child(
                        Button::new("fetch-models")
                            .label(if self.editor.fetching {
                                "获取中…"
                            } else {
                                "获取模型"
                            })
                            .icon(IconName::RotateCw)
                            .loading(self.editor.fetching)
                            .on_click(cx.listener(|this, _, _, cx| this.fetch_models(cx))),
                    )
                    .child(
                        Button::new("save-provider")
                            .label("保存")
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.save_provider(cx))),
                    );
                if let Some(id) = self.editor.id.clone() {
                    let is_selected = self.store.selected_provider.as_deref() == Some(id.as_str());
                    actions = actions
                        .child(
                            Button::new("use-edited")
                                .label("设为当前")
                                .disabled(is_selected)
                                .on_click(cx.listener({
                                    let id = id.clone();
                                    move |this, _, _, cx| {
                                        this.select_provider(&id, cx);
                                    }
                                })),
                        )
                        .child(
                            Button::new("delete-edited")
                                .label("删除")
                                .danger()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.delete_provider(&id, window, cx);
                                })),
                        );
                }

                let mut editor = v_flex()
                    .gap_3()
                    .p_4()
                    .flex_1()
                    .min_w_0()
                    .rounded(radius_lg)
                    .border_1()
                    .border_color(border)
                    .bg(surface)
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(if self.editor.id.is_some() {
                                "编辑供应商"
                            } else {
                                "新增供应商"
                            }),
                    )
                    .child(field("接口格式", format_row(&self.editor.api, cx)))
                    .child(field("名称", Input::new(&self.name_input).into_any_element()))
                    .child(field("Base URL", Input::new(&self.base_input).into_any_element()))
                    .child(endpoint_hint(
                        self.base_input.read(cx).value().as_ref(),
                        self.editor.api,
                        muted,
                    ))
                    .child(field("API Key", Input::new(&self.key_input).into_any_element()))
                    .child(actions)
                    .when_some(selection, |this, selection| this.child(selection));
                if !self.editor.status.is_empty() {
                    editor = editor.child(
                        div().text_xs().text_color(muted).child(self.editor.status.clone()),
                    );
                }

                h_flex()
                    .gap_4()
                    .items_start()
                    .child(
                        v_flex()
                            .w(px(240.))
                            .flex_none()
                            .gap_1()
                            .p_2()
                            .rounded(radius_lg)
                            .border_1()
                            .border_color(border)
                            .bg(surface)
                            .child(
                                Button::new("new-provider")
                                    .label("添加供应商")
                                    .icon(IconName::Plus)
                                    .small()
                                    .w_full()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.edit_provider(None, window, cx);
                                    })),
                            )
                            .children(rows),
                    )
                    .child(editor)
                    .into_any_element()
            }
        };

        div()
            .id("settings-page")
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap_4()
                    .p_6()
                    .max_w(px(920.))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_lg().font_semibold().child(self.section.title()))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child(self.section.description()),
                            ),
                    )
                    .child(body),
            )
            .into_any_element()
    }
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

/// Applies a color scheme, then repaints the app's own palette over it.
///
/// `Theme::change` re-applies whatever theme JSON is current, so the palette
/// has to land after it — and the base layer (scrollbars, resize handles) only
/// follows through `Theme::sync_base`.
fn apply_theme(mode: ThemeMode, window: &mut Window, cx: &mut App) {
    ThemeGlobal::change(mode, Some(window), cx);
    let dark = mode.is_dark();
    let palette = if dark { DARK_PALETTE } else { LIGHT_PALETTE };
    let colors = &mut ThemeGlobal::global_mut(cx).colors;
    for (slot, hex) in colors_of(colors).into_iter().zip(palette) {
        *slot = hsl(hex);
    }
    ThemeGlobal::sync_base(cx);
    window.refresh();
}

/// Handles to the tokens this app paints with, in palette order.
fn colors_of(colors: &mut ThemeColor) -> [&mut Hsla; 28] {
    [
        &mut colors.background,
        &mut colors.foreground,
        &mut colors.border,
        &mut colors.input,
        &mut colors.primary,
        &mut colors.primary_foreground,
        &mut colors.primary_hover,
        &mut colors.primary_active,
        &mut colors.secondary,
        &mut colors.secondary_foreground,
        &mut colors.secondary_hover,
        &mut colors.secondary_active,
        &mut colors.accent,
        &mut colors.accent_foreground,
        &mut colors.muted,
        &mut colors.muted_foreground,
        &mut colors.selection,
        &mut colors.caret,
        &mut colors.popover,
        &mut colors.popover_foreground,
        &mut colors.sidebar,
        &mut colors.sidebar_foreground,
        &mut colors.sidebar_accent,
        &mut colors.sidebar_accent_foreground,
        &mut colors.sidebar_border,
        &mut colors.title_bar,
        &mut colors.title_bar_border,
        &mut colors.scrollbar_thumb,
    ]
}

fn hsl(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// DeepSeek's web palette: white surfaces, a pale blue for user bubbles and
/// selected rows, and their #4D6BFE brand blue for accents.
const LIGHT_PALETTE: [u32; 28] = [
    0xffffff, // background
    0x1b1d22, // foreground
    0xe5e8f0, // border
    0xffffff, // input
    0x4d6bfe, // primary
    0xffffff, // primary_foreground
    0x3f5aea, // primary_hover
    0x3550d8, // primary_active
    0xeff3ff, // secondary (user bubble, cards)
    0x1b1d22, // secondary_foreground
    0xe4eaff, // secondary_hover
    0xdbe4ff, // secondary_active
    0xe3eaff, // accent (selected rows)
    0x1b1d22, // accent_foreground
    0xf4f6fb, // muted
    0x6b7280, // muted_foreground
    0xd6e0ff, // selection
    0x4d6bfe, // caret
    0xffffff, // popover
    0x1b1d22, // popover_foreground
    0xf7f8fc, // sidebar
    0x1b1d22, // sidebar_foreground
    0xe3eaff, // sidebar_accent
    0x1b1d22, // sidebar_accent_foreground
    0xe9ebf2, // sidebar_border
    0xf7f8fc, // title_bar
    0xe9ebf2, // title_bar_border
    0xd9dced, // scrollbar_thumb
];

const DARK_PALETTE: [u32; 28] = [
    0x17181c, // background
    0xe7e9ee, // foreground
    0x2a2d36, // border
    0x1e2026, // input
    0x5b78ff, // primary
    0xffffff, // primary_foreground
    0x6b85ff, // primary_hover
    0x7590ff, // primary_active
    0x232838, // secondary
    0xe7e9ee, // secondary_foreground
    0x2a3046, // secondary_hover
    0x303752, // secondary_active
    0x2b3350, // accent
    0xe7e9ee, // accent_foreground
    0x21232a, // muted
    0x9aa1ae, // muted_foreground
    0x33406b, // selection
    0x5b78ff, // caret
    0x1e2026, // popover
    0xe7e9ee, // popover_foreground
    0x121316, // sidebar
    0xe7e9ee, // sidebar_foreground
    0x262c40, // sidebar_accent
    0xe7e9ee, // sidebar_accent_foreground
    0x23262e, // sidebar_border
    0x121316, // title_bar
    0x23262e, // title_bar_border
    0x3a3f4d, // scrollbar_thumb
];

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
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(active).text_color(ink))
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
        .child(
            div()
                .id("endpoint-dialogue")
                .role(gpui::Role::Label)
                .aria_label(dialogue.clone())
                .child(dialogue),
        )
        .child(
            div()
                .id("endpoint-models")
                .role(gpui::Role::Label)
                .aria_label(models.clone())
                .child(models),
        )
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

fn theme_mode(theme: Theme) -> ThemeMode {
    if theme.is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
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