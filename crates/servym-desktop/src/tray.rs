//! Icône de la zone de notification : les projets et leurs actions, puis Servym.
//!
//! ```text
//! Servym · 2 en ligne
//! ● MangaFlow — en ligne        › Ouvrir · Redémarrer · Arrêter
//! ○ Radar — arrêté              › Démarrer
//! ✕ Bogue — en échec            › Relancer
//! Ouvrir Servym · Ouvrir dans le navigateur · ✓ Démarrer avec Windows · Quitter
//! ```
//!
//! Sur la machine hôte seulement, sans mot de passe : démarrer, arrêter, redémarrer, ouvrir ;
//! rien qui touche aux comptes ni aux réglages (hormis le démarrage avec la session, comme la
//! clé Run elle-même). L'icône passe au rouge quand un projet est en échec. Le menu est refait
//! quand un projet change d'état, de nom ou d'ordre.

use servym_server::TrayAction;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::app::Desktop;

const TRAY: &str = "servym";

/// Ce que le menu affiche : refait seulement quand cela change.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    id: String,
    name: String,
    state: String,
    kind: String,
    url: Option<String>,
}

struct Shown {
    rows: Vec<Row>,
    autostart: bool,
}

fn rows(desktop: &Desktop) -> Vec<Row> {
    desktop
        .app
        .manager()
        .snapshot()
        .iter()
        .map(|p| Row {
            id: p["id"].as_str().unwrap_or_default().to_string(),
            name: p["name"].as_str().unwrap_or_default().to_string(),
            state: p["state"].as_str().unwrap_or_default().to_string(),
            kind: p["type"].as_str().unwrap_or_default().to_string(),
            url: p["url"].as_str().map(str::to_string),
        })
        .collect()
}

fn state_label(desktop: &Desktop, state: &str) -> Option<String> {
    let key = match state {
        "running" => "tray.stateRunning",
        "starting" => "tray.stateStarting",
        "stopping" => "tray.stateStopping",
        "stopped" => "tray.stateStopped",
        "failed" => "tray.stateFailed",
        _ => return None,
    };
    Some(desktop.app.tr(key))
}

fn item(app: &AppHandle, id: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}

fn build(app: &AppHandle, desktop: &Desktop, shown: &Shown) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let online = shown.rows.iter().filter(|r| r.state == "running").count();
    let failed = shown.rows.iter().filter(|r| r.state == "failed").count();
    let mut title = format!(
        "Servym · {}",
        desktop
            .app
            .tr_with("tray.online", servym_core::params![count = online])
    );
    if failed > 0 {
        title.push_str(&format!(
            " · {}",
            desktop
                .app
                .tr_with("tray.failed", servym_core::params![count = failed])
        ));
    }
    menu.append(&MenuItem::with_id(app, "title", title, false, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if shown.rows.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "none",
            desktop.app.tr("tray.noProject"),
            false,
            None::<&str>,
        )?)?;
    }
    for row in &shown.rows {
        let sub_id = format!("project:{}", row.id);
        if row.kind == "file" {
            // Page locale : pas d'état, seulement l'ouvrir.
            let open = item(app, &format!("open-file:{}", row.id), &row.name)?;
            menu.append(&open)?;
            continue;
        }
        let mark = match row.state.as_str() {
            "running" => "●",
            "failed" => "✕",
            "starting" | "stopping" => "◐",
            _ => "○",
        };
        let label = match state_label(desktop, &row.state) {
            Some(state) => format!("{mark}  {} — {state}", row.name),
            None => format!("{mark}  {}", row.name),
        };
        let sub = Submenu::with_id(app, sub_id, label, true)?;
        let action =
            |action: &str, key: &str| item(app, &format!("{action}:{}", row.id), &desktop.app.tr(key));
        match row.state.as_str() {
            "running" => {
                if row.url.is_some() {
                    sub.append(&action("open", "tray.openProject")?)?;
                }
                sub.append(&action("restart", "tray.restart")?)?;
                sub.append(&action("stop", "tray.stop")?)?;
            }
            "starting" => sub.append(&action("stop", "tray.stop")?)?,
            "failed" => sub.append(&action("start", "tray.retry")?)?,
            "stopping" => {
                let label = state_label(desktop, "stopping").unwrap_or_default();
                sub.append(&MenuItem::with_id(
                    app,
                    format!("busy:{}", row.id),
                    label,
                    false,
                    None::<&str>,
                )?)?;
            }
            _ => sub.append(&action("start", "tray.start")?)?,
        }
        menu.append(&sub)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "ui", &desktop.app.tr("tray.open"))?)?;
    menu.append(&item(app, "browser", &desktop.app.tr("tray.openBrowser"))?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        "autostart",
        desktop.app.tr("tray.autostart"),
        true,
        shown.autostart,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item(app, "quit", &desktop.app.tr("tray.quit"))?)?;
    Ok(menu)
}

