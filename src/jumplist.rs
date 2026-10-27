//! The tasks in WinDict's Jump List, under its entry in Start and its
//! taskbar button: each starts WinDict.exe with a switch, which the copy
//! already running is handed (see `app::forward`), to open the box to type
//! a word in, as a shortcut pressed with nothing selected would.
//!
//! Windows ties the list to the Start shortcut by their app ID, which the
//! installer gives the shortcut (installer/windict.iss) and [`APP_ID`] the
//! process: both builds, so it's the same whichever one has UI access.

use windows::core::{w, Interface, Result, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0, PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VT_LPWSTR;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};

use crate::config::{Shortcut, SHORTCUTS};
use crate::i18n::tr;
use crate::installed;

/// Never change it: it's what ties the Jump List to the Start shortcut.
pub const APP_ID: PCWSTR = w!("BosioPietro.WinDict");

/// The switch each task starts WinDict with.
pub fn switch(task: Shortcut) -> &'static str {
    match task {
        Shortcut::Lookup => "--look-up",
        Shortcut::Translate => "--translate",
    }
}

/// The task this process was started for, if it was.
pub fn from_args() -> Option<Shortcut> {
    let arg = std::env::args_os().nth(1)?;
    SHORTCUTS.into_iter().find(|&task| arg == switch(task))
}

/// Puts the tasks in the Jump List, named in the current language. On the
/// UI thread, which COM is initialized on.
pub fn set() -> Result<()> {
    // The installed copy's tasks start WinDict.exe, which hands over to the
    // UI access build when it's on.
    let exe = installed::dir()
        .map(|dir| dir.join(installed::LAUNCHER))
        .or_else(|| std::env::current_exe().ok());
    let Some(exe) = exe else {
        return Ok(());
    };
    let exe = HSTRING::from(exe.as_os_str());
    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;
        list.SetAppID(APP_ID)?;
        let mut slots = 0;
        let _removed: IObjectArray = list.BeginList(&mut slots)?;
        let tasks: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
        for task in SHORTCUTS {
            let title = match task {
                Shortcut::Lookup => tr().search_title,
                Shortcut::Translate => tr().translate_title,
            };
            tasks.AddObject(&link(&exe, switch(task), title)?)?;
        }
        list.AddUserTasks(&tasks.cast::<IObjectArray>()?)?;
        list.CommitList()
    }
}

/// A task: `exe` started with `switch`, shown as `title` with its icon.
unsafe fn link(exe: &HSTRING, switch: &str, title: &str) -> Result<IShellLinkW> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
    link.SetPath(exe)?;
    link.SetArguments(&HSTRING::from(switch))?;
    link.SetIconLocation(exe, 0)?;
    // A task's name is its title, not a description.
    let mut title: Vec<u16> = title.encode_utf16().chain([0]).collect();
    let value = PROPVARIANT {
        Anonymous: PROPVARIANT_0 {
            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                vt: VT_LPWSTR,
                Anonymous: PROPVARIANT_0_0_0 {
                    pwszVal: PWSTR(title.as_mut_ptr()),
                },
                ..Default::default()
            }),
        },
    };
    // The store copies the value: `title` is still this function's to free.
    let store: IPropertyStore = link.cast()?;
    store.SetValue(&PKEY_Title, &value)?;
    store.Commit()?;
    Ok(link)
}
