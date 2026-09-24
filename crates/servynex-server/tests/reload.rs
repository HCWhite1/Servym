//! tests/reload.test.js : relecture automatique de projects.json.

mod common;

use std::time::Duration;

use common::Servynex;
use serde_json::{Value, json};
use servynex_fixtures::{assert_match, assert_no_match, path_str, read, sleep, until, write};

/// Servynex de test qui surveille projects.json toutes les 50 ms.
async fn make_app() -> Servynex {
    let dir = servynex_fixtures::temp_dir("servynex-reload-");
    let site = path_str(dir.path().join("site"));
    write(dir.path().join("site").join("index.html"), "ok");
    let config = json!({
        "instance": { "port": 1 },
        "projects": [
            { "id": "site", "name": "Site", "type": "static", "cwd": site, "port": servynex_fixtures::free_port() },
            { "id": "doc", "name": "Doc", "type": "file", "cwd": site, "open": "index.html" },
        ],
    });
    write(
        dir.path().join("projects.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    );
    let s = Servynex::start_in(dir, config.clone(), |b| b.watch(Duration::from_millis(50))).await;
    // start_in écrit la configuration compacte : on remet la version mise en forme.
    write(s.file(), serde_json::to_string_pretty(&config).unwrap());
    s.app.manager.init().await;
    s
}

fn edit(s: &Servynex, change: impl FnOnce(&mut Value)) {
    let mut next = s.disk();
    change(&mut next);
    write(s.file(), serde_json::to_string_pretty(&next).unwrap());
}

async fn names(s: &Servynex) -> Vec<String> {
    s.call("GET", "/api/projects").await.json()["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

fn own(s: &Servynex) -> String {
    read(s.path().join("logs").join("servynex.log"))
}

async fn config_error(s: &Servynex) -> Value {
    s.call("GET", "/api/projects").await.json()["instance"]["configError"].clone()
}

/// JS : reload.test.js › relecture automatique de projects.json › une modification à la main est appliquée seule, et inscrite à l’activité
#[tokio::test]
#[ignore = "phase 5"]
async fn modification_a_la_main_appliquee() {
    let s = make_app().await;
    edit(&s, |c| c["projects"][1]["name"] = json!("Documentation"));
    until(
        || async { names(&s).await.contains(&"Documentation".to_string()) },
        5000,
    )
    .await;
    assert_match!(own(&s), "relu automatiquement");
    let audit = Servynex::audit_texts(&s.audit(None).await).join("\n");
    assert_match!(
        audit,
        r#"Modifié dans projects\.json : name "Doc" → "Documentation""#
    );
    s.close(&[]).await;
}

/// JS : reload.test.js › relecture automatique de projects.json › fichier invalide : erreur affichée, dernière configuration gardée ; corrigé, appliqué
#[tokio::test]
#[ignore = "phase 5"]
async fn fichier_invalide_puis_corrige() {
    let s = make_app().await;
    let good = read(s.file());
    write(s.file(), good.replace("\"Doc\"", "\"Doc"));
    until(|| async { config_error(&s).await.is_string() }, 5000).await;
    assert_eq!(names(&s).await, ["Site", "Doc"], "configuration gardée");
    write(s.file(), good.replace("\"Doc\"", "\"Doc corrigée\""));
    until(
        || async { names(&s).await.contains(&"Doc corrigée".to_string()) },
        5000,
    )
    .await;
    assert_eq!(config_error(&s).await, Value::Null);
    s.close(&[]).await;
}

/// JS : reload.test.js › relecture automatique de projects.json › les écritures de Servynex lui-même ne sont pas relues
#[tokio::test]
#[ignore = "phase 5"]
async fn ecritures_de_servynex_non_relues() {
    let s = make_app().await;
    assert_eq!(
        s.call_json(
            "PATCH",
            "/api/projects/doc",
            json!({ "desc": "depuis la page" }),
            None
        )
        .await
        .status,
        200
    );
    sleep(400).await;
    assert_no_match!(own(&s), "relu automatiquement");
    assert_no_match!(own(&s), "Modifié dans projects.json");
    s.close(&[]).await;
}

/// JS : reload.test.js › relecture automatique de projects.json › retirer à la main un projet en marche : refusé, signalé
#[tokio::test]
#[ignore = "phase 5"]
async fn retirer_a_la_main_un_projet_en_marche() {
    let s = make_app().await;
    s.call("POST", "/api/projects/site/start").await;
    until(
        || async { s.call("GET", "/api/projects").await.json()["projects"][0]["state"] == "running" },
        5000,
    )
    .await;
    edit(&s, |c| {
        c["projects"].as_array_mut().unwrap().remove(0);
    });
    until(|| async { config_error(&s).await.is_string() }, 5000).await;
    let list = s.call("GET", "/api/projects").await.json();
    assert_match!(list["instance"]["configError"].as_str().unwrap(), "Site");
    assert_eq!(
        list["projects"][0]["state"], "running",
        "toujours là, toujours en marche"
    );
    s.call("POST", "/api/projects/site/stop").await;
    // Arrêté : le retrait est appliqué sans autre modification du fichier.
    until(|| async { !names(&s).await.contains(&"Site".to_string()) }, 5000).await;
    assert_eq!(config_error(&s).await, Value::Null);
    assert_eq!(
        own(&s).matches("pas appliqué").count(),
        1,
        "signalé une seule fois"
    );
    s.close(&[]).await;
}
