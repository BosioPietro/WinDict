//! `wd`, WinDict on the command line: `wd define` looks a word up,
//! `wd translate` translates text and `wd gloss` picks out the uncommon
//! words of a passage, the way the card does, and prints what the card
//! would show. They go by WinDict's settings, and what's given on
//! the command line goes before them.
//!
//! Answers go to stdout, styled and wrapped to the window when it's a
//! console, as plain lines when it's piped or redirected; what went wrong
//! goes to stderr. It exits with 0 for an answer, 1 for none, and 2 when
//! the command line can't be made sense of.

use std::io::{self, IsTerminal, Read, StdoutLock, Write};
use std::ops::Range;
use std::process::ExitCode;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
use windows::Win32::System::Console::{
    GetConsoleCP, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle, SetConsoleMode,
    CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_OUTPUT_HANDLE,
};

use crate::config::{self, Config};
use crate::dictionary::{self, Definition, Detail, Lookup, NormalizeError};
use crate::i18n::{self, fill, tr};
use crate::lang::LangCode;
use crate::render::capitalize;
use crate::translate::{self, Direction, Translation};
use crate::wikipedia::{self, Article};
use crate::{dicts, gloss, offline, online};

/// Exit codes, besides success.
const NO_ANSWER: u8 = 1;
const USAGE: u8 = 2;

/// Where a sense's text starts, past its number.
const SENSE_INDENT: usize = 4;
/// Where the translations' meanings back start, past the widest of them,
/// unless that's wider than this.
const MAX_TERM_WIDTH: usize = 24;
/// The widest lines are, however wide the window: longer ones are hard to
/// read.
const MAX_COLUMNS: usize = 88;

