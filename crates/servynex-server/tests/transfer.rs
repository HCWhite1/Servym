//! tests/transfer.test.js : export et import de la configuration.

mod common;

use common::Servynex;
use serde_json::{Value, json};
use servynex_fixtures::{Req, assert_match, path_str, read};

struct Transfer {
    s: Servynex,
    site: String,
    config: Value,
}

/// Un site statique (démarrable sans processus), une page, une API.
async fn make_app() -> Transfer {
    let dir = servynex_fixtures::temp_dir("servynex-transfer-");
    let site = path_str(dir.path().join("site"));
    servynex_fixtures::write(dir.path().join("site").join("index.html"), "ok");
    let config = json!({
        "instance": { "port": 1, "name": "Maison" },
        "projects": [
            { "id": "site", "name": "Site", "type": "static", "cwd": site, "port": servynex_fixtures::free_port(), "desc": "Avant" },
            { "id": "doc", "name": "Doc", "type": "file", "cwd": site, "open": "index.html" },
            { "id": "api", "name": "API", "cwd": site, "cmd": "node", "args": ["server.js"], "port": servynex_fixtures::free_port(), "hostControl": "env", "env": { "TOKEN": "secret" } },
        ],
    });
    // Écrit mis en forme, comme un fichier tenu à la main.
    servynex_fixtures::write(
        dir.path().join("projects.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    );
    let s = Servynex::start_in(dir, config.clone(), |b| b).await;
    servynex_fixtures::write(s.file(), serde_json::to_string_pretty(&config).unwrap());
    s.app.manager.init().await;
    Transfer { s, site, config }
}

fn import(body: Value, cookie: Option<&str>) -> Req {
    Req::post("/api/config/import")
        .servynex()
        .cookie(cookie)
        .json(&body)
}

/// JS : transfer.test.js › export et import de la configuration › export : tout projects.json, en fichier à télécharger
#[tokio::test]
#[ignore = "phase 5"]
async fn export() {
    let t = make_app().await;
    assert_eq!(
        t.s.send(Req::get("/api/config/export")).await.status,
        403,
        "sans en-tête"
    );
    let res = t.s.call("GET", "/api/config/export").await;
    assert_eq!(res.status, 200);
    assert_match!(res.header("content-type").unwrap_or_default(), "application/json");
    assert_match!(
        res.header("content-disposition").unwrap_or_default(),
        r#"^attachment; filename="servynex-projects-\d{4}-\d{2}-\d{2}\.json"$"#
    );
    assert_eq!(
        res.json(),
        t.s.disk(),
        "le fichier tel qu’il est, variables d’environnement comprises"
    );
    t.s.close(&[]).await;
}

/// JS : transfer.test.js › export et import de la configuration › aperçu : ajouts, retraits, modifications, sans rien écrire
#[tokio::test]
#[ignore = "phase 5"]
async fn apercu() {
    let t = make_app().await;
    let before = read(t.s.file());
    let mut site = t.config["projects"][0].clone();
    site["desc"] = json!("Après");
    let projects = json!([site, t.config["projects"][2], { "id": "blog", "name": "Blog", "type": "file", "cwd": t.site, "open": "index.html" }]);
    let res =
        t.s.send(import(
            json!({ "config": { "instance": { "port": 1 }, "projects": projects } }),
            None,
        ))
        .await;
    assert_eq!(res.status, 200);
    let body = res.json();
    assert_eq!(body["applied"], false);
    assert_eq!(body["added"], json!([{ "id": "blog", "name": "Blog" }]));
    assert_eq!(body["removed"], json!([{ "id": "doc", "name": "Doc" }]));
    assert_eq!(
        body["changed"],
        json!([{ "id": "site", "name": "Site", "changes": "desc \"Avant\" → \"Après\"" }])
    );
    assert_eq!(body["unchanged"], 1);
    assert_eq!(read(t.s.file()), before, "fichier intact");
    t.s.close(&[]).await;
}

/// JS : transfer.test.js › export et import de la configuration › appliqué : les projets remplacés, la section instance de cette machine gardée
#[tokio::test]
#[ignore = "phase 5"]
async fn applique() {
    let t = make_app().await;
    let projects = json!([t.config["projects"][1], { "id": "blog", "name": "Blog", "type": "file", "cwd": t.site, "open": "index.html" }]);
    let body = json!({ "config": { "instance": { "port": 4242, "access": "lan", "name": "Autre PC" }, "projects": projects }, "apply": true });
    let res = t.s.send(import(body, None)).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["applied"], true);
    assert_eq!(
        t.s.disk(),
        json!({ "instance": t.config["instance"], "projects": projects })
    );
    let live = t.s.call("GET", "/api/projects").await.json();
    let ids: Vec<&str> = live["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["doc", "blog"]);
    let audit = Servynex::audit_texts(&t.s.audit(None).await).join("\n");
    assert_match!(audit, "(?i)import");
    assert_match!(audit, "Blog");
    t.s.close(&[]).await;
}

/// JS : transfer.test.js › export et import de la configuration › refusé : fichier invalide, ou projet en marche retiré
#[tokio::test]
#[ignore = "phase 5"]
async fn refuse() {
    let t = make_app().await;
    let before = read(t.s.file());
    assert_eq!(
        t.s.send(import(
            json!({ "config": { "projects": "non" }, "apply": true }),
            None
        ))
        .await
        .status,
        400
    );
    assert_eq!(t.s.send(import(json!({ "apply": true }), None)).await.status, 400);
    let doc = t.config["projects"][1].clone();
    let twice =
        t.s.send(import(
            json!({ "config": { "projects": [doc, doc] }, "apply": true }),
            None,
        ))
        .await;
    assert_eq!(twice.status, 400);
    assert_match!(twice.error(), "(?i)doc");

    t.s.call("POST", "/api/projects/site/start").await;
    let res =
        t.s.send(import(
            json!({ "config": { "projects": [doc] }, "apply": true }),
            None,
        ))
        .await;
    assert_eq!(res.status, 409);
    assert_match!(res.error(), "Site");
    assert_eq!(read(t.s.file()), before, "fichier intact");
    t.s.call("POST", "/api/projects/site/stop").await;
    t.s.close(&[]).await;
}

/// JS : transfer.test.js › export et import de la configuration › une grande configuration passe
#[tokio::test]
#[ignore = "phase 5"]
async fn grande_configuration() {
    let t = make_app().await;
    let projects: Vec<Value> = (0..150)
        .map(|i| json!({ "id": format!("page-{i}"), "name": format!("Page {i}"), "type": "file", "cwd": t.site, "open": "index.html" }))
        .collect();
    assert!(serde_json::to_string(&projects).unwrap().len() > 16_000);
    let res =
        t.s.send(import(json!({ "config": { "projects": projects } }), None))
            .await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["added"].as_array().unwrap().len(), 150);
    t.s.close(&[]).await;
}

/// JS : transfer.test.js › export et import de la configuration › permission « commandes » pour exporter comme pour importer
#[tokio::test]
#[ignore = "phase 5"]
async fn permission_commandes() {
    let t = make_app().await;
    let admin = t.s.setup_admin().await;
    let editor = t.s.join(&admin, &["edit"], "ed", "mot de passe 2").await;
    assert_eq!(
        t.s.call_as("GET", "/api/config/export", Some(&editor))
            .await
            .status,
        403
    );
    assert_eq!(
        t.s.send(import(json!({ "config": t.config }), Some(&editor)))
            .await
            .status,
        403
    );
    let commands = t.s.join(&admin, &["commands"], "cmd", "mot de passe 2").await;
    assert_eq!(
        t.s.call_as("GET", "/api/config/export", Some(&commands))
            .await
            .status,
        200
    );
    assert_eq!(
        t.s.send(import(json!({ "config": t.config }), Some(&commands)))
            .await
            .status,
        200
    );
    t.s.close(&[]).await;
}
