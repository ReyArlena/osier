//! The theme store: bundled Catppuccin, generated Dux Shell themes and user theme files,
//! with hot reload of both the user directory and the Dux Shell cache.
//!
//! gpui-kit's own `ThemeRegistry` drops themes loaded from strings whenever its directory
//! watcher reloads, so Osier keeps its own list and applies configs directly to the `Theme`.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{Theme, ThemeConfig, ThemeSet};
use gpui_kit::{App, Global, Pixels, SharedString, px};
use notify::Watcher as _;
use osier_config::settings::Appearance;
use osier_theme::Mode;

/// Bundled themes, first one wins on duplicate names.
const BUNDLED: &[&str] = &[
    include_str!("../../../themes/osier-catppuccin.json"),
    include_str!("../../../themes/catppuccin.json"),
    include_str!("../../../themes/osier-classics.json"),
];
pub const DUX_DARK: &str = "Dux Shell Dark";
pub const DUX_LIGHT: &str = "Dux Shell Light";

/// Where the Dux Shell colour file comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuxSource {
    /// `$XDG_CACHE_HOME/dms/dms-colors.json` with the legacy fallback.
    Auto,
    /// A fixed file (tests).
    #[cfg(test)]
    File(PathBuf),
    /// Never look (tests, so a developer's own Dux Shell cache cannot leak in).
    Disabled,
}

pub struct ThemeStore {
    themes: Vec<Rc<ThemeConfig>>,
    user_dir: PathBuf,
    dux_source: DuxSource,
    dux_mode: Option<Mode>,
    dirty: Arc<AtomicBool>,
    /// Fonts gpui-kit resolved at startup, restored before each theme is applied so a theme
    /// that sets fonts does not leak them into the next one.
    base_fonts: Option<(SharedString, SharedString, Pixels, Pixels)>,
    _watchers: Vec<notify::RecommendedWatcher>,
}

impl Global for ThemeStore {}

impl ThemeStore {
    /// Install the global store once (the app and the headless tests both call this).
    pub fn ensure(user_dir: PathBuf, watch: bool, cx: &mut App) {
        if cx.has_global::<Self>() {
            return;
        }
        let source = if cfg!(test) {
            DuxSource::Disabled
        } else {
            DuxSource::Auto
        };
        cx.set_global(Self::new(user_dir, source, watch && !cfg!(test)));
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    pub fn global_mut(cx: &mut App) -> &mut Self {
        cx.global_mut::<Self>()
    }

    pub fn new(user_dir: PathBuf, dux_source: DuxSource, watch: bool) -> Self {
        if let Err(error) = std::fs::create_dir_all(&user_dir) {
            tracing::warn!(%error, dir = %user_dir.display(), "cannot create the themes directory");
        }
        let mut store = Self {
            themes: Vec::new(),
            user_dir,
            dux_source,
            dux_mode: None,
            dirty: Arc::new(AtomicBool::new(false)),
            base_fonts: None,
            _watchers: Vec::new(),
        };
        store.reload();
        if watch {
            store.start_watchers();
        }
        store
    }

    fn dux_file(&self) -> Option<PathBuf> {
        match &self.dux_source {
            DuxSource::Auto => osier_theme::dux::color_file(),
            #[cfg(test)]
            DuxSource::File(p) => Some(p.clone()),
            DuxSource::Disabled => None,
        }
    }

    /// Point the store at a specific Dux Shell colour file (tests).
    #[cfg(test)]
    pub fn set_dux_source(&mut self, source: DuxSource) {
        self.dux_source = source;
        self.reload();
    }

    /// Re-read everything: user files first, then Dux Shell, then bundled.
    pub fn reload(&mut self) {
        let mut themes: Vec<Rc<ThemeConfig>> = Vec::new();
        let mut push = |source: &str, origin: &str| -> bool {
            match serde_json::from_str::<ThemeSet>(source) {
                Ok(set) => {
                    for theme in set.themes {
                        if !themes.iter().any(|t| t.name == theme.name) {
                            themes.push(Rc::new(theme));
                        }
                    }
                    true
                }
                Err(error) => {
                    tracing::warn!(%error, origin, "ignored invalid theme file");
                    false
                }
            }
        };
        for path in user_theme_files(&self.user_dir) {
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    push(&text, &path.display().to_string());
                }
                Err(error) => tracing::warn!(%error, path = %path.display(), "cannot read theme"),
            }
        }
        self.dux_mode = None;
        if let Some(path) = self.dux_file()
            && let Some((doc, mode)) = osier_theme::dux::load_from(&path)
            && push(&doc, "dux shell")
        {
            self.dux_mode = Some(mode);
        }
        for source in BUNDLED {
            push(source, "bundled");
        }
        self.themes = themes;
    }

    fn start_watchers(&mut self) {
        let mut dirs = vec![self.user_dir.clone()];
        for candidate in osier_theme::dux::color_file_candidates() {
            if let Some(parent) = candidate.parent()
                && parent.is_dir()
            {
                dirs.push(parent.to_path_buf());
            }
        }
        for dir in dirs {
            let flag = self.dirty.clone();
            let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res
                    && matches!(
                        event.kind,
                        notify::EventKind::Any
                            | notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    )
                    && event.paths.iter().any(|p| is_theme_input(p))
                {
                    flag.store(true, Ordering::Relaxed);
                }
            });
            match watcher {
                Ok(mut watcher) => match watcher.watch(&dir, notify::RecursiveMode::NonRecursive) {
                    Ok(()) => self._watchers.push(watcher),
                    Err(error) => {
                        tracing::warn!(%error, dir = %dir.display(), "cannot watch theme directory")
                    }
                },
                Err(error) => tracing::warn!(%error, "cannot create theme watcher"),
            }
        }
    }

    /// True once after any watched theme file changed.
    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::Relaxed)
    }

    pub fn get(&self, name: &str) -> Option<Rc<ThemeConfig>> {
        self.themes
            .iter()
            .find(|t| t.name.as_ref() == name)
            .cloned()
    }

    /// Theme names for one mode, Dux Shell excluded (it has its own toggle), sorted.
    pub fn names(&self, dark: bool) -> Vec<SharedString> {
        let mut names: Vec<SharedString> = self
            .themes
            .iter()
            .filter(|t| t.mode.is_dark() == dark)
            .filter(|t| t.name.as_ref() != DUX_DARK && t.name.as_ref() != DUX_LIGHT)
            .map(|t| t.name.clone())
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
    }

    pub fn dux_available(&self) -> bool {
        self.dux_mode.is_some()
    }

    /// The mode Dux Shell is currently in, when its colour file was found.
    pub fn dux_mode(&self) -> Option<Mode> {
        self.dux_mode
    }
}

