//! The words WinDict shows, kept apart from the code that shows them.
//!
//! Every user-facing string is a field of [`Strings`], read with [`t`]:
//! `tr().quit`. [`EN`] is the table the others are made from. A new language
//! is a `Strings` of its own that lists what it translates and takes the
//! rest from English with `..EN`, so a missing string reads in English
//! instead of not at all; then it's added to [`Language`]. Nothing outside
//! this module needs to know which language is current.
//!
//! A string that has a value in it holds `{name}` where it goes, filled in
//! with [`fill`]: the translation decides where it falls in the sentence.
//!
//! What stays as it is: names of languages (each in itself, so they can be
//! found whichever language WinDict is in), key names, and "WinDict".

mod it;

use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

macro_rules! strings {
    ($($(#[$doc:meta])* $name:ident: $text:expr,)*) => {
        /// Every string, in one language.
        pub struct Strings {
            $($(#[$doc])* pub $name: &'static str,)*
        }

        /// English, which the other languages fall back on.
        pub const EN: Strings = Strings {
            $($name: $text,)*
        };

        #[cfg(test)]
        impl Strings {
            /// Every string, by name.
            fn all(&self) -> Vec<(&'static str, &'static str)> {
                vec![$((stringify!($name), self.$name),)*]
            }
        }
    };
}

strings! {
    // Notifications from the tray icon.
    settings_not_loaded: "Settings not loaded, using the defaults",
    settings_not_reloaded: "Settings not reloaded, nothing changed",
    settings_not_saved: "Settings not saved",
    dictionary_not_loaded: "Dictionary not loaded",
    running: "WinDict is running",
    /// `{hotkey}`.
    running_detail: "Select a word anywhere and press {hotkey}.",
    shortcut_unavailable: "Shortcut unavailable",
    /// `{hotkey}`.
    shortcut_unavailable_detail: "{hotkey} is already used by another app. Choose another one in Settings.",
    shortcut_invalid: "Invalid shortcut",
    updated: "WinDict updated",
    /// `{version}`.
    updated_detail: "This is version {version}.",
    updating: "Updating WinDict",
    /// `{version}`.
    updating_detail: "Installing version {version}. WinDict will start again by itself.",
    update_failed: "WinDict couldn\u{2019}t update",
    /// Once WinDict has restarted for UI access to be turned on or off.
    ui_access_on: "WinDict shows above Start",
    ui_access_on_detail: "The card now opens on top of Start and Search.",
    ui_access_off: "WinDict no longer shows above Start",
    ui_access_off_detail: "WinDict closes Start and Search before it shows the card.",
    ui_access_unchanged: "UI access didn\u{2019}t change",
    ui_access_unchanged_detail: "WinDict restarted, but Windows started it as it was.",
    draw_window_failed: "WinDict couldn\u{2019}t draw its window",
    draw_menu_failed: "WinDict couldn\u{2019}t draw its menu",
    draw_settings_failed: "WinDict couldn\u{2019}t draw its settings",
    /// `{language}`: a dictionary that couldn't be downloaded at start.
    not_downloaded: "{language} not downloaded",
    settings_reloaded: "Settings reloaded",
    /// `{hotkey}`.
    settings_reloaded_detail: "Look up words with {hotkey}.",
    /// `{percent}`: how far a dictionary's download has got.
    downloading: "Downloading\u{2026} {percent}%",

    // The card: messages.
    too_long: "That\u{2019}s a lot of text",
    too_long_detail: "Select a single word or a short phrase.",
    nothing_selected: "Nothing selected",
    /// `{hotkey}`.
    nothing_selected_detail: "Select a word in any app, then press {hotkey}.",
    not_found: "No definition found",
    not_found_detail: "This word isn\u{2019}t in the dictionary.",
    not_found_check_spelling: "This word isn\u{2019}t in the dictionary. Check the spelling?",
    lookup_failed: "Couldn\u{2019}t look that up",
    /// `{word}`, which is the link.
    did_you_mean: "Did you mean {word}?",

    // The card: the search box.
    search_placeholder: "Type a word to look it up",
    search_title: "Look up a word",
    search_too_long: "Try a single word or a short phrase",
    /// The search box for the translation shortcut.
    translate_title: "Translate sentences",
    translate_placeholder: "Type text to translate",
    no_suggestions: "No matches in the offline dictionary",
    no_suggestions_online: "No offline matches \u{2014} Enter searches online",
    /// Beside the Enter key.
    key_look_up: "Look up",
    key_translate: "Translate",
    /// Beside the Esc key.
    key_close: "Close",

    // The card: a definition.
    similar: "Similar",
    opposite: "Opposite",
    /// `{word}`: shown when a word is a form of another.
    also_form_of: "Also a form of \u{201C}{word}\u{201D}",
    /// `{word}`: shown when the definition is of the word's base form.
    base_form_of: "Base form of \u{201C}{word}\u{201D}",
    pos_noun: "noun",
    pos_verb: "verb",
    pos_adjective: "adjective",
    pos_adverb: "adverb",
    pos_phrase: "phrase",
    pos_interjection: "interjection",
    pos_pronoun: "pronoun",
    pos_preposition: "preposition",
    pos_conjunction: "conjunction",
    pos_article: "article",
    pos_abbreviation: "abbreviation",
    pos_other: "other",

    // The card: translating.
    back_tip: "Back (Backspace)",
    translate_tip: "Translate (Ctrl+T)",
    speak_tip: "Listen",
    pin_tip: "Pin window",
    unpin_tip: "Unpin",
    swap_tip: "Swap languages",
    /// Shown for a moment once Ctrl+C has copied a word.
    copied: "Copied",
    /// `{word}`, `{from}`, `{to}`.
    translate_ambiguous: "\u{201C}{word}\u{201D} is a word in both {from} and {to}.",
    /// `{language}`.
    translate_into: "Translate into {language}",
    translating: "Translating\u{2026}",
    translate_failed: "Couldn\u{2019}t translate it",
    /// Under "That's a lot of text", for the translation shortcut.
    translate_too_long: "Select a shorter passage to translate.",
    translate_offline: "Translation is off",
    translate_offline_detail: "Turn on the online dictionary in Settings to translate.",
    /// The heading over translations that have no part of speech.
    also: "Also",
    no_languages_match: "No languages match",

    // The tray menu.
    menu_update: "Update app",
    menu_updating: "Updating\u{2026}",
    /// Under the shortcut, over the tray icon. `{version}`.
    update_available: "Update available: {version}",
    menu_reload: "Reload settings",
    menu_quit: "Quit",

    // The introduction, shown the first time WinDict starts.
    intro_title: "Welcome to WinDict",
    intro_back: "Back",
    intro_next: "Next",
    intro_done: "Done",
    intro_lookup: "Look up any word",
    /// `{hotkey}`, which shows as keys.
    intro_lookup_detail: "Select a word in any app and press {hotkey}: its definition \
        opens in a card next to it. With nothing selected, the same keys open a box \
        to type a word into.",
    intro_translate: "Translate as you read",
    /// `{hotkey}`, and `{card_hotkey}`, the keys that translate on a card;
    /// each shows as keys.
    intro_translate_detail: "Select a word, a phrase or a whole passage and press \
        {hotkey}: the card opens on its translation. On a definition, {card_hotkey} \
        translates the word.",
    intro_above_start: "Show the card above Start",
    intro_above_start_detail: "Windows keeps Start and Search above every other window, \
        so WinDict closes them before showing a definition. With UI access, the card \
        shows on top of them instead. Windows asks for permission to turn it on.",
    intro_ready: "You\u{2019}re all set",
    intro_ready_detail: "WinDict waits in the notification area, by the clock. Click \
        its icon for the settings, and for updates when there\u{2019}s one.",
    /// Opens a card on an example, as the step's shortcut would.
    intro_show_me: "Show me",
    /// Over "Choose another", when the lookup shortcut couldn't be had.
    intro_shortcut_taken: "Another app is using this shortcut.",
    intro_choose_shortcut: "Choose another",
    /// The switch that turns UI access on.
    intro_ui_access: "Show above Start",
    /// Dots follow it, one, then two, then three, and again.
    intro_ui_access_waiting: "Waiting for Windows",
    intro_ui_access_cancelled: "Windows didn\u{2019}t get permission, so nothing changed.",
    /// `{error}`.
    intro_ui_access_failed: "That didn\u{2019}t work: {error}",

    // Settings: the window.
    settings: "Settings",
    settings_window_title: "WinDict Settings",
    config_link: "Open config.json",
    language: "Language",
    language_detail: "The language WinDict is shown in",
    off: "Off",
    theme_system: "System",
    theme_light: "Light",
    theme_dark: "Dark",
    /// Over the info symbol by the email field.
    email_tip: "MyMemory, the translator used when Google Translate can\u{2019}t \
        be reached, allows 5,000 characters a day, or 50,000 with an email address. \
        It\u{2019}s sent only to MyMemory, with the words translated.",

    // Settings: recording a shortcut.
    shortcut_detail: "To look up words",
    recording_detail: "Press the new shortcut, or Esc to cancel",
    recording_hint: "Press keys",
    needs_modifier: "Hold Ctrl, Alt or Win with the key",
    unusable_key: "That key can\u{2019}t be part of a shortcut",
    shortcut_taken: "Already used by Windows or another app",
    shortcut_clash: "Already used by WinDict\u{2019}s other shortcut",

    // Settings: sections and rows, each with its description under it.
    general: "General",
    shortcuts: "Shortcuts",
    /// What the shortcuts row says, under it. `{lookup}` and `{translate}`:
    /// their keys.
    shortcuts_summary: "{lookup} to look up, {translate} to translate",
    lookup_shortcut: "Look up",
    start_with_windows: "Start with Windows",
    start_with_windows_detail: "Runs WinDict when you sign in",
    above_start: "Show above Start",
    above_start_detail: "Opens the card on top of Start and Search",
    check_updates: "Check for updates",
    check_updates_detail: "Once a day; the tray icon shows a dot when there\u{2019}s one",
    card: "Card",
    theme: "Theme",
    theme_detail: "Card and menu colors",
    text_size: "Text size",
    text_size_detail: "Also Ctrl+plus, Ctrl+minus or Ctrl+wheel on the card",
    definitions: "Definitions",
    definitions_detail: "Senses shown per part of speech",
    synonyms: "Synonyms",
    synonyms_detail: "Similar words under each part of speech",
    examples: "Examples",
    examples_detail: "Sentences showing each sense in use",
    animations: "Animations",
    animations_detail: "Starts as Windows has them; turn on to override",
    translation: "Translation",
    translate_shortcut: "Translate",
    translate_shortcut_detail: "To translate text",
    default_languages: "Default languages",
    default_languages_detail: "Translated from and into when it can\u{2019}t be inferred",
    mymemory_email: "MyMemory email",
    mymemory_email_detail: "Raises the daily limit",
    lookup: "Lookup",
    online_dictionary: "Online dictionary",
    online_dictionary_detail: "For pronunciations, and words the built-in one lacks",
    read_selection: "Read selections",
    // Its options, each explained under them once chosen.
    selection_automatic: "Automatic",
    selection_automatic_detail: "Reads the selection directly, and copies it in apps \
        where that isn\u{2019}t possible. Works in every app",
    selection_direct: "Direct only",
    selection_direct_detail: "Opens faster, but misses selections in apps that \
        can\u{2019}t be read directly. Switch back if words go missing",
    selection_clipboard: "Clipboard only",
    selection_clipboard_detail: "Always copies the selection, which is slower. Try it \
        if an app gives the wrong text",
    languages: "Languages",

    // Errors, which reach the user as the detail of a message.
    /// `{hotkey}`.
    hotkey_many_keys: "\"{hotkey}\" has more than one non-modifier key",
    /// `{key}`, `{hotkey}`.
    hotkey_unknown_key: "Unknown key \"{key}\" in \"{hotkey}\"",
    /// `{hotkey}`.
    hotkey_no_key: "\"{hotkey}\" has no key besides modifiers",
    /// `{hotkey}`.
    hotkey_no_modifier: "\"{hotkey}\" needs at least one modifier (Ctrl, Alt, Shift, Win)",
    network_unavailable: "Couldn't reach the dictionary. Check your connection.",
    network_init_failed: "Could not initialise the network stack.",
    response_too_large: "The dictionary response was unexpectedly large.",
    too_many_lookups: "Too many lookups, try again in a moment.",
    /// `{code}`: an HTTP status.
    lookup_service_error: "The dictionary service returned an error ({code}).",
    too_many_translations: "Too many translations, try again in a moment.",
    /// `{code}`: an HTTP status.
    translation_service_error: "The translation service returned an error ({code}).",
    translation_unreachable: "Couldn\u{2019}t reach the translation service. Check your connection.",
    no_translation: "Google Translate had no translation.",
    mymemory_no_translation: "MyMemory had no translation.",
    mymemory_unreadable: "MyMemory\u{2019}s answer couldn\u{2019}t be read.",
    mymemory_quota: "Today\u{2019}s free translations have run out. An email in \
        Settings \u{2192} Translation raises the limit.",
    /// `{language}`.
    not_language_code: "\u{201C}{language}\u{201D} isn't a language code, like \u{201C}en\u{201D}.",
    /// `{path}`, `{language}`.
    not_dictionary: "{path} isn't a WinDict dictionary for \u{201C}{language}\u{201D}, or is for another version of WinDict.",
    download_unreachable: "Couldn\u{2019}t connect. Try again later.",
    download_damaged: "The download was damaged. Try again.",
    /// `{status}`: an HTTP status.
    download_status: "The download failed ({status}).",
    update_task_missing: "Install WinDict again with its setup to update it from the menu.",
    /// `{reason}`: why the file couldn't be written.
    save_failed: "Couldn\u{2019}t save it: {reason}",
}

/// The languages WinDict can be shown in.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    English,
    Italian,
}

impl Language {
    pub const ALL: &'static [Language] = &[Language::English, Language::Italian];

    fn strings(self) -> &'static Strings {
        match self {
            Language::English => &EN,
            Language::Italian => &it::IT,
        }
    }

    /// The language's name, in itself.
    pub fn name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Italian => "Italiano",
        }
    }

    /// The language Windows is shown in, or English if WinDict has no
    /// translation for it.
    pub fn system() -> Language {
        Language::from_lang_id(unsafe { GetUserDefaultUILanguage() })
    }

    /// The language a Windows language identifier (LANGID) is of, whatever
    /// its region: the low ten bits are the language, the rest the region.
    fn from_lang_id(id: u16) -> Language {
        const LANG_ITALIAN: u16 = 0x10;
        match id & 0x3FF {
            LANG_ITALIAN => Language::Italian,
            _ => Language::English,
        }
    }
}

