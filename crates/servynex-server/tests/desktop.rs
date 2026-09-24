//! Ce que l'application de bureau (phase 6) ajoute au serveur : le choix « fenêtre ou
//! navigateur » (devant la machine seulement) et les actions de la zone de notification.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Servynex, program, until_state, with};
use serde_json::json;
use servynex_core::manager::State;
use servynex_fixtures::{assert_match, free_port, read, read_json, until};
use servynex_server::testing::{FakeAutostart, FakeNotifier, Reply};
use servynex_server::{Shell, TrayAction};

#[derive(Default)]
struct FakeShell {
    browser: AtomicUsize,
    closed: AtomicUsize,
}

impl Shell for FakeShell {
    fn open_browser(&self) {
        self.browser.fetch_add(1, Ordering::SeqCst);
    }
    fn close_window(&self) {
        self.closed.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn fenetre_ou_navigateur() {
    let shell = Arc::new(FakeShell::default());
    let s = Servynex::start(
        "servynex-desktop-",
        json!({ "instance": { "port": 1 }, "projects": [] }),
        {
            let shell = shell.clone();
            move |b| b.shell(shell)
        },
    )
    .await;
    let info = || async { s.call("GET", "/api/projects").await.json()["instance"]["app"].clone() };
    assert_eq!(info().await, json!({ "opener": "window" }), "fenêtre par défaut");

    let set = |opener: &str| s.call_json("PUT", "/api/instance/opener", json!({ "opener": opener }), None);
    assert_eq!(set("onglet").await.status, 400);
    let res = set("browser").await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json(), json!({ "opener": "browser" }));
    assert_eq!(info().await["opener"], "browser");
    let ui = read_json(s.path().join("runtime/ui.json"));
    assert_eq!(ui["opener"], "browser", "réglage de la machine");

    assert_eq!(s.call("POST", "/api/instance/browser").await.status, 200);
    assert_eq!(shell.browser.load(Ordering::SeqCst), 1);
    assert_eq!(shell.closed.load(Ordering::SeqCst), 0);
    // Navigateur choisi depuis la fenêtre : elle se ferme ensuite.
    let res = s
        .call_json(
            "POST",
            "/api/instance/browser",
            json!({ "closeWindow": true }),
            None,
        )
        .await;
    assert_eq!(res.status, 200);
    assert_eq!(shell.browser.load(Ordering::SeqCst), 2);
    assert_eq!(shell.closed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fenetre_ou_navigateur_devant_la_machine_seulement() {
    let shell = Arc::new(FakeShell::default());
    let s = Servynex::start(
        "servynex-desktop-",
        json!({ "instance": { "port": 1 }, "projects": [] }),
        {
            let shell = shell.clone();
            move |b| b.shell(shell).local_viewer(Arc::new(|_| false))
        },
    )
    .await;
    let res = s
        .call_json(
            "PUT",
            "/api/instance/opener",
            json!({ "opener": "browser" }),
            None,
        )
        .await;
    assert_eq!(res.status, 403);
    assert_eq!(s.call("POST", "/api/instance/browser").await.status, 403);
    assert_eq!(shell.browser.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sans_application_de_bureau() {
    let s = Servynex::start(
        "servynex-desktop-",
        json!({ "instance": { "port": 1 }, "projects": [] }),
        |b| b,
    )
    .await;
    let info = s.call("GET", "/api/projects").await.json();
    assert!(info["instance"]["app"].is_null());
    assert_eq!(s.call("POST", "/api/instance/browser").await.status, 404);
}

/// Démarrer et arrêter depuis la zone de notification : même chemin que l'interface, avec l'auteur
/// dans le journal et l'historique.
#[tokio::test]
async fn actions_de_la_zone_de_notification() {
    let dir = servynex_fixtures::temp_dir("servynex-desktop-");
    let cwd = dir.path().join("app");
    std::fs::create_dir_all(&cwd).unwrap();
    let config = json!({ "instance": { "port": 1 }, "projects": [
        with(program("api", "Mon API", &cwd, "server", free_port()), json!({ "health": { "expect": "Factice" } })),
    ] });
    servynex_fixtures::write(dir.path().join("projects.json"), config.to_string());
    let notifier = FakeNotifier::new(Reply::Sent);
    let autostart = Arc::new(FakeAutostart::default());
    let app = common::builder(dir.path(), &notifier, &autostart)
        .timings(common::fast())
        .build();
    let running = app.listen("127.0.0.1:0").await.unwrap();
    app.manager().init().await;

    app.tray_action("api", TrayAction::Start).await.unwrap();
    until_state(app.manager(), "api", |p| p.state == State::Running).await;
    app.tray_action("api", TrayAction::Stop).await.unwrap();
    until_state(app.manager(), "api", |p| p.state == State::Stopped).await;
    let err = app.tray_action("inconnu", TrayAction::Start).await.unwrap_err();
    assert!(!err.is_empty());

    let log = dir.path().join("logs/servynex.log");
    until(|| async { read(&log).contains("running → stopping") }, 5000).await;
    let log = read(&log);
    assert_match!(log, r"\[start\] api · via la zone de notification");
    assert_match!(
        log,
        r"\[state\] api stopped → starting · via la zone de notification"
    );
    assert_match!(
        log,
        r"\[state\] api running → stopping · .*via la zone de notification"
    );
    running.close().await;
}
