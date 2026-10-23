//! User settings, stored as JSON in `%APPDATA%\WinDict\config.json`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Language;
use crate::lang::LangCode;
use crate::{translate, ui};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Dark,
    Light,
}

/// How the selection is read.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SelectionMode {
    /// Through UI Automation, falling back to the clipboard in apps it
    /// can't read.
    Automatic,
    /// Through UI Automation only: no copy to wait for, but blind in apps
    /// it can't read.
    Direct,
    /// By copying it, in every app.
    Clipboard,
}

/// What a global shortcut does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shortcut {
    /// Looks the selection up.
    #[default]
    Lookup,
    /// Translates the selection, a word or a whole passage, straight away.
    Translate,
}

pub const SHORTCUTS: [Shortcut; 2] = [Shortcut::Lookup, Shortcut::Translate];

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Global shortcut, e.g. "Ctrl+Alt+D", "Win+Shift+F1".
    pub hotkey: String,
    /// Global shortcut that translates the selection instead.
    pub translate_hotkey: String,
    /// The language WinDict is shown in, "english" or "italian". Unset, it's
    /// the language of Windows if WinDict has it, else English.
    pub ui_language: Language,
    /// "system", "dark" or "light".
    pub theme: ThemeMode,
    /// Max definitions shown per part of speech.
    pub max_definitions: usize,
    /// Max synonyms shown per part of speech (0 hides them).
    pub max_synonyms: usize,
    /// Show the example sentences under the senses.
    pub show_examples: bool,
    /// How large the card is drawn, in percent of the size Windows gives
    /// it: one of [`ZOOMS`].
    pub zoom: u32,
    /// "automatic", "direct" or "clipboard".
    pub selection: SelectionMode,
    /// What older versions saved instead: false was "clipboard".
    #[serde(skip_serializing)]
    use_ui_automation: Option<bool>,
    /// Use the online dictionary for words (and pronunciations) the bundled one lacks.
    pub online_lookup: bool,
    /// The languages words are looked up in, as codes in order of preference:
    /// "en" is built in, another is read from
    /// `%LOCALAPPDATA%\WinDict\dicts\<code>.wdict`.
    #[serde(alias = "language", deserialize_with = "one_or_more")]
    pub languages: Vec<LangCode>,
    /// The languages words are translated from and into, as codes, when the
    /// card can't tell (see `translate::pick`).
    pub translate_from: LangCode,
    pub translate_to: LangCode,
    /// Sent to MyMemory, and only to it, for its larger daily allowance.
    pub mymemory_email: String,
    /// Move things into place instead of jumping them there; unset, as
    /// Windows has it (Accessibility > Visual effects > Animation effects).
    pub animations: Option<bool>,
    /// Look for a new version of WinDict once a day.
    pub check_updates: bool,
    /// Whether the introduction has been shown (see `app::onboarding`).
    pub onboarded: bool,
}

/// The sizes the card can be zoomed to, in percent.
pub const ZOOMS: [u32; 8] = [80, 90, 100, 110, 125, 150, 175, 200];

impl Config {
    /// The keys of `shortcut`, as written.
    pub fn hotkey(&self, shortcut: Shortcut) -> &str {
        match shortcut {
            Shortcut::Lookup => &self.hotkey,
            Shortcut::Translate => &self.translate_hotkey,
        }
    }

    pub fn hotkey_mut(&mut self, shortcut: Shortcut) -> &mut String {
        match shortcut {
            Shortcut::Lookup => &mut self.hotkey,
            Shortcut::Translate => &mut self.translate_hotkey,
        }
    }

    /// Whether the windows animate.
    pub fn animated(&self) -> bool {
        self.animations.unwrap_or_else(ui::system_motion)
    }

    /// The card's zoom, as a factor.
    pub fn zoom_factor(&self) -> f32 {
        self.zoom as f32 / 100.0
    }
}

/// The zoom `by` steps of [`ZOOMS`] from `zoom`, stopping at either end.
pub fn step_zoom(zoom: u32, by: isize) -> u32 {
    let at = zoom_index(zoom) as isize + by;
    ZOOMS[at.clamp(0, ZOOMS.len() as isize - 1) as usize]
}

