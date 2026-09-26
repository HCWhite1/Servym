//! `Servym.exe` (Windows) ou `servym-desktop` (Linux) : l'application de bureau. Voir `lib.rs`.

// Pas de fenêtre de console : Servym vit dans la zone de notification.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(windows, target_os = "linux"))]
fn main() -> std::process::ExitCode {
    servym_desktop::run()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() -> std::process::ExitCode {
    eprintln!("L'application de bureau vise Windows et Linux ; ailleurs, utilisez `servym` (sans écran).");
    std::process::ExitCode::FAILURE
}