fn is_theme_input(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.ends_with(".json"),
        None => false,
    }
}

fn user_theme_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    files
}

/// Apply a theme by name, then the user's font overrides. False when the name is unknown.
/// Whether Higher contrast is on, for views that draw their own tints (badges).
#[derive(Default)]
pub struct HighContrast(pub bool);

impl Global for HighContrast {}

pub fn high_contrast_on(cx: &App) -> bool {
    cx.try_global::<HighContrast>().is_some_and(|h| h.0)
}

/// Background and border of a primary-tinted badge (ref labels, the palette badge): faint as
/// the theme intends, or opaque and stronger with Higher contrast.
pub fn badge_tint(cx: &App) -> (gpui_kit::Hsla, gpui_kit::Hsla) {
    let theme = gpui_kit::component::ActiveTheme::theme(cx);
    if high_contrast_on(cx) {
        (
            theme.background.blend(theme.primary.opacity(0.42)),
            theme.primary,
        )
    } else {
        (theme.primary.opacity(0.18), theme.primary.opacity(0.32))
    }
}

pub fn apply(name: &str, appearance: &Appearance, cx: &mut App) -> bool {
    let Some(config) = ThemeStore::global(cx).get(name) else {
        return false;
    };
    cx.set_global(HighContrast(appearance.high_contrast));
    let base = ThemeStore::global(cx)
        .base_fonts
        .clone()
        .unwrap_or_else(|| {
            let theme = Theme::global(cx);
            (
                theme.font_family.clone(),
                theme.mono_font_family.clone(),
                theme.font_size,
                theme.mono_font_size,
            )
        });
    ThemeStore::global_mut(cx).base_fonts = Some(base.clone());
    // A font the system does not have would make gpui log a warning per glyph run and fall
    // back anyway, so only honour families that are installed.
    let installed = cx.text_system().all_font_names();
    let known = |name: &str| installed.iter().any(|f| f.eq_ignore_ascii_case(name));
    let ui_font = appearance
        .ui_font
        .clone()
        .or_else(|| config.font_family.as_ref().map(|f| f.to_string()))
        .filter(|f| known(f));
    let mono_font = appearance
        .mono_font
        .clone()
        .or_else(|| config.mono_font_family.as_ref().map(|f| f.to_string()))
        .filter(|f| known(f));
    Theme::update(cx, |theme| {
        theme.font_family = base.0.clone();
        theme.mono_font_family = base.1.clone();
        theme.font_size = base.2;
        theme.mono_font_size = base.3;
        theme.apply_config(&config);
        theme.font_family = ui_font.map(SharedString::from).unwrap_or(base.0.clone());
        theme.mono_font_family = mono_font.map(SharedString::from).unwrap_or(base.1.clone());
        if let Some(size) = appearance.font_size {
            theme.font_size = px(size);
            theme.mono_font_size = px(size);
        }
        if appearance.high_contrast {
            high_contrast(theme);
        }
        if appearance.syntax_contrast {
            syntax_contrast(theme);
        }
    });
    // The theme is a global: views do not hear about it, and the main view is cached
    // (`cached_root.rs`), so redraw every window in full.
    cx.refresh_windows();
    true
}

