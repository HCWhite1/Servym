//! tests/access.test.js : accès depuis le réseau privé, langue du serveur (partie serveur).

mod common;

use std::sync::Arc;

use common::Servynex;
use serde_json::{Value, json};
use servynex_fixtures::{Req, assert_match, path_str, read_json};
use servynex_server::{Listening, bind_host};

fn doc_config(dir: &std::path::Path, instance: Value) -> Value {
    json!({ "instance": instance, "projects": [{ "id": "doc", "name": "Doc", "type": "file", "cwd": path_str(dir.join("site")), "open": "index.html" }] })
}

async fn make_app(
    instance: Value,
    tweak: impl FnOnce(servynex_server::AppBuilder) -> servynex_server::AppBuilder,
) -> Servynex {
    let dir = servynex_fixtures::temp_dir("servynex-access-");
    std::fs::create_dir_all(dir.path().join("site")).unwrap();
    let config = doc_config(dir.path(), instance);
    Servynex::start_in(dir, config, tweak).await
}

/// JS : access.test.js › accès depuis le réseau privé › adresse d’écoute : le réseau seulement avec la protection par mot de passe
#[test]
#[ignore = "phase 5"]
fn adresse_d_ecoute() {
    assert_eq!(bind_host(&json!({ "access": "lan" }), true), "0.0.0.0");
    assert_eq!(
        bind_host(&json!({ "access": "lan" }), false),
        "127.0.0.1",
        "sans protection : ce PC seulement"
    );
    assert_eq!(bind_host(&json!({ "access": "local" }), true), "127.0.0.1");
    assert_eq!(
        bind_host(&json!({}), true),
        "127.0.0.1",
        "par défaut : ce PC seulement"
    );
}

/// JS : access.test.js › accès depuis le réseau privé › réglage depuis l’interface › refusé tant que la protection n’est pas active
#[tokio::test]
#[ignore = "phase 5"]
async fn reglage_refuse_sans_protection() {
    let s = make_app(json!({ "port": 1 }), |b| b).await;
    let res = s
        .call_json(
            "PUT",
            "/api/instance/access",
            json!({ "access": "lan", "password": "x" }),
            None,
        )
        .await;
    assert_eq!(res.status, 409);
    assert_match!(res.error(), "protection");
    s.close(&[]).await;
}

/// Protection active, un administrateur (alice) et un compte « instance » (bob).
async fn with_admin_and_bob(s: &Servynex) -> (String, String) {
    let admin = s.setup_admin().await;
    let bob = s.join(&admin, &["instance"], "bob", "mot de passe 2").await;
    (admin, bob)
}

/// JS : access.test.js › accès depuis le réseau privé › réglage depuis l’interface › administrateur et mot de passe ; appliqué au prochain démarrage
#[tokio::test]
#[ignore = "phase 5"]
async fn reglage_administrateur_et_mot_de_passe() {
    let s = make_app(json!({ "port": 1 }), |b| b).await;
    let (admin, bob) = with_admin_and_bob(&s).await;
    let put = |body: Value, cookie: &str| s.call_json("PUT", "/api/instance/access", body, Some(cookie));
    assert_eq!(
        put(json!({ "access": "lan", "password": "mot de passe 2" }), &bob)
            .await
            .status,
        403,
        "administrateur seulement"
    );
    assert_eq!(
        put(json!({ "access": "lan", "password": "faux" }), &admin)
            .await
            .status,
        403
    );
    assert_eq!(
        put(
            json!({ "access": "partout", "password": "mot de passe 1" }),
            &admin
        )
        .await
        .status,
        400
    );
    assert_eq!(
        put(json!({ "access": "lan", "password": "mot de passe 1" }), &admin)
            .await
            .status,
        200
    );
    assert_eq!(read_json(s.file())["instance"]["access"], "lan");
    let instance = s.call_as("GET", "/api/projects", Some(&admin)).await.json()["instance"].clone();
    assert_eq!(instance["access"], "lan", "réglage enregistré");
    assert_eq!(
        instance["listening"], "local",
        "toujours sur ce PC jusqu’au redémarrage"
    );
    assert_eq!(instance["viewerLocal"], true);
    let audit = s.audit(Some(&admin)).await;
    assert!(
        Servynex::audit_texts(&audit)
            .iter()
            .any(|t| t.to_lowercase().contains("réseau")),
        "inscrit au journal d’audit"
    );
    s.close(&[]).await;
}

/// JS : access.test.js › accès depuis le réseau privé › réglage depuis l’interface › la protection ne peut pas être retirée tant que l’accès réseau est ouvert
#[tokio::test]
#[ignore = "phase 5"]
async fn protection_gardee_tant_que_le_reseau_est_ouvert() {
    let s = make_app(json!({ "port": 1 }), |b| b).await;
    let (admin, _) = with_admin_and_bob(&s).await;
    s.call_json(
        "PUT",
        "/api/instance/access",
        json!({ "access": "lan", "password": "mot de passe 1" }),
        Some(&admin),
    )
    .await;
    let disable = || {
        s.call_json(
            "POST",
            "/api/auth/disable",
            json!({ "password": "mot de passe 1" }),
            Some(&admin),
        )
    };
    assert_eq!(disable().await.status, 409);
    let local = s
        .call_json(
            "PUT",
            "/api/instance/access",
            json!({ "access": "local" }),
            Some(&admin),
        )
        .await;
    assert_eq!(local.status, 200, "revenir à ce PC seulement : sans mot de passe");
    assert_eq!(disable().await.status, 200);
    s.close(&[]).await;
}

