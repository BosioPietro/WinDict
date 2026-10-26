//! What the settings window has in it, apart from how it looks: the values
//! it shows, the changes it asks for, its rows grouped into sections, and
//! what in them can be clicked or have the keyboard.

use super::*;

/// What the settings window shows: the settings, and whether WinDict
/// starts with Windows, and shows above Start, which are kept in the
/// registry instead.
#[derive(Clone)]
pub struct Values {
    pub config: Config,
    pub autostart: bool,
    /// Whether UI access is on, in the installed copy: only it can have it.
    pub ui_access: Option<bool>,
}

/// Something done in the window, for the app to carry out.
#[derive(Debug, PartialEq)]
pub enum Change {
    /// Recording a new shortcut started: the current one should stop
    /// working meanwhile.
    RecordingStarted,
    /// Recording ended without a new shortcut.
    RecordingStopped,
    Hotkey(Shortcut, String),
    Theme(ThemeMode),
    /// The language WinDict is shown in.
    Interface(Language),
    /// A language's code, like "en", turned on or off.
    Language(LangCode, bool),
    MaxDefinitions(usize),
    MaxSynonyms(usize),
    ShowExamples(bool),
    /// How large the card is drawn, in percent.
    Zoom(u32),
    OnlineLookup(bool),
    Selection(SelectionMode),
    Animations(bool),
    Autostart(bool),
    /// Turn UI access on or off. The switch waits, until
    /// [`Settings::settle_ui_access`].
    UiAccess(bool),
    CheckUpdates(bool),
    /// The languages translated from and into when the card can't tell,
    /// as codes.
    TranslateLanguages(LangCode, LangCode),
    MyMemoryEmail(String),
    OpenConfig,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Switch {
    Autostart,
    /// Only in the installed copy.
    UiAccess,
    Online,
    Examples,
    Animations,
    Updates,
}

pub(super) const SWITCHES: [Switch; 6] = [
    Switch::Autostart,
    Switch::UiAccess,
    Switch::Online,
    Switch::Examples,
    Switch::Animations,
    Switch::Updates,
];

/// A number stepped through with − and + buttons.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Counter {
    Definitions,
    Synonyms,
    /// Which of [`ZOOMS`] the card is drawn at.
    Zoom,
}

pub(super) const COUNTERS: [Counter; 3] = [Counter::Definitions, Counter::Synonyms, Counter::Zoom];

impl Counter {
    /// The lowest and highest values.
    pub(super) fn range(self) -> (usize, usize) {
        match self {
            Counter::Definitions => (1, 10),
            Counter::Synonyms => (0, 12),
            Counter::Zoom => (0, ZOOMS.len() - 1),
        }
    }

    pub(super) fn text(self, n: usize) -> String {
        match (self, n) {
            (Counter::Zoom, _) => format!("{}%", ZOOMS[n]),
            (_, 0) => tr().off.into(),
            _ => n.to_string(),
        }
    }

    pub(super) fn width(self) -> f32 {
        2.0 * STEP_SIZE + VALUE_W
    }
}

pub(super) const THEMES: [(ThemeMode, Label); 3] = [
    (ThemeMode::System, |s| s.theme_system),
    (ThemeMode::Light, |s| s.theme_light),
    (ThemeMode::Dark, |s| s.theme_dark),
];

/// The ways of reading the selection, each with what it does.
pub(super) const SELECTIONS: [(SelectionMode, Label, Label); 3] = [
    (
        SelectionMode::Automatic,
        |s| s.selection_automatic,
        |s| s.selection_automatic_detail,
    ),
    (
        SelectionMode::Direct,
        |s| s.selection_direct,
        |s| s.selection_direct_detail,
    ),
    (
        SelectionMode::Clipboard,
        |s| s.selection_clipboard,
        |s| s.selection_clipboard_detail,
    ),
];

/// What the chosen way of reading the selection does.
pub(super) fn selection_detail(mode: SelectionMode) -> &'static str {
    let (_, _, detail) = SELECTIONS
        .iter()
        .find(|(m, _, _)| *m == mode)
        .unwrap_or(&SELECTIONS[0]);
    detail(tr())
}

/// A setting chosen from a few options, side by side.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Choice {
    Theme,
    Language,
    Selection,
}

pub(super) const CHOICES: [Choice; 3] = [Choice::Theme, Choice::Language, Choice::Selection];

impl Choice {
    pub(super) fn names(self) -> Vec<&'static str> {
        match self {
            Choice::Theme => THEMES.iter().map(|(_, name)| name(tr())).collect(),
            // Each in itself, so that it's found whichever language this is.
            Choice::Language => Language::ALL.iter().map(|l| l.name()).collect(),
            Choice::Selection => SELECTIONS.iter().map(|(_, name, _)| name(tr())).collect(),
        }
    }

    /// How wide each option is: the way of reading the selection has the
    /// row to itself, the others share it with their names.
    pub(super) fn segment_w(self) -> f32 {
        match self {
            Choice::Selection => (GROUP_W - TEXT_X - ROW_PAD) / SELECTIONS.len() as f32,
            Choice::Theme | Choice::Language => SEGMENT_W,
        }
    }
}

