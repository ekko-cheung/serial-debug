#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod codec;
mod model;
mod serial;
mod storage;
mod text_input;

use gpui::{
    prelude::*, px, size, App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};

fn main() {
    Application::new().run(|cx: &mut App| {
        text_input::bind_keys(cx);
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("串口调试助手".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(app::SerialApp::new),
        )
        .unwrap();
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}
