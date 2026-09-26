//! Partie « serveur » de tests/linux.test.js.

mod common;

use common::Servym;
use serde_json::json;
use servym_fixtures::{assert_match, path_str};
use servym_server::is_supervised;

async fn make_app(desktop: Option<bool>) -> Servym {
    let dir = servym_fixtures::temp_dir("servym-linux-");
    let config = json!({ "instance": { "port": 1 }, "projects": [{ "id": "doc", "name": "Doc", "type": "file", "cwd": path_str(dir.path()), "open": "index.html" }] });
    Servym::start_in(dir, config, |b| {
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
fn superviseur_reconnu() {
    assert!(is_supervised(Some("servym")), "mode --supervised");
    assert!(is_supervised(Some("systemd")));
    assert!(!is_supervised(None));
    assert!(
        !is_supervised(Some("vbs")),
        "superviseur de la version Node, retirée"
    );
}

/// JS : linux.test.js › Linux : serveur › système indiqué à la page
#[tokio::test]
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
