//! Popular editor themes as Osier palettes: One Dark/Light, Monokai, Dracula, Nord, Gruvbox,
//! Solarized and Tokyo Night. Colours follow each scheme's published palette; the remaining
//! roles (surfaces, subtext) are picked from the same family.

use crate::palette::{Mode, Palette};

#[allow(clippy::too_many_arguments)]
fn palette(
    name: &str,
    mode: Mode,
    [base, mantle, crust]: [&str; 3],
    [surface0, surface1, surface2]: [&str; 3],
    [text, subtext, overlay]: [&str; 3],
    [primary, on_primary, link]: [&str; 3],
    [red, maroon, green, yellow, peach, blue, cyan, magenta]: [&str; 8],
) -> Palette {
    Palette {
        name: name.into(),
        mode,
        base: base.into(),
        mantle: mantle.into(),
        crust: crust.into(),
        surface0: surface0.into(),
        surface1: surface1.into(),
        surface2: surface2.into(),
        text: text.into(),
        subtext: subtext.into(),
        overlay: overlay.into(),
        primary: primary.into(),
        on_primary: on_primary.into(),
        link: link.into(),
        red: red.into(),
        maroon: maroon.into(),
        green: green.into(),
        yellow: yellow.into(),
        peach: peach.into(),
        blue: blue.into(),
        cyan: cyan.into(),
        magenta: magenta.into(),
        font_family: None,
        mono_font_family: None,
    }
}

pub fn all() -> Vec<Palette> {
    use Mode::{Dark, Light};
    vec![
        palette(
            "One Dark",
            Dark,
            ["#282c34", "#21252b", "#181a1f"],
            ["#2c313a", "#3e4451", "#4b5263"],
            ["#abb2bf", "#9da5b4", "#5c6370"],
            ["#61afef", "#282c34", "#61afef"],
            [
                "#e06c75", "#be5046", "#98c379", "#e5c07b", "#d19a66", "#61afef", "#56b6c2",
                "#c678dd",
            ],
        ),
        palette(
            "One Light",
            Light,
            ["#fafafa", "#eaeaeb", "#dbdbdc"],
            ["#e5e5e6", "#d3d3d4", "#c2c2c3"],
            ["#383a42", "#696c77", "#a0a1a7"],
            ["#4078f2", "#fafafa", "#4078f2"],
            [
                "#e45649", "#ca1243", "#50a14f", "#c18401", "#986801", "#4078f2", "#0184bc",
                "#a626a4",
            ],
        ),
        palette(
            "Monokai",
            Dark,
            ["#272822", "#1e1f1c", "#171814"],
            ["#3e3d32", "#49483e", "#5c5b50"],
            ["#f8f8f2", "#cfcfc2", "#75715e"],
            ["#66d9ef", "#272822", "#66d9ef"],
            [
                "#f92672", "#f92672", "#a6e22e", "#e6db74", "#fd971f", "#66d9ef", "#a1efe4",
                "#ae81ff",
            ],
        ),
        palette(
            "Dracula",
            Dark,
            ["#282a36", "#21222c", "#191a21"],
            ["#343746", "#44475a", "#565761"],
            ["#f8f8f2", "#c5c8d6", "#6272a4"],
            ["#bd93f9", "#282a36", "#8be9fd"],
            [
                "#ff5555", "#ff6e6e", "#50fa7b", "#f1fa8c", "#ffb86c", "#8be9fd", "#8be9fd",
                "#ff79c6",
            ],
        ),
        palette(
            "Nord",
            Dark,
            ["#2e3440", "#272c36", "#20242c"],
            ["#3b4252", "#434c5e", "#4c566a"],
            ["#e5e9f0", "#d8dee9", "#616e88"],
            ["#88c0d0", "#2e3440", "#81a1c1"],
            [
                "#bf616a", "#bf616a", "#a3be8c", "#ebcb8b", "#d08770", "#81a1c1", "#8fbcbb",
                "#b48ead",
            ],
        ),
        palette(
            "Gruvbox Dark",
            Dark,
            ["#282828", "#1d2021", "#141617"],
            ["#32302f", "#3c3836", "#504945"],
            ["#ebdbb2", "#d5c4a1", "#928374"],
            ["#83a598", "#282828", "#83a598"],
            [
                "#fb4934", "#cc241d", "#b8bb26", "#fabd2f", "#fe8019", "#83a598", "#8ec07c",
                "#d3869b",
            ],
        ),
        palette(
            "Gruvbox Light",
            Light,
            ["#fbf1c7", "#f2e5bc", "#e5d5ad"],
            ["#ebdbb2", "#d5c4a1", "#bdae93"],
            ["#3c3836", "#504945", "#928374"],
            ["#076678", "#fbf1c7", "#076678"],
            [
                "#9d0006", "#cc241d", "#79740e", "#b57614", "#af3a03", "#076678", "#427b58",
                "#8f3f71",
            ],
        ),
        palette(
            "Solarized Dark",
            Dark,
            ["#002b36", "#00252f", "#001e26"],
            ["#073642", "#0b4150", "#1d4b56"],
            ["#93a1a1", "#839496", "#586e75"],
            ["#268bd2", "#fdf6e3", "#2aa198"],
            [
                "#dc322f", "#d33682", "#859900", "#b58900", "#cb4b16", "#268bd2", "#2aa198",
                "#6c71c4",
            ],
        ),
        palette(
            "Solarized Light",
            Light,
            ["#fdf6e3", "#eee8d5", "#e3dcc6"],
            ["#e8e1cb", "#ddd6c1", "#c9c2ad"],
            ["#586e75", "#657b83", "#93a1a1"],
            ["#268bd2", "#fdf6e3", "#2aa198"],
            [
                "#dc322f", "#d33682", "#859900", "#b58900", "#cb4b16", "#268bd2", "#2aa198",
                "#6c71c4",
            ],
        ),
        palette(
            "Tokyo Night",
            Dark,
            ["#1a1b26", "#16161e", "#101014"],
            ["#1f2335", "#292e42", "#3b4261"],
            ["#c0caf5", "#a9b1d6", "#565f89"],
            ["#7aa2f7", "#1a1b26", "#7dcfff"],
            [
                "#f7768e", "#db4b4b", "#9ece6a", "#e0af68", "#ff9e64", "#7aa2f7", "#7dcfff",
                "#bb9af7",
            ],
        ),
    ]
}