/// Where in [`ZOOMS`] `zoom` is, or the nearest to it.
pub fn zoom_index(zoom: u32) -> usize {
    (0..ZOOMS.len())
        .min_by_key(|&i| ZOOMS[i].abs_diff(zoom))
        .unwrap_or(0)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: "Alt+D".into(),
            translate_hotkey: "Alt+T".into(),
            ui_language: Language::system(),
            theme: ThemeMode::System,
            max_definitions: 4,
            max_synonyms: 6,
            show_examples: true,
            zoom: 100,
            selection: SelectionMode::Automatic,
            use_ui_automation: None,
            online_lookup: true,
            languages: vec!["en".into()],
            translate_from: "en".into(),
            translate_to: "it".into(),
            mymemory_email: String::new(),
            animations: None,
            check_updates: true,
            onboarded: false,
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
    load_from(&path())
}

fn load_from(path: &Path) -> Result<(Config, bool), String> {
    match std::fs::read(path) {
        Ok(bytes) => parse(&bytes).map(|config| (config, false)),
        // Only a missing file is a first run: one that can't be read right
        // now (locked, no access) mustn't be overwritten with the defaults.
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let config = Config::default();
            let _ = save_to(path, &config);
            Ok((config, true))
        }
        Err(e) => Err(format!("config.json: {e}")),
    }
}

/// Writes the settings where [`load`] reads them.
pub fn save(config: &Config) -> Result<(), String> {
    save_to(&path(), config)
}

fn save_to(path: &Path, config: &Config) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config).map_err(|e| format!("config.json: {e}"))?;
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path, json))
        .map_err(|e| format!("config.json: {e}"))
}

fn parse(bytes: &[u8]) -> Result<Config, String> {
    let mut config: Config =
        serde_json::from_slice(bytes).map_err(|e| format!("config.json: {e}"))?;
    // At least one definition per part of speech, or the card would be empty.
    config.max_definitions = config.max_definitions.max(1);
    // Each language once, and at least one, or nothing could be looked up.
    let mut languages: Vec<LangCode> = Vec::new();
    for language in config.languages.drain(..) {
        let language = LangCode::new(language.trim().to_ascii_lowercase());
        if !language.is_empty() && !languages.contains(&language) {
            languages.push(language);
        }
    }
    if languages.is_empty() {
        languages.push("en".into());
    }
    config.languages = languages;
    // A language that can't be translated into, or both the same: the defaults.
    let defaults = Config::default();
    if !translate::supported(&config.translate_from) {
        config.translate_from = defaults.translate_from.clone();
    }
    if !translate::supported(&config.translate_to) || config.translate_to == config.translate_from {
        config.translate_to = [defaults.translate_to, defaults.translate_from]
            .into_iter()
            .find(|l| *l != config.translate_from)
            .unwrap_or_default();
    }
    config.mymemory_email = config.mymemory_email.trim().to_string();
    config.zoom = ZOOMS[zoom_index(config.zoom)];
    if config.use_ui_automation.take() == Some(false) {
        config.selection = SelectionMode::Clipboard;
    }
    Ok(config)
}