/// Runs the command on the command line, and says how that went.
pub fn run() -> ExitCode {
    let (config, config_error) = match config::read() {
        Ok(config) => (config, None),
        Err(e) => (Config::default(), Some(e)),
    };
    i18n::set(config.ui_language);
    let given: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // `wd` alone, typed at the prompt: how to use it.
    if given.is_empty() && io::stdin().is_terminal() {
        return help();
    }
    let args = match parse(given) {
        Ok(args) => args,
        Err(e) => return usage_error(&e),
    };
    // Said only when they're gone by.
    let answers = matches!(
        args.command,
        Command::Define | Command::Translate | Command::Gloss
    );
    if let (Some(e), true) = (&config_error, answers) {
        message(tr().settings_not_loaded, e);
    }
    let mut out = Printer::new();
    match args.command {
        Command::Help => help(),
        Command::Version => {
            let _ = writeln!(out.out, "WinDict {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Command::Define => define(&args, &config, &mut out),
        Command::Translate => translate_text(&args, &config, &mut out),
        Command::Gloss => gloss_text(&args, &config, &mut out),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Define,
    Translate,
    Gloss,
    Help,
    Version,
}

/// What the command line asks for. What it leaves out is as the settings
/// have it.
#[derive(Debug, PartialEq)]
struct Args {
    command: Command,
    /// What's to be looked up or translated, a word to an argument.
    text: Vec<String>,
    /// `--lang`: only the definition in this language, or the language
    /// the passage to gloss is in.
    language: Option<String>,
    /// `--from` and `--to`: which way to translate.
    from: Option<String>,
    to: Option<String>,
    senses: Option<usize>,
    synonyms: Option<usize>,
    examples: Option<bool>,
    online: Option<bool>,
}

/// Makes sense of the command line, without the program's name. A first
/// word that isn't a command is the word to look up. The error says what
/// couldn't be made sense of.
fn parse(given: Vec<String>) -> Result<Args, String> {
    let (command, rest) = match given.first().map(String::as_str) {
        Some("define" | "d") => (Command::Define, &given[1..]),
        Some("translate" | "t") => (Command::Translate, &given[1..]),
        Some("gloss" | "g") => (Command::Gloss, &given[1..]),
        Some("help") => (Command::Help, &given[1..]),
        _ => (Command::Define, &given[..]),
    };
    let mut args = Args {
        command,
        text: Vec::new(),
        language: None,
        from: None,
        to: None,
        senses: None,
        synonyms: None,
        examples: None,
        online: None,
    };
    let mut rest = rest.iter();
    let mut only_text = false;
    while let Some(arg) = rest.next() {
        // "-" alone, and anything after "--", is text, even "-5°C".
        if only_text || !arg.starts_with('-') || arg == "-" {
            args.text.push(arg.clone());
            continue;
        }
        if arg == "--" {
            only_text = true;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => (name, Some(value)),
            _ => (arg.as_str(), None),
        };
        let mut value = || match inline {
            Some(value) => Ok(value.to_string()),
            None => rest
                .next()
                .cloned()
                .ok_or_else(|| fill(tr().cli_missing_value, &[("option", &name)])),
        };
        let number = |value: String| {
            value.parse::<usize>().map_err(|_| {
                fill(
                    tr().cli_not_a_number,
                    &[("option", &name), ("value", &value)],
                )
            })
        };
        let flag = matches!(
            name,
            "-h" | "--help"
                | "-V"
                | "--version"
                | "--online"
                | "--offline"
                | "--examples"
                | "--no-examples"
        );
        // "--offline=false" mustn't be taken for "--offline".
        if flag && inline.is_some() {
            return Err(fill(tr().cli_takes_no_value, &[("option", &name)]));
        }
        match (command, name) {
            // What else is given doesn't matter then.
            (_, "-h" | "--help") => {
                args.command = Command::Help;
                return Ok(args);
            }
            (_, "-V" | "--version") => {
                args.command = Command::Version;
                return Ok(args);
            }
            (Command::Help, _) => {}
            (_, "--online") => args.online = Some(true),
            (_, "--offline") => args.online = Some(false),
            (Command::Define, "-l" | "--lang") => args.language = Some(value()?),
            (Command::Define, "-n" | "--senses") => args.senses = Some(number(value()?)?),
            (Command::Define, "-s" | "--synonyms") => args.synonyms = Some(number(value()?)?),
            (Command::Define, "--examples") => args.examples = Some(true),
            (Command::Define, "--no-examples") => args.examples = Some(false),
            (Command::Translate, "-f" | "--from") => args.from = Some(value()?),
            (Command::Translate, "-t" | "--to") => args.to = Some(value()?),
            (Command::Gloss, "-l" | "--lang") => args.language = Some(value()?),
            _ => return Err(fill(tr().cli_unknown_option, &[("option", &name)])),
        }
    }
    Ok(args)
}

/// Looks the word up, offline and then online, as the card does, and
/// prints its definition, or else its Wikipedia article, or what the word
/// was likely a typo of.
fn define(args: &Args, config: &Config, out: &mut Printer) -> ExitCode {
    let written = dictionary::as_written(&text(&args.text)).ok();
    let word = match dictionary::normalize(&text(&args.text)) {
        Ok(word) => word,
        Err(NormalizeError::Empty) => return usage_error(tr().cli_no_word),
        Err(NormalizeError::TooLong) => return failure(tr().too_long, tr().search_too_long),
    };
    let detail = Detail {
        senses: args.senses.unwrap_or(config.max_definitions).max(1),
        synonyms: args.synonyms.unwrap_or(config.max_synonyms),
        examples: args.examples.unwrap_or(config.show_examples),
    };
    // In one language, its dictionary is used, if it's been downloaded,
    // whether it's in the settings or not.
    let language = match args
        .language
        .as_deref()
        .map(dictionary_language)
        .transpose()
    {
        Ok(language) => language,
        Err(e) => return usage_error(&e),
    };
    match &language {
        Some(language) => {
            let _ = offline::set_languages(std::slice::from_ref(language));
        }
        None => load_dictionaries(config),
    }
    let within = |def: Definition| match &language {
        Some(language) => def.only(language),
        None => Some(def),
    };
    let local = offline::lookup(&word, detail).and_then(within);
    let use_online = args.online.unwrap_or(config.online_lookup);
    let online = use_online.then(|| {
        // The word the offline entry is of, when it's a form of another.
        let key = match &local {
            Some(def) => def.word.to_lowercase(),
            None => word.clone(),
        };
        let languages = match &language {
            Some(language) => vec![language.clone()],
            None => offline::languages(),
        };
        let _waiting = Waiting::show(tr().looking_up);
        online::lookup(&key, detail, &languages)
    });
    let result = match (local, online) {
        (Some(local), online) => Lookup::Found(dictionary::merge(
            local,
            online.as_ref(),
            &offline::languages(),
        )),
        (None, Some(online)) => online,
        (None, None) => Lookup::NotFound,
    };
    let (title, detail) = match &result {
        Lookup::Found(def) => {
            out.definition(def);
            return ExitCode::SUCCESS;
        }
        Lookup::NotFound => (tr().not_found, None),
        Lookup::Failed(error) => (tr().lookup_failed, Some(error.as_str())),
    };
    // People, places and brands, which the dictionaries skip.
    if use_online && config.wikipedia {
        let languages = match &language {
            Some(language) => vec![language.clone()],
            None => offline::languages(),
        };
        let title = written.unwrap_or_else(|| word.clone());
        let titles: Vec<_> = languages.into_iter().map(|l| (l, title.clone())).collect();
        let found = {
            let _waiting = Waiting::show(tr().looking_up);
            wikipedia::find(&titles)
        };
        if let Ok(Some(article)) = found {
            out.article(&article);
            return ExitCode::SUCCESS;
        }
    }
    let words = offline::Words::load();
    let correction = words.correct(&word);
    let detail = detail.unwrap_or(match correction {
        Some(_) => tr().not_found_detail,
        None => tr().not_found_check_spelling,
    });
    message(title, detail);
    if let Some(correction) = correction {
        eprintln!("{}", fill(tr().did_you_mean, &[("word", &correction)]));
    }
    ExitCode::from(NO_ANSWER)
}

/// Translates the text and prints the translation. A dictionary word goes
/// the way the translation shortcut sends it; a passage, the way the
/// "Translate sentences" box does, by the language it seems to be in.
fn translate_text(args: &Args, config: &Config, out: &mut Printer) -> ExitCode {
    let text = match translate::passage(&text(&args.text)) {
        Ok(text) => text,
        Err(NormalizeError::Empty) => return usage_error(tr().cli_no_text),
        Err(NormalizeError::TooLong) => return failure(tr().too_long, tr().cli_translate_too_long),
    };
    let from = match args.from.as_deref().map(translation_language).transpose() {
        Ok(from) => from,
        Err(e) => return usage_error(&e),
    };
    let to = match args.to.as_deref().map(translation_language).transpose() {
        Ok(to) => to,
        Err(e) => return usage_error(&e),
    };
    if from.is_some() && from == to {
        return usage_error(tr().cli_same_language);
    }
    // Translations come from the internet, like the online answers.
    if !args.online.unwrap_or(config.online_lookup) {
        return failure(tr().translate_offline, tr().cli_translate_offline_detail);
    }
    load_dictionaries(config);
    // A word the dictionary has: which languages it's in tells which way
    // it goes, and its first sense in the one it's from, what it means.
    let brief = Detail {
        senses: 1,
        synonyms: 0,
        examples: false,
    };
    let def = dictionary::normalize(&text)
        .ok()
        .and_then(|word| offline::lookup(&word, brief));
    let direction = direction(&text, def.as_ref(), from, to, config);
    if !direction.sure {
        let word = translate::heading(&text);
        let from = translate::name(&direction.from);
        let to = translate::name(&direction.to);
        let guessed = fill(tr().cli_guessed_direction, &[("from", &from), ("to", &to)]);
        // A word in the dictionary is in both languages; anything else, in
        // one that couldn't be told.
        match def {
            Some(_) => {
                let names: [(&str, &dyn std::fmt::Display); 3] =
                    [("word", &word), ("from", &from), ("to", &to)];
                eprintln!("{} {guessed}", fill(tr().translate_ambiguous, &names));
            }
            None => eprintln!("{guessed}"),
        }
    }
    let context = def.as_ref().and_then(|def| {
        let section = def.sections().find(|s| s.language == direction.from)?;
        Some(section.meanings.first()?.senses.first()?.text.as_str())
    });
    let result = {
        let _waiting = Waiting::show(tr().translating);
        translate::translate(
            &text,
            &direction.from,
            &direction.to,
            context,
            &config.mymemory_email,
        )
    };
    match result {
        Ok(translation) => {
            out.translation(&translation, &text, &direction);
            ExitCode::SUCCESS
        }
        Err(e) => failure(tr().translate_failed, &e),
    }
}

/// Picks out the uncommon words of the passage, by the language it's in,
/// as the card does, and prints it with them marked, then what each of
/// them means: offline, and then online, all at once.
fn gloss_text(args: &Args, config: &Config, out: &mut Printer) -> ExitCode {
    let text = match translate::passage(&text(&args.text)) {
        Ok(text) => text,
        Err(NormalizeError::Empty) => return usage_error(tr().cli_no_passage),
        Err(NormalizeError::TooLong) => return failure(tr().too_long, tr().cli_gloss_too_long),
    };
    // Every language WinDict translates has its list of common words.
    let language = match args
        .language
        .as_deref()
        .map(translation_language)
        .transpose()
    {
        Ok(language) => language,
        Err(e) => return usage_error(&e),
    };
    load_dictionaries(config);
    let Some(gloss) = gloss::with_dictionaries(&text, language.as_ref(), &config.languages) else {
        return failure(tr().gloss_unknown, tr().cli_gloss_unknown_detail);
    };
    // Each word once, as it's first written.
    let mut words: Vec<&str> = Vec::new();
    for word in gloss.words() {
        if !words
            .iter()
            .any(|w| w.to_lowercase() == word.to_lowercase())
        {
            words.push(word);
        }
    }
    let brief = Detail {
        senses: 1,
        synonyms: 0,
        examples: false,
    };
    let language = gloss.lookup_language();
    let local = |word: &str| {
        let word = dictionary::normalize(word).ok()?;
        offline::lookup(&word, brief)?.only(&language)
    };
    let mut found: Vec<Option<Definition>> = words.iter().map(|w| local(w)).collect();
    if args.online.unwrap_or(config.online_lookup) && found.iter().any(Option::is_none) {
        let _waiting = Waiting::show(tr().looking_up);
        let languages = std::slice::from_ref(&language);
        std::thread::scope(|scope| {
            let lookups: Vec<_> = (words.iter().zip(&found))
                .map(|(word, def)| {
                    let word = dictionary::normalize(word).ok().filter(|_| def.is_none())?;
                    Some(scope.spawn(move || online::lookup(&word, brief, languages)))
                })
                .collect();
            for (def, lookup) in found.iter_mut().zip(lookups) {
                if let Some(Ok(Lookup::Found(online))) = lookup.map(|l| l.join()) {
                    *def = online.only(&language);
                }
            }
        });
    }
    let glossary: Vec<(&str, Option<Definition>)> = words.into_iter().zip(found).collect();
    out.gloss(&gloss, &glossary);
    ExitCode::SUCCESS
}

/// Which way to translate `text`, which the dictionary has as `def`, if
/// it's a word it has: as the translation shortcut would, unless it's
/// `from` or `to` a language given. Not `sure` only when it was a guess.
fn direction(
    text: &str,
    def: Option<&Definition>,
    from: Option<LangCode>,
    to: Option<LangCode>,
    config: &Config,
) -> Direction {
    let defaults = (config.translate_from.as_str(), config.translate_to.as_str());
    let enabled = &config.languages;
    let guess = || match def {
        Some(def) => {
            let found: Vec<&str> = def.sections().map(|s| s.language.as_str()).collect();
            translate::pick(&found, enabled, defaults)
        }
        // Typed, as far as WinDict can tell: from the language most of its
        // words are in.
        None => translate::typed(offline::detect(text).as_deref(), enabled, defaults),
    };
    match (from, to) {
        (Some(from), Some(to)) => Direction {
            from,
            to,
            sure: true,
        },
        (Some(from), None) => Direction {
            sure: true,
            ..translate::typed(Some(&from), enabled, defaults)
        },
        // Into the language it seems to be in: then it's from the other.
        (None, Some(to)) => {
            let guess = guess();
            let from = if guess.from == to {
                guess.to
            } else {
                guess.from
            };
            Direction {
                from,
                to,
                sure: true,
            }
        }
        (None, None) => guess(),
    }
}

/// The dictionaries of the settings, saying so on stderr when one can't
/// be read.
fn load_dictionaries(config: &Config) {
    if let Err(e) = offline::set_languages(&config.languages) {
        message(tr().dictionary_not_loaded, &e);
    }
}

/// What's to be looked up or translated: what's on the command line, else
/// what's piped in. Nothing when there's neither.
fn text(given: &[String]) -> String {
    if !given.is_empty() {
        return given.join(" ");
    }
    let mut stdin = io::stdin();
    if stdin.is_terminal() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = stdin.read_to_end(&mut bytes);
    decode(&bytes, unsafe { GetConsoleCP() })
}

/// Text piped in: UTF-8, with or without its mark, UTF-16 with its mark
/// (a "Unicode" file), or else in the console's `code_page`, as cmd.exe
/// pipes it (`echo città | wd`).
fn decode(bytes: &[u8], code_page: u32) -> String {
    if let Some(utf16) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = utf16
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_le_bytes(pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_owned();
    }
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    let len = unsafe { MultiByteToWideChar(code_page, flags, bytes, None) };
    let mut units = vec![0; len.max(0) as usize];
    let len = unsafe { MultiByteToWideChar(code_page, flags, bytes, Some(&mut units)) };
    match len {
        0 => String::from_utf8_lossy(bytes).into_owned(),
        len => String::from_utf16_lossy(&units[..len as usize]),
    }
}

/// A language to translate from or into, given as its code ("de") or its
/// name ("German", "Deutsch"), whatever the case, or a part of it that's
/// only its own ("germ").
fn translation_language(given: &str) -> Result<LangCode, String> {
    let given = given.trim();
    let code = translate::LANGUAGES
        .iter()
        .map(|(code, _)| *code)
        .find(|code| code.eq_ignore_ascii_case(given));
    let code = code.or(match translate::search(given)[..] {
        [code] => Some(code),
        _ => None,
    });
    code.map(LangCode::new)
        .ok_or_else(|| fill(tr().cli_unknown_language, &[("language", &given)]))
}

/// A language to look words up in, given as its code or its name, like a
/// language to translate into; a code WinDict doesn't know is taken as it
/// is, for Wiktionary, if it looks like one ("grc", "zh-min-nan").
fn dictionary_language(given: &str) -> Result<LangCode, String> {
    let given = given.trim();
    let known = dicts::languages()
        .find(|(code, name)| code.eq_ignore_ascii_case(given) || name.eq_ignore_ascii_case(given))
        .map(|(code, _)| LangCode::new(code))
        .or_else(|| translation_language(given).ok());
    if let Some(code) = known {
        return Ok(code);
    }
    let mut parts = given.split('-');
    let first = parts.next().unwrap_or_default();
    let code_like = (2..=3).contains(&first.len())
        && first.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()));
    match code_like {
        true => Ok(LangCode::new(given.to_ascii_lowercase())),
        false => Err(fill(tr().cli_unknown_language, &[("language", &given)])),
    }
}

/// How to use `wd`. A pipe closed early isn't anything to tell anyone
/// about, as for the answers.
fn help() -> ExitCode {
    let _ = writeln!(io::stdout(), "{}", tr().cli_help);
    ExitCode::SUCCESS
}

/// A message on stderr: what went wrong, then why, or what to do.
fn message(title: &str, detail: &str) {
    eprintln!("{title}");
    if !detail.is_empty() {
        eprintln!("{detail}");
    }
}

fn failure(title: &str, detail: &str) -> ExitCode {
    message(title, detail);
    ExitCode::from(NO_ANSWER)
}

fn usage_error(error: &str) -> ExitCode {
    message(error, tr().cli_see_help);
    ExitCode::from(USAGE)
}

/// What's being waited for, on stderr while it's a console, rubbed out
/// again once it's dropped.
struct Waiting(usize);

impl Waiting {
    fn show(text: &str) -> Waiting {
        if !io::stderr().is_terminal() {
            return Waiting(0);
        }
        eprint!("{text}");
        Waiting(text.chars().count())
    }
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if self.0 > 0 {
            eprint!("\r{}\r", " ".repeat(self.0));
        }
    }
}

