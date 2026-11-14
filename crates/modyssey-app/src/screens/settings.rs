//! Settings screen (§6.2 #6) — v1: display of the saved settings plus the
//! data-folder location. Editing arrives with the interactive phase.
use gpui::{div, prelude::*, px, rgb, ParentElement, Styled};

use crate::ui::{screen_title, stat_row, tokens, Card};

pub fn render(_state: &mut super::super::workspace::WorkspaceState) -> impl IntoElement {
    let settings = modyssey_core::settings::Settings::load();
    let config_dir = modyssey_core::settings::Settings::config_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<unavailable: {e}>"));

    let masked_key = if settings.nexus_api_key.is_empty() {
        "not set".to_string()
    } else {
        format!(
            "{}… ({} chars, saved locally)",
            &settings.nexus_api_key[..4.min(settings.nexus_api_key.len())],
            settings.nexus_api_key.len()
        )
    };

    div()
        .flex()
        .flex_col()
        .gap_6()
        .child(screen_title("Settings", Some("Saved in settings.json — editing arrives with the interactive phase")))
        .child(
            Card::titled(
                "Downloads & installs",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(stat_row("Default install dir", settings.default_install_dir.clone()))
                    .child(stat_row("Default downloads dir", settings.default_downloads_dir.clone()))
                    .child(stat_row("Patch cache dir", settings.patch_cache_dir.clone())),
            ),
        )
        .child(
            Card::titled(
                "Nexus Mods",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(stat_row("API key", masked_key))
                    .child(stat_row(
                        "Note",
                        "A free account works: the manual lane opens each mod's page for a click and watches your Downloads folder.",
                    )),
            ),
        )
        .child(
            Card::titled(
                "Texture encoding",
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(stat_row(
                        "GPU",
                        if settings.gpu_name.is_empty() {
                            "auto-select".to_string()
                        } else {
                            settings.gpu_name.clone()
                        },
                    ))
                    .child(stat_row(
                        "GPU index",
                        settings
                            .gpu_index
                            .map(|i| i.to_string())
                            .unwrap_or_else(|| "auto".to_string()),
                    )),
            ),
        )
        .child(
            Card::titled(
                "Storage",
                div().flex().flex_col().gap_2().child(stat_row("Config dir", config_dir)),
            ),
        )
        .child(
            div().text_size(px(tokens::text::CAPTION)).text_color(rgb(
                tokens::TEXT_SECONDARY,
            )).child("Log viewer arrives with the install phase (per-session logs, filterable)."),
        )
}
