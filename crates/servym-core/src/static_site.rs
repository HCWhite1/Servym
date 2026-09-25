//! Projets « static » : Servym sert lui-même les fichiers d'un dossier sur un port, avec des
//! adresses propres (comme GitHub Pages) et un journal des requêtes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::static_files::{content_type, resolve_inside};

/// Site en service ; `stop` ferme le port et le journal.
pub struct StaticSite {
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl StaticSite {
    pub async fn stop(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        // Connexion encore ouverte (navigateur) : fermée d'office, comme closeAllConnections().
        if tokio::time::timeout(std::time::Duration::from_secs(1), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}

/// Fichier servi pour un chemin : le fichier, `index.html` d'un dossier, ou (adresses propres)
/// `chemin.html`.
async fn file_at(file: &Path, clean_urls: bool) -> Option<(PathBuf, std::fs::Metadata)> {
    async fn found(p: &Path) -> Option<(PathBuf, std::fs::Metadata)> {
        let info = tokio::fs::metadata(p).await.ok()?;
        if info.is_file() {
            return Some((p.to_path_buf(), info));
        }
        if info.is_dir() {
            let index = p.join("index.html");
            let info = tokio::fs::metadata(&index).await.ok()?;
            return info.is_file().then_some((index, info));
        }
        None
    }
    if let Some(hit) = found(file).await {
        return Some(hit);
    }
    if clean_urls && file.extension().is_none() {
        let mut html = file.as_os_str().to_owned();
        html.push(".html");
        return found(Path::new(&html)).await;
    }
    None
}

fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("Introuvable"))
        .unwrap()
}

/// Réponse d'un site statique (ou de l'interface) : `None` si rien ne correspond.
pub async fn serve(root: &Path, method: &Method, path: &str, clean_urls: bool) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    let file = resolve_inside(root, path)?;
    let (file, info) = file_at(&file, clean_urls).await?;
    let modified = info.modified().ok().map(|t| {
        chrono::DateTime::<chrono::Utc>::from(t)
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string()
    });
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(tokio::fs::read(&file).await.ok()?)
    };
    let mut res = Response::builder()
        .header(header::CONTENT_TYPE, content_type(&file))
        .header(header::CONTENT_LENGTH, info.len())
        .header(header::CACHE_CONTROL, "no-cache");
    if let Some(m) = modified {
        res = res.header(header::LAST_MODIFIED, m);
    }
    Some(res.body(body).unwrap())
}

/// Ouvre le site sur `host:port` et rend la fonction d'arrêt. Le journal est remplacé (un en-tête,
/// puis une ligne par requête : « 200 GET /page »).
pub async fn start(
    root: &Path,
    port: u16,
    host: &str,
    log_file: &Path,
    header_line: &str,
) -> std::io::Result<StaticSite> {
    let bind = if host == "0.0.0.0" { "0.0.0.0" } else { host };
    let listener = tokio::net::TcpListener::bind((bind, port)).await?;
    let mut log = std::fs::File::create(log_file)?;
    writeln!(log, "{header_line}")?;
    let log = Arc::new(Mutex::new(log));
    let root = root.to_path_buf();
    let app = axum::Router::new().fallback(move |req: Request| {
        let (root, log) = (root.clone(), log.clone());
        async move {
            let line = format!("{} {}", req.method(), req.uri());
            let (method, path) = (req.method().clone(), req.uri().path().to_string());
            let mut res = serve(&root, &method, &path, true).await.unwrap_or_else(not_found);
            res.headers_mut()
                .insert("x-content-type-options", HeaderValue::from_static("nosniff"));
            if let Ok(mut f) = log.lock() {
                let _ = writeln!(f, "{} {line}", res.status().as_u16());
            }
            res
        }
    });
    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    Ok(StaticSite {
        shutdown: Some(tx),
        task,
    })
}