/// How a piece of the answer is printed, in a console that shows styles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Plain,
    /// The word, and the translation.
    Strong,
    /// What's around them: pronunciations, notes, sources.
    Faint,
    /// The headings over the parts of speech, as the card has them.
    Accent,
    /// Examples.
    Quote,
    /// A passage's uncommon words.
    Marked,
}

impl Style {
    /// Its SGR parameters.
    fn sgr(self) -> &'static str {
        match self {
            Style::Plain => "",
            Style::Strong => "1",
            Style::Faint => "2",
            Style::Accent => "36",
            Style::Quote => "2;3",
            Style::Marked => "4;36",
        }
    }
}

/// Prints answers to stdout.
struct Printer {
    out: StdoutLock<'static>,
    styled: bool,
    /// How wide lines can be.
    columns: usize,
    /// The console's mode before styles were turned on, to put back.
    restore: Option<(HANDLE, CONSOLE_MODE)>,
}

impl Drop for Printer {
    fn drop(&mut self) {
        if let Some((handle, mode)) = self.restore {
            let _ = self.out.flush();
            let _ = unsafe { SetConsoleMode(handle, mode) };
        }
    }
}

impl Printer {
    fn new() -> Printer {
        let console = unsafe { console() };
        let restore = console.and_then(|c| c.restore);
        let (styled, columns) = console.map_or((false, usize::MAX), |c| {
            // A column spare: the console moves to the next line once the
            // last is written.
            (c.styled, c.columns.saturating_sub(1).min(MAX_COLUMNS))
        });
        // https://no-color.org
        let styled = styled && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
        Printer {
            out: io::stdout().lock(),
            styled,
            columns,
            restore,
        }
    }

