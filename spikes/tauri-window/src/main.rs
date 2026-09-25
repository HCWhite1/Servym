//! Essai 3 : une fenêtre Tauri sur la version Node actuelle (http://localhost:9000).
//!
//! - Aucune IPC : aucune capability n'est accordée au contenu distant.
//! - `window.open` vers une autre origine → navigateur par défaut.
//! - `window.open` vers Servym lui-même (page d'attente) → petite fenêtre Tauri qui partage les
//!   cookies ; quand elle quitte l'origine de Servym (redirection vers le projet), l'adresse part
//!   dans le navigateur et la petite fenêtre se ferme.
//! - Téléchargements (export de la configuration) : journalisés, dossier par défaut.
//! - Fermer la fenêtre la détruit ; l'application reste dans la zone de notification.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::webview::{DownloadEvent, NewWindowResponse};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, Wry};

const ORIGIN: &str = "http://localhost:9000";

static POPUPS: AtomicU32 = AtomicU32::new(0);

fn log(msg: impl std::fmt::Display) {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    println!("[{t:7.2}] {msg}");
}

fn same_origin(url: &Url) -> bool {
    url.origin() == Url::parse(ORIGIN).unwrap().origin()
}

fn to_browser(url: &Url) {
    log(format_args!("→ navigateur : {url}"));
    if let Err(e) = tauri_plugin_opener::open_url(url.as_str(), None::<&str>) {
        log(format_args!("ouverture impossible : {e}"));
    }
}

fn open_main(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("main") {
        w.show()?;
        w.unminimize()?;
        return w.set_focus();
    }
    let handle = app.clone();
    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(ORIGIN.parse().unwrap()))
        .title("Servym (essai Tauri)")
        .inner_size(1280.0, 820.0)
        .on_navigation(|url| {
            log(format_args!("navigation : {url}"));
            true
        })
        .on_new_window(move |url, features| {
            log(format_args!("window.open : {url}"));
            if !same_origin(&url) {
                to_browser(&url);
                return NewWindowResponse::Deny;
            }
            // Page de Servym (attente) : petite fenêtre qui garde la session.
            let label = format!("popup-{}", POPUPS.fetch_add(1, Ordering::Relaxed));
            let popup_label = label.clone();
            let app = handle.clone();
            let window =
                WebviewWindowBuilder::new(&handle, &label, WebviewUrl::External("about:blank".parse().unwrap()))
                    .window_features(features)
                    .title("Servym")
                    .inner_size(520.0, 420.0)
                    .on_navigation(move |next| {
                        if next.scheme() == "about" || same_origin(next) {
                            log(format_args!("{popup_label} : {next}"));
                            return true;
                        }
                        to_browser(next);
                        if let Some(w) = app.get_webview_window(&popup_label) {
                            let _ = w.close();
                        }
                        false
                    })
                    .build();
            match window {
                Ok(window) => NewWindowResponse::Create { window },
                Err(e) => {
                    log(format_args!("fenêtre impossible : {e}"));
                    NewWindowResponse::Deny
                }
            }
        })
        .on_download(|_, event| {
            match event {
                DownloadEvent::Requested { url, destination } => {
                    log(format_args!("téléchargement demandé : {url} → {}", destination.display()))
                }
                DownloadEvent::Finished { url, path, success } => {
                    log(format_args!("téléchargement fini : {url} → {path:?} (réussi : {success})"))
                }
                _ => {}
            }
            true
        })
        .build()?;
    Ok(())
}

fn main() {
    log("démarrage");
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let open = MenuItem::with_id(app, "open", "Ouvrir Servym", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            TrayIconBuilder::<Wry>::with_id("tray")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Servym (essai)")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => {
                        log("zone de notification : ouvrir");
                        let _ = open_main(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            open_main(app.handle())?;
            log("fenêtre créée");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("application Tauri");
    app.run(|_, event| {
        if let RunEvent::WindowEvent { label, event: tauri::WindowEvent::Destroyed, .. } = &event {
            log(format_args!("fenêtre {label} détruite"));
        }
        // Dernière fenêtre fermée : on reste dans la zone de notification.
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            log("plus aucune fenêtre : reste dans la zone de notification");
            api.prevent_exit();
        }
    });
}
