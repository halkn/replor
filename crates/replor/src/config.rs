//! The configuration file and the environment variables that override it
//! (`docs/design/p1-core.md`, "設定ファイル").

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use replor_render::diagram::{MermaidMode, MermaidWidth};
use replor_render::flavor::Flavor;
use replor_render::layout::icons::IconSet;
use replor_render::layout::theme::ThemeChoice;

/// `auto` reads the environment; naming a protocol forces it, which is the only
/// way to get images inside a multiplexer such as herdr.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ImagesWhen {
    Auto,
    Kitty,
    Iterm2,
    Never,
}

/// `always` is missing because the TUI always draws in colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum HighlightWhen {
    Auto,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub images: ImagesWhen,
    pub mermaid: MermaidMode,
    pub mermaid_width: MermaidWidth,
    pub theme: ThemeChoice,
    pub highlight: HighlightWhen,
    pub icons: IconSet,
    pub flavor: Flavor,
    /// Unknown keys. A newer replor sharing the same dotfiles may have added
    /// them, so they warn rather than stop an older one from starting.
    pub warnings: Vec<String>,
}

/// The defaults are mdvu's.
impl Default for Settings {
    fn default() -> Self {
        Self {
            images: ImagesWhen::Auto,
            mermaid: MermaidMode::Unicode,
            mermaid_width: MermaidWidth::Unicode,
            theme: ThemeChoice::Auto,
            highlight: HighlightWhen::Auto,
            icons: IconSet::Unicode,
            flavor: Flavor::AzureDevops,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("{origin}: `{key}` cannot be {value:?}; expected one of {expected}")]
    Value {
        origin: String,
        key: &'static str,
        value: String,
        expected: String,
    },
}

/// Where the configuration file is, and whether it must exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// `REPLOR_CONFIG` names it, so a missing file is an error.
    Explicit(PathBuf),
    /// The default place, which may hold nothing.
    Default(PathBuf),
    /// `REPLOR_CONFIG` is empty, or there is no home directory.
    None,
}

/// Reads the environment through `var`, so tests do not depend on the
/// machine they run on.
pub fn location(var: impl Fn(&str) -> Option<String>) -> Location {
    if let Some(path) = var("REPLOR_CONFIG") {
        return match path.is_empty() {
            true => Location::None,
            false => Location::Explicit(PathBuf::from(path)),
        };
    }
    // The same place on every OS, so one dotfile serves all of them.
    let base = match var("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => match var("HOME").filter(|dir| !dir.is_empty()) {
            Some(home) => Path::new(&home).join(".config"),
            None => return Location::None,
        },
    };
    Location::Default(base.join("replor").join("config.toml"))
}

