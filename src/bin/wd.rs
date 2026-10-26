//! `wd`, WinDict on the command line: it looks words up and translates
//! them, printing what the card would show (see `cli` in the library).

fn main() -> std::process::ExitCode {
    windict::wd()
}
