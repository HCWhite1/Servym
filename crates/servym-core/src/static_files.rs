//! Fichiers des sites statiques : un chemin d'URL ne sort jamais du dossier du site.

use std::path::{Path, PathBuf};

/// `decodeURIComponent` : `None` si un `%` est mal formé ou si le résultat n'est pas de l'UTF-8.
pub fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Fichier du dossier `root` désigné par un chemin d'URL ; `None` s'il sortirait du dossier, ou
/// s'il désigne un fichier caché, un flux NTFS ou contient un octet nul.
///
/// `/` et `\` séparent les segments sur tous les systèmes ; les barres de tête sont ignorées
/// (« //serveur » serait lu comme un chemin réseau).
pub fn resolve_inside(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = decode_uri_component(url_path)?;
    // « : » ouvrirait les flux alternatifs NTFS (fichier::$DATA) ; les segments en point
    // cachent .git, .env… (et « .. » en fait partie).
    if rel.contains('\0') || rel.contains(':') || rel.split(['/', '\\']).any(|s| s.starts_with('.')) {
        return None;
    }
    let mut file = root.to_path_buf();
    for segment in rel.split(['/', '\\']).filter(|s| !s.is_empty()) {
        file.push(segment);
    }
    Some(file)
}

/// Type MIME d'un fichier servi, selon son extension.
pub fn content_type(file: &Path) -> &'static str {
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}
