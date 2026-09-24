//! Partie « serveur » de tests/notify.test.js : notifications système.

mod common;

use common::{Servynex, program, until_state, with};
use serde_json::json;
use servynex_core::manager::State;
use servynex_fixtures::{assert_match, free_port, read, read_json, sleep, until};
use servynex_server::testing::Reply;

/// Un projet qui plante et un qui tourne ; réponse du faux notificateur au choix.
async fn make_app(reply: Reply) -> Servynex {
    let dir = servynex_fixtures::temp_dir("servynex-notify-");
    let app = dir.path().join("app");
    std::fs::create_dir_all(&app).unwrap();
    let config = json!({ "instance": { "port": 1 }, "projects": [
        program("crash", "Mon API", &app, "crash", free_port()),
        with(program("ok", "Factice", &app, "server", free_port()), json!({ "health": { "expect": "Factice" } })),
    ] });
    let s = Servynex::start_in(dir, config, |b| b.timings(common::fast())).await;
    *s.notifier.reply.lock().unwrap() = reply;
    s.app.manager.init().await;
    s
}

/// JS : notify.test.js › notifications système › désactivées par défaut ; activées, un plantage prévient, un arrêt demandé non
#[tokio::test]
async fn desactivees_par_defaut_puis_activees() {
    let s = make_app(Reply::Sent).await;
    assert_eq!(
        s.call("GET", "/api/projects").await.json()["instance"]["notify"],
        false
    );
    s.call("POST", "/api/projects/crash/start").await;
    until_state(&s.app.manager, "crash", |p| p.state == State::Failed).await;
    sleep(300).await;
    assert!(s.notifier.sent().is_empty(), "désactivées par défaut");

    assert_eq!(
        s.call_json("PUT", "/api/instance/notify", json!({ "notify": "oui" }), None)
            .await
            .status,
        400
    );
    assert_eq!(
        s.call_json("PUT", "/api/instance/notify", json!({ "notify": true }), None)
            .await
            .status,
        200
    );
    assert_eq!(read_json(s.file())["instance"]["notify"], true);
    assert_eq!(
        s.call("GET", "/api/projects").await.json()["instance"]["notify"],
        true
    );

    assert_eq!(s.call("POST", "/api/projects/crash/start").await.status, 200);
    until(|| async { s.notifier.sent().len() == 1 }, 10_000).await;
    let sent = s.notifier.sent();
    assert_match!(sent[0].title, "Mon API");
    assert_eq!(sent[0].body, "Error: base introuvable");
    assert_eq!(sent[0].source.as_deref(), Some("Servynex"));

    s.call("POST", "/api/projects/ok/start").await;
    until_state(&s.app.manager, "ok", |p| p.state == State::Running).await;
    s.call("POST", "/api/projects/ok/stop").await;
    until_state(&s.app.manager, "ok", |p| p.state == State::Stopped).await;
    sleep(300).await;
    assert_eq!(s.notifier.sent().len(), 1, "arrêt demandé : pas de notification");

    assert_eq!(
        s.call_json("PUT", "/api/instance/notify", json!({ "notify": false }), None)
            .await
            .status,
        200
    );
    assert!(read_json(s.file())["instance"].get("notify").is_none());
    let audit = Servynex::audit_texts(&s.audit(None).await);
    assert!(
        audit.iter().any(|t| t.to_lowercase().contains("notification")),
        "inscrit au journal d’audit"
    );
    s.close(&["ok"]).await;
}

/// JS : notify.test.js › notifications système › essai : envoyé même désactivées ; indisponible signalé
#[tokio::test]
async fn essai_envoye_meme_desactivees() {
    let s = make_app(Reply::Sent).await;
    let res = s.call("POST", "/api/instance/notify-test").await;
    assert_eq!(res.status, 200);
    assert_eq!(res.json()["sent"], true);
    assert_eq!(s.notifier.sent().len(), 1);
    assert_match!(s.notifier.sent()[0].title, "Servynex");
    *s.notifier.reply.lock().unwrap() = Reply::Unsupported;
    assert_eq!(
        s.call("POST", "/api/instance/notify-test").await.json()["sent"],
        false
    );
    s.close(&[]).await;
}

/// JS : notify.test.js › notifications système › notificateur en échec : consigné, sans effet sur Servynex
#[tokio::test]
async fn notificateur_en_echec() {
    let s = make_app(Reply::Fail("toast refusé".into())).await;
    s.call_json("PUT", "/api/instance/notify", json!({ "notify": true }), None)
        .await;
    s.call("POST", "/api/projects/crash/start").await;
    until_state(&s.app.manager, "crash", |p| p.state == State::Failed).await;
    let own = s.path().join("logs").join("servynex.log");
    until(
        || {
            let own = own.clone();
            async move { own.exists() && read(&own).contains("toast refusé") }
        },
        10_000,
    )
    .await;
    assert_eq!(s.call("GET", "/api/projects").await.status, 200);
    let res = s.call("POST", "/api/instance/notify-test").await;
    assert_eq!(res.status, 502);
    assert_match!(res.error(), "toast refusé");
    s.close(&[]).await;
}

/// JS : notify.test.js › notifications système › permission « Servynex »
#[tokio::test]
async fn permission_servynex() {
    let s = make_app(Reply::Sent).await;
    let admin = s.setup_admin().await;
    let editor = s.join(&admin, &["edit"], "ed", "mot de passe 2").await;
    assert_eq!(
        s.call_json(
            "PUT",
            "/api/instance/notify",
            json!({ "notify": true }),
            Some(&editor)
        )
        .await
        .status,
        403
    );
    assert_eq!(
        s.call_as("POST", "/api/instance/notify-test", Some(&editor))
            .await
            .status,
        403
    );
    let inst = s.join(&admin, &["instance"], "inst", "mot de passe 2").await;
    assert_eq!(
        s.call_json(
            "PUT",
            "/api/instance/notify",
            json!({ "notify": true }),
            Some(&inst)
        )
        .await
        .status,
        200
    );
    s.close(&[]).await;
}
