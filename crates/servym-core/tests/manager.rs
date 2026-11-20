//! Gestionnaire de processus, sans HTTP (phase 4) : lancement, surveillance, arrêt, dépendances,
//! sites statiques, relance après plantage, et ce que la migration ajoute : reprise vérifiée
//! (PID réutilisé refusé), plantage d'un projet repris détecté, projet qui survit à Servym.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use servym_core::i18n::Lang;
use servym_core::manager::{AutoRestart, Event, Manager, ProjectState, State, Timings};
use servym_fixtures::{
    Req, TempDir, fixture_exe, free_port, listening, path_str, read, sleep, temp_dir, write,
};

fn program(id: &str, dir: &Path, mode: &str, port: u16, extra: Value) -> Value {
    let mut p = json!({
        "id": id, "name": id.to_uppercase(), "cwd": path_str(dir), "cmd": path_str(fixture_exe()),
        "args": [mode], "port": port, "hostControl": "env",
    });
    for (k, v) in extra.as_object().unwrap() {
        p[k] = v.clone();
    }
    p
}

fn timings() -> Timings {
    Timings {
        tick: Duration::from_millis(300),
        running_tick: Duration::from_millis(300),
        start_timeout: Duration::from_secs(8),
        relaunch: Duration::from_secs(3),
        launcher: Duration::from_millis(1500),
        restart_delays: vec![
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(300),
        ],
        ..Timings::default()
    }
}

fn manager(dir: &TempDir, projects: Vec<Value>) -> Manager {
    Manager::builder(json!({ "instance": { "port": 1 }, "projects": projects }))
        .logs_dir(&dir.path().join("logs"))
        .runtime_dir(&dir.path().join("runtime"))
        .timings(timings())
        .boot_time(1_000_000)
        .lang(Lang::Fr)
        .build()
}

async fn until_state(m: &Manager, id: &str, check: impl Fn(&ProjectState) -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let s = m.state(id);
        if check(&s) {
            return;
        }
        assert!(std::time::Instant::now() < end, "délai dépassé : {id} est {s:?}");
        sleep(100).await;
    }
}

fn app_dir(dir: &TempDir) -> std::path::PathBuf {
    let app = dir.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    app
}

async fn stop_all(m: &Manager, ids: &[&str]) {
    for id in ids {
        if m.state(id).managed {
            let _ = m.stop(id).await;
        }
    }
    m.close().await;
}

#[tokio::test]
async fn lance_repond_et_arrete() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let port = free_port();
    let m = manager(
        &dir,
        vec![program(
            "ok",
            &app,
            "server",
            port,
            json!({ "health": { "expect": "Factice" }, "env": { "SERVYM_TEST_ENV": "bonjour", "PORT": "1" } }),
        )],
    );
    let mut events = m.subscribe();
    m.init().await;
    m.start("ok").unwrap();
    assert!(m.start("ok").is_err(), "double lancement refusé");
    until_state(&m, "ok", |s| s.state == State::Running).await;
    let s = m.state("ok");
    assert!(s.managed && s.pid.is_some() && s.started_at.is_some());
    assert_eq!(Req::get("/").send(port).await.body, "<title>Factice</title>");
    let log = read(m.log_file("ok"));
    assert!(log.starts_with("[servym] "), "{log}");
    sleep(300).await;
    assert!(
        read(m.log_file("ok")).contains(&format!("démarrage sur 127.0.0.1 {port} bonjour")),
        "HOST et PORT transmis, env gardé"
    );
    m.stop("ok").await.unwrap();
    assert_eq!(m.state("ok").state, State::Stopped);
    assert!(!listening(port).await);
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let Event::Transition(t) = event {
            seen.push((t.from, t.to));
        }
    }
    assert_eq!(
        seen,
        [
            (State::Stopped, State::Starting),
            (State::Starting, State::Running),
            (State::Running, State::Stopping),
            (State::Stopping, State::Stopped)
        ]
    );
    m.close().await;
}

#[tokio::test]
async fn plantage_code_et_ligne_d_erreur() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let m = manager(
        &dir,
        vec![program("crash", &app, "crash", free_port(), json!({}))],
    );
    m.init().await;
    m.start("crash").unwrap();
    until_state(&m, "crash", |s| s.state == State::Failed).await;
    let s = m.state("crash");
    assert_eq!(s.exit_code, Some(3));
    assert_eq!(s.error.as_deref(), Some("Error: base introuvable"));
    // Relançable tout de suite.
    m.start("crash").unwrap();
    until_state(&m, "crash", |s| s.state == State::Failed).await;
    assert!(
        m.prev_log_file("crash").exists(),
        "journal du lancement précédent"
    );
    m.close().await;
}

