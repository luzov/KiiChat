//! KiiChat — a lightweight, Cherry Studio-style chat client for any
//! OpenAI-compatible endpoint.

mod api;
mod app;
mod icons;
mod store;

use gpui::{App, AppContext as _, Bounds, WindowBounds, WindowOptions, prelude::*, px, size};

use gpui_component::{ActiveTheme as _, Root, TitleBar};

use app::KiiChat;

fn main() {
    gpui_platform::application()
        .with_assets(icons::Assets)
        .run(|cx: &mut App| {
            // Initializes gpui-component too, so the application never calls both.
            gpui_ai::init(cx);

            let bounds = Bounds::centered(None, size(px(1080.), px(720.)), cx);
            // Client-side decorations: the app draws its own title bar, so the
            // window chrome matches the theme instead of the OS default.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.), px(480.))),
                ..TitleBar::window_options()
            };

            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    let mut view = KiiChat::new(window, cx);
                    view.open(cx);
                    view
                });
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("opening the main window");
            cx.activate(true);
        });
}