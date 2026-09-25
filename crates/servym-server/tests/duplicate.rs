//! Partie « API » de tests/duplicate.test.js.

mod common;

use common::Servym;
use serde_json::json;
use servym_fixtures::{assert_match, path_str};

async fn make_app() -> Servym {
    let dir = servym_fixtures::temp_dir("servym-dup-");
    let cwd = path_str(dir.path());
    let config = json!({
        "instance": { "port": 1 },
        "projects": [
            { "id": "api", "name": "API", "cwd": cwd, "cmd": "node", "args": ["server.js"], "port": 39881, "hostControl": "env" },
            { "id": "doc", "name": "Doc", "type": "file", "cwd": cwd, "open": "index.html" },
        ],
    });
    Servym::start_in(dir, config, |b| b).await
}

/// JS : duplicate.test.js › dupliquer : API › copie créée, avec un port libre, inscrite à l’activité
#[tokio::test]
async fn copie_creee_avec_un_port_libre() {
    let s = make_app().await;
    let res = s.call("POST", "/api/projects/api/duplicate").await;
    assert_eq!(res.status, 201);
    assert_eq!(res.json()["id"], "api-2");
    assert_eq!(res.json()["name"], "API (copie)");
    assert!(res.json()["port"].as_u64().unwrap() > 39881, "port suivant libre");
    let ids: Vec<String> = s.disk()["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(ids, ["api", "api-2", "doc"]);
    assert_match!(
        Servym::audit_texts(&s.audit(None).await).join("\n"),
        "Dupliqué depuis API"
    );
    assert!(
        s.call("POST", "/api/projects/doc/duplicate")
            .await
            .json()
            .get("port")
            .is_none()
    );
    assert_eq!(s.call("POST", "/api/projects/zz/duplicate").await.status, 404);
    s.close(&[]).await;
}

/// JS : duplicate.test.js › dupliquer : API › permission « commandes »
#[tokio::test]
async fn permission_commandes() {
    let s = make_app().await;
    let admin = s.setup_admin().await;
    let ed = s.join(&admin, &["edit"], "ed", "mot de passe 2").await;
    assert_eq!(
        s.call_as("POST", "/api/projects/api/duplicate", Some(&ed))
            .await
            .status,
        403
    );
    let cmd = s.join(&admin, &["commands"], "cmd", "mot de passe 2").await;
    assert_eq!(
        s.call_as("POST", "/api/projects/api/duplicate", Some(&cmd))
            .await
            .status,
        201
    );
    s.close(&[]).await;
}
