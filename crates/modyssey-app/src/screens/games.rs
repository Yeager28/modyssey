//! Games manager screen (§6.2 #5) — the data-driven registry as a table.
//! Real data: `modyssey_games::games()` (embedded defaults + user overlay).

use gpui::{
    div, prelude::*, px, rgb, FontWeight, InteractiveElement, ParentElement, Styled, Window,
};

use crate::ui::{screen_title, tokens, Card, Pill};
use crate::workspace::{Workspace, WorkspaceState};

pub fn render(
    _state: &mut WorkspaceState,
    _window: &mut Window,
    _cx: &mut gpui::Context<Workspace>,
) -> impl IntoElement {
    let registry = modyssey_games::games();
    let schema = modyssey_games::GAMES_SCHEMA_VERSION;
    let overlay = modyssey_games::overlay_path();

    let header = div()
        .flex()
        .w_full()
        .gap_4()
        .px_4()
        .py_2()
        .child(th("Game", 340.))
        .child(th("Steam app ID", 140.))
        .child(th("Wabbajack type", 200.))
        .child(th("GOG", 100.));

    let mut rows = div().flex().flex_col().w_full();
    for game in registry {
        let wj_type = game.wabbajack_type.clone().unwrap_or_default();
        let flagship = wj_type == "DarkSoulsII";
        rows = rows.child(
            div()
                .flex()
                .w_full()
                .gap_4()
                .px_4()
                .py_2()
                .rounded_sm()
                .hover(|row| row.bg(rgb(tokens::BG_CARD_HOVER)))
                .child(td(&game.name, 340., flagship))
                .child(td(&game.steam_app_id, 140., false))
                .child(td(&wj_type, 200., flagship))
                .child(td(
                    &game.gog_app_id.clone().unwrap_or_default(),
                    100.,
                    false,
                )),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_6()
        .child(screen_title(
            "Games",
            Some(&format!(
                "{} entries · registry schema {}",
                registry.len(),
                schema
            )),
        ))
        .child(
            Card::titled(
                "Registry",
                div()
                    .flex()
                    .flex_col()
                    .w_full()
                    .gap_1()
                    .child(header)
                    .child(rows),
            )
            .title_trailing(Pill::new("DS2 first-class")),
        )
        .when_some(overlay, |el, overlay| {
            el.child(
                div()
                    .text_size(px(tokens::text::CAPTION))
                    .text_color(rgb(tokens::TEXT_SECONDARY))
                    .child(format!("User overlay: {}", overlay.display())),
            )
        })
}

fn th(label: &str, width: f32) -> impl IntoElement {
    div()
        .w(px(width))
        .flex_shrink_0()
        .text_size(px(tokens::text::CAPTION))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(tokens::TEXT_SECONDARY))
        .child(label.to_string())
}

fn td(value: &str, width: f32, accent: bool) -> impl IntoElement {
    div()
        .w(px(width))
        .flex_shrink_0()
        .text_size(px(tokens::text::BODY))
        .text_color(rgb(if accent {
            tokens::ACCENT
        } else {
            tokens::TEXT_PRIMARY
        }))
        .child(if value.is_empty() { "—" } else { value }.to_string())
}
