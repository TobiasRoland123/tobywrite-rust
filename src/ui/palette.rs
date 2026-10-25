use serde::Deserialize;
use std::{
    env,
    fs::{self, File},
    io::Read,
    path::PathBuf,
};

const MAX_COLORS_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Palette {
    pub dark: bool,
    pub background: String,
    pub foreground: String,
    pub muted: String,
    pub border: String,
    pub selection: String,
    pub selection_foreground: String,
    pub accent: String,
    pub accent_foreground: String,
    pub code_background: String,
    pub quote: String,
    pub focus: String,
}

impl Palette {
    pub fn fallback(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                background: "#232421".into(),
                foreground: "#d7d7ce".into(),
                muted: "#96988d".into(),
                border: "#3b3d36".into(),
                selection: "#484f43".into(),
                selection_foreground: "#ffffff".into(),
                accent: "#bdcaa8".into(),
                accent_foreground: "#232421".into(),
                code_background: "#2e302a".into(),
                quote: "#b1b6a2".into(),
                focus: "#96988d".into(),
            }
        } else {
            Self {
                dark,
                background: "#f8f7f2".into(),
                foreground: "#353731".into(),
                muted: "#62665c".into(),
                border: "#d0d1c8".into(),
                selection: "#dce5d2".into(),
                selection_foreground: "#353731".into(),
                accent: "#596b43".into(),
                accent_foreground: "#ffffff".into(),
                code_background: "#eeeee6".into(),
                quote: "#596b43".into(),
                focus: "#62665c".into(),
            }
        }
    }

    pub fn from_toml(source: &str, legacy_light: bool) -> Option<Self> {
        let colors: ThemeColors = toml::from_str(source).ok()?;
        let background = first_color([
            colors.background.as_deref(),
            colors.bg.as_deref(),
            colors.color0.as_deref(),
        ])?;
        let raw_foreground = first_color([
            colors.foreground.as_deref(),
            colors.fg.as_deref(),
            colors.color7.as_deref(),
        ])?;

        let declared_mode = colors
            .mode
            .as_deref()
            .and_then(parse_mode)
            .or_else(|| colors.theme_type.as_deref().and_then(parse_mode));
        let dark = declared_mode.unwrap_or_else(|| {
            if legacy_light {
                false
            } else {
                !is_light_background(&background)
            }
        });

        let fallback = Self::fallback(dark);
        let foreground = readable_color(&raw_foreground, &background, 4.5);
        let raw_muted = first_color([
            colors.muted.as_deref(),
            colors.dark_foreground.as_deref(),
            colors.dark_fg.as_deref(),
            colors.light_foreground.as_deref(),
            colors.light_fg.as_deref(),
            colors.color8.as_deref(),
        ])
        .unwrap_or_else(|| fallback.muted.clone());
        let muted = readable_color(&raw_muted, &background, 4.5);
        let selection = optional_color(colors.selection.as_deref())
            .or_else(|| optional_color(colors.selection_background.as_deref()))
            .unwrap_or_else(|| mix(&background, &fallback.accent, 0.24));
        let accent = optional_color(colors.accent.as_deref())
            .or_else(|| optional_color(colors.blue.as_deref()))
            .or_else(|| optional_color(colors.green.as_deref()))
            .unwrap_or_else(|| fallback.accent.clone());
        let accent = readable_color(&accent, &background, 4.5);
        let code_background =
            code_surface(&background, &foreground, if dark { 0.12 } else { 0.08 });
        let raw_quote = first_color([
            colors.cyan.as_deref(),
            colors.green.as_deref(),
            colors.light_foreground.as_deref(),
            colors.light_fg.as_deref(),
        ])
        .unwrap_or_else(|| fallback.quote.clone());
        let border = optional_color(colors.muted.as_deref())
            .or_else(|| optional_color(colors.dark_foreground.as_deref()))
            .or_else(|| optional_color(colors.dark_fg.as_deref()))
            .unwrap_or_else(|| fallback.border.clone());
        let selection_foreground = readable_color(
            optional_color(colors.selection_foreground.as_deref())
                .as_deref()
                .unwrap_or(&foreground),
            &selection,
            4.5,
        );
        let accent_foreground = readable_color(&foreground, &accent, 4.5);
        let quote = readable_color(&raw_quote, &background, 4.5);
        let focus = readable_on_surfaces(
            &raw_muted,
            &[&background, &code_background],
            3.0,
            &foreground,
        );

        Some(Self {
            dark,
            background,
            foreground,
            muted: muted.clone(),
            border,
            selection: selection.clone(),
            selection_foreground,
            accent_foreground,
            accent,
            code_background,
            quote,
            focus,
        })
    }
}

