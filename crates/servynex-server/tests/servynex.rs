//! Partie « serveur et processus » de tests/servynex.test.js.
//!
//! En JavaScript, ces tests partagent une même application et s'enchaînent. Ici, chacun prépare la
//! sienne (mêmes projets, ports libres) et crée lui-même ce dont il dépend : ils sont indépendants.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{Servynex, block_port, fast, program, until_state, with};
use serde_json::{Value, json};
use servynex_core::i18n::Lang;
use servynex_core::manager::{Event, Manager, State, Timings};
use servynex_fixtures::{Req, assert_match, free_port, listening, path_str, read, sleep, until, write};
use servynex_server::testing::FakeAutostart;

/// Ports des projets de test.
struct Ports {
    ok: u16,
    crash: u16,
    site: u16,
    busy: u16,
    wrong: u16,
    respawn: u16,
    auto: u16,
    db: u16,
    web: u16,
    solo: u16,
    sdb: u16,
}

const ALL: &[&str] = &[
    "ok",
    "launcher",
    "crash",
    "site",
    "respawn",
    "auto",
    "busy",
    "wrong",
    "dep",
    "db",
    "web",
    "sdb",
    "solo",
    "fan",
    "nouveau-projet",
];

fn timings() -> Timings {
    Timings {
        relaunch: Duration::from_secs(10),
        launcher: Duration::from_millis(1500),
        ..fast()
    }
}

/// Même configuration que le test JS. `auto` (démarré avec le PC) seulement si demandé.
async fn setup(with_auto: bool) -> (Servynex, Ports) {
    let dir = servynex_fixtures::temp_dir("servynex-test-");
    let app = dir.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    let site = dir.path().join("site");
    write(site.join("sub").join(".keep"), "");
    write(site.join("index.html"), "accueil");
    write(site.join("page.html"), "page");
    write(site.join(".env"), "SECRET");
    let ports = Ports {
        ok: free_port(),
        crash: free_port(),
        site: free_port(),
        busy: free_port(),
        wrong: free_port(),
        respawn: free_port(),
        auto: free_port(),
        db: free_port(),
        web: free_port(),
        solo: free_port(),
        sdb: free_port(),
    };
    let factice = json!({ "expect": "Factice" });
    let mut projects = vec![
        with(
            program("ok", "Factice", &app, "server", ports.ok),
            json!({ "health": factice, "url": "http://localhost:{port}", "env": { "SERVYNEX_TEST_ENV": "bonjour", "PORT": "1" } }),
        ),
        program("launcher", "Lanceur", &app, "launcher", free_port()),
        program("crash", "Crash", &app, "crash", ports.crash),
        json!({ "id": "site", "name": "Site", "cwd": path_str(&site), "type": "static", "port": ports.site }),
        with(
            program("respawn", "Relance", &app, "respawn", ports.respawn),
            json!({ "health": { "expect": "Relance" } }),
        ),
        json!({ "id": "doc", "name": "Doc", "cwd": path_str(&site), "type": "file", "open": "absent.html" }),
        program("busy", "Occupé", &app, "server", ports.busy),
        with(
            program("wrong", "Autre", &app, "server", ports.wrong),
            json!({ "health": { "expect": "Pas ça" } }),
        ),
        with(
            program("dep", "Dépendant", &app, "crash", free_port()),
            json!({ "requires": ["crash"] }),
        ),
        with(
            program("db", "Base", &app, "server", ports.db),
            json!({ "service": true, "health": factice }),
        ),
        with(
            program("web", "Web", &app, "server", ports.web),
            json!({ "requires": ["db"], "health": factice }),
        ),
        with(
            program("sdb", "Base solo", &app, "server", ports.sdb),
            json!({ "service": true, "health": factice }),
        ),
        with(
            program("solo", "Solo", &app, "server", ports.solo),
            json!({ "requires": ["sdb"], "health": factice }),
        ),
        with(
            program("fan", "Fan", &app, "server", free_port()),
            json!({ "requires": ["solo", "db"] }),
        ),
    ];
    if with_auto {
        projects.insert(
            5,
            with(
                program("auto", "Auto", &app, "server", ports.auto),
                json!({ "autostart": true, "health": factice }),
            ),
        );
    }
    let config = json!({ "instance": { "port": 1 }, "projects": projects });
    let s = Servynex::start_in(dir, config, |b| b.timings(timings()).boot_time(1_000_000)).await;
    s.app.manager.init().await;
    (s, ports)
}

async fn post(s: &Servynex, path: &str) -> servynex_fixtures::Res {
    s.call("POST", path).await
}

async fn get(s: &Servynex, path: &str) -> Value {
    s.call("GET", path).await.json()
}

async fn until_running(s: &Servynex, id: &str) {
    until_state(&s.app.manager, id, |p| p.state == State::Running).await;
}