/// The bundled theme document.
pub fn document() -> String {
    crate::theme_document(
        "Osier Classics",
        "Osier (palettes after One Dark/Light, Monokai, Dracula, Nord, Gruvbox, Solarized and Tokyo Night)",
        &all(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLED: &str = "../../themes/osier-classics.json";

    /// `themes/osier-classics.json` is generated from this crate. Regenerate it with
    /// `cargo test -p osier-theme -- --ignored write_bundled_classics`.
    #[test]
    fn bundled_classics_file_is_up_to_date() {
        let on_disk = std::fs::read_to_string(BUNDLED).expect("bundled file present");
        assert_eq!(
            on_disk.trim_end(),
            document().trim_end(),
            "run the ignored write_bundled_classics test"
        );
    }

    #[test]
    #[ignore = "writes the bundled theme file"]
    fn write_bundled_classics() {
        std::fs::write(BUNDLED, format!("{}\n", document())).unwrap();
    }

    #[test]
    fn names_are_unique_and_colours_are_hex() {
        let all = all();
        let mut names: Vec<&str> = all.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all.len());
        for p in &all {
            for c in [&p.base, &p.text, &p.primary, &p.red, &p.green, &p.magenta] {
                assert!(
                    c.len() == 7
                        && c.starts_with('#')
                        && c[1..].chars().all(|h| h.is_ascii_hexdigit()),
                    "{}: {c}",
                    p.name
                );
            }
        }
    }
}
