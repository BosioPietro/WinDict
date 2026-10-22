//! WinDict in Italian.

use super::{Strings, EN};

pub const IT: Strings = Strings {
    // Notifications from the tray icon.
    settings_not_loaded: "Impostazioni non caricate, uso quelle predefinite",
    settings_not_reloaded: "Impostazioni non ricaricate, nulla è cambiato",
    settings_not_saved: "Impostazioni non salvate",
    dictionary_not_loaded: "Dizionario non caricato",
    running: "WinDict è in esecuzione",
    running_detail: "Seleziona una parola in qualsiasi app e premi {hotkey}.",
    shortcut_unavailable: "Scorciatoia non disponibile",
    shortcut_unavailable_detail: "{hotkey} è già usata da un\u{2019}altra app.",
    shortcut_invalid: "Scorciatoia non valida",
    draw_window_failed: "WinDict non è riuscito a disegnare la finestra",
    draw_menu_failed: "WinDict non è riuscito a disegnare il menu",
    draw_settings_failed: "WinDict non è riuscito a disegnare le sue impostazioni",
    not_downloaded: "{language} non scaricato",
    settings_reloaded: "Impostazioni ricaricate",
    settings_reloaded_detail: "Cerca le parole con {hotkey}.",
    downloading: "Download in corso\u{2026} {percent}%",

    // The card: messages.
    too_long: "Troppo testo",
    too_long_detail: "Seleziona una sola parola o una frase breve.",
    nothing_selected: "Nessuna selezione",
    nothing_selected_detail: "Seleziona una parola in un\u{2019}app qualsiasi, poi premi {hotkey}.",
    not_found: "Nessuna definizione trovata",
    not_found_detail: "Questa parola non è nel dizionario.",
    not_found_check_spelling: "Questa parola non è nel dizionario. Controlla l\u{2019}ortografia",
    lookup_failed: "Impossibile cercarla",
    did_you_mean: "Intendevi {word}?",

    // The card: the search box.
    search_placeholder: "Cerca una parola",
    search_title: "Nessun testo selezionato",
    search_again_title: "Cerca un\u{2019}altra parola",
    search_too_long: "Prova con una sola parola o una frase breve",
    no_suggestions: "Nessun risultato nel dizionario offline",
    no_suggestions_online: "Nessun risultato offline \u{2014} Invio cerca online",
    key_look_up: "Cerca",
    key_close: "Chiudi",

    // The card: a definition.
    similar: "Sinonimi",
    opposite: "Contrari",
    also_form_of: "Anche forma di \u{201C}{word}\u{201D}",
    base_form_of: "Forma base di \u{201C}{word}\u{201D}",
    pos_noun: "sostantivo",
    pos_verb: "verbo",
    pos_adjective: "aggettivo",
    pos_adverb: "avverbio",
    pos_phrase: "locuzione",
    pos_interjection: "interiezione",
    pos_pronoun: "pronome",
    pos_preposition: "preposizione",
    pos_conjunction: "congiunzione",
    pos_article: "articolo",
    pos_abbreviation: "abbreviazione",
    pos_other: "altro",

    // The card: translating.
    back_tip: "Indietro (Backspace)",
    translate_tip: "Traduci (Ctrl+T)",
    swap_tip: "Inverti le lingue",
    translate_ambiguous: "\u{201C}{word}\u{201D} è una parola sia in {from} che in {to}.",
    translate_into: "Traduci in {language}",
    translating: "Traduzione in corso\u{2026}",
    translate_failed: "Impossibile tradurre",
    also: "Anche",
    no_languages_match: "Nessuna lingua corrisponde",

    // The tray menu.
    menu_reload: "Ricarica le impostazioni",
    menu_quit: "Esci",

    // Settings: the window.
    settings: "Impostazioni",
    settings_window_title: "Impostazioni di WinDict",
    config_link: "Apri config.json",
    language: "Lingua",
    language_detail: "La lingua dell\u{2019}applicazione",
    theme_system: "Sistema",
    theme_light: "Chiaro",
    theme_dark: "Scuro",
    email_tip: "MyMemory, il traduttore usato quando Google Traduttore non è raggiungibile, \
        consente 5.000 caratteri al giorno, o 50.000 con un indirizzo email. \
        Viene inviato solo a MyMemory, insieme alle parole tradotte.",

    // Settings: recording a shortcut.
    shortcut_detail: "Per cercare la parola selezionata",
    recording_detail: "Premi la nuova scorciatoia, o Esc per annullare",
    recording_hint: "Premi i tasti",
    needs_modifier: "Tieni premuto Ctrl, Alt o Win insieme al tasto",
    unusable_key: "Questo tasto non può far parte di una scorciatoia",
    shortcut_taken: "Già usata da Windows o da un\u{2019}altra app",

    // Settings: sections and rows, each with its description under it.
    general: "Generali",
    shortcut: "Scorciatoia",
    start_with_windows: "Avvia con Windows",
    start_with_windows_detail: "Avvia WinDict insieme al PC",
    card: "Scheda",
    theme: "Tema",
    theme_detail: "Colori dell\u{2019}applicazione",
    definitions: "Definizioni",
    definitions_detail: "Significati mostrati per parola",
    synonyms: "Sinonimi",
    synonyms_detail: "Parole simili sotto ogni significato",
    examples: "Esempi",
    examples_detail: "Frasi per ogni significato",
    animations: "Animazioni",
    animations_detail: "Usa le animazioni nell\u{2019}app",
    translation: "Traduzione",
    default_languages: "Lingue predefinite",
    default_languages_detail: "Da e verso cui si traduce quando non si può determinare",
    mymemory_email: "Email di MyMemory",
    mymemory_email_detail: "Alza il limite giornaliero",
    lookup: "Ricerca",
    online_dictionary: "Dizionario online",
    online_dictionary_detail: "Offre risultati migliori",
    read_selection: "Leggi direttamente la selezione",
    read_selection_detail: "Non usare gli appunti nelle app che lo permettono",
    languages: "Lingue",

    // Errors, which reach the user as the detail of a message.
    hotkey_many_keys: "\"{hotkey}\" ha più di un tasto oltre ai modificatori",
    hotkey_unknown_key: "Tasto sconosciuto \"{key}\" in \"{hotkey}\"",
    hotkey_no_key: "\"{hotkey}\" non ha nessun tasto oltre ai modificatori",
    hotkey_no_modifier: "\"{hotkey}\" richiede almeno un modificatore (Ctrl, Alt, Maiusc, Win)",
    network_unavailable: "Impossibile raggiungere il dizionario. Controlla la connessione.",
    network_init_failed: "Impossibile connettersi alla rete.",
    response_too_large: "La risposta del dizionario era inaspettatamente grande.",
    too_many_lookups: "Troppe ricerche, riprova tra un attimo.",
    lookup_service_error: "Il servizio del dizionario ha restituito un errore ({code}).",
    too_many_translations: "Troppe traduzioni, riprova tra un attimo.",
    translation_service_error: "Il servizio di traduzione ha restituito un errore ({code}).",
    translation_unreachable: "Impossibile raggiungere il servizio di traduzione. Controlla la connessione.",
    no_translation: "Google Traduttore non ha trovato nessuna traduzione.",
    mymemory_no_translation: "MyMemory non ha trovato nessuna traduzione.",
    mymemory_unreadable: "Impossibile leggere la risposta di MyMemory.",
    mymemory_quota: "Le traduzioni gratuite di oggi sono finite. Un\u{2019}email in \
        Impostazioni \u{2192} Traduzione alza il limite.",
    not_language_code: "\u{201C}{language}\u{201D} non è un codice lingua, come \u{201C}en\u{201D}.",
    not_dictionary: "{path} non è un dizionario WinDict per \u{201C}{language}\u{201D}, oppure è per un\u{2019}altra versione di WinDict.",
    download_unreachable: "Impossibile connettersi. Riprova più tardi.",
    download_damaged: "Il download era corrotto. Riprova.",
    download_status: "Download non riuscito ({status}).",
    save_failed: "Impossibile salvarlo: {reason}",

    // `off` stays as it is: "Off" reads the same, and fits its place.
    ..EN
};
