//! Partie « serveur » de tests/linux.test.js.

mod common;

use common::Servynex;
use serde_json::json;
use servynex_fixtures::{assert_match, path_str};
use servynex_server::is_supervised;

async fn make_app(desktop: Option<bool>) -> Servynex {
    let dir = servynex_fixtures::temp_dir("servynex-linux-");
    let config = json!({ "instance": { "port": 1 }, "projects": [{ "id": "doc", "name": "Doc", "type": "file", "cwd": path_str(dir.path()), "open": "index.html" }] });
    Servynex::start_in(dir, config, |b| {
        let b = b.platform_name("linux");
        match desktop {
            Some(d) => b.desktop(d),
            None => b,
        }
    })
    .await
}

/// JS : linux.test.js › Linux : démarrage avec la session (systemd) › superviseur reconnu : Windows (vbs) ou systemd
#[test]
#[ignore = "phase 5"]
fn superviseur_reconnu() {
    assert!(
        is_supervised(Some("servynex")),
        "mode --supervised (remplace le VBS)"
    );
    assert!(is_supervised(Some("systemd")));
    assert!(!is_supervised(None));
}

/// JS : linux.test.js › Linux : serveur › système indiqué à la page
#[tokio::test]
#[ignore = "phase 5"]
async fn systeme_indique_a_la_page() {
    let s = make_app(None).await;
    assert_eq!(
        s.call("GET", "/api/projects").await.json()["instance"]["platform"],
        "linux"
    );
    s.close(&[]).await;
}

/// JS : linux.test.js › Linux : serveur › sans écran : rien ne s’ouvre, même devant la machine
#[tokio::test]
#[ignore = "phase 5"]
async fn sans_ecran_rien_ne_s_ouvre() {
    let s = make_app(Some(false)).await;
    let instance = s.call("GET", "/api/projects").await.json()["instance"].clone();
    assert_eq!(instance["viewerLocal"], false);
    assert_eq!(
        instance["tools"],
        json!({ "desktop": false, "code": false, "terminal": null }),
        "aucun outil annoncé"
    );
    let res = s.call("POST", "/api/projects/doc/open").await;
    assert_eq!(res.status, 403);
    assert_match!(res.error(), "sans écran");
    s.close(&[]).await;
}
