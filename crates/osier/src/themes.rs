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
pub fn apply(name: &str, appearance: &Appearance, cx: &mut App) -> bool {
    let Some(config) = ThemeStore::global(cx).get(name) else {
        return false;
    };
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
    });
    true
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