/// Contrast floor for syntax colours against the editor background: WCAG AA for body text.
/// Comments included: the added-line tint of a diff costs about 0.8 on top, and the
/// Catppuccin comment grey starts at 3.4 on Mocha and 2.3 on Latte.
pub(crate) const SYNTAX_CONTRAST: f32 = 4.5;

/// Lift every syntax colour under its floor away from the background: lighter on a dark
/// theme, darker on a light one. Hue and saturation stay, so the theme still looks like
/// itself. Themes map comments to a UI grey meant for borders, which is where it bites.
fn syntax_contrast(theme: &mut Theme) {
    let bg = theme
        .highlight_theme
        .style
        .editor_background
        .unwrap_or(theme.background);
    // `ThemeStyle`'s fields are private: go through its JSON form.
    let Ok(mut value) = serde_json::to_value(&*theme.highlight_theme) else {
        return;
    };
    let Some(syntax) = value
        .pointer_mut("/style/syntax")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    for style in syntax.values_mut() {
        let Some(hex) = style.get("color").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(rgba) = gpui_kit::Rgba::try_from(hex) else {
            continue;
        };
        let lifted = with_contrast(rgba.into(), bg, SYNTAX_CONTRAST);
        if let Ok(json) = serde_json::to_value(lifted) {
            style["color"] = json;
        }
    }
    if let Ok(highlight) = serde_json::from_value(value) {
        theme.highlight_theme = std::sync::Arc::new(highlight);
    }
}