    /// Writes `spans` as a paragraph, see [`wrap`]. Errors, like a pipe
    /// closed early, aren't anything to tell anyone about.
    fn line(&mut self, margin: usize, lead: (Style, &str), indent: usize, spans: &[(Style, &str)]) {
        let text = wrap(
            margin,
            lead,
            indent,
            spans,
            self.columns.max(indent + 20),
            self.styled,
        );
        let _ = writeln!(self.out, "{text}");
    }

    fn text(&mut self, spans: &[(Style, &str)]) {
        self.line(0, (Style::Plain, ""), 0, spans);
    }

    fn blank(&mut self) {
        let _ = writeln!(self.out);
    }

    /// The word and how it's said, then each language's part (under its
    /// name, when there are several) with its parts of speech, then where
    /// it all comes from.
    fn definition(&mut self, def: &Definition) {
        let mut title = vec![(Style::Strong, def.word.as_str())];
        if let Some(phonetic) = &def.phonetic {
            title.push((Style::Faint, phonetic));
        }
        self.text(&title);
        let grouped = !def.others.is_empty();
        for section in def.sections() {
            if grouped {
                self.blank();
                let name = dicts::name(&section.language);
                let mut heading = vec![(Style::Strong, name.as_str())];
                // The base form of what was looked up.
                if section.word.to_lowercase() != def.word.to_lowercase() {
                    heading.push((Style::Faint, &section.word));
                }
                self.text(&heading);
            }
            if let Some(note) = &section.note {
                self.text(&[(Style::Faint, note)]);
            }
            for (i, meaning) in section.meanings.iter().enumerate() {
                if i > 0 || !grouped {
                    self.blank();
                }
                let pos = tr().part_of_speech(&meaning.part_of_speech).to_uppercase();
                self.text(&[(Style::Accent, &pos)]);
                for (n, sense) in meaning.senses.iter().enumerate() {
                    let number = format!("{}.", n + 1);
                    let lead = (Style::Faint, number.as_str());
                    self.line(0, lead, SENSE_INDENT, &[(Style::Plain, &sense.text)]);
                    if let Some(example) = &sense.example {
                        let quoted = format!("\u{201C}{example}\u{201D}");
                        let none = (Style::Plain, "");
                        self.line(SENSE_INDENT, none, SENSE_INDENT, &[(Style::Quote, &quoted)]);
                    }
                }
                for (label, words) in [
                    (tr().similar, &meaning.synonyms),
                    (tr().opposite, &meaning.antonyms),
                ] {
                    if !words.is_empty() {
                        let label = format!("{label}:");
                        let lead = (Style::Faint, label.as_str());
                        let words = words.join(", ");
                        self.line(SENSE_INDENT, lead, SENSE_INDENT, &[(Style::Plain, &words)]);
                    }
                }
            }
        }
        self.blank();
        for (source, word) in def.all_sources() {
            let url = dictionary::source_url(source, word);
            self.source(source, url.as_deref());
        }
    }

