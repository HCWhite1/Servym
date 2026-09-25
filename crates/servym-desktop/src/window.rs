//! La fenêtre : l'interface servie par le serveur embarqué, comme un onglet. Aucune capability
//! n'est accordée à cette adresse : la page ne peut appeler aucune fonction native.
//!
//! - Liens vers une autre origine (projets, documentation) : navigateur par défaut.
//! - `window.open` vers Servym lui-même (page d'attente) : petite fenêtre qui partage la session ;
//!   dès qu'elle quitte Servym (le projet répond), l'adresse part dans le navigateur et la petite
//!   fenêtre se ferme (essai 3 de la phase 0).
//! - Connexion GitHub : la fenêtre principale peut naviguer sur github.com, pour que le cookie de
//!   session arrive dans la fenêtre et non dans le navigateur.
//! - Fermer la fenêtre la détruit : WebView2 rend sa mémoire.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindowBuilder};

static POPUPS: AtomicU32 = AtomicU32::new(0);

/// Adresse ouverte dans le navigateur par défaut.
pub fn to_browser(url: &str) {
    let _ = tauri_plugin_opener::open_url(url, None::<&str>);
}

fn origin(port: u16) -> Url {
    Url::parse(&format!("http://localhost:{port}/")).expect("adresse locale")
}

fn same_origin(url: &Url, port: u16) -> bool {
    url.origin() == origin(port).origin()
}

/// Parcours de connexion GitHub (autorisation, identification, double authentification).
fn github(url: &Url) -> bool {
    url.scheme() == "https" && url.host_str() == Some("github.com")
}

/// Montre la fenêtre principale, créée au besoin.
pub fn show_main(app: &AppHandle, port: u16) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        return window.set_focus();
    }
    let handle = app.clone();
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::External(origin(port)))
        .title("Servym")
        .inner_size(1280.0, 820.0)
        .min_inner_size(420.0, 480.0)
        // Signale à la page qu'elle est dans la fenêtre (choix « fenêtre ou navigateur ») : une
        // simple valeur, aucune fonction native.
        .initialization_script(r#"Object.defineProperty(window, "servymWindow", { value: true });"#)
        .on_navigation(move |url| {
            if same_origin(url, port) || github(url) || url.scheme() == "about" {
                return true;
            }
            to_browser(url.as_str());
            false
        })
        .on_new_window(move |url, features| {
            if !same_origin(&url, port) {
                to_browser(url.as_str());
                return NewWindowResponse::Deny;
            }
            let label = format!("popup-{}", POPUPS.fetch_add(1, Ordering::Relaxed));
            let popup_label = label.clone();
            let app = handle.clone();
            let popup = WebviewWindowBuilder::new(
                &handle,
                &label,
                WebviewUrl::External("about:blank".parse().expect("about:blank")),
            )
            .window_features(features)
            .title("Servym")
            .inner_size(520.0, 420.0)
            .on_navigation(move |next| {
                if next.scheme() == "about" || same_origin(next, port) {
                    return true;
                }
                to_browser(next.as_str());
                if let Some(w) = app.get_webview_window(&popup_label) {
                    let _ = w.close();
                }
                false
            })
            .build();
            match popup {
                Ok(window) => NewWindowResponse::Create { window },
                Err(_) => NewWindowResponse::Deny,
            }
        })
        .build()?;
    window.set_focus()
}
