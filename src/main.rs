//! WinDict: select a word anywhere, press a hotkey, read its definition.

#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod dictionary;
mod hotkey;
mod http;
mod offline;
mod popup;
mod render;
mod selection;
mod theme;
mod tray;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::WinRT::*;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::UI::Composition::Compositor;

fn main() -> windows::core::Result<()> {
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