/// Index into [`Language::ALL`]. Read from the threads that look words up,
/// so it's atomic; it changes only when the setting does.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Shows WinDict in `language` from now on.
pub fn set(language: Language) {
    let index = Language::ALL.iter().position(|&l| l == language);
    CURRENT.store(index.unwrap_or(0), Ordering::Relaxed);
}

fn current() -> Language {
    Language::ALL[CURRENT.load(Ordering::Relaxed).min(Language::ALL.len() - 1)]
}

/// The strings of the current language.
pub fn tr() -> &'static Strings {
    current().strings()
}

/// `template` with each `{name}` replaced by its value in `args`. A name
/// that isn't in `args` is left as it is, braces included.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        rest = &rest[open..];
        let value = rest
            .find('}')
            .and_then(|close| args.iter().find(|(name, _)| *name == &rest[1..close]));
        match value {
            Some((name, value)) => {
                out.push_str(&value.to_string());
                rest = &rest[name.len() + 2..];
            }
            None => {
                out.push('{');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

impl Strings {
    /// How a part of speech, as the dictionaries name it ("noun"), is
    /// written. Anything else, like what a translator calls one, as it is.
    pub fn part_of_speech<'a>(&'a self, key: &'a str) -> &'a str {
        match key {
            "noun" => self.pos_noun,
            "verb" => self.pos_verb,
            "adjective" => self.pos_adjective,
            "adverb" => self.pos_adverb,
            "phrase" => self.pos_phrase,
            "interjection" => self.pos_interjection,
            "pronoun" => self.pos_pronoun,
            "preposition" => self.pos_preposition,
            "conjunction" => self.pos_conjunction,
            "article" => self.pos_article,
            "abbreviation" => self.pos_abbreviation,
            "other" => self.pos_other,
            _ => key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_values() {
        assert_eq!(
            fill("{a} and {b}, {a}", &[("a", &1), ("b", &"two")]),
            "1 and two, 1"
        );
        // Where a translation puts them is up to it.
        assert_eq!(fill("{b}{a}", &[("a", &1), ("b", &"two")]), "two1");
        // Unknown names, stray braces and values holding braces are kept.
        assert_eq!(fill("{x} { {a}", &[("a", &"{b}"), ("b", &0)]), "{x} { {b}");
        assert_eq!(fill("no values", &[]), "no values");
        assert_eq!(fill("open {", &[]), "open {");
    }

    #[test]
    fn languages_inherit_english() {
        // How a translation is written: what it doesn't list reads in English.
        const PARTIAL: Strings = Strings {
            menu_quit: "Esci",
            ..EN
        };
        assert_eq!(PARTIAL.menu_quit, "Esci");
        assert_eq!(PARTIAL.menu_reload, EN.menu_reload);
    }

    #[test]
    fn reads_system_languages() {
        // Italy's, Switzerland's, and the neutral Italian.
        for id in [0x0410, 0x0810, 0x0010] {
            assert_eq!(Language::from_lang_id(id), Language::Italian);
        }
        // English, and what WinDict has no translation for.
        for id in [0x0409, 0x0809, 0x040C, 0x0000] {
            assert_eq!(Language::from_lang_id(id), Language::English);
        }
    }

    #[test]
    fn translations_keep_their_values() {
        // A translation that dropped or misspelled a `{name}` would lose the
        // value, or show the braces.
        fn names(text: &str) -> Vec<&str> {
            let mut names: Vec<_> = text
                .split('{')
                .skip(1)
                .filter_map(|s| s.split('}').next())
                .collect();
            names.sort_unstable();
            names
        }
        for language in Language::ALL {
            for ((name, text), (_, english)) in language.strings().all().into_iter().zip(EN.all()) {
                assert_eq!(names(text), names(english), "{language:?}: {name}");
            }
        }
    }

    #[test]
    fn names_parts_of_speech() {
        assert_eq!(EN.part_of_speech("noun"), "noun");
        assert_eq!(EN.part_of_speech("nom"), "nom");
    }

    #[test]
    fn templates_name_their_values() {
        // A template that lost a name would show it to the user.
        let hotkey: &dyn Display = &"Ctrl+D";
        assert!(!fill(EN.running_detail, &[("hotkey", hotkey)]).contains('{'));
        assert!(EN.did_you_mean.contains("{word}"));
    }
}
