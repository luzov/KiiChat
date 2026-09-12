//! Settings pages: appearance, network, provider editor.

use super::*;

impl KiiChat {
    pub(super) fn render_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
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

                // Right-side catalog drawer (Cherry Studio-style): +/− per row,
                // closing the panel saves the kept set.
                let drawer: Option<AnyElement> = if self.editor.drawer_open {
                    let matching = self.fetched_matching(cx);
                    let rows: Vec<AnyElement> = matching
                        .iter()
                        .enumerate()
                        .map(|(ix, model)| {
                            let keep = self.editor.picked.contains(&model.id);
                            let id = model.id.clone();
                            let toggle = cx.listener({
                                let id = id.clone();
                                move |this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                                    let keep = !this.editor.picked.contains(&id);
                                    this.toggle_model_pick(&id, keep, cx);
                                }
                            });
                            let mut meta = Vec::new();
                            if let Some(ctx) = model.context_window {
                                meta.push(format!("上下文 {ctx}"));
                            }
                            if let Some(max) = model.max_tokens {
                                meta.push(format!("输出上限 {max}"));
                            }
                            let meta = if meta.is_empty() {
                                "—".to_string()
                            } else {
                                meta.join(" · ")
                            };
                            h_flex()
                                .id(("catalog-model", ix))
                                .gap_2()
                                .px_2()
                                .py_1()
                                .rounded(radius)
                                .when(keep, |this| this.bg(theme.accent))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(id.clone()),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(if keep {
                                            theme.accent_foreground
                                        } else {
                                            muted
                                        })
                                        .child(meta),
                                )
                                .child(
                                    Button::new(("toggle-model", ix))
                                        .icon(if keep { IconName::Minus } else { IconName::Plus })
                                        .ghost()
                                        .small()
                                        .tooltip(if keep { "从供应商移除" } else { "加入供应商" })
                                        .accessibility_label(if keep {
                                            format!("移除模型 {id}")
                                        } else {
                                            format!("添加模型 {id}")
                                        })
                                        .on_click(toggle),
                                )
                                .into_any_element()
                        })
                        .collect();

                    let close = cx.listener(|this: &mut Self, _: &ClickEvent, _, cx: &mut Context<Self>| {
                        this.close_models_drawer(cx);
                    });
                    Some(
                        v_flex()
                            .id("models-drawer")
                            .w(px(320.))
                            .flex_none()
                            .max_h(px(560.))
                            .gap_2()
                            .p_3()
                            .rounded(radius_lg)
                            .border_1()
                            .border_color(border)
                            .bg(theme.popover)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_sm()
                                            .font_semibold()
                                            .child("选择模型"),
                                    )
                                    .child(
                                        Button::new("close-drawer")
                                            .label("完成并保存")
                                            .primary()
                                            .small()
                                            .on_click(close),
                                    ),
                            )
                            .child(Input::new(&self.models_search))
                            .child(
                                div()
                                    .id("fetched-models")
                                    .max_h(px(400.))
                                    .overflow_y_scrollbar()
                                    .child(v_flex().gap(px(2.)).children(rows)),
                            )
                            .into_any_element(),
                    )
                } else {
                    None
                };

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
                    .child(field(
                        "默认输出上限（模型未单独设置时）",
                        Input::new(&self.max_tokens_input).into_any_element(),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child("Anthropic Messages 必填。各模型的独立上限在「获取模型」面板里显示；默认 8192。"),
                    )
                    .child(actions);
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
                    .when_some(drawer, |this, drawer| this.child(drawer))
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
