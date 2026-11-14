//! Workspace shell (§6.3): the root entity owning navigation and shared
//! state. Screens render from it; the left nav rail switches `active`.

use gpui::{
    div, prelude::*, px, rgb, Context, FontWeight, InteractiveElement, ParentElement, Render,
    SharedString, Styled, Window,
};

use crate::ui::tokens;

/// Which screen is active. The library/install-wizard screens arrive with
/// the engine-bridge phase; the shell is built so adding one is a new enum
/// variant + a render arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Games,
    Settings,
}

impl Screen {
    fn label(self) -> &'static str {
        match self {
            Screen::Home => "Home",
            Screen::Games => "Games",
            Screen::Settings => "Settings",
        }
    }
}

/// Shared workspace state (§6.3). Grows into LibraryStore / SettingsStore /
/// InstallSession entities as the screens gain interactivity.
pub struct WorkspaceState {
    pub active: Screen,
}

pub struct Workspace {
    state: WorkspaceState,
}

impl Workspace {
    pub fn new() -> Self {
        Self {
            state: WorkspaceState {
                active: Screen::Home,
            },
        }
    }

    fn select(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.state.active = screen;
        cx.notify();
    }

    fn nav_rail(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .w(px(220.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .child(
                div()
                    .px_3()
                    .py_4()
                    .text_size(px(tokens::text::H2))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(tokens::ACCENT))
                    .child("Modyssey"),
            )
            .children(
                [Screen::Home, Screen::Games, Screen::Settings].map(|screen| {
                    let selected = self.state.active == screen;
                    div()
                        .id(SharedString::from(format!("nav-{}", screen.label())))
                        .on_click({
                            cx.listener(move |this, _event, _window, cx| {
                                this.select(screen, cx);
                            })
                        })
                        .px_3()
                        .py_2()
                        .rounded_sm()
                        .flex()
                        .cursor_pointer()
                        .text_size(px(tokens::text::BODY))
                        .font_weight(FontWeight::MEDIUM)
                        .map(|el| {
                            if selected {
                                el.bg(rgb(tokens::BG_CARD))
                                    .text_color(rgb(tokens::TEXT_PRIMARY))
                            } else {
                                el.text_color(rgb(tokens::TEXT_SECONDARY))
                                    .hover(|el| el.bg(rgb(tokens::BG_CARD_HOVER)))
                            }
                        })
                        .child(screen.label())
                }),
            )
    }

    fn screen(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &mut self.state;
        let content = match state.active {
            Screen::Home => crate::screens::home::render(state).into_any_element(),
            Screen::Settings => crate::screens::settings::render(state).into_any_element(),
            Screen::Games => crate::screens::games::render(state, window, cx).into_any_element(),
        };
        div()
            .id("screen-scroll")
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(tokens::BG_SURFACE))
            .p_8()
            .child(content)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(rgb(tokens::BG_APP))
            .size_full()
            .flex()
            .font_family(tokens::FONT_FAMILY)
            .child(self.nav_rail(cx))
            .child(self.screen(window, cx))
    }
}
