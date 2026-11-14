//! Home / Discover screen (§6.2 #1) — v1: hero drop-zone card + the
//! showcase list card. List-detail and real library scanning arrive with the
//! install-wizard phase.

use gpui::{div, prelude::*, px, rgb, ParentElement, Styled};

use crate::ui::{screen_title, stat_row, tokens, Card, Pill, PrimaryButton, SecondaryButton, Tone};

/// Immutable facts about a known list, rendered as a library card.
pub struct ListCard {
    pub title: String,
    pub author: String,
    pub version: String,
    pub game: String,
    pub license: String,
}

pub fn render(_state: &mut super::super::workspace::WorkspaceState) -> impl IntoElement {
    let showcase = ListCard {
        title: "DS2 Modpack by Bonkfyre".into(),
        author: "himanshudbg".into(),
        version: "1.0.2".into(),
        game: "Dark Souls II: Scholar of the First Sin".into(),
        license: "CC BY-NC-SA 4.0".into(),
    };

    div()
        .flex()
        .flex_col()
        .gap_6()
        .child(screen_title("Modyssey", Some("Modlists, kept warm — a Wabbajack alternative")))
        .child(
            Card::titled(
                "Install a modlist",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .items_start()
                    .child(
                        div()
                            .text_size(px(tokens::text::BODY))
                            .text_color(rgb(tokens::TEXT_SECONDARY))
                            .child("Drop a .modyssey or .wabbajack file here, or browse for one. Everything stays local — no account needed to start."),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_3()
                            .child(PrimaryButton::new("Browse for a modlist"))
                            .child(SecondaryButton::new("Add a list repository")),
                    ),
            )
            .title_trailing(Pill::new("v1")),
        )
        .child(list_card(&showcase, true))
        .child(
            Card::titled(
                "Recent lists",
                crate::ui::empty_state(
                    "Nothing here yet",
                    "Lists you open or install will appear here.",
                    None,
                ),
            ),
        )
}

fn list_card(list: &ListCard, _installed: bool) -> impl IntoElement {
    Card::titled(
        list.title.clone(),
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(px(tokens::text::BODY))
                    .text_color(rgb(tokens::TEXT_SECONDARY))
                    .child(format!(
                        "{} · v{} · {}",
                        list.game, list.version, list.author
                    )),
            )
            .child(stat_row("License", list.license.clone()))
            .child(stat_row("Status", "Never installed")),
    )
    .title_trailing(Pill::new("DarkSoulsII").tone(Tone::Neutral))
}