#[derive(Deserialize)]
struct ThemeColors {
    mode: Option<String>,
    theme_type: Option<String>,
    background: Option<String>,
    bg: Option<String>,
    foreground: Option<String>,
    fg: Option<String>,
    dark_fg: Option<String>,
    light_fg: Option<String>,
    color0: Option<String>,
    color7: Option<String>,
    color8: Option<String>,
    muted: Option<String>,
    dark_foreground: Option<String>,
    light_foreground: Option<String>,
    selection: Option<String>,
    selection_background: Option<String>,
    selection_foreground: Option<String>,
    accent: Option<String>,
    blue: Option<String>,
    green: Option<String>,
    cyan: Option<String>,
}

pub(super) fn theme_paths() -> Vec<PathBuf> {
    if let Some(path) = env::var_os("TOBYWRITE_THEME_DIR") {
        return vec![PathBuf::from(path)];
    }

    let home = env::var_os("HOME").map(PathBuf::from);
    let mut paths = Vec::new();
    if let Some(home) = &home {
        paths.push(home.join(".local/state/omarchy/current/theme"));
    }
    if let Some(state_home) = env::var_os("XDG_STATE_HOME") {
        paths.push(PathBuf::from(state_home).join("omarchy/current/theme"));
    }
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        paths.push(PathBuf::from(config_home).join("omarchy/current/theme"));
    }
    if let Some(home) = home {
        paths.push(home.join(".config/omarchy/current/theme"));
    }
    deduplicate(paths)
}

pub(super) struct ThemeSource {
    candidates: Vec<PathBuf>,
    selected: Option<PathBuf>,
}

impl ThemeSource {
    pub fn new(candidates: Vec<PathBuf>) -> Self {
        Self {
            candidates,
            selected: None,
        }
    }

    pub fn read(&mut self) -> Option<Palette> {
        // Once discovered, a temporarily missing current theme must not switch
        // us to a stale palette left at an older Omarchy location.
        let paths = self
            .selected
            .as_ref()
            .map(std::slice::from_ref)
            .unwrap_or(&self.candidates);
        let (directory, palette) = read_theme(paths)?;
        self.selected = Some(directory);
        Some(palette)
    }
}

fn read_theme(paths: &[PathBuf]) -> Option<(PathBuf, Palette)> {
    for directory in paths {
        let file = directory.join("colors.toml");
        let Ok(metadata) = fs::metadata(&file) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > MAX_COLORS_FILE_BYTES {
            return None;
        }
        let mut source = String::new();
        File::open(&file)
            .ok()?
            .take(MAX_COLORS_FILE_BYTES + 1)
            .read_to_string(&mut source)
            .ok()?;
        if source.len() as u64 > MAX_COLORS_FILE_BYTES {
            return None;
        }
        let legacy_light = directory.join("light.mode").is_file();
        return Palette::from_toml(&source, legacy_light)
            .map(|palette| (directory.clone(), palette));
    }
    None
}

