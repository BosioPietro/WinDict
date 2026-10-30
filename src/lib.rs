//! WinDict: select a word anywhere, press a hotkey, read its definition.
//! `main.rs` only calls [`run`]; the tests are this library's.
//!
//! Everything runs on one UI thread (`app`), which owns the state and the
//! windows; reading the selection and online lookups run on threads of
//! their own and post their results back to it.
//!
//! - `app`: the hotkey → selection → lookup → card flow, the tray menu and
//!   settings, and the window procedures that feed them.
//! - `selection`: reads the selected text (UI Automation, else the clipboard).
//! - `gloss`: what of a passage too long to look up is uncommon, by lists
//!   of each language's commonest words.
//! - `dictionary`: entries as the card shows them; `offline` (the bundled
//!   WordNet) and `online` (Free Dictionary API, Wiktionary, and each
//!   language's own Wiktionary, through `kaikki` and `wikizionario`, over
//!   `http`) fill them; `translate` translates the word (Google Translate,
//!   else MyMemory).
//! - `popup` (the card, with `searchbox`), `menu`, `settings` and `intro`:
//!   the windows, each a composition visual tree built from `ui`, drawn by
//!   `render` in the colors of `theme`.
//! - `config`, `hotkey`, `autostart`, `tray`: settings and their storage, the
//!   shortcut, "Start with Windows", and the notification-area icon.
//! - `speech`: saying the word out loud, in its language's voice.
//! - `snip` and `ocr`: picking an area of the screen, and reading the text
//!   in it into the search box.
//! - `update`: looking for a new version, and installing it.
//! - `installed`: the copy the installer put in Program Files, and handing
//!   over to its UI access build.
//! - `jumplist`: the tasks under WinDict in Start, which open the box to
//!   type a word in.
//! - `cli`: `wd`, which looks words up and translates them from the
//!   command line, as the card would.
//! - `setup`: the window of the setup downloaded from the releases page,
//!   which runs that installer (the `windict-setup` executable).
//! - `i18n`: every string the user reads, so a language is a table, not a
//!   change across the code.
//! - `win`: safe wrappers for the Win32 calls shared across modules.

mod app;
mod autostart;
mod cli;
mod config;
mod dictionary;
mod dicts;
mod gloss;
mod hotkey;
mod http;
mod i18n;
mod installed;
mod intro;
mod jumplist;
mod kaikki;
mod lang;
mod menu;
mod ocr;
mod offline;
mod online;
mod popup;
mod render;
mod searchbox;
mod selection;
mod settings;
mod setup;
mod snip;
mod speech;
mod theme;
mod translate;
mod tray;
mod ui;
mod update;
mod wikizionario;
mod win;

use windows::core::w;
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::UI::Composition::Compositor;

/// Runs WinDict until it's quit from the tray, unless it's running already.
pub fn run() -> windows::core::Result<()> {
    // Started from the Jump List.
    let task = jumplist::from_args();
    // With UI access on, the installed WinDict.exe runs the build that has
    // it, which is given the task.
    if installed::hand_over(task.map(jumplist::switch)) {
        return Ok(());
    }
    // Started by the version this one replaces: it has to be gone first.
    let replacing = update::Replacing::from_args();
    unsafe {
        // One instance per user session: a task is the running one's to do.
        let instance = CreateMutexW(None, true, w!("Local\\WinDict.Instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS
            && !replacing.as_ref().is_some_and(|r| r.wait(instance))
        {
            if let Some(task) = task {
                app::forward(task);
            }
            return Ok(());
        }
        let _ = SetCurrentProcessExplicitAppUserModelID(jumplist::APP_ID);
        let (_queue, compositor) = start_ui()?;
        app::run(compositor, replacing, task)
    }
}

/// Runs the setup downloaded from the releases page until it's closed:
/// `installer` is the Inno Setup installer it runs (installer/windict.iss).
/// Given switches, as WinGet and "Update app" give it, it runs the
/// installer with them instead, without a window, and exits with its code.
pub fn setup(installer: &'static [u8]) -> windows::core::Result<()> {
    let switches: Vec<_> = std::env::args_os().skip(1).collect();
    if !switches.is_empty() {
        std::process::exit(setup::quietly(installer, &switches));
    }
    let (_queue, compositor) = unsafe { start_ui() }?;
    setup::run(compositor, installer)
}

/// Runs `wd`, a command given on the command line, until it has printed
/// its answer; see `cli`.
pub fn wd() -> std::process::ExitCode {
    cli::run()
}

/// Makes this thread the UI thread: aware of each monitor's scale, and
/// with what Windows.UI.Composition needs, a dispatcher queue, kept alive
/// by the caller.
unsafe fn start_ui() -> windows::core::Result<(DispatcherQueueController, Compositor)> {
    let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    RoInitialize(RO_INIT_SINGLETHREADED)?;
    let queue = CreateDispatcherQueueController(DispatcherQueueOptions {
        dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
        threadType: DQTYPE_THREAD_CURRENT,
        apartmentType: DQTAT_COM_NONE,
    })?;
    Ok((queue, Compositor::new()?))
}