/// JS : access.test.js › accès depuis le réseau privé › adresses acceptées : celles de la machine en accès réseau, jamais un autre nom
#[tokio::test]
#[ignore = "phase 5"]
async fn adresses_acceptees() {
    let names = Arc::new(|| vec!["192.168.1.50".to_string(), "mon-pc".to_string()]);
    let lan = make_app(json!({ "port": 1, "access": "lan" }), |b| {
        b.hostnames(names.clone()).listening(Listening::Lan)
    })
    .await;
    let status = |host: String, port: u16| async move {
        Req::get("/api/auth/status").host(&host).send(port).await.status
    };
    assert_eq!(status(format!("192.168.1.50:{}", lan.port), lan.port).await, 200);
    assert_eq!(status(format!("MON-PC:{}", lan.port), lan.port).await, 200);
    assert_eq!(
        status(format!("evil.example:{}", lan.port), lan.port).await,
        421,
        "DNS rebinding"
    );
    lan.close(&[]).await;
    let local = make_app(json!({ "port": 1 }), |b| b.hostnames(names.clone())).await;
    assert_eq!(
        status(format!("192.168.1.50:{}", local.port), local.port).await,
        421,
        "ce PC seulement : localhost uniquement"
    );
    local.close(&[]).await;
}

/// JS : access.test.js › accès depuis le réseau privé › visiteur distant : rien ne s’ouvre sur la machine de Servynex
#[tokio::test]
#[ignore = "phase 5"]
async fn visiteur_distant_rien_ne_s_ouvre() {
    let s = make_app(json!({ "port": 1 }), |b| b.local_viewer(Arc::new(|_| false))).await;
    assert_eq!(
        s.call("GET", "/api/projects").await.json()["instance"]["viewerLocal"],
        false
    );
    for action in ["folder", "code", "terminal", "open"] {
        let res = s.call("POST", &format!("/api/projects/doc/{action}")).await;
        assert_eq!(res.status, 403, "{action}");
        assert_match!(res.error(), "machine");
    }
    for action in ["folder", "config", "log"] {
        assert_eq!(
            s.call("POST", &format!("/api/instance/{action}")).await.status,
            403,
            "instance/{action}"
        );
    }
    s.close(&[]).await;
}

/// JS : access.test.js › langue du serveur › réglable depuis l’interface : appliquée tout de suite, retirée pour suivre le système
#[tokio::test]
#[ignore = "phase 5"]
async fn langue_reglable_depuis_l_interface() {
    let s = make_app(json!({ "port": 1 }), |b| b).await;
    let put = |lang: Value| s.call_json("PUT", "/api/instance/lang", json!({ "lang": lang }), None);
    assert_eq!(put(json!("de")).await.status, 400);
    assert_eq!(put(json!("en")).await.status, 200);
    assert_eq!(read_json(s.file())["instance"]["lang"], "en");
    let instance = s.call("GET", "/api/projects").await.json()["instance"].clone();
    assert_eq!(instance["lang"], "en");
    assert_eq!(instance["serverLang"], "en", "appliquée sans redémarrage");
    assert_eq!(
        s.call("POST", "/api/projects/absent/start").await.error(),
        "Unknown project",
        "messages du serveur en anglais"
    );
    assert_eq!(put(Value::Null).await.status, 200);
    assert!(read_json(s.file())["instance"].get("lang").is_none());
    let instance = s.call("GET", "/api/projects").await.json()["instance"].clone();
    assert_eq!(instance["lang"], Value::Null);
    assert_eq!(instance["serverLang"], "fr", "de nouveau celle du système");
    s.close(&[]).await;
}

/// JS : access.test.js › langue du serveur › permission « Servynex »
#[tokio::test]
#[ignore = "phase 5"]
async fn langue_permission_servynex() {
    let s = make_app(json!({ "port": 1 }), |b| b).await;
    let admin = s.setup_admin().await;
    let ed = s.join(&admin, &["edit"], "ed", "mot de passe 2").await;
    assert_eq!(
        s.call_json("PUT", "/api/instance/lang", json!({ "lang": "en" }), Some(&ed))
            .await
            .status,
        403
    );
    let inst = s.join(&admin, &["instance"], "inst", "mot de passe 2").await;
    assert_eq!(
        s.call_json("PUT", "/api/instance/lang", json!({ "lang": "fr" }), Some(&inst))
            .await
            .status,
        200
    );
    s.close(&[]).await;
}
