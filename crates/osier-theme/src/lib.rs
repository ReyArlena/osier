//! Theme generation for Osier.
//!
//! Themes are gpui-kit theme JSON (`{"name", "author", "themes": [{"name", "mode", "colors",
//! "highlight"}]}`). This crate produces that JSON from a small semantic [`Palette`], so the
//! bundled Catppuccin themes and the live Dux Shell theme use the same token mapping and
//! stay consistent with each other. Users can still drop any gpui-kit theme file into
//! `~/.config/osier/themes/`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod catppuccin;
pub mod classics;
pub mod dux;
pub mod palette;

pub use palette::{Mode, Palette};

/// Append an alpha byte (0..=255) to a `#rrggbb` colour.
pub fn with_alpha(hex: &str, alpha: u8) -> String {
    let base = hex.trim_start_matches('#');
    let rgb = &base[..base.len().min(6)];
    format!("#{rgb}{alpha:02x}")
}

/// Build the full gpui-kit theme document for a set of palettes.
pub fn theme_document(name: &str, author: &str, palettes: &[Palette]) -> String {
    let themes: Vec<serde_json::Value> = palettes.iter().map(Palette::to_theme_value).collect();
    let doc = serde_json::json!({
        "$schema": "https://gpui-kit.com/schemas/theme.json",
        "name": name,
        "author": author,
        "themes": themes,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_suffix() {
        assert_eq!(with_alpha("#89b4fa", 0x15), "#89b4fa15");
        assert_eq!(with_alpha("1e1e2e", 255), "#1e1e2eff");
    }
}