fn deduplicate(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn parse_mode(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

fn code_surface(background: &str, foreground: &str, preferred_mix: f64) -> String {
    let mut low = 0.0;
    let mut high = preferred_mix;
    for _ in 0..24 {
        let amount = (low + high) / 2.0;
        let candidate = mix(background, foreground, amount);
        if contrast(&candidate, foreground) >= 4.5 {
            low = amount;
        } else {
            high = amount;
        }
    }
    mix(background, foreground, low)
}

fn readable_on_surfaces(
    color: &str,
    backgrounds: &[&str],
    minimum: f64,
    safe_fallback: &str,
) -> String {
    if backgrounds
        .iter()
        .all(|background| contrast(color, background) >= minimum)
    {
        return color.to_owned();
    }

    let mut best: Option<(String, f64)> = None;
    for toward in ["#000000", "#ffffff"] {
        if backgrounds
            .iter()
            .any(|background| contrast(toward, background) < minimum)
        {
            continue;
        }
        let mut low = 0.0;
        let mut high = 1.0;
        for _ in 0..24 {
            let amount = (low + high) / 2.0;
            let candidate = mix(color, toward, amount);
            if backgrounds
                .iter()
                .all(|background| contrast(&candidate, background) >= minimum)
            {
                high = amount;
            } else {
                low = amount;
            }
        }
        let candidate = mix(color, toward, high);
        if best.as_ref().is_none_or(|(_, amount)| high < *amount) {
            best = Some((candidate, high));
        }
    }
    best.map(|(candidate, _)| candidate)
        .unwrap_or_else(|| safe_fallback.to_owned())
}

fn first_color<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find_map(|value| optional_color(Some(value)))
}

fn optional_color(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn parse_rgb(color: &str) -> [u8; 3] {
    [
        u8::from_str_radix(&color[1..3], 16).expect("validated RGB color"),
        u8::from_str_radix(&color[3..5], 16).expect("validated RGB color"),
        u8::from_str_radix(&color[5..7], 16).expect("validated RGB color"),
    ]
}

fn is_light_background(color: &str) -> bool {
    let [red, green, blue] = parse_rgb(color);
    u16::from(red) + u16::from(green) + u16::from(blue) > 382
}

fn luminance(color: &str) -> f64 {
    let [red, green, blue] = parse_rgb(color);
    let linear = |component: u8| {
        let value = f64::from(component) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue)
}

fn contrast(first: &str, second: &str) -> f64 {
    let first = luminance(first);
    let second = luminance(second);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}

fn readable_color(color: &str, background: &str, minimum: f64) -> String {
    if contrast(color, background) >= minimum {
        return color.to_owned();
    }
    adjust_contrast(color, background, minimum)
}

fn adjust_contrast(color: &str, background: &str, minimum: f64) -> String {
    let toward = if contrast("#000000", background) >= contrast("#ffffff", background) {
        "#000000"
    } else {
        "#ffffff"
    };
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..24 {
        let amount = (low + high) / 2.0;
        let candidate = mix(color, toward, amount);
        if contrast(&candidate, background) >= minimum {
            high = amount;
        } else {
            low = amount;
        }
    }
    mix(color, toward, high)
}

fn mix(first: &str, second: &str, amount: f64) -> String {
    let first = parse_rgb(first);
    let second = parse_rgb(second);
    let channel = |index| {
        (f64::from(first[index]) * (1.0 - amount) + f64::from(second[index]) * amount).round() as u8
    };
    format!("#{:02x}{:02x}{:02x}", channel(0), channel(1), channel(2))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const DARK_FIXTURE: &str = r##"
mode = "dark"
accent = "#faa968"
selection = "#134e5a"
muted = "#2a6b78"
background = "#05182e"
foreground = "#f6dcac"
dark_foreground = "#3f8f8a"
light_foreground = "#a7c9c6"
lighter_background = "#0a2540"
dark_background = "#031222"
cyan = "#8cbfb8"
blue = "#3f8f8a"
green = "#028391"
"##;

    const LIGHT_FIXTURE: &str = r##"
mode = "light"
accent = "#56949f"
selection = "#dfdad9"
muted = "#cecacd"
background = "#faf4ed"
foreground = "#575279"
dark_foreground = "#9893a5"
light_foreground = "#6e6a86"
dark_background = "#ede7e1"
lighter_background = "#f2e9e1"
cyan = "#d7827e"
blue = "#56949f"
green = "#286983"
"##;

    #[test]
    fn parses_dark_and_light_theme_fixtures_with_readable_roles() {
        let dark = Palette::from_toml(DARK_FIXTURE, false).unwrap();
        let light = Palette::from_toml(LIGHT_FIXTURE, false).unwrap();
        assert!(dark.dark);
        assert!(!light.dark);
        for palette in [dark, light] {
            assert!(contrast(&palette.foreground, &palette.background) >= 4.5);
            assert!(contrast(&palette.muted, &palette.background) >= 4.5);
            assert!(contrast(&palette.focus, &palette.background) >= 3.0);
            assert!(contrast(&palette.quote, &palette.background) >= 4.5);
            assert!(contrast(&palette.foreground, &palette.code_background) >= 4.5);
            assert!(contrast(&palette.accent, &palette.background) >= 4.5);
            assert!(contrast(&palette.focus, &palette.code_background) >= 3.0);
            assert!(contrast(&palette.selection_foreground, &palette.selection) >= 4.5);
            assert!(contrast(&palette.accent_foreground, &palette.accent) >= 4.5);
        }
    }

    #[test]
    fn supports_legacy_aliases_and_infers_mode_like_omarchy() {
        let light = Palette::from_toml(
            "theme_type = 'light'\nbg = '#ffffff'\nfg = '#777777'\ncolor8 = '#999999'",
            false,
        )
        .unwrap();
        assert!(!light.dark);
        assert_eq!(light.background, "#ffffff");

        let dark = Palette::from_toml(
            "color0 = '#101010'\ncolor7 = '#eeeeee'\ncolor8 = '#aaaaaa'",
            false,
        )
        .unwrap();
        assert!(dark.dark);

        let marked_light =
            Palette::from_toml("background = '#202020'\nforeground = '#ffffff'", true).unwrap();
        assert!(!marked_light.dark);
    }

    #[test]
    fn malformed_or_incomplete_palettes_are_rejected() {
        assert!(Palette::from_toml("background = [", false).is_none());
        assert!(Palette::from_toml("background = '#fff'\nforeground = '#ffffff'", false).is_none());
        assert!(Palette::from_toml("background = '#101010'\nforeground = 'red'", false).is_none());

        let palette = Palette::from_toml(
            "background = '#101010'\nforeground = '#ffffff'\naccent = 'url(x)'\nselection = '#bad'",
            false,
        )
        .unwrap();
        assert_eq!(palette.accent, Palette::fallback(true).accent);
        assert_ne!(palette.selection, "#bad");
    }

    #[test]
    fn reads_current_directory_after_theme_replacement_and_rejects_invalid_file() {
        let root = tempdir().unwrap();
        let theme = root.path().join("theme");
        fs::create_dir(&theme).unwrap();
        fs::write(
            theme.join("colors.toml"),
            "background = '#101010'\nforeground = '#ffffff'\n",
        )
        .unwrap();
        assert!(read_theme(std::slice::from_ref(&theme)).unwrap().1.dark);

        let old = root.path().join("old-theme");
        fs::rename(&theme, &old).unwrap();
        fs::create_dir(&theme).unwrap();
        fs::write(
            theme.join("colors.toml"),
            "background = '#ffffff'\nforeground = '#000000'\n",
        )
        .unwrap();
        assert!(!read_theme(std::slice::from_ref(&theme)).unwrap().1.dark);

        fs::write(theme.join("colors.toml"), "background = [").unwrap();
        assert!(read_theme(std::slice::from_ref(&theme)).is_none());
    }

    #[test]
    fn invalid_primary_palette_does_not_fall_through_to_stale_candidate() {
        let root = tempdir().unwrap();
        let current = root.path().join("current");
        let stale = root.path().join("stale");
        fs::create_dir(&current).unwrap();
        fs::create_dir(&stale).unwrap();
        fs::write(current.join("colors.toml"), "background = [").unwrap();
        fs::write(
            stale.join("colors.toml"),
            "background = '#101010'\nforeground = '#ffffff'\n",
        )
        .unwrap();
        assert!(read_theme(&[current, stale]).is_none());
    }

    #[test]
    fn missing_selected_source_does_not_load_stale_legacy_palette() {
        let root = tempdir().unwrap();
        let current = root.path().join("current");
        let legacy = root.path().join("legacy");
        fs::create_dir(&current).unwrap();
        fs::create_dir(&legacy).unwrap();
        fs::write(current.join("colors.toml"), DARK_FIXTURE).unwrap();
        fs::write(legacy.join("colors.toml"), LIGHT_FIXTURE).unwrap();
        let mut source = ThemeSource::new(vec![current.clone(), legacy]);
        assert!(source.read().unwrap().dark);

        fs::remove_dir_all(&current).unwrap();
        assert!(source.read().is_none());

        fs::create_dir(&current).unwrap();
        fs::write(current.join("colors.toml"), LIGHT_FIXTURE).unwrap();
        assert!(!source.read().unwrap().dark);
    }

    #[test]
    fn discovers_theme_created_after_startup() {
        let root = tempdir().unwrap();
        let theme = root.path().join("theme");
        let mut source = ThemeSource::new(vec![theme.clone()]);
        assert!(source.read().is_none());

        fs::create_dir(&theme).unwrap();
        fs::write(theme.join("colors.toml"), DARK_FIXTURE).unwrap();
        assert!(source.read().unwrap().dark);
    }

    #[test]
    fn rejects_oversized_palette_files() {
        let root = tempdir().unwrap();
        fs::write(
            root.path().join("colors.toml"),
            vec![b' '; MAX_COLORS_FILE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(read_theme(&[root.path().to_path_buf()]).is_none());
    }
}
