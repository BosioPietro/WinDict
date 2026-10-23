//! The WinDict setup people download from the releases page: it installs
//! WinDict with the Inno Setup installer it carries, from a window of its
//! own (see `setup` in the library).

#![windows_subsystem = "windows"]

/// The installer built from installer/windict.iss.
static INSTALLER: &[u8] = include_bytes!(env!("WINDICT_SETUP"));

fn main() -> windows::core::Result<()> {
    windict::setup(INSTALLER)
}