#[tokio::test]
async fn lanceur_sorti_sans_repondre() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let m = manager(
        &dir,
        vec![program("launcher", &app, "launcher", free_port(), json!({}))],
    );
    m.init().await;
    m.start("launcher").unwrap();
    until_state(&m, "launcher", |s| {
        s.detail
            .as_deref()
            .is_some_and(|d| d.starts_with("Lanceur terminé"))
    })
    .await;
    assert_eq!(m.state("launcher").state, State::Starting);
    until_state(&m, "launcher", |s| s.state == State::Failed).await;
    let s = m.state("launcher");
    assert_eq!(s.exit_code, Some(0));
    assert!(
        s.error
            .unwrap()
            .ends_with("[WARNING] The local web server is already running")
    );
    m.close().await;
}

#[tokio::test]
async fn commande_introuvable() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let mut p = program("nf", &app, "server", free_port(), json!({}));
    p["cmd"] = json!("commande-qui-n-existe-pas");
    let m = manager(&dir, vec![p]);
    m.init().await;
    m.start("nf").unwrap();
    until_state(&m, "nf", |s| s.state == State::Failed).await;
    let error = m.state("nf").error.unwrap();
    let log = read(m.log_file("nf"));
    assert!(
        error.contains("commande-qui-n-existe-pas") && !error.contains('\u{fffd}'),
        "{error}\n{log:?}"
    );
    m.close().await;
}

#[tokio::test]
async fn dependances_en_cascade() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let factice = json!({ "health": { "expect": "Factice" } });
    let mut web = program("web", &app, "server", free_port(), factice.clone());
    web["requires"] = json!(["db"]);
    let m = manager(
        &dir,
        vec![program("db", &app, "server", free_port(), factice), web],
    );
    m.init().await;
    m.start("web").unwrap();
    until_state(&m, "web", |s| s.state == State::Running).await;
    assert!(m.state("db").dependency);
    m.stop_cascade("web").await.unwrap();
    until_state(&m, "db", |s| s.state == State::Stopped).await;
    m.close().await;
}

#[tokio::test]
async fn dependance_en_echec() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let mut dep = program("dep", &app, "server", free_port(), json!({}));
    dep["requires"] = json!(["crash"]);
    let m = manager(
        &dir,
        vec![program("crash", &app, "crash", free_port(), json!({})), dep],
    );
    m.init().await;
    m.start("dep").unwrap();
    until_state(&m, "dep", |s| s.state == State::Failed).await;
    assert!(m.state("dep").error.unwrap().contains("CRASH n'a pas démarré"));
    m.close().await;
}

#[tokio::test]
async fn port_deja_pris() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let port = free_port();
    let _blocker = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let m = manager(&dir, vec![program("busy", &app, "server", port, json!({}))]);
    m.init().await;
    m.start("busy").unwrap();
    until_state(&m, "busy", |s| s.state == State::Failed).await;
    assert!(
        m.state("busy")
            .error
            .unwrap()
            .contains(&format!("port {port} est déjà pris"))
    );
    m.close().await;
}

#[tokio::test]
async fn site_statique() {
    let dir = temp_dir("servym-m-");
    let site = dir.path().join("site");
    write(site.join("index.html"), "accueil");
    write(site.join("page.html"), "page");
    write(site.join(".env"), "SECRET");
    let port = free_port();
    let m = manager(
        &dir,
        vec![json!({ "id": "site", "name": "Site", "cwd": path_str(&site), "type": "static", "port": port })],
    );
    m.init().await;
    m.start("site").unwrap();
    until_state(&m, "site", |s| s.state == State::Running).await;
    assert_eq!(Req::get("/").send(port).await.body, "accueil");
    assert_eq!(Req::get("/page").send(port).await.body, "page");
    assert_eq!(Req::get("/.env").send(port).await.status, 404);
    assert!(read(m.log_file("site")).contains("200 GET /page"));
    m.stop("site").await.unwrap();
    assert!(Req::get("/").try_send(port).await.is_err());
    m.close().await;
}

#[tokio::test]
async fn relance_apres_plantage() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let m = manager(
        &dir,
        vec![program(
            "api",
            &app,
            "flaky",
            free_port(),
            json!({ "env": { "CRASHES": "1" }, "health": { "expect": "Factice" }, "restartOnCrash": true }),
        )],
    );
    m.init().await;
    m.start("api").unwrap();
    until_state(&m, "api", |s| s.state == State::Running).await;
    until_state(&m, "api", |s| {
        s.auto_restart == Some(AutoRestart::Scheduled) || s.state == State::Starting
    })
    .await;
    until_state(&m, "api", |s| s.state == State::Running).await;
    assert_eq!(read(app.join("runs.txt")), "2");
    stop_all(&m, &["api"]).await;
}

