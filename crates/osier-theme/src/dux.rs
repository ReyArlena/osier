//! Dux Shell (DankMaterialShell) colour sync.
//!
//! DMS writes `$XDG_CACHE_HOME/dms/dms-colors.json` (legacy: `.../DankMaterialShell/`) with
//! Material tokens for dark and light plus a 16-colour terminal palette, and
//! `dms-typography.json` with font families. We map them to a [`Palette`] per mode.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::palette::{Mode, Palette};

#[derive(Debug, Deserialize)]
struct ColorsFile {
    #[serde(default)]
    mode: String,
    colors: Modes,
    #[serde(default)]
    dank16: std::collections::BTreeMap<String, Dank16Entry>,
}

#[derive(Debug, Deserialize)]
struct Modes {
    dark: std::collections::BTreeMap<String, String>,
    light: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Dank16Entry {
    dark: String,
    light: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Typography {
    #[serde(default)]
    pub font_family: Option<String>,
    #[serde(default)]
    pub mono_font_family: Option<String>,
}

/// Candidate colour files, preferred first.
pub fn color_file_candidates() -> Vec<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    let Some(cache) = cache else {
        return Vec::new();
    };
    vec![
        cache.join("dms/dms-colors.json"),
        cache.join("DankMaterialShell/dms-colors.json"),
    ]
}

/// The first colour file that exists.
pub fn color_file() -> Option<PathBuf> {
    color_file_candidates().into_iter().find(|p| p.is_file())
}

pub fn typography_file() -> Option<PathBuf> {
    let f = color_file()?.with_file_name("dms-typography.json");
    f.is_file().then_some(f)
}

/// Which mode DMS is currently in, from the colour file.
pub fn current_mode(json: &str) -> Option<Mode> {
    let file: ColorsFile = serde_json::from_str(json).ok()?;
    Some(if file.mode == "light" {
        Mode::Light
    } else {
        Mode::Dark
    })
}

/// Build the dark and light palettes from the colour file contents.
pub fn palettes(
    json: &str,
    typography: Option<&Typography>,
) -> Result<[Palette; 2], serde_json::Error> {
    let file: ColorsFile = serde_json::from_str(json)?;
    let term = |n: u8, mode: Mode| -> Option<String> {
        file.dank16.get(&format!("color{n}")).map(|e| match mode {
            Mode::Dark => e.dark.clone(),
            Mode::Light => e.light.clone(),
        })
    };
    let build = |mode: Mode, c: &std::collections::BTreeMap<String, String>| -> Palette {
        let get =
            |k: &str, fallback: &str| c.get(k).cloned().unwrap_or_else(|| fallback.to_string());
        let dark = mode == Mode::Dark;
        let base = get("background", if dark { "#1e1e2e" } else { "#eff1f5" });
        Palette {
            name: if dark {
                "Dux Shell Dark".into()
            } else {
                "Dux Shell Light".into()
            },
            mode,
            mantle: get("surface_container_low", &get("surface", &base)),
            crust: get("surface_container_lowest", &get("surface_dim", &base)),
            surface0: get("surface_container", &base),
            surface1: get("surface_container_high", &get("outline_variant", "#45475a")),
            surface2: get("surface_container_highest", &get("outline", "#585b70")),
            text: get("on_surface", if dark { "#cdd6f4" } else { "#4c4f69" }),
            subtext: get(
                "on_surface_variant",
                if dark { "#a6adc8" } else { "#6c6f85" },
            ),
            overlay: get("outline", if dark { "#6c7086" } else { "#9ca0b0" }),
            primary: get("primary", "#89b4fa"),
            on_primary: get("on_primary", &base),
            link: get("tertiary", &get("primary", "#89b4fa")),
            red: term(1, mode).unwrap_or_else(|| get("error", "#f38ba8")),
            maroon: term(9, mode).unwrap_or_else(|| get("error", "#eba0ac")),
            green: term(2, mode).unwrap_or_else(|| get("tertiary", "#a6e3a1")),
            yellow: term(3, mode).unwrap_or_else(|| get("secondary", "#f9e2af")),
            peach: term(9, mode).unwrap_or_else(|| get("secondary", "#fab387")),
            blue: term(4, mode).unwrap_or_else(|| get("primary", "#89b4fa")),
            cyan: term(6, mode).unwrap_or_else(|| get("tertiary", "#74c7ec")),
            magenta: term(5, mode).unwrap_or_else(|| get("tertiary", "#cba6f7")),
            font_family: typography.and_then(|t| t.font_family.clone()),
            mono_font_family: typography.and_then(|t| t.mono_font_family.clone()),
            base,
        }
    };
    Ok([
        build(Mode::Dark, &file.colors.dark),
        build(Mode::Light, &file.colors.light),
    ])
}

/// Read the files from disk and produce the theme document, plus the current DMS mode.
pub fn load() -> Option<(String, Mode)> {
    let path = color_file()?;
    load_from(&path)
}

pub fn load_from(path: &Path) -> Option<(String, Mode)> {
    let json = std::fs::read_to_string(path).ok()?;
    let typography = std::fs::read_to_string(path.with_file_name("dms-typography.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Typography>(&t).ok());
    let palettes = palettes(&json, typography.as_ref()).ok()?;
    let mode = current_mode(&json).unwrap_or(Mode::Dark);
    Some((
        crate::theme_document("Dux Shell", "generated from DankMaterialShell", &palettes),
        mode,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
      "mode": "dark",
      "colors": {
        "dark": {"background": "#101418", "surface": "#101418", "surface_container_low": "#181c20",
                 "surface_container": "#1c2024", "surface_container_high": "#272b2f",
                 "surface_container_highest": "#32363a", "on_surface": "#e0e3e8",
                 "on_surface_variant": "#c0c7d0", "outline": "#8a919a", "primary": "#9ecbff",
                 "on_primary": "#003355", "error": "#ffb4ab", "tertiary": "#d0bcff"},
        "light": {"background": "#f7f9fe", "on_surface": "#181c20", "primary": "#1f5f98",
                  "on_primary": "#ffffff"}
      },
      "dank16": {"color1": {"dark": "#ff7275", "light": "#b7001d"},
                 "color2": {"dark": "#79e79a", "light": "#007a3a"}}
    }"##;

    #[test]
    fn maps_material_tokens_and_terminal_colours() {
        let [dark, light] = palettes(
            SAMPLE,
            Some(&Typography {
                font_family: Some("Inter".into()),
                mono_font_family: None,
            }),
        )
        .unwrap();
        assert_eq!(dark.name, "Dux Shell Dark");
        assert_eq!(dark.base, "#101418");
        assert_eq!(dark.mantle, "#181c20");
        assert_eq!(dark.primary, "#9ecbff");
        assert_eq!(dark.red, "#ff7275", "terminal red wins over error");
        assert_eq!(dark.green, "#79e79a");
        assert_eq!(dark.font_family.as_deref(), Some("Inter"));
        assert_eq!(light.mode, Mode::Light);
        assert_eq!(light.base, "#f7f9fe");
        assert_eq!(light.red, "#b7001d");
        assert_eq!(current_mode(SAMPLE), Some(Mode::Dark));
        let doc: serde_json::Value =
            serde_json::from_str(&crate::theme_document("Dux Shell", "t", &[dark, light])).unwrap();
        assert_eq!(doc["themes"][0]["font.family"], "Inter");
        assert_eq!(doc["themes"][0]["colors"]["sidebar.background"], "#181c20");
    }

    #[test]
    fn candidates_prefer_the_new_directory() {
        let c = color_file_candidates();
        assert!(c.len() == 2 || c.is_empty());
        if c.len() == 2 {
            assert!(c[0].ends_with("dms/dms-colors.json"));
            assert!(c[1].ends_with("DankMaterialShell/dms-colors.json"));
        }
    }
}
