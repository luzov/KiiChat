//! The KiiChat window: a session sidebar, a conversation pane, and the
//! provider settings page.
//!
//! The view owns all durable state — providers, sessions, messages — and hands
//! `gpui-ai` snapshots to render. A completion runs on a worker thread; the UI
//! task only appends deltas and re-snapshots.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_ai::chat::{
    Chat, ChatEvent, ChatMessage, ChatMessageAppearance, ChatRole, ChatWelcome, MessageAlignment,
    MessageBubble,
};
use gpui_ai::prompt_bar::{PromptBar, PromptBarEvent, PromptModel};
use gpui_ai::stream::{ProgressState, StreamedContent};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputState};

use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
    Theme as ThemeGlobal, ThemeMode, TitleBar, h_flex, v_flex,
};

use crate::api::{self, StreamEvent};
use crate::store::{Msg, Provider, Role, Session, Store, Theme};

const SIDEBAR_WIDTH: f32 = 232.;
const DEFAULT_TITLE: &str = "新对话";



#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Chat,
    Settings,
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
    /// Models fetched but not yet attached to a saved provider.
    models: Vec<String>,
    status: SharedString,
    fetching: bool,
}

pub struct KiiChat {
    store: Store,
    page: Page,
    prompt: Entity<PromptBar>,
    chat: Entity<Chat>,
    current: Option<String>,
    stream: Option<Stream>,
    editor: Editor,
    notice: Option<SharedString>,
    name_input: Entity<InputState>,
    base_input: Entity<InputState>,
    key_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl KiiChat {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prompt = cx.new(|cx| PromptBar::new("kiichat-prompt", window, cx));
        let chat = cx.new(|cx| Chat::new("kiichat-chat", prompt.clone(), window, cx));
        chat.update(cx, |chat, cx| chat.set_welcome(Some(welcome()), cx));

        let name_input = cx.new(|cx| InputState::new(window, cx).placeholder("名称，例如 OpenAI"));
        let base_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Base URL，例如 https://api.openai.com/v1")
        });
        let key_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("API Key").masked(true));
        let subscriptions = vec![cx.subscribe_in(
            &chat,
            window,
            |this: &mut Self, _, event: &ChatEvent, window, cx| {
                this.on_chat_event(event, window, cx);
            },
        )];

        let this = Self {
            store: Store::load(),
            page: Page::Chat,
            prompt,
            chat,
            current: None,
            stream: None,
            editor: Editor::default(),
            notice: None,
            name_input,
            base_input,
            key_input,
            _subscriptions: subscriptions,
        };
        // The window exists by now, so the stored color scheme applies to the
        // first frame instead of flashing the default one.
        ThemeGlobal::change(theme_mode(this.store.theme), Some(window), cx);
        this
    }

    /// Selects the conversation to show, creating one when the store is empty.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.sessions.is_empty() {
            self.store.sessions.push(Session::new());
        }
        if self.current.is_none() {
            self.current = Some(self.store.sessions[0].id.clone());
        }
        self.sync_chat(window, cx);
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

    /// Pushes the current conversation into the chat component.
    fn sync_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = self.snapshot();
        self.chat
            .update(cx, |chat, cx| chat.set_messages(snapshot, window, cx));
        self.chat.update(cx, |chat, cx| chat.scroll_to_latest(cx));
    }

    fn snapshot(&self) -> Arc<[ChatMessage]> {
        let Some(session) = self.current_session() else {
            return Arc::from([]);
        };
        let streaming = self
            .stream
            .as_ref()
            .map(|stream| stream.message_id.as_str());
        session
            .messages
            .iter()
            .map(|msg| {
                let content = match &msg.error {
                    Some(error) => StreamedContent::failed(msg.content.clone(), error.clone()),
                    None if streaming == Some(msg.id.as_str()) => {
                        StreamedContent::running(msg.content.clone())
                    }
                    None => StreamedContent::done(msg.content.clone()),
                };
                let role = match msg.role {
                    Role::User => ChatRole::User,
                    Role::Assistant => ChatRole::Assistant,
                    Role::System => ChatRole::System,
                };
                let appearance = match role {
                    ChatRole::User => {
                        ChatMessageAppearance::new(MessageAlignment::Trailing, MessageBubble::Filled)
                    }
                    _ => ChatMessageAppearance::default(),
                };
                let failed = msg.error.is_some();
                ChatMessage::new(msg.id.clone(), role, content)
                    .with_appearance(appearance)
                    .retryable(failed)
            })
            .collect::<Vec<_>>()
            .into()
    }

    /// Points the composer's model picker at the conversation's provider.
    fn sync_prompt(&mut self, cx: &mut Context<Self>) {
        let provider = self
            .current
            .as_deref()
            .and_then(|id| self.store.provider_for_session(id));
        let models: Vec<PromptModel> = provider
            .map(|provider| {
                provider
                    .models
                    .iter()
                    .map(|model| {
                        PromptModel::new(model.clone(), model.clone()).provider(provider.name.clone())
                    })
                    .collect()
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
        ThemeGlobal::change(theme_mode(theme), Some(window), cx);
        self.persist(cx);
        cx.notify();
    }

    fn toggle_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_theme(self.store.theme.toggled(), window, cx);
    }

    // ---------------------------------------------------------------- events

    fn on_chat_event(&mut self, event: &ChatEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ChatEvent::Prompt(PromptBarEvent::Submit { submission, .. }) => {
                let text = submission.text().to_string();
                let model = submission.model_id().cloned();
                self.send(text, model, window, cx);
            }
            ChatEvent::Prompt(PromptBarEvent::ModelChanged { model_id, .. }) => {
                if let Some(id) = self.current.clone() {
                    if let Some(session) = self.store.session_mut(&id) {
                        session.model = Some(model_id.to_string());
                    }
                    self.persist(cx);
                }
            }
            ChatEvent::Prompt(PromptBarEvent::CancelRequested { .. }) => self.cancel_stream(cx),
            ChatEvent::RetryRequested { message_id } => self.retry(message_id, window, cx),
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
        window: &mut Window,
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
        self.launch(session_id, provider, model, message_id, request, window, cx);
    }

    /// Re-runs the completion that failed, dropping it and everything after it.
    fn retry(&mut self, message_id: &str, window: &mut Window, cx: &mut Context<Self>) {
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
        self.launch(session_id, provider, model, new_id, request, window, cx);
    }

    fn launch(
        &mut self,
        session_id: String,
        provider: Provider,
        model: String,
        message_id: String,
        request: Vec<(Role, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.stream = Some(Stream {
            session_id,
            message_id,
            cancel: cancel.clone(),
        });
        self.set_progress(ProgressState::Running, cx);
        self.sync_chat(window, cx);
        cx.notify();

        let receiver = api::stream_chat(provider.base_url, provider.api_key, model, request, cancel);
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
        let Some(stream) = self.stream.as_ref() else {
            return;
        };
        let session_id = stream.session_id.clone();
        let message_id = stream.message_id.clone();

        match event {
            StreamEvent::Delta(text) => {
                if let Some(message) = self.message_mut(&session_id, &message_id) {
                    message.content.push_str(&text);
                }
            }
            StreamEvent::Done => self.finish_stream(None, window, cx),
            StreamEvent::Error(error) => self.finish_stream(Some(error), window, cx),
        }
        self.sync_chat(window, cx);
        cx.notify();
    }

    fn finish_stream(&mut self, error: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(stream) = self.stream.take() {
            if let Some(message) = self.message_mut(&stream.session_id, &stream.message_id) {
                match error {
                    Some(error) => message.error = Some(error),
                    None if message.content.is_empty() => {
                        message.content = "（没有返回内容）".into()
                    }
                    None => {}
                }
            }
        }
        self.set_progress(ProgressState::Pending, cx);
        self.sync_chat(window, cx);
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

    fn message_mut(&mut self, session_id: &str, message_id: &str) -> Option<&mut Msg> {
        self.store
            .session_mut(session_id)?
            .messages
            .iter_mut()
            .find(|msg| msg.id == message_id)
    }

    // -------------------------------------------------------------- sessions

    fn new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = Session::new();
        let id = session.id.clone();
        self.store.sessions.insert(0, session);
        self.current = Some(id);
        self.page = Page::Chat;
        self.sync_chat(window, cx);
        self.sync_prompt(cx);
        self.persist(cx);
        cx.notify();
    }

    fn select_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.current.as_deref() == Some(id) {
            return;
        }
        self.cancel_stream(cx);
        self.current = Some(id.to_string());
        self.page = Page::Chat;
        self.sync_chat(window, cx);
        self.sync_prompt(cx);
        cx.notify();
    }

    fn delete_session(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_stream(cx);
        self.store.sessions.retain(|session| session.id != id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
            self.open(window, cx);
        }
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
        let (id, name, base, key, models) = match provider {
            Some(provider) => (
                Some(provider.id.clone()),
                provider.name.clone(),
                provider.base_url.clone(),
                provider.api_key.clone(),
                provider.models.clone(),
            ),
            None => (None, String::new(), String::new(), String::new(), Vec::new()),
        };
        self.editor = Editor {
            id,
            models,
            status: SharedString::default(),
            fetching: false,
        };
        self.name_input
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.base_input
            .update(cx, |input, cx| input.set_value(base, window, cx));
        self.key_input
            .update(cx, |input, cx| input.set_value(key, window, cx));
        cx.notify();
    }

    fn save_provider(&mut self, cx: &mut Context<Self>) {
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
                    provider.models = self.editor.models.clone();
                }
                id
            }
            None => {
                let mut provider = Provider::new(name, base_url);
                provider.api_key = api_key;
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
        if base_url.is_empty() {
            self.editor.status = "先填写 Base URL。".into();
            cx.notify();
            return;
        }
        self.editor.fetching = true;
        self.editor.status = "正在获取模型列表…".into();
        cx.notify();

        let receiver = api::fetch_models(base_url, api_key);
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
                self.editor.status = format!("获取到 {} 个模型，保存后生效。", models.len()).into();
                self.editor.models = models;
                if let Some(id) = self.editor.id.clone() {
                    if let Some(provider) = self.store.provider_mut(&id) {
                        provider.models = self.editor.models.clone();
                    }
                    self.persist(cx);
                    self.sync_prompt(cx);
                }
            }
            Err(error) => self.editor.status = format!("获取失败: {error}").into(),
        }
        cx.notify();
    }

    // ---------------------------------------------------------------- render

    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let dark = self.store.theme.is_dark();
        let sidebar_bg = theme.sidebar;
        let sidebar_border = theme.sidebar_border;
        let sidebar_fg = theme.sidebar_foreground;
        let accent = theme.sidebar_accent;
        let accent_fg = theme.sidebar_accent_foreground;
        let muted = theme.muted_foreground;

        let rows: Vec<AnyElement> = self
            .store
            .sessions
            .iter()
            .map(|session| {
                let active = self.current.as_deref() == Some(session.id.as_str());
                let select = cx.listener({
                    let id = session.id.clone();
                    move |this: &mut Self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>| {
                        this.select_session(&id, window, cx);
                    }
                });
                let remove = cx.listener({
                    let id = session.id.clone();
                    move |this: &mut Self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>| {
                        this.delete_session(&id, window, cx);
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
                        div()
                            .opacity(0.)
                            .group_hover("session-row", |this| this.opacity(1.))
                            .child(
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
            .collect();

        let provider_label = self
            .store
            .selected_provider
            .as_deref()
            .and_then(|id| self.store.provider(id))
            .map(|provider| format!("当前供应商：{}", provider.name))
            .unwrap_or_else(|| "还没有配置供应商".into());

        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(sidebar_border)
            .text_color(sidebar_fg)
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .child(
                        div()
                            .px_1()
                            .text_base()
                            .font_semibold()
                            .child("KiiChat"),
                    )
                    .child(
                        Button::new("new-session")
                            .label("新建对话")
                            .icon(IconName::Plus)
                            .small()
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| this.new_session(window, cx))),
                    ),
            )
            .child(
                div()
                    .id("session-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(v_flex().px_2().py_1().gap(px(2.)).children(rows)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .child(div().px_1().text_xs().text_color(muted).child(provider_label))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("toggle-page")
                                    .label(match self.page {
                                        Page::Chat => "设置",
                                        Page::Settings => "返回对话",
                                    })
                                    .icon(IconName::Settings)
                                    .small()
                                    .flex_1()
                                    .on_click(cx.listener(|this, _, window, cx| match this.page {
                                        Page::Chat => this.show_settings(window, cx),
                                        Page::Settings => {
                                            this.page = Page::Chat;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                // Outside the title bar: its drag region swallows
                                // clicks on anything inside it.
                                Button::new("toggle-theme")
                                    .icon(if dark { IconName::Sun } else { IconName::Moon })
                                    .ghost()
                                    .small()
                                    .flex_none()
                                    .tooltip(if dark { "切换到浅色" } else { "切换到深色" })
                                    .accessibility_label("切换深浅色")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_theme(window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.notice = None;
                                cx.notify();
                            })),
                    ),
            );
        }
        pane.child(div().flex_1().min_h_0().child(self.chat.clone()))
            .into_any_element()
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
        let row_active = theme.accent;
        let row_active_fg = theme.accent_foreground;

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
                    .when(is_editing, |this| this.bg(row_active).text_color(row_active_fg))
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
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child("无模型"),
                        )
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
            .rounded(radius_lg)
            .border_1()
            .border_color(border)
            .bg(theme.background)
            .child(
                div()
                    .text_base()
                    .font_semibold()
                    .child(if self.editor.id.is_some() {
                        "编辑供应商"
                    } else {
                        "新增供应商"
                    }),
            )
            .child(field("名称", Input::new(&self.name_input).into_any_element()))
            .child(field("Base URL", Input::new(&self.base_input).into_any_element()))
            .child(field("API Key", Input::new(&self.key_input).into_any_element()))
            .child(actions);
        if !self.editor.status.is_empty() {
            editor = editor.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(self.editor.status.clone()),
            );
        }

        let dark = self.store.theme.is_dark();
        let appearance = v_flex()
            .gap_3()
            .p_4()
            .rounded(radius_lg)
            .border_1()
            .border_color(border)
            .bg(surface)
            .child(div().text_sm().font_semibold().child("外观"))
            .child(
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
                    ),
            );

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
                            .child(div().text_lg().font_semibold().child("设置"))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(muted)
                                    .child("选择一个主题，并管理 OpenAI 兼容的供应商。"),
                            ),
                    )
                    .child(appearance)
                    .child(
                        v_flex()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().text_lg().font_semibold().child("供应商"))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(muted)
                                            .child("添加任意 OpenAI 兼容的接口，点「获取模型」一键拉取模型列表。"),
                                    ),
                            )
                            .child(
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
                                    .child(div().flex_1().min_w_0().child(editor)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for KiiChat {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.theme().background;
        let sidebar = self.render_sidebar(cx);
        let page = match self.page {
            Page::Chat => self.render_chat(cx),
            Page::Settings => self.render_settings(cx),
        };

        v_flex()
            .size_full()
            .min_w_0()
            .bg(background)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .w_full()
                        .pr_2()
                        .child(div().pl_2().text_sm().font_semibold().child("KiiChat")),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(sidebar)
                    .child(page),
            )
    }
}

fn theme_mode(theme: Theme) -> ThemeMode {
    if theme.is_dark() {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    }
}

fn welcome() -> ChatWelcome {
    ChatWelcome::new("开始新的对话").description(
        "在左侧新建会话，在下方输入消息。首次使用请先打开「设置」，添加一个 OpenAI 兼容的接口。",
    )
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