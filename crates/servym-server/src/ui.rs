//! Interface (le dossier `public/`, inchangé) embarquée dans le binaire.

use axum::body::Body;
use axum::http::{Method, header};
use axum::response::Response;
use servym_core::static_files::{content_type, decode_uri_component};

#[derive(rust_embed::Embed)]
#[folder = "../../public"]
struct Ui;

/// Fichier de l'interface pour ce chemin (mêmes refus que les sites statiques : fichiers cachés,
/// flux NTFS, octets nuls, sortie du dossier) ; un dossier sert son `index.html`.
pub fn serve(method: &Method, path: &str) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    let rel = decode_uri_component(path)?;
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
