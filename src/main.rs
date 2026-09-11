use gpui::{App, AppContext as _, Entity, WindowOptions, div, prelude::*, px};
use gpui_component::{ActiveTheme as _, Root, v_flex};

struct KiiChat;

impl Render for KiiChat {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .bg(cx.theme().background)
            .child(div().text_xl().child("KiiChat"))
    }
}

fn main() {
    gpui_platform::application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_ai::init(cx);

            let view: Entity<KiiChat> = cx.new(|_| KiiChat);
            cx.open_window(WindowOptions::default(), move |window, cx| {
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("a window");
            cx.activate(true);
        });
}