/// WCAG relative luminance.
fn luminance(color: gpui_kit::Hsla) -> f32 {
    let rgb = color.to_rgb();
    let channel = |v: f32| {
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
}

/// WCAG contrast ratio, 1 to 21.
pub(crate) fn contrast(a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `color` moved in lightness away from `background` until it reaches `floor`, or as far as
/// it can go. Unchanged when it already passes.
pub(crate) fn with_contrast(
    color: gpui_kit::Hsla,
    background: gpui_kit::Hsla,
    floor: f32,
) -> gpui_kit::Hsla {
    let lighten = luminance(background) < 0.5;
    let mut lifted = color;
    while contrast(lifted, background) < floor {
        let next = if lighten {
            (lifted.l + 0.02).min(1.)
        } else {
            (lifted.l - 0.02).max(0.)
        };
        if next == lifted.l {
            break;
        }
        lifted.l = next;
    }
    lifted
}

/// Rebuild the faint selection and hover colours from the primary colour, mixed in more
/// strongly. Opaque mixes, so nothing shows through.
fn high_contrast(theme: &mut Theme) {
    let primary = theme.primary;
    let bg = theme.background;
    let mix = |base: gpui_kit::Hsla, amount: f32| base.blend(primary.opacity(amount));
    // Widgets draw from `tokens`; `Theme::update` reconciles the two copies by comparing
    // with the state before the edit, which is already high-contrast on a re-apply. Set
    // both so neither side wins with the theme file's value.
    macro_rules! set {
        ($($field:ident = $value:expr;)+) => {$(
            let value = $value;
            theme.$field = value;
            theme.tokens.$field = value.into();
        )+};
    }
    let sidebar = theme.sidebar;
    set! {
        list_active = mix(bg, 0.55);
        list_active_border = primary;
        list_hover = mix(bg, 0.24);
        table_active = mix(bg, 0.55);
        table_hover = mix(bg, 0.24);
        accent = mix(bg, 0.42);
        sidebar_accent = mix(sidebar, 0.58);
        sidebar_border = mix(sidebar, 0.35);
    }
}

#[cfg(test)]
mod contrast_tests {
    use super::{SYNTAX_CONTRAST, contrast, with_contrast};
    use gpui_kit::{Hsla, Rgba};

    fn hsla(hex: &str) -> Hsla {
        Rgba::try_from(hex).expect("hex").into()
    }

    #[test]
    fn dim_colours_are_lifted_to_the_floor_and_keep_their_hue() {
        // Catppuccin Mocha: comment grey on base reads 3.4:1 on its own and 1.2:1 inside a
        // changed word; the floor lifts it.
        let base = hsla("#1e1e2e");
        let comment = hsla("#6c7086");
        let lifted = with_contrast(comment, base, SYNTAX_CONTRAST);
        assert!(contrast(lifted, base) >= SYNTAX_CONTRAST);
        assert!((lifted.h - comment.h).abs() < 0.001, "hue kept");
        assert!(lifted.l > comment.l, "lighter on a dark theme");
        // Latte: the same grey on a light base goes darker instead.
        let light = hsla("#eff1f5");
        let darkened = with_contrast(hsla("#9ca0b0"), light, SYNTAX_CONTRAST);
        assert!(contrast(darkened, light) >= SYNTAX_CONTRAST);
        assert!(darkened.l < hsla("#9ca0b0").l);
    }

    #[test]
    fn colours_that_already_pass_are_untouched() {
        let base = hsla("#1e1e2e");
        let green = hsla("#a6e3a1");
        assert_eq!(with_contrast(green, base, 4.5), green);
    }

    #[test]
    fn a_colour_that_cannot_reach_the_floor_stops_at_the_end() {
        // Pure white on near-white: darkening ends at black, which does pass; a mid grey on a
        // mid grey background can only go so far.
        let grey = hsla("#808080");
        let far = with_contrast(grey, grey, 21.);
        assert!(far.l == 0. || far.l == 1.);
    }
}

#[cfg(test)]
mod tests {
    use super::{DuxSource, ThemeStore};

    #[test]
    fn user_files_override_bundled_and_invalid_files_are_skipped() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("mine.json"),
            r##"{"name":"Mine","themes":[{"name":"My Dark","mode":"dark","colors":{"background":"#000000"}},
                 {"name":"Catppuccin Mocha","mode":"dark","colors":{"background":"#010203"}}]}"##,
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.json"), "{ nope").unwrap();
        let store = ThemeStore::new(dir.path().to_path_buf(), DuxSource::Disabled, false);
        assert!(store.get("My Dark").is_some());
        assert!(
            store.get("Catppuccin Latte").is_some(),
            "bundled still present"
        );
        let mocha = store.get("Catppuccin Mocha").unwrap();
        assert_eq!(
            mocha.colors.background.as_deref(),
            Some("#010203"),
            "user file wins"
        );
        assert!(store.names(true).iter().any(|n| n.as_ref() == "My Dark"));
        assert!(!store.dux_available());
        assert!(!store.take_dirty());
    }
}

#[cfg(test)]
mod parse_tests {
    use gpui_kit::component::ThemeSet;

    #[test]
    fn generated_documents_parse_as_gpui_kit_themes() {
        for (origin, source) in [
            ("osier-catppuccin", super::BUNDLED[0].to_string()),
            ("upstream-catppuccin", super::BUNDLED[1].to_string()),
            ("osier-classics", super::BUNDLED[2].to_string()),
        ] {
            let set = serde_json::from_str::<ThemeSet>(&source)
                .unwrap_or_else(|e| panic!("{origin}: {e}"));
            assert!(!set.themes.is_empty(), "{origin} has themes");
        }
    }
}

/// Apply the user's saved theme without watching anything: for the short-lived windows
/// (askpass, `osier merge`, `osier diff`). Mode "system" counts as dark.
pub(crate) fn apply_saved(cx: &mut gpui_kit::App) {
    if let Ok(paths) = osier_config::Paths::discover() {
        let settings = osier_config::Settings::load(&paths).unwrap_or_default();
        ThemeStore::ensure(paths.themes_dir(), false, cx);
        let a = &settings.appearance;
        let dark = a.mode != osier_config::settings::ThemeMode::Light;
        let name = if dark { &a.dark_theme } else { &a.light_theme };
        if !apply(name, a, cx) {
            let fallback = if dark {
                "Catppuccin Mocha"
            } else {
                "Catppuccin Latte"
            };
            apply(fallback, a, cx);
        }
    }
}
