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
/// Returns the config and whether this is the first run.
pub fn load() -> (Config, bool) {
    let path = path();
    match std::fs::read(&path) {
        Ok(bytes) => (serde_json::from_slice(&bytes).unwrap_or_default(), false),
        Err(_) => {
            let config = Config::default();
            let _ = std::fs::create_dir_all(dir());
            if let Ok(json) = serde_json::to_string_pretty(&config) {
                let _ = std::fs::write(&path, json);
            }
            (config, true)
        }
    }
}