/// Reads a list of languages, or the single one older versions saved.
fn one_or_more<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<LangCode>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Languages {
        One(LangCode),
        More(Vec<LangCode>),
    }
    Ok(match Languages::deserialize(deserializer)? {
        Languages::One(language) => vec![language],
        Languages::More(languages) => languages,
    })
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
        assert_eq!(config.hotkey(Shortcut::Translate), "Alt+T");
        assert_eq!(config.max_synonyms, 6);
        assert!(config.online_lookup);
        assert!(config.check_updates);
        assert!(config.show_examples);
        assert_eq!(config.selection, SelectionMode::Automatic);
        assert_eq!(config.languages, ["en"]);
        assert_eq!(config.zoom, 100);
        assert_eq!(
            parse(br#"{"max_definitions": 0}"#).unwrap().max_definitions,
            1
        );
    }

    #[test]
    fn reads_and_steps_the_zoom() {
        let zoom = |json: &str| parse(json.as_bytes()).unwrap().zoom;
        assert_eq!(zoom(r#"{"zoom": 125}"#), 125);
        // Between the sizes there are, the nearest.
        assert_eq!(zoom(r#"{"zoom": 120}"#), 125);
        assert_eq!(zoom(r#"{"zoom": 5}"#), 80);
        assert_eq!(zoom(r#"{"zoom": 900}"#), 200);
        assert_eq!(step_zoom(100, 1), 110);
        assert_eq!(step_zoom(100, -2), 80);
        assert_eq!(step_zoom(200, 1), 200);
        assert_eq!(step_zoom(80, -1), 80);
    }

    #[test]
    fn reads_the_interface_language() {
        let language = |json: &str| parse(json.as_bytes()).unwrap().ui_language;
        assert_eq!(language(r#"{"ui_language": "italian"}"#), Language::Italian);
        assert_eq!(language(r#"{"ui_language": "english"}"#), Language::English);
        // Unset, it's the system's.
        assert_eq!(language("{}"), Language::system());
        assert!(parse(br#"{"ui_language": "klingon"}"#).is_err());
    }

    #[test]
    fn reads_the_selection_mode() {
        let mode = |json: &str| parse(json.as_bytes()).unwrap().selection;
        assert_eq!(mode(r#"{"selection": "direct"}"#), SelectionMode::Direct);
        // The switch older versions saved.
        assert_eq!(
            mode(r#"{"use_ui_automation": false}"#),
            SelectionMode::Clipboard
        );
        assert_eq!(
            mode(r#"{"use_ui_automation": true}"#),
            SelectionMode::Automatic
        );
        let json = serde_json::to_string(&parse(br#"{"use_ui_automation": false}"#).unwrap());
        assert!(!json.unwrap().contains("use_ui_automation"));
    }

    #[test]
    fn reads_languages() {
        let languages = |json: &str| parse(json.as_bytes()).unwrap().languages;
        // The single language older versions saved.
        assert_eq!(languages(r#"{"language": "it"}"#), ["it"]);
        assert_eq!(
            languages(r#"{"languages": ["it", "EN", "it", " "]}"#),
            ["it", "en"]
        );
        assert_eq!(languages(r#"{"languages": []}"#), ["en"]);
    }

    #[test]
    fn reads_translation_languages() {
        let config = parse(br#"{"translate_from": "fr", "translate_to": "de"}"#).unwrap();
        assert_eq!(
            (config.translate_from.as_str(), config.translate_to.as_str()),
            ("fr", "de")
        );
        assert!(config.mymemory_email.is_empty());
        let config = parse(br#"{"translate_from": "xx", "translate_to": "en"}"#).unwrap();
        assert_eq!(
            (config.translate_from.as_str(), config.translate_to.as_str()),
            ("en", "it")
        );
        let config = parse(br#"{"translate_from": "it", "translate_to": "it"}"#).unwrap();
        assert_eq!(
            (config.translate_from.as_str(), config.translate_to.as_str()),
            ("it", "en")
        );
        let config = parse(br#"{"mymemory_email": " me@example.com "}"#).unwrap();
        assert_eq!(config.mymemory_email, "me@example.com");
    }

    #[test]
    fn reports_invalid_settings() {
        let error = parse(b"{\n  \"hotkey\": \"Ctrl+D\"\n  \"theme\": \"dark\"\n}").unwrap_err();
        assert!(error.contains("line 3"), "{error}");
        assert!(parse(br#"{"theme": "purple"}"#).is_err());
    }

    #[test]
    fn writes_defaults_only_when_missing() {
        let dir = std::env::temp_dir().join(format!("windict-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = std::fs::remove_dir_all(&dir);
        let (config, first_run) = load_from(&path).unwrap();
        assert!(first_run);
        assert_eq!(config.hotkey, "Alt+D");
        assert!(path.exists());
        assert!(!load_from(&path).unwrap().1);
        // A path that can't be read as a file is an error, and stays as it is.
        let unreadable = dir.join("folder");
        std::fs::create_dir(&unreadable).unwrap();
        assert!(load_from(&unreadable).is_err());
        assert!(unreadable.is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
