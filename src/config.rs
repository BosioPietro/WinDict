//! User settings, stored as JSON in `%APPDATA%\WinDict\config.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Global shortcut, e.g. "Ctrl+Alt+D", "Win+Shift+F1".
    pub hotkey: String,
    /// "system", "dark" or "light".
    pub theme: ThemeMode,
    /// Max definitions shown per part of speech.
    pub max_definitions: usize,
    /// Max synonyms shown per part of speech (0 hides them).
    pub max_synonyms: usize,
    /// Read the selection through UI Automation before falling back to the clipboard.
    pub use_ui_automation: bool,
    /// Use the online dictionary for words (and pronunciations) the bundled one lacks.
    pub online_lookup: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Ctrl+Alt+D".into(),
            theme: ThemeMode::System,
            max_definitions: 4,
            max_synonyms: 6,
            use_ui_automation: true,
            online_lookup: true,
        }
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("WinDict")
}

pub fn path() -> PathBuf {
    dir().join("config.json")
}

/// Loads the config, writing the defaults on first run.
/// Returns the config and whether this is the first run, or why the file
/// couldn't be read as settings.
pub fn load() -> Result<(Config, bool), String> {
    let path = path();
    match std::fs::read(&path) {
        Ok(bytes) => parse(&bytes).map(|config| (config, false)),
        Err(_) => {
            let config = Config::default();
            let _ = save(&config);
            Ok((config, true))
        }
    }
}

/// Writes the settings where [`load`] reads them.
pub fn save(config: &Config) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| format!("config.json: {e}"))?;
    std::fs::create_dir_all(dir())
        .and_then(|_| std::fs::write(path(), json))
        .map_err(|e| format!("config.json: {e}"))
}

fn parse(bytes: &[u8]) -> Result<Config, String> {
    let mut config: Config =
        serde_json::from_slice(bytes).map_err(|e| format!("config.json: {e}"))?;
    // At least one definition per part of speech, or the card would be empty.
    config.max_definitions = config.max_definitions.max(1);
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_settings() {
        let config = parse(br#"{"hotkey": "Win+F1", "theme": "dark"}"#).unwrap();
        assert_eq!(config.hotkey, "Win+F1");
        assert_eq!(config.theme, ThemeMode::Dark);
        // Missing fields keep their defaults.
        assert_eq!(config.max_synonyms, 6);
        assert!(config.online_lookup);
        assert_eq!(
            parse(br#"{"max_definitions": 0}"#).unwrap().max_definitions,
            1
        );
    }

    #[test]
    fn reports_invalid_settings() {
        let error = parse(b"{\n  \"hotkey\": \"Ctrl+D\"\n  \"theme\": \"dark\"\n}").unwrap_err();
        assert!(error.contains("line 3"), "{error}");
        assert!(parse(br#"{"theme": "purple"}"#).is_err());
    }
}
