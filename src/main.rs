//! The WinDict executable; everything else is in the library (`lib.rs`).

#![windows_subsystem = "windows"]

fn main() -> windows::core::Result<()> {
    windict::run()
}
