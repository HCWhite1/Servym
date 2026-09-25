fn main() {
    // Tauri n'est construit que sous Windows (voir Cargo.toml).
    #[cfg(windows)]
    tauri_build::build();
}