    /// Which way it went, the translation and what the word was taken to
    /// mean, the other translations by part of speech, each with what it
    /// means back, then where it comes from.
    fn translation(&mut self, translation: &Translation, text: &str, direction: &Direction) {
        let (from, to) = (&direction.from, &direction.to);
        let way = format!("{} \u{2192} {}", translate::name(from), translate::name(to));
        self.text(&[(Style::Faint, &way)]);
        // A passage is read, not glanced at.
        let style = if translate::is_passage(&translation.text) {
            Style::Plain
        } else {
            Style::Strong
        };
        self.text(&[(style, &capitalize(&translation.text))]);
        if let Some(context) = &translation.context {
            let quoted = format!("\u{201C}{}\u{201D}", capitalize(context));
            self.text(&[(Style::Quote, &quoted)]);
        }
        for group in &translation.groups {
            self.blank();
            let label = match group.part_of_speech.as_str() {
                "" => tr().also,
                pos => tr().part_of_speech(pos),
            };
            self.text(&[(Style::Accent, &label.to_uppercase())]);
            let widest = group.terms.iter().map(|t| t.text.chars().count()).max();
            let indent = 2 + widest.unwrap_or(0).min(MAX_TERM_WIDTH) + 2;
            for term in &group.terms {
                let word = capitalize(&term.text);
                let back = capitalize(&term.back.join(", "));
                self.line(2, (Style::Plain, &word), indent, &[(Style::Faint, &back)]);
            }
        }
        self.blank();
        let source = translation.source;
        let url = (source == translate::GOOGLE).then(|| {
            format!(
                "https://translate.google.com/?sl={from}&tl={to}&text={}&op=translate",
                dictionary::percent_encode(text)
            )
        });
        self.source(source, url.as_deref());
    }