/// The names of `languages`, as the languages row sums them up: "English,
/// Italiano".
pub(super) fn summary(languages: &[LangCode]) -> String {
    let names: Vec<_> = dicts::languages()
        .filter(|(code, _)| languages.iter().any(|l| l == code))
        .map(|(_, name)| name)
        .collect();
    names.join(", ")
}

/// What the shortcuts row sums up: each shortcut's keys.
pub(super) fn shortcuts_summary(config: &Config) -> String {
    let keys = |shortcut| key_names(config.hotkey(shortcut)).join("+");
    i18n::fill(
        tr().shortcuts_summary,
        &[
            ("lookup", &keys(Shortcut::Lookup)),
            ("translate", &keys(Shortcut::Translate)),
        ],
    )
}

/// A row that opens out into a list of more under it, and shuts again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Fold {
    /// The shortcuts, each a row like the other settings.
    Shortcuts,
    /// The languages words are looked up in, each with a box to tick.
    Languages,
}

/// In the order they're in on the card.
pub(super) const FOLDS: [Fold; 2] = [Fold::Shortcuts, Fold::Languages];

/// The rows in the shortcuts' list.
pub(super) const SHORTCUT_ROWS: [Row; SHORTCUTS.len()] = [
    row(
        "\u{E721}",
        |s| s.lookup_shortcut,
        |s| s.shortcut_detail,
        Control::Shortcut(Shortcut::Lookup),
    ),
    row(
        "\u{E8C1}",
        |s| s.translate_shortcut,
        |s| s.translate_shortcut_detail,
        Control::Shortcut(Shortcut::Translate),
    ),
];

/// A setting's description that can change to say what's going on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Note {
    Shortcuts,
    Shortcut,
    TranslateShortcut,
    Language,
    Selection,
    /// How turning UI access on or off is going.
    UiAccess,
}

pub(super) const NOTES: [Note; 6] = [
    Note::Shortcuts,
    Note::Shortcut,
    Note::TranslateShortcut,
    Note::Language,
    Note::Selection,
    Note::UiAccess,
];

impl Note {
    /// The description under `shortcut`.
    pub(super) fn of(shortcut: Shortcut) -> Note {
        match shortcut {
            Shortcut::Lookup => Note::Shortcut,
            Shortcut::Translate => Note::TranslateShortcut,
        }
    }

    pub(super) fn usual(self, values: &Values) -> String {
        match self {
            Note::Shortcuts => shortcuts_summary(&values.config),
            Note::Shortcut => tr().shortcut_detail.into(),
            Note::TranslateShortcut => tr().translate_shortcut_detail.into(),
            Note::Language => summary(&values.config.languages),
            Note::Selection => selection_detail(values.config.selection).into(),
            Note::UiAccess => tr().above_start_detail.into(),
        }
    }
}

/// What's under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Hit {
    Close,
    Shortcut(Shortcut),
    Switch(Switch),
    /// One of a choice's options.
    Choice(Choice, usize),
    /// A counter's down (false) or up (true) button.
    Step(Counter, bool),
    /// A row that opens and shuts the list under it.
    Fold(Fold),
    /// A language in that list, by its place in [`dicts::languages`].
    Language(usize),
    /// The email field.
    Email,
    /// The info symbol beside it, with its tooltip.
    Info,
    /// A default language's box, which lists the others to pick from.
    Select(Side),
    /// The button between them, which swaps them.
    Swap,
    /// A language in that list, by its place among the matches.
    Pick(usize),
    Config,
}

/// What Tab stops at, in order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Stop {
    Shortcut(Shortcut),
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    Email,
    Select(Side),
    Swap,
    Fold(Fold),
    Language(usize),
    Config,
}

impl Hit {
    pub(super) fn stop(self) -> Option<Stop> {
        match self {
            Hit::Close => None,
            Hit::Shortcut(shortcut) => Some(Stop::Shortcut(shortcut)),
            Hit::Switch(sw) => Some(Stop::Switch(sw)),
            Hit::Choice(choice, _) => Some(Stop::Choice(choice)),
            Hit::Step(counter, _) => Some(Stop::Counter(counter)),
            Hit::Fold(fold) => Some(Stop::Fold(fold)),
            Hit::Language(i) => Some(Stop::Language(i)),
            Hit::Email => Some(Stop::Email),
            Hit::Info | Hit::Pick(_) => None,
            Hit::Select(side) => Some(Stop::Select(side)),
            Hit::Swap => Some(Stop::Swap),
            Hit::Config => Some(Stop::Config),
        }
    }

    /// The fold whose list it's in, if it's in one.
    pub(super) fn folded(self) -> Option<Fold> {
        self.stop().and_then(Stop::folded)
    }
}

