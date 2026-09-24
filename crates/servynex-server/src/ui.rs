//! Interface (le dossier `public/`, inchangé) embarquée dans le binaire.

use axum::body::Body;
use axum::http::{Method, header};
use axum::response::Response;
use servynex_core::static_files::content_type;

#[derive(rust_embed::Embed)]
#[folder = "../../public"]
struct Ui;

/// `decodeURIComponent` : `None` si un `%` est mal formé ou si le résultat n'est pas de l'UTF-8.
fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Fichier de l'interface pour ce chemin (mêmes refus que les sites statiques : fichiers cachés,
/// flux NTFS, octets nuls, sortie du dossier) ; un dossier sert son `index.html`.
pub fn serve(method: &Method, path: &str) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    let rel = decode(path)?;
    if rel.contains('\0') || rel.contains(':') || rel.split(['/', '\\']).any(|s| s.starts_with('.')) {
        return None;
    }
    let key = rel
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let (key, file) = match Ui::get(&key) {
        Some(file) if !key.is_empty() => (key, file),
        _ => {
            let index = if key.is_empty() {
                "index.html".to_string()
            } else {
                format!("{key}/index.html")
            };
            let file = Ui::get(&index)?;
            (index, file)
        }
    };
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(file.data.into_owned())
    };
    Some(
        Response::builder()
            .header(header::CONTENT_TYPE, content_type(std::path::Path::new(&key)))
            .header(header::CACHE_CONTROL, "no-cache")
            .body(body)
            .unwrap(),
    )
}
