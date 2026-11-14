//! Reusable components styled from the token sheet (PLAN.md §6.1:
//! "Component kit" — Card, list row, pill chip, buttons, progress).
use super::tokens;
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, FontWeight, ParentElement, RenderOnce,
    SharedString, Styled, Window,
};

/// A rounded content card (radius `md` = 12, `bg.card`, hairline border).
#[derive(IntoElement)]
pub struct Card {
    title: Option<SharedString>,
    /// Optional trailing element on the title row (e.g. a pill chip).
    title_trailing: Option<AnyElement>,
    body: AnyElement,
}

impl Card {
    pub fn new(body: impl IntoElement) -> Self {
        Self {
            title: None,
            title_trailing: None,
            body: body.into_any_element(),
        }
    }

    pub fn titled(title: impl Into<SharedString>, body: impl IntoElement) -> Self {
        Self {
            title: Some(title.into()),
            title_trailing: None,
            body: body.into_any_element(),
        }
    }

    pub fn title_trailing(mut self, trailing: impl IntoElement) -> Self {
        self.title_trailing = Some(trailing.into_any_element());
        self
    }
}

impl RenderOnce for Card {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let body = div().flex().flex_col().gap_3().child(self.body);
        let card = div()
            .bg(rgb(tokens::BG_CARD))
            .rounded_xl() // 12px — radius md per §6.1
            .border_1()
            .border_color(rgba(tokens::HAIRLINE))
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .items_start()
            .child(body);
        match self.title {
            Some(title) => {
                let title_row = div()
                    .flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(tokens::text::H2))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(tokens::TEXT_PRIMARY))
                            .child(title),
                    )
                    .when_some(self.title_trailing, |row, trailing| row.child(trailing));
                card.child(title_row)
            }
            None => card,
        }
    }
}

/// A pill chip (`rounded_full`), the standard status/meta chip.
#[derive(IntoElement)]
pub struct Pill {
    text: SharedString,
    tone: Tone,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Accent green chip (`accent.chip`-style: accent bg, dark text).
    #[default]
    Accent,
    /// Neutral meta chip (card bg, secondary text, hairline border).
    Neutral,
    /// Warning chip.
    Warning,
    /// Danger chip.
    Danger,
}

impl Pill {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Accent,
        }
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    fn colors(tone: Tone) -> (u32, u32) {
        match tone {
            Tone::Accent => (tokens::ACCENT, tokens::ACCENT_FG),
            Tone::Neutral => (tokens::BG_CARD, tokens::TEXT_SECONDARY),
            Tone::Warning => (tokens::WARNING, tokens::ACCENT_FG),
            Tone::Danger => (tokens::DANGER, tokens::ACCENT_FG),
        }
    }
}

impl RenderOnce for Pill {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (bg, fg) = Self::colors(self.tone);
        let neutral = self.tone == Tone::Neutral;
        div()
            .px_3()
            .py_1()
            .flex()
            .rounded_full()
            .bg(rgb(bg))
            .when(neutral, |el| {
                el.border_1().border_color(rgba(tokens::HAIRLINE))
            })
            .text_size(px(tokens::text::CAPTION))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(fg))
            .child(self.text)
    }
}

/// A primary (solid accent) button label. v1 buttons are display-only;
/// interactive wiring lands with the install wizard.
#[derive(IntoElement)]
pub struct PrimaryButton {
    text: SharedString,
}

impl PrimaryButton {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for PrimaryButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .px_4()
            .py_2()
            .flex()
            .rounded_xl()
            .bg(rgb(tokens::ACCENT))
            .text_size(px(tokens::text::LABEL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(tokens::ACCENT_FG))
            .child(self.text)
    }
}

/// A secondary button (card bg, primary text, hairline border).
#[derive(IntoElement)]
pub struct SecondaryButton {
    text: SharedString,
}

impl SecondaryButton {
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self { text: text.into() }
    }
}

impl RenderOnce for SecondaryButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .px_4()
            .py_2()
            .flex()
            .rounded_xl()
            .bg(rgb(tokens::BG_CARD))
            .border_1()
            .border_color(rgba(tokens::HAIRLINE))
            .text_size(px(tokens::text::LABEL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(tokens::TEXT_PRIMARY))
            .child(self.text)
    }
}

/// A label/value meta row (settings, list detail).
pub fn stat_row(label: &str, value: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex()
        .w_full()
        .gap_4()
        .items_baseline()
        .child(
            div()
                .w(px(220.))
                .flex_shrink_0()
                .text_size(px(tokens::text::BODY))
                .text_color(rgb(tokens::TEXT_SECONDARY))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_size(px(tokens::text::BODY))
                .text_color(rgb(tokens::TEXT_PRIMARY))
                .child(value.into()),
        )
}

/// An empty-state block: title, description, optional primary action.
pub fn empty_state(
    title: &str,
    description: &str,
    action: Option<PrimaryButton>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .items_start()
        .p_6()
        .child(
            div()
                .text_size(px(tokens::text::H2))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tokens::TEXT_PRIMARY))
                .child(title.to_string()),
        )
        .child(
            div()
                .text_size(px(tokens::text::BODY))
                .text_color(rgb(tokens::TEXT_SECONDARY))
                .child(description.to_string()),
        )
        .when_some(action, |el, action| el.child(action))
}

/// Screen section title (h2-style).
pub fn section_title(title: &str) -> impl IntoElement {
    div()
        .text_size(px(tokens::text::H2))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(tokens::TEXT_PRIMARY))
        .child(title.to_string())
}

/// Screen display title (24/700) with an optional meta line underneath.
pub fn screen_title(title: &str, meta: Option<&str>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(tokens::text::DISPLAY))
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(tokens::TEXT_PRIMARY))
                .child(title.to_string()),
        )
        .when_some(meta, |el, meta| {
            el.child(
                div()
                    .text_size(px(tokens::text::CAPTION))
                    .text_color(rgb(tokens::TEXT_SECONDARY))
                    .child(meta.to_string()),
            )
        })
}