    /// The language of the passage, the passage with its uncommon words
    /// marked, then each of them with its first sense, under the base form
    /// it's of, when it's another.
    fn gloss(&mut self, gloss: &gloss::Gloss, glossary: &[(&str, Option<Definition>)]) {
        self.text(&[(Style::Faint, &translate::name(&gloss.language))]);
        let columns = self.columns.max(20);
        let passage = mark(&gloss.text, &gloss.words, columns, self.styled);
        let _ = writeln!(self.out, "{passage}");
        self.blank();
        if glossary.is_empty() {
            self.text(&[(Style::Faint, tr().gloss_none)]);
            return;
        }
        let widest = glossary.iter().map(|(w, _)| w.chars().count()).max();
        let indent = 2 + widest.unwrap_or(0).min(MAX_TERM_WIDTH) + 2;
        for (word, def) in glossary {
            let mut spans = Vec::new();
            let sense = def.as_ref().and_then(|def| {
                if def.word.to_lowercase() != word.to_lowercase() {
                    spans.push((Style::Faint, def.word.as_str()));
                }
                Some(def.meanings.first()?.senses.first()?.text.as_str())
            });
            match sense {
                Some(sense) => spans.push((Style::Plain, sense)),
                None => spans = vec![(Style::Faint, tr().not_found)],
            }
            self.line(2, (Style::Strong, word), indent, &spans);
        }
    }

    /// The article's title and what it's about, its first sentences, and
    /// its page.
    fn article(&mut self, article: &Article) {
        let mut title = vec![(Style::Strong, article.title.as_str())];
        if let Some(description) = &article.description {
            title.push((Style::Faint, description));
        }
        self.text(&title);
        self.blank();
        match article.ambiguous {
            true => self.text(&[(Style::Faint, tr().wikipedia_ambiguous)]),
            false => self.text(&[(Style::Plain, &article.extract)]),
        }
        self.blank();
        self.source(tr().wikipedia, Some(&article.url));
    }

    /// Where the answer comes from, and its page there, if it has one.
    fn source(&mut self, name: &str, url: Option<&str>) {
        let mut spans = vec![(Style::Faint, name)];
        if let Some(url) = url {
            spans.extend([(Style::Faint, "\u{B7}"), (Style::Faint, url)]);
        }
        self.text(&spans);
    }
}

/// The console stdout writes to.
#[derive(Clone, Copy)]
struct Console {
    /// Whether it shows styles.
    styled: bool,
    /// How many columns its window has.
    columns: usize,
    /// Its mode before styles were turned on, if they were.
    restore: Option<(HANDLE, CONSOLE_MODE)>,
}

