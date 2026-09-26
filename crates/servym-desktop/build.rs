fn main() {
    // Tauri n'est construit que sous Windows et Linux (voir Cargo.toml).
    #[cfg(any(windows, target_os = "linux"))]
    tauri_build::build();
}
