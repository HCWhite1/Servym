//! tests/restart.test.js : relancer un projet s'il plante (partie serveur).

mod common;

use std::time::Duration;

use common::{Servynex, fast, program, until_state, until_state_for, with};
use serde_json::{Value, json};
use servynex_core::manager::{AutoRestart, State, Timings};
use servynex_fixtures::{assert_match, free_port, read, sleep, temp_dir, until};

/// « api » répond, puis plante 1,5 s après chaque lancement tant qu'il n'a pas planté `crashes`
/// fois (compté dans runs.txt) ; « dead » plante avant de répondre (échec au lancement).
async fn make_app(crashes: u32, restart_on_crash: bool, delays_ms: [u64; 3]) -> Servynex {
    let dir = temp_dir("servynex-restart-");
    let app = dir.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    let mut api = with(
        program("api", "Mon API", &app, "flaky", free_port()),
        json!({ "env": { "CRASHES": crashes.to_string() }, "health": { "expect": "Factice" } }),
    );
    if restart_on_crash {
        api["restartOnCrash"] = json!(true);
    }
    let dead = with(
        program("dead", "Morte", &app, "dead", free_port()),
        json!({ "restartOnCrash": true }),
    );
    let config = json!({ "instance": { "port": 1, "notify": true }, "projects": [api, dead] });
    let timings = Timings {
        restart_delays: delays_ms.iter().map(|ms| Duration::from_millis(*ms)).collect(),
        ..fast()
    };
    let s = Servynex::start_in(dir, config, |b| b.timings(timings)).await;
    s.app.manager.init().await;
    s
}

fn runs(s: &Servynex) -> u32 {
    read(s.path().join("app").join("runs.txt"))
        .trim()
        .parse()
        .unwrap()
}

async fn history(s: &Servynex, id: &str) -> Vec<Value> {
    s.call("GET", &format!("/api/projects/{id}/history")).await.json()["entries"]
        .as_array()
        .unwrap()
        .clone()
}

/// JS : restart.test.js › relancer s’il plante › désactivé par défaut : un plantage laisse le projet en échec
#[tokio::test]
async fn desactive_par_defaut() {
    let s = make_app(1, false, [100, 200, 300]).await;
    let list = s.call("GET", "/api/projects").await.json();
    let api = list["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "api")
        .cloned()
        .unwrap();
    assert_eq!(api["restartOnCrash"], false);
    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| p.state == State::Running).await;
    until_state(&s.app.manager, "api", |p| p.state == State::Failed).await;
    sleep(800).await;
    assert_eq!(s.state("api").state, State::Failed);
    assert_eq!(s.state("api").auto_restart, None);
    assert_eq!(runs(&s), 1);
    assert_match!(
        s.notifier
            .sent()
            .first()
            .map(|n| n.title.clone())
            .unwrap_or_default(),
        "Mon API s'est arrêté"
    );
    s.close(&["api"]).await;
}

/// JS : restart.test.js › relancer s’il plante › un plantage est relancé, sans notification, et inscrit à l’historique
#[tokio::test]
async fn plantage_relance() {
    let s = make_app(1, true, [100, 200, 300]).await;
    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| p.state == State::Running).await;
    until_state(&s.app.manager, "api", |p| p.state == State::Failed).await;
    until(
        || async { runs(&s) == 2 && s.state("api").state == State::Running },
        20_000,
    )
    .await;
    assert!(s.state("api").managed);
    sleep(2000).await;
    assert_eq!(s.state("api").state, State::Running, "reste en ligne");
    assert!(
        s.notifier.sent().is_empty(),
        "réparé tout seul : pas de notification"
    );
    let details: Vec<String> = history(&s, "api")
        .await
        .iter()
        .map(|e| {
            format!(
                "{} → {} · {}",
                e["from"].as_str().unwrap(),
                e["to"].as_str().unwrap(),
                e["detail"].as_str().unwrap_or_default()
            )
        })
        .collect();
    let has = |re: &str| details.iter().any(|d| regex::Regex::new(re).unwrap().is_match(d));
    assert!(
        has(r"^running → failed · .*Relance dans 1 s \(1/3\)"),
        "{}",
        details.join(" | ")
    );
    assert!(
        has(r"^failed → starting · Relancé automatiquement \(1/3\)"),
        "{}",
        details.join(" | ")
    );
    s.close(&["api"]).await;
}