/// The console stdout writes to, its styles turned on if they're off;
/// nothing when stdout isn't a console.
unsafe fn console() -> Option<Console> {
    let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
    let mut mode = CONSOLE_MODE::default();
    GetConsoleMode(handle, &mut mode).ok()?;
    let on = mode.contains(ENABLE_VIRTUAL_TERMINAL_PROCESSING);
    let turned_on =
        !on && SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING).is_ok();
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    GetConsoleScreenBufferInfo(handle, &mut info).ok()?;
    let columns = info.srWindow.Right - info.srWindow.Left + 1;
    Some(Console {
        styled: on || turned_on,
        columns: columns.max(0) as usize,
        restore: turned_on.then_some((handle, mode)),
    })
}

/// `spans` as a paragraph of lines at most `columns` wide, broken between
/// words, which are one space apart. The first line starts `margin` in,
/// with `lead` (a sense's number, say), and its text `indent` in, or a
/// space after a longer lead; the others start `indent` in.
fn wrap(
    margin: usize,
    lead: (Style, &str),
    indent: usize,
    spans: &[(Style, &str)],
    columns: usize,
    styled: bool,
) -> String {
    let paint = |out: &mut String, (style, text): (Style, &str)| {
        if styled && style != Style::Plain {
            out.push_str(&format!("\x1b[{}m{text}\x1b[0m", style.sgr()));
        } else {
            out.push_str(text);
        }
    };
    let mut out = " ".repeat(margin);
    paint(&mut out, lead);
    let mut column = margin + lead.1.chars().count();
    let mut start = true;
    for &(style, text) in spans {
        for word in text.split_whitespace() {
            let width = word.chars().count();
            if start {
                let to = indent.max(column + usize::from(!lead.1.is_empty()));
                out.push_str(&" ".repeat(to - column));
                column = to;
                start = false;
            } else if column + 1 + width > columns {
                out.push('\n');
                out.push_str(&" ".repeat(indent));
                column = indent;
            } else {
                out.push(' ');
                column += 1;
            }
            paint(&mut out, (style, word));
            column += width;
        }
    }
    out
}