fn icon(alert: bool) -> tauri::Result<Image<'static>> {
    if alert {
        Image::from_bytes(include_bytes!("../icons/tray-alert.png"))
    } else {
        Image::from_bytes(include_bytes!("../icons/tray.png"))
    }
}

/// Refait le menu, l'icône et l'infobulle si ce qu'ils montrent a changé.
fn refresh(app: &AppHandle, last: &mut Option<(Vec<Row>, bool)>) {
    let desktop = app.state::<Desktop>();
    let shown = Shown {
        rows: rows(&desktop),
        autostart: desktop.runtime.block_on(desktop.app.autostart().enabled()),
    };
    let key = (shown.rows.clone(), shown.autostart);
    if last.as_ref() == Some(&key) {
        return;
    }
    *last = Some(key);
    let Some(tray) = app.tray_by_id(TRAY) else { return };
    if let Ok(menu) = build(app, &desktop, &shown) {
        let _ = tray.set_menu(Some(menu));
    }
    let failed = shown.rows.iter().filter(|r| r.state == "failed").count();
    let online = shown.rows.iter().filter(|r| r.state == "running").count();
    if let Ok(image) = icon(failed > 0) {
        let _ = tray.set_icon(Some(image));
    }
    let mut tip = format!(
        "Servym · {}",
        desktop
            .app
            .tr_with("tray.online", servym_core::params![count = online])
    );
    if failed > 0 {
        tip.push_str(&format!(
            " · {}",
            desktop
                .app
                .tr_with("tray.failed", servym_core::params![count = failed])
        ));
    }
    let _ = tray.set_tooltip(Some(tip));
}

fn on_menu(app: &AppHandle, event: MenuEvent) {
    let desktop = app.state::<Desktop>();
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "ui" => desktop.open_ui(app),
        "browser" => crate::window::to_browser(&desktop.url()),
        "quit" => app.exit(0),
        "autostart" => {
            let autostart = desktop.app.autostart();
            let on = !desktop.runtime.block_on(autostart.enabled());
            if let Err(error) = desktop.runtime.block_on(autostart.set(on)) {
                desktop.app.log(&format!("[autostart] {error}"));
            } else {
                let key = if on { "log.autostartOn" } else { "log.autostartOff" };
                desktop.app.log(&format!("[autostart] {}", desktop.app.tr(key)));
            }
            // Coche à jour tout de suite (ce n’est pas un changement de projet).
            let app = app.clone();
            std::thread::spawn(move || refresh(&app, &mut None));
        }
        _ => {
            let Some((action, project)) = id.split_once(':') else {
                return;
            };
            let project = project.to_string();
            let tray_action = match action {
                "start" => TrayAction::Start,
                "restart" => TrayAction::Restart,
                "stop" => TrayAction::Stop,
                "open" | "open-file" => {
                    open_project(&desktop, &project, action == "open-file");
                    return;
                }
                _ => return,
            };
            let server = desktop.app.clone();
            desktop.runtime.spawn(async move {
                if let Err(error) = server.tray_action(&project, tray_action).await {
                    let name = server
                        .manager()
                        .project(&project)
                        .ok()
                        .and_then(|p| p["name"].as_str().map(str::to_string))
                        .unwrap_or(project);
                    server.log(&format!(
                        "[tray] {}",
                        server.tr_with(
                            "tray.actionFailed",
                            servym_core::params![name = name, error = error]
                        )
                    ));
                }
            });
        }
    }
}

/// Ouvre l'adresse du projet dans le navigateur (page locale : par le gestionnaire, comme
/// l'interface).
fn open_project(desktop: &Desktop, id: &str, file: bool) {
    if file {
        if let Err(error) = desktop.app.manager().open_file(id) {
            desktop.app.log(&format!(
                "[tray] {}",
                error.message_in(desktop.app.manager().lang())
            ));
        }
        return;
    }
    let url = desktop
        .app
        .manager()
        .snapshot()
        .into_iter()
        .find(|p| p["id"] == id)
        .and_then(|p| p["url"].as_str().map(str::to_string));
    if let Some(url) = url {
        crate::window::to_browser(&url);
    }
}

pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let tray: TrayIcon<Wry> = TrayIconBuilder::with_id(TRAY)
        .icon(icon(false)?)
        .tooltip("Servym")
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            // Clic gauche : ouvrir Servym ; clic droit : le menu.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                app.state::<Desktop>().open_ui(app);
            }
        })
        .build(app)?;
    let _ = tray;
    // Menu refait à chaque changement des projets, sur un fil à part (le menu est construit sur le
    // fil principal par Tauri).
    let app = app.clone();
    let mut events = app.state::<Desktop>().app.manager().subscribe();
    std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            let mut last = None;
            refresh(&app, &mut last);
            let runtime = app.state::<Desktop>().runtime.clone();
            loop {
                match runtime.block_on(events.recv()) {
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        refresh(&app, &mut last)
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        })
        .map_err(tauri::Error::Io)?;
    Ok(())
}
