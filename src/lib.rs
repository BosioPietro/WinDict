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
//! - `dictionary`: entries as the card shows them; `offline` (the bundled
//!   WordNet) and `online` (Free Dictionary API, Wiktionary, and
//!   `wikizionario` for Italian, over `http`) fill them; `translate`
//!   translates the word (Google Translate, else MyMemory).
//! - `popup` (the card, with `searchbox`), `menu` and `settings`: the windows,
//!   each a composition visual tree built from `ui`, drawn by `render` in
//!   the colors of `theme`.
//! - `config`, `hotkey`, `autostart`, `tray`: settings and their storage, the
//!   shortcut, "Start with Windows", and the notification-area icon.
//! - `i18n`: every string the user reads, so a language is a table, not a
//!   change across the code.
//! - `win`: safe wrappers for the Win32 calls shared across modules.

mod app;
mod autostart;
mod config;
mod dictionary;
mod dicts;
mod hotkey;
mod http;
mod i18n;
mod menu;
mod offline;
mod online;
mod popup;
mod render;
mod searchbox;
mod selection;
mod settings;
mod theme;
mod translate;
mod tray;
mod ui;
mod wikizionario;
mod win;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::UI::Composition::Compositor;

/// Runs WinDict until it's quit from the tray, unless it's running already.
pub fn run() -> windows::core::Result<()> {
    unsafe {
        // One instance per user session.
        let _instance = CreateMutexW(None, true, w!("Local\\WinDict.Instance"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        RoInitialize(RO_INIT_SINGLETHREADED)?;
        // Windows.UI.Composition needs a dispatcher queue on the UI thread.
        let _queue = CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_NONE,
        })?;
        let compositor = Compositor::new()?;
        app::run(compositor)
    }
}
