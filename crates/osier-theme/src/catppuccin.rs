//! Catppuccin Mocha and Latte from the official palette.

use crate::palette::{Mode, Palette};

pub fn mocha() -> Palette {
    Palette {
        name: "Catppuccin Mocha".into(),
        mode: Mode::Dark,
        base: "#1e1e2e".into(),
        mantle: "#181825".into(),
        crust: "#11111b".into(),
        surface0: "#313244".into(),
        surface1: "#45475a".into(),
        surface2: "#585b70".into(),
        text: "#cdd6f4".into(),
        subtext: "#a6adc8".into(),
        overlay: "#6c7086".into(),
        primary: "#89b4fa".into(),
        on_primary: "#1e1e2e".into(),
        link: "#b4befe".into(),
        red: "#f38ba8".into(),
        maroon: "#eba0ac".into(),
        green: "#a6e3a1".into(),
        yellow: "#f9e2af".into(),
        peach: "#fab387".into(),
        blue: "#89b4fa".into(),
        cyan: "#74c7ec".into(),
        magenta: "#cba6f7".into(),
        font_family: None,
        mono_font_family: None,
    }
}

pub fn latte() -> Palette {
    Palette {
        name: "Catppuccin Latte".into(),
        mode: Mode::Light,
        base: "#eff1f5".into(),
        mantle: "#e6e9ef".into(),
        crust: "#dce0e8".into(),
        surface0: "#ccd0da".into(),
        surface1: "#bcc0cc".into(),
        surface2: "#acb0be".into(),
        text: "#4c4f69".into(),
        subtext: "#6c6f85".into(),
        overlay: "#9ca0b0".into(),
        primary: "#1e66f5".into(),
        on_primary: "#eff1f5".into(),
        link: "#7287fd".into(),
        red: "#d20f39".into(),
        maroon: "#e64553".into(),
        green: "#40a02b".into(),
        yellow: "#df8e1d".into(),
        peach: "#fe640b".into(),
        blue: "#1e66f5".into(),
        cyan: "#209fb5".into(),
        magenta: "#8839ef".into(),
        font_family: None,
        mono_font_family: None,
    }
}

/// The bundled theme document.
pub fn document() -> String {
    crate::theme_document(
        "Osier Catppuccin",
        "Osier (palette by Catppuccin, MIT)",
        &[mocha(), latte()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = "../../themes/osier-catppuccin.json";

    /// `themes/osier-catppuccin.json` is generated from this crate. Regenerate it with
    /// `cargo test -p osier-theme -- --ignored write_bundled_catppuccin`.
    #[test]
    fn bundled_catppuccin_file_is_up_to_date() {
        let on_disk = std::fs::read_to_string(BUNDLED).expect("bundled file present");
        assert_eq!(
            on_disk.trim_end(),
            document().trim_end(),
            "run the ignored write_bundled_catppuccin test"
        );
    }

    #[test]
    #[ignore = "writes the bundled theme file"]
    fn write_bundled_catppuccin() {
        std::fs::write(BUNDLED, format!("{}\n", document())).unwrap();
    }

    #[test]
    fn document_has_both_flavours_and_core_tokens() {
        let doc: serde_json::Value = serde_json::from_str(&document()).unwrap();
        let themes = doc["themes"].as_array().unwrap();
        assert_eq!(themes.len(), 2);
        assert_eq!(themes[0]["name"], "Catppuccin Mocha");
        assert_eq!(themes[0]["mode"], "dark");
        assert_eq!(themes[0]["colors"]["background"], "#1e1e2e");
        assert_eq!(themes[0]["colors"]["sidebar.background"], "#181825");
        assert_eq!(themes[0]["highlight"]["created"], "#a6e3a1");
        assert_eq!(themes[1]["mode"], "light");
        assert_eq!(themes[1]["colors"]["primary.background"], "#1e66f5");
    }
}
