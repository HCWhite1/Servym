//! `Servym.exe` : l'application de bureau (Windows). Voir `lib.rs`.

// Pas de fenêtre de console : Servym vit dans la zone de notification.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    servym_desktop::run()
}

#[cfg(not(windows))]
fn main() -> std::process::ExitCode {
    eprintln!("L'application de bureau vise Windows ; ailleurs, utilisez `servym` (sans écran).");
    std::process::ExitCode::FAILURE
}