impl Stop {
    /// The fold whose list it's in, if it's in one.
    pub(super) fn folded(self) -> Option<Fold> {
        match self {
            Stop::Shortcut(_) => Some(Fold::Shortcuts),
            Stop::Language(_) => Some(Fold::Languages),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Control {
    Shortcut(Shortcut),
    Switch(Switch),
    Choice(Choice),
    Counter(Counter),
    /// Opens the list under it. It's the last row of its group, so that
    /// only what's below the group moves as the list opens.
    Fold(Fold),
    /// The field for the MyMemory email, with an info symbol after the
    /// setting's name.
    Email,
    /// The default languages' boxes, with the list under the one being
    /// picked. It's taller than the other rows, and as tall as the list
    /// when that's open.
    Translate,
    /// The ways of reading the selection, under the setting's name and over
    /// what the chosen one does.
    Selection,
}

impl Control {
    pub(super) fn width(self) -> f32 {
        match self {
            Control::Shortcut(_) => SHORTCUT_W,
            Control::Switch(_) => SWITCH_W,
            Control::Choice(choice) => choice.names().len() as f32 * choice.segment_w(),
            Control::Counter(counter) => counter.width(),
            Control::Fold(_) => CHEVRON_W,
            Control::Email => FIELD_W,
            Control::Translate | Control::Selection => 0.0,
        }
    }
}

/// A string of the current language, picked out of its [`Strings`]: the rows
/// are consts, written before the language is known.
pub(super) type Label = fn(&Strings) -> &'static str;

pub(super) struct Row {
    /// A Segoe Fluent Icons glyph shown before the name.
    pub(super) icon: &'static str,
    pub(super) title: Label,
    pub(super) detail: Label,
    pub(super) control: Control,
}

impl Row {
    pub(super) fn title(&self) -> &'static str {
        (self.title)(tr())
    }

    pub(super) fn detail(&self) -> &'static str {
        (self.detail)(tr())
    }

    /// Whether the row shows, with `values`: the one for UI access only in
    /// the installed copy.
    pub(super) fn shown(&self, values: &Values) -> bool {
        !matches!(self.control, Control::Switch(Switch::UiAccess)) || values.ui_access.is_some()
    }
}

pub(super) const fn row(icon: &'static str, title: Label, detail: Label, control: Control) -> Row {
    Row {
        icon,
        title,
        detail,
        control,
    }
}

pub(super) const SECTIONS: &[(Label, &[Row])] = &[
    (
        |s| s.general,
        &[
            row(
                "\u{E7E8}",
                |s| s.start_with_windows,
                |s| s.start_with_windows_detail,
                Control::Switch(Switch::Autostart),
            ),
            row(
                "\u{E81E}",
                |s| s.above_start,
                |s| s.above_start_detail,
                Control::Switch(Switch::UiAccess),
            ),
            row(
                "\u{E895}",
                |s| s.check_updates,
                |s| s.check_updates_detail,
                Control::Switch(Switch::Updates),
            ),
            row(
                "\u{F2B7}",
                |s| s.language,
                |s| s.language_detail,
                Control::Choice(Choice::Language),
            ),
            // Its description sums up the shortcuts' keys.
            row(
                "\u{E765}",
                |s| s.shortcuts,
                |_| "",
                Control::Fold(Fold::Shortcuts),
            ),
        ],
    ),
    (
        |s| s.card,
        &[
            row(
                "\u{E790}",
                |s| s.theme,
                |s| s.theme_detail,
                Control::Choice(Choice::Theme),
            ),
            row(
                "\u{E8E9}",
                |s| s.text_size,
                |s| s.text_size_detail,
                Control::Counter(Counter::Zoom),
            ),
            row(
                "\u{E736}",
                |s| s.definitions,
                |s| s.definitions_detail,
                Control::Counter(Counter::Definitions),
            ),
            row(
                "\u{E8FD}",
                |s| s.synonyms,
                |s| s.synonyms_detail,
                Control::Counter(Counter::Synonyms),
            ),
            row(
                "\u{E8BD}",
                |s| s.examples,
                |s| s.examples_detail,
                Control::Switch(Switch::Examples),
            ),
            row(
                "\u{E7F8}",
                |s| s.animations,
                |s| s.animations_detail,
                Control::Switch(Switch::Animations),
            ),
        ],
    ),
    (
        |s| s.lookup,
        &[
            row(
                "\u{E753}",
                |s| s.online_dictionary,
                |s| s.online_dictionary_detail,
                Control::Switch(Switch::Online),
            ),
            // Its description is what the chosen way does.
            row("\u{E77F}", |s| s.read_selection, |_| "", Control::Selection),
            // Its description sums up the languages chosen, each in itself.
            row(
                "\u{E774}",
                |s| s.languages,
                |_| "English",
                Control::Fold(Fold::Languages),
            ),
        ],
    ),
    (
        |s| s.translation,
        &[
            row(
                "\u{E8C1}",
                |s| s.default_languages,
                |s| s.default_languages_detail,
                Control::Translate,
            ),
            row(
                "\u{E715}",
                |s| s.mymemory_email,
                |s| s.mymemory_email_detail,
                Control::Email,
            ),
        ],
    ),
];
