//! Modyssey GUI — GPUI shell entry point (milestone M3).
//!
//! Mirrors the pattern proven by the GPUI spike report:
//! `Application::new().run(...)` with the Windows auto-quit hook. The
//! published `gpui = "=0.2.2"` crate API, not the main-branch website docs.
use gpui::{
    prelude::*, px, size, App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions,
};
use modyssey_app::Workspace;

fn main() {
    Application::new().run(|cx: &mut App| {
        // Quit when the last window closes (Windows process-lifetime fix
        // from the spike, mirroring the crate's on_window_close_quit example).
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Modyssey".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Workspace::new()),
        )
        .unwrap();
        cx.activate(true);
    });
}