pub fn load(var: impl Fn(&str) -> Option<String>) -> Result<Settings, ConfigError> {
    let text = match location(&var) {
        Location::None => None,
        Location::Explicit(path) => Some((read(&path)?, path)),
        Location::Default(path) => match std::fs::read_to_string(&path) {
            Ok(text) => Some((text, path)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(ConfigError::Read { path, source }),
        },
    };
    let mut settings = Settings::default();
    if let Some((text, path)) = text {
        apply_file(&mut settings, &text, &path)?;
    }
    apply_env(&mut settings, &var)?;
    Ok(settings)
}

fn read(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

const KEYS: &[&str] = &[
    "images",
    "mermaid",
    "mermaid_width",
    "theme",
    "highlight",
    "icons",
    "flavor",
];

fn apply_file(settings: &mut Settings, text: &str, path: &Path) -> Result<(), ConfigError> {
    let table: toml::Table = text
        .parse()
        .map_err(|err: toml::de::Error| ConfigError::Parse {
            path: path.to_path_buf(),
            message: err.message().to_string(),
        })?;
    let origin = path.display().to_string();
    for (key, value) in &table {
        let Some(key) = KEYS.iter().copied().find(|known| known == key) else {
            settings
                .warnings
                .push(format!("{origin}: unknown key `{key}` is ignored"));
            continue;
        };
        match value {
            toml::Value::String(text) => set(settings, key, text, &origin)?,
            other => set(settings, key, other.type_str(), &origin)?,
        }
    }
    Ok(())
}

fn apply_env(
    settings: &mut Settings,
    var: &impl Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
    for key in KEYS {
        let name = format!("REPLOR_{}", key.to_ascii_uppercase());
        if let Some(value) = var(&name) {
            set(settings, key, &value, &name)?;
        }
    }
    Ok(())
}

fn set(
    settings: &mut Settings,
    key: &'static str,
    value: &str,
    origin: &str,
) -> Result<(), ConfigError> {
    match key {
        "images" => settings.images = parse(key, value, origin)?,
        "mermaid" => settings.mermaid = parse(key, value, origin)?,
        "mermaid_width" => settings.mermaid_width = parse(key, value, origin)?,
        "theme" => settings.theme = parse(key, value, origin)?,
        "highlight" => settings.highlight = parse(key, value, origin)?,
        "icons" => settings.icons = parse(key, value, origin)?,
        "flavor" => settings.flavor = parse(key, value, origin)?,
        _ => unreachable!("every key in KEYS is handled"),
    }
    Ok(())
}

/// Spelled as the enum's `ValueEnum` names, ignoring case.
fn parse<T: ValueEnum>(key: &'static str, value: &str, origin: &str) -> Result<T, ConfigError> {
    T::from_str(value, true).map_err(|_| ConfigError::Value {
        origin: origin.to_string(),
        key,
        value: value.to_string(),
        expected: T::value_variants()
            .iter()
            .filter_map(|v| v.to_possible_value())
            .map(|v| v.get_name().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn the_default_place_follows_xdg_then_home() {
        assert_eq!(
            location(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])),
            Location::Default(PathBuf::from("/x/replor/config.toml"))
        );
        assert_eq!(
            location(env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])),
            Location::Default(PathBuf::from("/h/.config/replor/config.toml"))
        );
        assert_eq!(location(env(&[])), Location::None);
    }

    #[test]
    fn replor_config_names_the_file_or_turns_it_off() {
        assert_eq!(
            location(env(&[("REPLOR_CONFIG", "/c.toml"), ("HOME", "/h")])),
            Location::Explicit(PathBuf::from("/c.toml"))
        );
        assert_eq!(
            location(env(&[("REPLOR_CONFIG", ""), ("HOME", "/h")])),
            Location::None
        );
    }

    #[test]
    fn a_missing_explicit_file_is_an_error_but_a_missing_default_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.toml");
        let explicit = env(&[("REPLOR_CONFIG", missing.to_str().unwrap())]);
        assert!(matches!(load(explicit), Err(ConfigError::Read { .. })));
        let default = env(&[("XDG_CONFIG_HOME", dir.path().to_str().unwrap())]);
        assert_eq!(load(default).unwrap(), Settings::default());
    }

    fn from_file(text: &str, extra: &[(&str, &str)]) -> Result<Settings, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let mut pairs = vec![("REPLOR_CONFIG", path.to_str().unwrap())];
        pairs.extend_from_slice(extra);
        load(env(&pairs))
    }

    #[test]
    fn every_key_is_read_ignoring_case() {
        let settings = from_file(
            "images = \"Kitty\"\nmermaid = \"ascii\"\nmermaid_width = \"cjk\"\ntheme = \"light\"\nhighlight = \"never\"\nicons = \"nerd\"\nflavor = \"gfm\"\n",
            &[],
        )
        .unwrap();
        assert_eq!(settings.images, ImagesWhen::Kitty);
        assert_eq!(settings.mermaid, MermaidMode::Ascii);
        assert_eq!(settings.mermaid_width, MermaidWidth::Cjk);
        assert_eq!(settings.theme, ThemeChoice::Light);
        assert_eq!(settings.highlight, HighlightWhen::Never);
        assert_eq!(settings.icons, IconSet::Nerd);
        assert_eq!(settings.flavor, Flavor::Gfm);
        assert!(settings.warnings.is_empty());
    }

    #[test]
    fn an_unknown_key_warns_and_a_bad_value_fails() {
        let settings = from_file("future = 1\n", &[]).unwrap();
        assert_eq!(settings.warnings.len(), 1, "{:?}", settings.warnings);
        assert!(settings.warnings[0].contains("future"));
        let err = from_file("images = \"sixel\"\n", &[]).unwrap_err();
        assert!(err.to_string().contains("kitty"), "{err}");
        assert!(matches!(
            from_file("images = [", &[]),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn the_environment_overrides_the_file() {
        let settings = from_file("images = \"never\"\n", &[("REPLOR_IMAGES", "kitty")]).unwrap();
        assert_eq!(settings.images, ImagesWhen::Kitty);
        let err = from_file("", &[("REPLOR_THEME", "blue")]).unwrap_err();
        assert!(err.to_string().contains("REPLOR_THEME"), "{err}");
    }
}
