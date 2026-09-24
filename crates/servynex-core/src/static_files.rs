//! Fichiers des sites statiques : un chemin d'URL ne sort jamais du dossier du site.

use std::path::{Path, PathBuf};

/// Fichier du dossier `root` désigné par un chemin d'URL ; `None` s'il sortirait du dossier, ou
/// s'il désigne un fichier caché, un flux NTFS ou contient un octet nul.
pub fn resolve_inside(root: &Path, url_path: &str) -> Option<PathBuf> {
    let _ = (root, url_path);
    unimplemented!("phase 2")
}