/// JS : restart.test.js › relancer s’il plante › plante en boucle : abandon après 3 relances, notifié
#[tokio::test]
async fn abandon_apres_trois_relances() {
    let s = make_app(99, true, [100, 200, 300]).await;
    s.call("POST", "/api/projects/api/start").await;
    until_state_for(
        &s.app.manager,
        "api",
        |p| p.auto_restart == Some(AutoRestart::GaveUp),
        30_000,
    )
    .await;
    assert_eq!(s.state("api").state, State::Failed);
    assert_eq!(runs(&s), 4, "lancé, puis relancé 3 fois");
    sleep(800).await;
    assert_eq!(runs(&s), 4, "plus de relance");
    let sent = s.notifier.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].title, "Mon API plante en boucle, relance abandonnée");
    assert_match!(sent[0].body, "plantage 4");
    let entries = history(&s, "api").await;
    assert_match!(
        entries[0]["detail"].as_str().unwrap_or_default(),
        "Relance abandonnée : 3 relances en 5 min"
    );
    for n in 1..=3 {
        let needle = format!("Relancé automatiquement ({n}/3)");
        assert!(
            entries
                .iter()
                .any(|e| e["detail"].as_str().unwrap_or_default().contains(&needle)),
            "relance {n}/3"
        );
    }

    // Relancé à la main : le compte repart de zéro.
    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| {
        p.state == State::Failed && p.auto_restart == Some(AutoRestart::Scheduled)
    })
    .await;
    until(|| async { runs(&s) == 6 }, 20_000).await;
    s.close(&["api"]).await;
}

/// JS : restart.test.js › relancer s’il plante › pas après un échec au lancement
#[tokio::test]
async fn pas_apres_un_echec_au_lancement() {
    let s = make_app(99, true, [100, 200, 300]).await;
    s.call("POST", "/api/projects/dead/start").await;
    until_state(&s.app.manager, "dead", |p| p.state == State::Failed).await;
    sleep(800).await;
    assert_eq!(s.state("dead").state, State::Failed);
    assert_eq!(s.state("dead").auto_restart, None);
    assert_match!(
        s.notifier
            .sent()
            .first()
            .map(|n| n.title.clone())
            .unwrap_or_default(),
        "n'a pas pu démarrer"
    );
    s.close(&["api"]).await;
}

/// JS : restart.test.js › relancer s’il plante › arrêté à la main : pas relancé ; relance prévue annulée par « Arrêter » ou en retirant le réglage
#[tokio::test]
async fn arret_a_la_main_annule_la_relance() {
    let s = make_app(99, true, [1500, 1500, 1500]).await;
    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| p.state == State::Running).await;
    s.call("POST", "/api/projects/api/stop").await;
    until_state(&s.app.manager, "api", |p| p.state == State::Stopped).await;

    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| {
        p.auto_restart == Some(AutoRestart::Scheduled)
    })
    .await;
    assert_eq!(s.call("POST", "/api/projects/api/stop").await.status, 200);
    assert_eq!(s.state("api").state, State::Failed);
    assert_eq!(s.state("api").auto_restart, None);
    assert_eq!(s.state("api").detail, None);

    s.call("POST", "/api/projects/api/start").await;
    until_state(&s.app.manager, "api", |p| {
        p.auto_restart == Some(AutoRestart::Scheduled)
    })
    .await;
    let mut config = s.app.manager.config();
    config["projects"][0]
        .as_object_mut()
        .unwrap()
        .remove("restartOnCrash");
    s.app.manager.set_config(config).unwrap();
    assert_eq!(s.state("api").auto_restart, None);

    let before = runs(&s);
    sleep(2000).await;
    assert_eq!(runs(&s), before, "aucune relance");
    assert_eq!(s.state("api").state, State::Failed);
    s.close(&["api"]).await;
}