/// `text`, its words one space apart, as a paragraph of lines at most
/// `columns` wide, with the stretches `marked` in it styled as such.
fn mark(text: &str, marked: &[Range<usize>], columns: usize, styled: bool) -> String {
    let mut out = String::new();
    let (mut start, mut column) = (0, 0);
    for word in text.split(' ') {
        let end = start + word.len();
        let width = word.chars().count();
        if column > 0 && column + 1 + width > columns {
            out.push('\n');
            column = 0;
        } else if column > 0 {
            out.push(' ');
            column += 1;
        }
        // A word may be marked only in part ("l'ornitorinco").
        let mut at = start;
        for range in marked.iter().filter(|r| r.start < end && r.end > start) {
            let (from, to) = (range.start.max(start), range.end.min(end));
            out.push_str(&text[at..from]);
            match styled {
                true => out.push_str(&format!(
                    "\x1b[{}m{}\x1b[0m",
                    Style::Marked.sgr(),
                    &text[from..to]
                )),
                false => out.push_str(&text[from..to]),
            }
            at = to;
        }
        out.push_str(&text[at..end]);
        column += width;
        start = end + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Result<Args, String> {
        parse(
            line.split(' ')
                .filter(|a| !a.is_empty())
                .map(String::from)
                .collect(),
        )
    }

    #[test]
    fn parses_commands() {
        let define = args("define ice cream -n 2 --lang=it --no-examples").unwrap();
        assert_eq!(define.command, Command::Define);
        assert_eq!(define.text, ["ice", "cream"]);
        assert_eq!(define.senses, Some(2));
        assert_eq!(define.language.as_deref(), Some("it"));
        assert_eq!(define.examples, Some(false));
        // Without a command, it's a word to look up.
        let bare = args("serendipity --offline").unwrap();
        assert_eq!(
            (bare.command, &bare.text[..]),
            (Command::Define, &["serendipity".to_string()][..])
        );
        assert_eq!(bare.online, Some(false));
        let translate = args("t hello -t de --from English").unwrap();
        assert_eq!(translate.command, Command::Translate);
        assert_eq!(
            (translate.from.as_deref(), translate.to.as_deref()),
            (Some("English"), Some("de"))
        );
        assert_eq!(args("translate -h").unwrap().command, Command::Help);
        assert_eq!(args("-V").unwrap().command, Command::Version);
        // After "--", and "-" alone, it's text.
        assert_eq!(args("t -- -5 -h").unwrap().text, ["-5", "-h"]);
        let gloss = args("g the obsequious clerk -l en --offline").unwrap();
        assert_eq!(gloss.command, Command::Gloss);
        assert_eq!(gloss.text, ["the", "obsequious", "clerk"]);
        assert_eq!(gloss.language.as_deref(), Some("en"));
        assert_eq!(gloss.online, Some(false));
    }

    #[test]
    fn rejects_what_it_cant_make_sense_of() {
        // Each command's options are its own.
        assert!(args("define cat --to it").is_err());
        assert!(args("translate cat --lang it").is_err());
        assert!(args("gloss cat --to it").is_err());
        assert!(args("cat --frobnicate").is_err());
        assert!(args("cat -n").is_err());
        assert!(args("cat -n many").is_err());
        assert!(args("cat --offline=false").is_err());
        assert!(args("cat --examples=no").is_err());
    }

    #[test]
    fn help_and_version_go_before_the_rest() {
        assert_eq!(args("help --lang it").unwrap().command, Command::Help);
        assert_eq!(args("t -V --lang it").unwrap().command, Command::Version);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)]
    fn marks_words_and_wraps() {
        let text = "the obsequious clerk l'ornitorinco";
        assert_eq!(mark(text, &[4..14], 80, false), text);
        assert_eq!(
            mark(text, &[4..14, 23..34], 80, true),
            "the \x1b[4;36mobsequious\x1b[0m clerk l'\x1b[4;36mornitorinco\x1b[0m"
        );
        assert_eq!(
            mark(text, &[], 15, false),
            "the obsequious\nclerk\nl'ornitorinco"
        );
    }

    #[test]
    fn decodes_what_is_piped_in() {
        assert_eq!(decode("città".as_bytes(), 850), "città");
        assert_eq!(decode(b"\xEF\xBB\xBFcat", 850), "cat");
        assert_eq!(decode(b"\xFF\xFEc\0a\0t\0", 850), "cat");
        // cmd.exe's OEM code page, where à is 0x85.
        assert_eq!(decode(b"citt\x85", 850), "città");
    }

    #[test]
    fn finds_languages() {
        let found = |given| translation_language(given).map(|l| l.as_str().to_string());
        assert_eq!(found("de").unwrap(), "de");
        assert_eq!(found("ZH-cn").unwrap(), "zh-CN");
        assert_eq!(found("German").unwrap(), "de");
        assert_eq!(found("deutsch").unwrap(), "de");
        assert!(found("klingon").is_err());
        assert!(found("").is_err());
        let dictionary = |given| dictionary_language(given).map(|l| l.as_str().to_string());
        assert_eq!(dictionary("Italiano").unwrap(), "it");
        assert_eq!(dictionary("EN").unwrap(), "en");
        assert_eq!(dictionary("grc").unwrap(), "grc");
        assert_eq!(dictionary("zh-min-nan").unwrap(), "zh-min-nan");
        assert!(dictionary("klingon").is_err());
        assert!(dictionary("").is_err());
    }

    #[test]
    fn picks_the_way_to_translate() {
        let mut config = Config::default();
        config.languages = vec!["en".into(), "it".into()];
        (config.translate_from, config.translate_to) = ("en".into(), "it".into());
        let way = |from: Option<&str>, to: Option<&str>| {
            let d = direction(
                "xyzzy",
                None,
                from.map(LangCode::new),
                to.map(LangCode::new),
                &config,
            );
            (
                d.from.as_str().to_string(),
                d.to.as_str().to_string(),
                d.sure,
            )
        };
        assert_eq!(
            way(Some("fr"), Some("de")),
            ("fr".into(), "de".into(), true)
        );
        // From Italian, into the other language in use.
        assert_eq!(way(Some("it"), None), ("it".into(), "en".into(), true));
        // Into what it was guessed to be in: from the other one.
        assert_eq!(way(None, Some("en")), ("it".into(), "en".into(), true));
        assert_eq!(way(None, Some("de")), ("en".into(), "de".into(), true));
    }

    #[test]
    fn wraps_paragraphs() {
        let plain = (Style::Plain, "");
        let spans = [(Style::Plain, "one two three four")];
        assert_eq!(wrap(0, plain, 0, &spans, 80, false), "one two three four");
        assert_eq!(
            wrap(0, plain, 2, &spans, 10, false),
            "  one two\n  three\n  four"
        );
        // A lead, with its text from the indent on, or a space after it.
        let number = (Style::Faint, "1.");
        assert_eq!(
            wrap(0, number, 4, &spans, 13, false),
            "1.  one two\n    three\n    four"
        );
        let label = (Style::Faint, "Similar:");
        assert_eq!(
            wrap(4, label, 4, &[(Style::Plain, "a, b")], 80, false),
            "    Similar: a, b"
        );
        // A word longer than the line has a line of its own.
        assert_eq!(
            wrap(0, plain, 0, &[(Style::Plain, "a bbbbbbbb c")], 4, false),
            "a\nbbbbbbbb\nc"
        );
        // Styles are word by word, and none for plain text.
        let styled = wrap(
            0,
            plain,
            0,
            &[(Style::Strong, "run"), (Style::Plain, "x")],
            80,
            true,
        );
        assert_eq!(styled, "\x1b[1mrun\x1b[0m x");
    }
}