/// Servym redémarré (le premier gestionnaire fermé, le projet laissé en marche) : le projet est
/// repris sans transition, puis arrêtable.
#[tokio::test]
async fn projet_repris_apres_redemarrage() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let port = free_port();
    let projects = vec![program(
        "ok",
        &app,
        "server",
        port,
        json!({ "health": { "expect": "Factice" } }),
    )];
    let first = manager(&dir, projects.clone());
    first.init().await;
    first.start("ok").unwrap();
    until_state(&first, "ok", |s| s.state == State::Running).await;
    let pid = first.state("ok").pid.unwrap();
    first.close().await;
    drop(first);
    assert!(listening(port).await, "le projet survit à Servym");

    let saved: Value = serde_json::from_str(&read(dir.path().join("runtime").join("pids.json"))).unwrap();
    assert!(
        saved["ok"]["createdAt"].is_i64(),
        "heure de création enregistrée : {saved}"
    );

    let next = manager(&dir, projects);
    let mut events = next.subscribe();
    next.init().await;
    let s = next.state("ok");
    assert_eq!((s.state, s.managed, s.pid), (State::Running, true, Some(pid)));
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, Event::Transition(_)),
            "aucune transition pour un projet repris : {event:?}"
        );
    }
    next.stop("ok").await.unwrap();
    assert!(!listening(port).await);
    next.close().await;
}

/// Après un redémarrage du PC, le PID enregistré peut désigner un autre programme : il n'est
/// jamais repris (ni arrêté).
#[tokio::test]
async fn pid_reutilise_refuse() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    // Un processus bien vivant qui n'est pas le projet : ce test lui-même.
    let foreign = std::process::id();
    let created = servym_sys::native().process_start(foreign).unwrap();
    write(
        dir.path().join("runtime").join("pids.json"),
        json!({
            // Version Rust : heure de création différente.
            "a": { "pid": foreign, "startedAt": created, "createdAt": created - 60_000 },
            // Fichier de la version Node : processus créé bien après le lancement enregistré.
            "b": { "pid": foreign, "startedAt": created - 3_600_000 },
        })
        .to_string(),
    );
    let m = manager(
        &dir,
        vec![
            program("a", &app, "server", free_port(), json!({})),
            program("b", &app, "server", free_port(), json!({})),
        ],
    );
    m.init().await;
    for id in ["a", "b"] {
        let s = m.state(id);
        assert!(!s.managed && s.pid.is_none(), "{id} : {s:?}");
    }
    m.close().await;
}

/// Un projet repris qui plante passe en échec, avec son code de sortie (Windows) ; la version Node
/// y voyait une sortie propre.
#[cfg(windows)]
#[tokio::test]
async fn plantage_d_un_projet_repris() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let projects = vec![program(
        "api",
        &app,
        "flaky",
        free_port(),
        json!({ "env": { "CRASHES": "1" }, "health": { "expect": "Factice" } }),
    )];
    let first = manager(&dir, projects.clone());
    first.init().await;
    first.start("api").unwrap();
    until_state(&first, "api", |s| s.state == State::Running).await;
    first.close().await;
    drop(first);
    let next = manager(&dir, projects);
    next.init().await;
    assert!(next.state("api").managed);
    until_state(&next, "api", |s| s.state == State::Failed).await;
    let s = next.state("api");
    assert_eq!(s.exit_code, Some(3));
    assert_eq!(s.error.as_deref(), Some("Error: plantage 1"));
    next.close().await;
}

#[tokio::test]
async fn demarrage_automatique_une_fois_par_demarrage() {
    let dir = temp_dir("servym-m-");
    let app = app_dir(&dir);
    let projects = vec![program(
        "auto",
        &app,
        "server",
        free_port(),
        json!({ "autostart": true, "health": { "expect": "Factice" } }),
    )];
    let build = |boot: i64| {
        Manager::builder(json!({ "instance": { "port": 1 }, "projects": projects.clone() }))
            .logs_dir(&dir.path().join("logs"))
            .runtime_dir(&dir.path().join("runtime"))
            .timings(timings())
            .boot_time(boot)
            .lang(Lang::Fr)
            .build()
    };
    let m = build(1_000_000);
    m.init().await;
    until_state(&m, "auto", |s| s.state == State::Running).await;
    m.stop("auto").await.unwrap();
    m.close().await;
    let again = build(1_030_000);
    again.init().await;
    sleep(1000).await;
    assert_eq!(
        again.state("auto").state,
        State::Stopped,
        "même démarrage du PC : pas relancé"
    );
    again.close().await;
    let reboot = build(1_000_000 + 3_600_000);
    reboot.init().await;
    until_state(&reboot, "auto", |s| s.state == State::Running).await;
    stop_all(&reboot, &["auto"]).await;
}

#[tokio::test]
async fn arret_propre_recoit_cwd_et_env() {
    let dir = temp_dir("servym-stop-");
    let app = app_dir(&dir);
    let port = free_port();
    let exe = path_str(fixture_exe());
    let m = manager(
        &dir,
        vec![program(
            "api",
            &app,
            "server",
            port,
            json!({
                "health": { "expect": "Factice" },
                "env": { "SERVYM_TEST_ENV": "stop-secret" },
                "stop": { "cmd": exe, "args": ["stop-probe", "stopped.txt"] },
            }),
        )],
    );
    m.init().await;
    m.start("api").unwrap();
    until_state(&m, "api", |s| s.state == State::Running).await;
    m.stop("api").await.unwrap();
    let probe = read(app.join("stopped.txt"));
    assert_eq!(probe, format!("PORT={port}\nENV=stop-secret\n"));
    m.close().await;
}