async fn until_stopped(s: &Servynex, id: &str) {
    until_state(&s.app.manager, id, |p| p.state == State::Stopped).await;
}

async fn until_failed(s: &Servynex, id: &str) {
    until_state(&s.app.manager, id, |p| p.state == State::Failed).await;
}

async fn patch(s: &Servynex, id: &str, body: Value) -> servynex_fixtures::Res {
    s.call_json("PATCH", &format!("/api/projects/{id}"), body, None)
        .await
}

fn ids(config: &Value) -> Vec<String> {
    config["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

fn manager_for(s: &Servynex, boot_time: i64) -> Manager {
    Manager::builder(s.app.manager.config())
        .logs_dir(&s.path().join("logs"))
        .runtime_dir(&s.path().join("runtime"))
        .boot_time(boot_time)
        .lang(Lang::Fr)
        .build()
}

/// JS : servynex.test.js › serveur et processus › protections HTTP
#[tokio::test]
#[ignore = "phase 5"]
async fn protections_http() {
    let (s, _) = setup(false).await;
    let port = s.port;
    assert_eq!(
        Req::get("/api/projects")
            .host("evil.example")
            .send(port)
            .await
            .status,
        421
    );
    assert_eq!(Req::post("/api/projects/ok/start").send(port).await.status, 403);
    assert_eq!(
        Req::post("/api/projects/ok/start")
            .servynex()
            .header("origin", "http://evil.example")
            .send(port)
            .await
            .status,
        403
    );
    assert_eq!(
        Req::post("/api/projects/ok/start")
            .servynex()
            .header("origin", "null")
            .send(port)
            .await
            .status,
        403
    );
    assert_eq!(
        Req::get("/api/projects")
            .header("sec-fetch-site", "cross-site")
            .send(port)
            .await
            .status,
        403
    );
    for route in [
        "/api/free-port?from=1024",
        "/api/git?fresh",
        "/api/ports",
        "/api/network",
        "/api/autostart",
    ] {
        assert_eq!(
            Req::get(route).send(port).await.status,
            403,
            "{route} sans en-tête"
        );
    }
    assert_eq!(Req::get("/server.js").send(port).await.status, 404);
    assert_eq!(Req::get("/projects.json").send(port).await.status, 404);
    assert_eq!(post(&s, "/api/projects/__proto__/start").await.status, 404);
    let page = Req::get("/").send(port).await;
    assert_eq!(page.status, 200);
    assert_match!(
        page.header("content-security-policy").unwrap_or_default(),
        "script-src 'self'"
    );
    assert_eq!(
        Req::new("PATCH", "/api/projects/ok")
            .servynex()
            .body("{}")
            .send(port)
            .await
            .status,
        415
    );
    assert_eq!(
        Req::post("/api/projects/ok/start")
            .header("x-hub-request", "1")
            .send(port)
            .await
            .status,
        403,
        "ancien en-tête X-Hub-Request refusé"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › ressources : processeur et mémoire des projets en marche
#[tokio::test]
#[ignore = "phase 5"]
async fn ressources() {
    let (s, _) = setup(false).await;
    assert_eq!(
        Req::get("/api/stats").send(s.port).await.status,
        403,
        "sans en-tête : une mesure coûte"
    );
    post(&s, "/api/projects/ok/start").await;
    until_running(&s, "ok").await;
    let first = get(&s, "/api/stats").await;
    assert!(
        first["projects"]["ok"]["memory"].as_u64().unwrap() > 1_000_000,
        "mémoire de l'application factice"
    );
    assert!(
        first["projects"]["ok"]["processes"].as_u64().unwrap() >= 2,
        "cmd.exe puis le programme : toute l’arborescence"
    );
    assert!(first["projects"].get("crash").is_none(), "arrêté : absent");
    assert!(
        first["self"]["memory"].as_u64().unwrap() > 1_000_000
            && first["self"]["processes"].as_u64().unwrap() >= 1,
        "Servynex lui-même"
    );
    let total = first["total"]["memory"].as_u64().unwrap();
    assert!(
        total
            >= first["projects"]["ok"]["memory"].as_u64().unwrap()
                + first["self"]["memory"].as_u64().unwrap(),
        "total"
    );
    sleep(2500).await;
    let second = get(&s, "/api/stats").await;
    let cpu = second["projects"]["ok"]["cpu"]
        .as_f64()
        .expect("processeur mesuré");
    assert!((0.0..=100.0).contains(&cpu));
    post(&s, "/api/projects/ok/stop").await;
    until_stopped(&s, "ok").await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › lance, suit le journal, arrête
#[tokio::test]
#[ignore = "phase 5"]
async fn lance_suit_le_journal_arrete() {
    let (s, ports) = setup(false).await;
    assert_eq!(post(&s, "/api/projects/ok/start").await.status, 200);
    assert_eq!(
        post(&s, "/api/projects/ok/start").await.status,
        409,
        "double lancement refusé"
    );
    until_running(&s, "ok").await;
    assert!(s.state("ok").managed && s.state("ok").pid.is_some());
    assert_match!(Req::get("/").send(ports.ok).await.body, "Factice");
    let log = s.app.manager.log_file("ok");
    until(
        || {
            let log = log.clone();
            async move {
                std::fs::read_to_string(log)
                    .unwrap_or_default()
                    .contains("démarrage sur 127.0.0.1")
            }
        },
        20_000,
    )
    .await;
    let tail = Req::get("/api/projects/ok/logs?tail").send(s.port).await.json();
    let lines: Vec<&str> = tail["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert!(
        lines.iter().any(|l| l.contains("démarrage sur 127.0.0.1")),
        "HOST=127.0.0.1 transmis, accents intacts"
    );
    assert!(
        lines.iter().any(|l| l.contains(&format!("{} bonjour", ports.ok))),
        "env du projet transmis, sans écraser PORT"
    );
    let overview = get(&s, "/api/ports").await;
    let row = overview["ports"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["port"] == ports.ok)
        .cloned()
        .expect("ligne du port");
    assert_eq!(row["project"], "ok");
    assert_eq!(row["exposed"], false);
    assert!(row["process"].as_str().unwrap().starts_with("servynex-fixtur"));

    // Servynex redémarré : le projet est repris, sans « stopped → running » dans l'historique.
    let next = manager_for(&s, 1_000_000);
    let mut events = next.subscribe();
    next.init().await;
    assert_eq!(next.state("ok").state, State::Running);
    assert!(next.state("ok").managed, "repris par son PID");
    while let Ok(event) = events.try_recv() {
        if let Event::Transition(t) = event {
            assert_ne!(t.id, "ok", "aucune transition pour le projet repris : {t:?}");
        }
    }
    next.close().await;

    let pid = s.state("ok").pid.unwrap();
    assert_eq!(post(&s, "/api/projects/ok/stop").await.status, 200);
    assert_eq!(s.state("ok").state, State::Stopped);
    until(
        || async move { Req::get("/").try_send(ports.ok).await.is_err() },
        20_000,
    )
    .await;
    assert!(!servynex_sys::native().is_alive(pid));
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › un plantage passe en échec avec la ligne d'erreur
#[tokio::test]
#[ignore = "phase 5"]
async fn plantage_en_echec() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/crash/start").await;
    until_failed(&s, "crash").await;
    assert_eq!(s.state("crash").exit_code, Some(3));
    assert_eq!(s.state("crash").error.as_deref(), Some("Error: base introuvable"));
    let entries = get(&s, "/api/projects/crash/history").await["entries"].clone();
    assert_eq!(
        entries[0],
        json!({ "at": entries[0]["at"], "id": "crash", "from": "starting", "to": "failed", "detail": "code 3 · Error: base introuvable" })
    );
    assert_eq!(entries[1]["to"], "starting", "du plus récent au plus ancien");
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › sorti sans erreur pendant le lancement, sans répondre : échec expliqué par son journal
#[tokio::test]
#[ignore = "phase 5"]
async fn lanceur_sorti_sans_repondre() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/launcher/start").await;
    until_state(&s.app.manager, "launcher", |p| {
        p.detail
            .as_deref()
            .is_some_and(|d| d.starts_with("Lanceur terminé"))
    })
    .await;
    assert_eq!(
        s.state("launcher").state,
        State::Starting,
        "le serveur a peut-être été laissé en arrière-plan : on attend le port"
    );
    until_failed(&s, "launcher").await;
    assert_eq!(s.state("launcher").exit_code, Some(0));
    assert_match!(
        s.state("launcher").error.unwrap_or_default(),
        r"^Lanceur s'est arrêté pendant son lancement sans répondre sur le port \d+ : \[WARNING\] The local web server is already running$"
    );
    let entries = get(&s, "/api/projects/launcher/history").await["entries"].clone();
    assert_eq!(entries[0]["from"], "starting");
    assert_eq!(entries[0]["to"], "failed");
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › en échec : relançable tout de suite
#[tokio::test]
#[ignore = "phase 5"]
async fn en_echec_relancable() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/crash/start").await;
    until_failed(&s, "crash").await;
    assert_eq!(post(&s, "/api/projects/crash/start").await.status, 200);
    // Guetté de près : la relance suit l'échec à quelques millisecondes.
    for _ in 0..2000 {
        if s.state("crash").state == State::Failed {
            break;
        }
        sleep(5).await;
    }
    let again = post(&s, "/api/projects/crash/start").await;
    assert_eq!(again.status, 200, "{}", again.body);
    until_failed(&s, "crash").await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › journal du lancement précédent
#[tokio::test]
#[ignore = "phase 5"]
async fn journal_du_lancement_precedent() {
    let (s, _) = setup(false).await;
    for _ in 0..2 {
        post(&s, "/api/projects/crash/start").await;
        until_failed(&s, "crash").await;
    }
    let view = |list: Value, id: &str| {
        list["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        view(get(&s, "/api/projects").await, "crash")["hasPrevLog"],
        true,
        "lancé plusieurs fois"
    );
    let lines = get(&s, "/api/projects/crash/logs?prev").await["lines"].clone();
    assert_match!(lines[0].as_str().unwrap(), r"^\[servynex\] ", "depuis son début");
    assert!(
        lines
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "Error: base introuvable")
    );
    let _ = std::fs::remove_file(s.path().join("logs").join("sdb.prev.log"));
    assert_eq!(view(get(&s, "/api/projects").await, "sdb")["hasPrevLog"], false);
    assert_eq!(
        get(&s, "/api/projects/sdb/logs?prev").await,
        json!({ "lines": [] })
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › une dépendance en échec bloque le lancement
#[tokio::test]
#[ignore = "phase 5"]
async fn dependance_en_echec() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/dep/start").await;
    until_failed(&s, "dep").await;
    assert_match!(s.state("dep").error.unwrap_or_default(), "Crash n'a pas démarré");
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › port déjà pris, ou pris par un autre site
#[tokio::test]
#[ignore = "phase 5"]
async fn port_deja_pris() {
    let (s, ports) = setup(false).await;
    let blocker = block_port(ports.busy).await;
    post(&s, "/api/projects/busy/start").await;
    until_failed(&s, "busy").await;
    assert_match!(
        s.state("busy").error.unwrap_or_default(),
        &format!("port {} est déjà pris", ports.busy)
    );
    drop(blocker);

    post(&s, "/api/projects/wrong/start").await;
    // « failed » à la fin du lancement, puis « conflict » dès la sonde suivante
    // (le port répond, mais pas le bon site) : les deux sont justes.
    until_state(&s.app.manager, "wrong", |p| {
        matches!(p.state, State::Failed | State::Conflict)
    })
    .await;
    assert_match!(s.state("wrong").error.unwrap_or_default(), "ce n'est pas Autre");
    assert_eq!(
        post(&s, "/api/projects/wrong/stop").await.status,
        200,
        "le processus lancé reste arrêtable"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › site statique intégré
#[tokio::test]
#[ignore = "phase 5"]
async fn site_statique_integre() {
    let (s, ports) = setup(false).await;
    post(&s, "/api/projects/site/start").await;
    until_running(&s, "site").await;
    assert_eq!(Req::get("/").send(ports.site).await.body, "accueil");
    assert_eq!(Req::get("/page").send(ports.site).await.body, "page");
    assert_eq!(Req::get("/.env").send(ports.site).await.status, 404);
    post(&s, "/api/projects/site/stop").await;
    assert_eq!(s.state("site").state, State::Stopped);
    assert!(Req::get("/").try_send(ports.site).await.is_err());
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › un projet qui se relance lui-même est repris, avec son journal
#[tokio::test]
#[ignore = "phase 5"]
async fn projet_qui_se_relance_lui_meme() {
    let (s, ports) = setup(false).await;
    post(&s, "/api/projects/respawn/start").await;
    until_running(&s, "respawn").await;
    let first = s.state("respawn").pid;
    let _ = Req::get("/restart").try_send(ports.respawn).await;
    until_state(&s.app.manager, "respawn", |p| {
        p.state == State::Running && p.pid != first
    })
    .await;
    assert!(s.state("respawn").managed, "repris comme lancé par Servynex");
    let log = read(s.app.manager.log_file("respawn"));
    assert_eq!(
        log.lines().filter(|l| l.starts_with("instance ")).count(),
        2,
        "la nouvelle instance écrit dans le même journal"
    );
    assert_eq!(post(&s, "/api/projects/respawn/stop").await.status, 200);
    assert_eq!(s.state("respawn").state, State::Stopped);
    assert!(Req::get("/").try_send(ports.respawn).await.is_err());
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › démarrage automatique : une seule fois par démarrage du PC
#[tokio::test]
#[ignore = "phase 5"]
async fn demarrage_automatique_une_fois_par_demarrage() {
    let (s, _) = setup(true).await;
    until_running(&s, "auto").await;
    assert!(s.state("auto").managed);
    post(&s, "/api/projects/auto/stop").await;
    let again = manager_for(&s, 1_000_000 + 30_000);
    again.init().await;
    sleep(1500).await;
    assert_eq!(
        again.state("auto").state,
        State::Stopped,
        "Servynex redémarré, même démarrage du PC : pas relancé"
    );
    again.close().await;
    let reboot = manager_for(&s, 1_000_000 + 3_600_000);
    reboot.init().await;
    until_state(&reboot, "auto", |p| p.state == State::Running).await;
    reboot.stop("auto").await.unwrap();
    reboot.close().await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › projet « file » : jamais lancé ni arrêté, fichier vérifié avant ouverture
#[tokio::test]
#[ignore = "phase 5"]
async fn projet_file_jamais_lance() {
    let (s, _) = setup(false).await;
    let doc = s.app.manager.view(&s.app.manager.project("doc").unwrap());
    assert_eq!(doc["state"], "file");
    assert_eq!(doc["file"], path_str(s.path().join("site").join("absent.html")));
    assert_eq!(post(&s, "/api/projects/doc/start").await.status, 409);
    assert_eq!(post(&s, "/api/projects/doc/stop").await.status, 409);
    assert_eq!(post(&s, "/api/projects/doc/restart").await.status, 409);
    let open = post(&s, "/api/projects/doc/open").await;
    assert_eq!(open.status, 404, "fichier absent : rien n'est lancé");
    assert_match!(open.error(), "introuvable");
    assert_eq!(post(&s, "/api/projects/ok/open").await.status, 400);
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › réglages : enregistrés dans le fichier, champs protégés
#[tokio::test]
#[ignore = "phase 5"]
async fn reglages_enregistres() {
    let (s, ports) = setup(false).await;
    assert_eq!(patch(&s, "ok", json!({ "id": "autre" })).await.status, 400);
    assert_eq!(patch(&s, "ok", json!({ "args": ["a & calc"] })).await.status, 400);
    assert_eq!(patch(&s, "ok", json!({ "port": ports.site })).await.status, 400);
    let res = patch(&s, "ok", json!({ "name": "Renommé", "expose": true })).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["host"], "0.0.0.0");
    assert_eq!(s.disk()["projects"][0]["name"], "Renommé");
    let free = get(&s, &format!("/api/free-port?from={}", ports.ok)).await["port"]
        .as_u64()
        .unwrap() as u16;
    assert!(free >= ports.ok && ![ports.site, ports.crash, ports.busy].contains(&free));
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › une modification faite à la main n'est pas écrasée par l'interface
#[tokio::test]
#[ignore = "phase 5"]
async fn modification_a_la_main_gardee() {
    let (s, _) = setup(false).await;
    let mut on_disk = s.disk();
    for p in on_disk["projects"].as_array_mut().unwrap() {
        if p["id"] == "site" {
            p["desc"] = json!("écrit à la main");
        }
    }
    write(s.file(), on_disk.to_string());
    assert_eq!(
        patch(&s, "crash", json!({ "desc": "depuis l'interface" }))
            .await
            .status,
        200
    );
    let after = s.disk();
    let desc = |id: &str| {
        after["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .unwrap()["desc"]
            .clone()
    };
    assert_eq!(desc("site"), "écrit à la main");
    assert_eq!(desc("crash"), "depuis l'interface");
    assert_eq!(
        s.app.manager.project("site").unwrap()["desc"],
        "écrit à la main",
        "la modification à la main est aussi appliquée"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › activité : chaque modification de réglage, avec ses valeurs
#[tokio::test]
#[ignore = "phase 5"]
async fn activite_modifications_de_reglages() {
    let (s, _) = setup(false).await;
    assert_eq!(
        patch(&s, "dep", json!({ "desc": "avant", "startTimeout": 20 }))
            .await
            .status,
        200
    );
    assert_eq!(
        patch(
            &s,
            "dep",
            json!({ "desc": "après", "startTimeout": 30, "env": { "SECRET_TOKEN": "abc123" } })
        )
        .await
        .status,
        200
    );
    let texts = |entries: Vec<Value>| {
        entries
            .iter()
            .filter(|e| e["id"] == "dep")
            .map(|e| e["text"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let all = texts(s.audit(None).await);
    assert!(
        all.iter().any(|t| t.contains("desc \"avant\" → \"après\"")
            && t.contains("startTimeout 20 → 30")
            && t.contains("env — → SECRET_TOKEN=•••")),
        "{}",
        all.join(" | ")
    );
    assert!(
        !all.iter().any(|t| t.contains("abc123")),
        "valeurs de env jamais écrites"
    );
    // Modifié à la main, puis relu : la différence est aussi inscrite.
    let mut conf = s.disk();
    for p in conf["projects"].as_array_mut().unwrap() {
        if p["id"] == "dep" {
            p["desc"] = json!("à la main");
        }
    }
    write(s.file(), conf.to_string());
    assert_eq!(post(&s, "/api/reload").await.status, 200);
    let all = texts(s.audit(None).await);
    assert!(
        all.iter()
            .any(|t| t.contains("projects.json") && t.contains("desc \"après\" → \"à la main\"")),
        "{}",
        all.join(" | ")
    );
    s.close(ALL).await;
}

async fn create_new_project(s: &Servynex) -> servynex_fixtures::Res {
    let app = s.path().join("app");
    let body = with(
        program("x", "Nouveau projet", &app, "server", free_port()),
        json!({ "health": { "expect": "Factice" } }),
    );
    let mut body = body;
    body.as_object_mut().unwrap().remove("id");
    body.as_object_mut().unwrap().remove("hostControl");
    s.call_json("POST", "/api/projects", body, None).await
}

/// JS : servynex.test.js › serveur et processus › ajout depuis l’interface, puis lancement
#[tokio::test]
#[ignore = "phase 5"]
async fn ajout_puis_lancement() {
    let (s, ports) = setup(false).await;
    let app = s.path().join("app");
    let mut body = with(
        program("x", "Nouveau projet", &app, "server", free_port()),
        json!({ "health": { "expect": "Factice" } }),
    );
    body.as_object_mut().unwrap().remove("id");
    body.as_object_mut().unwrap().remove("hostControl");
    let no_header = Req::post("/api/projects").json(&body).send(s.port).await;
    assert_eq!(no_header.status, 403, "sans en-tête");
    let mut taken = body.clone();
    taken["port"] = json!(ports.ok);
    assert_eq!(
        s.call_json("POST", "/api/projects", taken, None).await.status,
        400,
        "port déjà pris"
    );
    let res = s.call_json("POST", "/api/projects", body, None).await;
    assert_eq!(res.status, 201);
    assert_eq!(res.json()["id"], "nouveau-projet");
    assert!(ids(&s.disk()).contains(&"nouveau-projet".to_string()));
    assert_eq!(post(&s, "/api/projects/nouveau-projet/start").await.status, 200);
    until_running(&s, "nouveau-projet").await;
    post(&s, "/api/projects/nouveau-projet/stop").await;
    until_stopped(&s, "nouveau-projet").await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › retirer du fichier un projet qui tourne est refusé
#[tokio::test]
#[ignore = "phase 5"]
async fn retirer_un_projet_qui_tourne() {
    let (s, _) = setup(false).await;
    assert_eq!(create_new_project(&s).await.status, 201);
    post(&s, "/api/projects/nouveau-projet/start").await;
    until_running(&s, "nouveau-projet").await;
    let before = read(s.file());
    let mut conf: Value = serde_json::from_str(&before).unwrap();
    conf["projects"]
        .as_array_mut()
        .unwrap()
        .retain(|p| p["id"] != "nouveau-projet");
    write(s.file(), conf.to_string());
    let res = post(&s, "/api/reload").await;
    assert_eq!(res.status, 409);
    assert_match!(res.error(), "tourne encore");
    assert!(s.app.manager.project("nouveau-projet").is_ok(), "toujours suivi");
    write(s.file(), &before);
    post(&s, "/api/projects/nouveau-projet/stop").await;
    until_stopped(&s, "nouveau-projet").await;
    write(s.file(), conf.to_string());
    assert_eq!(
        post(&s, "/api/reload").await.status,
        200,
        "arrêté, il peut être retiré"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › arrêter un projet arrête la dépendance lancée pour lui, pas celle lancée à la main
#[tokio::test]
#[ignore = "phase 5"]
async fn arret_de_la_dependance_lancee_pour_lui() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/web/start").await;
    until_state(&s.app.manager, "web", |p| p.state == State::Running).await;
    until_running(&s, "db").await;
    assert!(s.state("db").dependency);
    post(&s, "/api/projects/web/stop").await;
    until_stopped(&s, "db").await;

    post(&s, "/api/projects/db/start").await;
    until_running(&s, "db").await;
    post(&s, "/api/projects/web/start").await;
    until_running(&s, "web").await;
    post(&s, "/api/projects/web/stop").await;
    until_stopped(&s, "web").await;
    sleep(500).await;
    assert_eq!(
        s.state("db").state,
        State::Running,
        "lancée à la main : reste en marche"
    );
    post(&s, "/api/projects/db/stop").await;
    until_stopped(&s, "db").await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › supprimer : confirmation par le nom, arrêt d’abord, dépendances nettoyées, journal gardé
#[tokio::test]
#[ignore = "phase 5"]
async fn supprimer() {
    let (s, ports) = setup(false).await;
    let delete = |body: Option<Value>| {
        let req = Req::new("DELETE", "/api/projects/solo").servynex();
        match body {
            Some(b) => req.json(&b),
            None => req,
        }
    };
    post(&s, "/api/projects/solo/start").await;
    until_running(&s, "solo").await;
    until_running(&s, "sdb").await;
    assert_eq!(s.send(delete(None)).await.status, 400, "sans confirmation");
    assert_eq!(
        s.send(delete(Some(json!({ "confirm": "solo" })))).await.status,
        400,
        "l'identifiant ne suffit pas : le nom exact"
    );
    assert_eq!(
        s.send(delete(Some(json!({ "confirm": "Autre" })))).await.status,
        400
    );
    assert!(ids(&s.disk()).contains(&"solo".to_string()), "rien de supprimé");
    assert_eq!(s.state("solo").state, State::Running, "rien arrêté");

    let res = s.send(delete(Some(json!({ "confirm": "Solo" })))).await;
    assert_eq!(res.status, 200);
    assert!(
        !ids(&s.disk()).contains(&"solo".to_string()),
        "retiré de projects.json"
    );
    assert!(
        !ids(&get(&s, "/api/projects").await).contains(&"solo".to_string()),
        "retiré de la liste"
    );
    let fan = s.disk()["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "fan")
        .cloned()
        .unwrap();
    assert_eq!(fan["requires"], json!(["db"]), "retiré des dépendances");
    assert!(!listening(ports.solo).await, "arrêté avant la suppression");
    until_stopped(&s, "sdb").await; // lancée pour lui : arrêtée avec lui
    assert!(s.path().join("logs").join("solo.log").exists(), "journal gardé");
    let audit = s.audit(None).await;
    assert!(
        audit
            .iter()
            .any(|e| e["id"] == "solo" && e["text"].as_str().unwrap().contains("Projet supprimé"))
    );
    assert_eq!(
        s.send(delete(Some(json!({ "confirm": "Solo" })))).await.status,
        404,
        "déjà supprimé"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › erreurs dans la langue demandée par la page (Accept-Language)
#[tokio::test]
#[ignore = "phase 5"]
async fn erreurs_dans_la_langue_de_la_page() {
    let (s, _) = setup(false).await;
    let del = |lang: &'static str| {
        Req::new("DELETE", "/api/projects/crash")
            .servynex()
            .header("accept-language", lang)
            .json(&json!({}))
    };
    assert_match!(
        s.send(del("en-US,en;q=0.9")).await.error(),
        "Type \"Crash\" to confirm"
    );
    assert_match!(
        s.send(del("fr-FR")).await.error(),
        "Tapez « Crash » pour confirmer"
    );
    let unknown = s
        .send(
            Req::post("/api/projects/absent/start")
                .servynex()
                .header("accept-language", "en"),
        )
        .await;
    assert_eq!(unknown.status, 404);
    assert_eq!(unknown.error(), "Unknown project");
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › réordonner : écrit dans projects.json, refuse une liste incomplète
#[tokio::test]
#[ignore = "phase 5"]
async fn reordonner() {
    let (s, _) = setup(false).await;
    let order = ids(&s.disk());
    let reversed: Vec<String> = order.iter().rev().cloned().collect();
    let put = |body: Value| s.call_json("PUT", "/api/projects", body, None);
    assert_eq!(put(json!({ "order": reversed })).await.status, 200);
    assert_eq!(ids(&s.disk()), reversed);
    assert_eq!(
        ids(&get(&s, "/api/projects").await),
        reversed,
        "la liste suit le nouvel ordre"
    );
    assert_eq!(
        put(json!({ "order": reversed[1..] })).await.status,
        400,
        "identifiant manquant"
    );
    let mut extra = reversed.clone();
    extra.push("x".into());
    assert_eq!(
        put(json!({ "order": extra })).await.status,
        400,
        "identifiant inconnu"
    );
    assert_eq!(put(json!({})).await.status, 400);
    assert_eq!(ids(&s.disk()), reversed, "refusé : fichier inchangé");
    assert_eq!(put(json!({ "order": order })).await.status, 200);
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › projects.json invalide au démarrage : Servynex démarre avec la dernière configuration valide
#[tokio::test]
#[ignore = "phase 5"]
async fn projects_json_invalide_au_demarrage() {
    let dir = servynex_fixtures::temp_dir("servynex-invalide-");
    let valid = json!({ "instance": { "port": 1 }, "projects": [{ "id": "v", "name": "Valide", "type": "file", "cwd": path_str(dir.path()), "open": "x.html" }] });
    write(
        dir.path().join("runtime").join("projects.valid.json"),
        valid.to_string(),
    );
    write(
        dir.path().join("projects.json"),
        "{ \"instance\": { \"port\": 1 }, \"projects\": [ , ] }",
    );
    let notifier = servynex_server::testing::FakeNotifier::new(servynex_server::testing::Reply::Sent);
    let autostart = Arc::new(FakeAutostart::default());
    let broken = common::builder(dir.path(), &notifier, &autostart)
        .listen("127.0.0.1:0")
        .await
        .unwrap();
    let info = Req::get("/api/projects").send(broken.port).await.json();
    assert_match!(
        info["instance"]["configError"].as_str().unwrap_or_default(),
        "illisible"
    );
    assert_eq!(ids(&info), ["v"]);
    let patch = Req::new("PATCH", "/api/projects/v")
        .servynex()
        .json(&json!({ "name": "W" }))
        .send(broken.port)
        .await;
    assert_eq!(patch.status, 400);
    assert_match!(patch.error(), "à corriger d'abord");
    broken.close().await;
}

/// JS : servynex.test.js › serveur et processus › redémarrage de Servynex : refusé sans superviseur, demandé sinon
#[tokio::test]
#[ignore = "phase 5"]
async fn redemarrage_de_servynex() {
    let (s, _) = setup(false).await;
    assert_eq!(
        Req::post("/api/restart").send(s.port).await.status,
        403,
        "sans en-tête"
    );
    let res = post(&s, "/api/restart").await;
    assert_eq!(res.status, 409);
    assert_match!(res.error(), "servynex");
    assert_eq!(get(&s, "/api/projects").await["instance"]["restartable"], false);

    let asked = Arc::new(AtomicUsize::new(0));
    let counter = asked.clone();
    let other = Servynex::start(
        "servynex-restart-",
        json!({ "instance": { "port": 1 }, "projects": [] }),
        |b| {
            b.restart(Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }))
        },
    )
    .await;
    assert_eq!(other.call("POST", "/api/restart").await.status, 200);
    until(
        || {
            let asked = asked.clone();
            async move { asked.load(Ordering::SeqCst) == 1 }
        },
        2000,
    )
    .await;
    assert_eq!(
        other.call("GET", "/api/projects").await.json()["instance"]["restartable"],
        true
    );
    other.close(&[]).await;
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › démarrage avec Windows et présentation, depuis les Paramètres
#[tokio::test]
#[ignore = "phase 5"]
async fn demarrage_avec_windows_et_presentation() {
    let s = Servynex::start(
        "servynex-ui-",
        json!({ "instance": { "port": 1 }, "projects": [] }),
        |b| b,
    )
    .await;
    assert_eq!(
        Req::get("/api/autostart").send(s.port).await.status,
        403,
        "sans en-tête"
    );
    assert_eq!(s.call("GET", "/api/autostart").await.json()["enabled"], false);
    assert_eq!(
        s.call_json("PUT", "/api/autostart", json!({ "enabled": "oui" }), None)
            .await
            .status,
        400
    );
    assert_eq!(
        s.call_json("PUT", "/api/autostart", json!({ "enabled": true }), None)
            .await
            .json()["enabled"],
        true
    );
    assert!(s.autostart.on.load(Ordering::SeqCst));

    let instance = || async { s.call("GET", "/api/projects").await.json()["instance"].clone() };
    assert_eq!(instance().await["onboarded"], false);
    assert_eq!(s.call("POST", "/api/instance/onboarded").await.status, 200);
    assert_eq!(instance().await["onboarded"], true);
    assert_match!(
        instance().await["version"].as_str().unwrap_or_default(),
        r"^\d+\.\d+\.\d+"
    );
    assert_eq!(s.call("POST", "/api/instance/calc").await.status, 404);
    s.close(&[]).await;
}

/// JS : servynex.test.js › serveur et processus › activité : changements d’état de tous les projets
#[tokio::test]
#[ignore = "phase 5"]
async fn activite_changements_d_etat() {
    let (s, _) = setup(false).await;
    post(&s, "/api/projects/ok/start").await;
    until_running(&s, "ok").await;
    post(&s, "/api/projects/crash/start").await;
    until_failed(&s, "crash").await;
    assert_eq!(create_new_project(&s).await.status, 201);
    let history = get(&s, "/api/history").await;
    let entries = history["entries"].as_array().unwrap();
    let seen: Vec<&str> = entries.iter().filter_map(|e| e["id"].as_str()).collect();
    assert!(seen.contains(&"ok") && seen.contains(&"crash"), "{seen:?}");
    assert!(
        entries
            .iter()
            .filter(|e| e["kind"] != "audit")
            .all(|e| e["at"].is_string() && e["from"].is_string() && e["to"].is_string())
    );
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "audit" && e["text"].as_str().unwrap().contains("Projet ajouté")),
        "audit des réglages même sans protection"
    );
    s.close(ALL).await;
}

/// JS : servynex.test.js › serveur et processus › flux d'évènements
#[tokio::test]
#[ignore = "phase 5"]
async fn flux_d_evenements() {
    let (s, _) = setup(false).await;
    let events = Req::get("/api/events").first_event(s.port).await;
    assert_match!(events, r"^event: snapshot\ndata: \[");
    s.close(ALL).await;
}
