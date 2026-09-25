//! tests/auth.test.js : protection par mot de passe (partie serveur). Chaque test crée les comptes
//! dont il a besoin, au lieu de s'appuyer sur les tests précédents.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use common::Servym;
use serde_json::{Value, json};
use servym_fixtures::{Req, assert_match, path_str, read, read_json, write};

const START: i64 = 1_800_000_000_000;

struct Auth {
    s: Servym,
    now: Arc<AtomicI64>,
}

impl Auth {
    fn advance(&self, ms: i64) {
        self.now.fetch_add(ms, Ordering::SeqCst);
    }
    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
    async fn login(&self, username: &str, password: &str) -> Option<String> {
        self.s
            .call_json(
                "POST",
                "/api/auth/login",
                json!({ "username": username, "password": password }),
                None,
            )
            .await
            .cookie("servym_session")
    }
}

/// Servym de test (deux pages locales) avec une horloge réglable ; protection inactive.
async fn make_app() -> Auth {
    let dir = servym_fixtures::temp_dir("servym-auth-");
    let cwd = path_str(dir.path());
    let config = json!({ "instance": { "port": 1 }, "projects": [
        { "id": "site", "name": "Site", "cwd": cwd, "type": "file", "open": "index.html" },
        { "id": "extra", "name": "Extra", "cwd": cwd, "type": "file", "open": "index.html" },
    ] });
    let now = Arc::new(AtomicI64::new(START));
    let clock = now.clone();
    let s = Servym::start_in(dir, config, |b| {
        b.clock(Arc::new(move || clock.load(Ordering::SeqCst)))
    })
    .await;
    Auth { s, now }
}

/// Protection active : administrateur alice ; rend son cookie et sa clé de secours.
async fn with_admin(a: &Auth) -> (String, String) {
    let res =
        a.s.call_json(
            "POST",
            "/api/auth/setup",
            json!({ "username": "Alice", "name": "Alice M.", "password": "mot de passe 1" }),
            None,
        )
        .await;
    (
        res.cookie("servym_session").unwrap(),
        res.json()["recovery"].as_str().unwrap().to_string(),
    )
}

/// JS : auth.test.js › authentification › désactivée par défaut : tout est ouvert
#[tokio::test]
async fn desactivee_par_defaut() {
    let a = make_app().await;
    assert_eq!(a.s.call("GET", "/api/projects").await.status, 200);
    let status = a.s.call("GET", "/api/auth/status").await.json();
    assert_eq!(status["enabled"], false);
    assert_eq!(status["hasAccounts"], false);
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › activation : compte administrateur, clé de secours, session
#[tokio::test]
async fn activation() {
    let a = make_app().await;
    let setup = |body: Value| a.s.call_json("POST", "/api/auth/setup", body, None);
    assert_eq!(
        setup(json!({ "username": "alice", "password": "court" }))
            .await
            .status,
        400
    );
    assert_eq!(
        setup(json!({ "username": "A B", "password": "assez long" }))
            .await
            .status,
        400
    );
    let res = setup(json!({ "username": "Alice", "name": "Alice M.", "password": "mot de passe 1" })).await;
    assert_eq!(res.status, 201);
    let recovery = res.json()["recovery"].as_str().unwrap().to_string();
    assert_match!(recovery, "^([A-Z2-9]{4}-){4}[A-Z2-9]{4}$");
    assert_match!(
        res.set_cookie("servym_session").unwrap_or_default(),
        "^servym_session=[^;]+; .*HttpOnly; SameSite=Strict"
    );
    assert_eq!(res.json()["user"]["username"], "alice");
    assert_eq!(res.json()["user"]["role"], "admin");
    let stored = read(a.s.path().join("runtime").join("auth.json"));
    assert!(
        !stored.contains("mot de passe 1") && !stored.contains(&recovery),
        "rien en clair"
    );
    assert_eq!(
        setup(json!({ "username": "autre", "password": "mot de passe 2" }))
            .await
            .status,
        409
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › sans session : seule la page de connexion répond
#[tokio::test]
async fn sans_session() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let res = a.s.call("GET", "/api/projects").await;
    assert_eq!(res.status, 401);
    assert_eq!(res.json()["auth"], "login");
    assert_eq!(a.s.call("GET", "/api/events").await.status, 401);
    assert_eq!(a.s.call("POST", "/api/projects/site/start").await.status, 401);
    assert_eq!(
        Req::get("/").send(a.s.port).await.status,
        200,
        "l'interface elle-même reste servie"
    );
    assert_eq!(
        a.s.call_as("GET", "/api/projects", Some(&admin)).await.status,
        200
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › connexion, erreurs et limite des essais
#[tokio::test]
async fn connexion_erreurs_limite() {
    let a = make_app().await;
    with_admin(&a).await;
    let login = |body: Value| a.s.call_json("POST", "/api/auth/login", body, None);
    assert_eq!(
        login(json!({ "username": "alice", "password": "faux" }))
            .await
            .status,
        401
    );
    let ok = login(json!({ "username": "ALICE", "password": "mot de passe 1", "remember": true })).await;
    assert_eq!(ok.status, 200);
    assert_match!(
        ok.set_cookie("servym_session").unwrap_or_default(),
        "Max-Age=2592000",
        "rester connecté : 30 jours"
    );
    for _ in 0..5 {
        login(json!({ "username": "zoe", "password": "faux" })).await;
    }
    let blocked = login(json!({ "username": "zoe", "password": "faux" })).await;
    assert_eq!(blocked.status, 429);
    assert_match!(blocked.error(), "Trop d’essais");
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › invitations et permissions
#[tokio::test]
async fn invitations_et_permissions() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let s = &a.s;
    let invites = |permissions: Value| {
        s.call_json(
            "POST",
            "/api/auth/invites",
            json!({ "permissions": permissions }),
            Some(&admin),
        )
    };
    let invite = |permissions: Value| {
        let res = invites(permissions);
        async move { res.await.json()["token"].as_str().unwrap().to_string() }
    };
    let join = |token: String, username: &str| {
        s.call_json(
            "POST",
            "/api/auth/invite",
            json!({ "token": token, "username": username, "password": "mot de passe 3" }),
            None,
        )
    };

    let t1 = invite(json!(["operate"])).await;
    assert_eq!(
        s.call("GET", &format!("/api/auth/invite/{t1}")).await.json()["role"],
        "operator"
    );
    let op = join(t1.clone(), "op").await.cookie("servym_session").unwrap();
    assert_eq!(
        join(t1, "op2").await.status,
        410,
        "une invitation ne sert qu’une fois"
    );
    assert_eq!(
        s.call_as("POST", "/api/projects/site/start", Some(&op))
            .await
            .status,
        409,
        "autorisé, mais un fichier ne se lance pas"
    );
    assert_eq!(
        s.call_json("PATCH", "/api/projects/site", json!({ "desc": "x" }), Some(&op))
            .await
            .status,
        403
    );
    assert_eq!(s.call_as("GET", "/api/auth/users", Some(&op)).await.status, 403);

    let ed = join(invite(json!(["edit"])).await, "ed")
        .await
        .cookie("servym_session")
        .unwrap();
    assert_eq!(
        s.call_json(
            "PATCH",
            "/api/projects/site",
            json!({ "desc": "modifié" }),
            Some(&ed)
        )
        .await
        .status,
        200
    );
    assert_eq!(
        s.call_json(
            "PATCH",
            "/api/projects/site",
            json!({ "cwd": "C:/Windows" }),
            Some(&ed)
        )
        .await
        .status,
        403,
        "dossier : permission Commandes"
    );
    let new_page = json!({ "name": "X", "cwd": path_str(s.path()), "type": "file", "open": "a.html" });
    assert_eq!(
        s.call_json("POST", "/api/projects", new_page, Some(&ed))
            .await
            .status,
        403
    );
    assert_eq!(
        s.call_as("POST", "/api/restart", Some(&ed)).await.status,
        403,
        "redémarrer Servym : permission Servym"
    );

    let ro = join(invite(json!([])).await, "lecteur")
        .await
        .cookie("servym_session")
        .unwrap();
    assert_eq!(s.call_as("GET", "/api/projects", Some(&ro)).await.status, 200);
    assert_eq!(
        s.call_as("POST", "/api/projects/site/start", Some(&ro))
            .await
            .status,
        403
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › journal d'audit : qui a fait quoi
#[tokio::test]
async fn journal_d_audit() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let ed = a.s.join(&admin, &["edit"], "ed", "mot de passe 3").await;
    a.s.call_json(
        "PATCH",
        "/api/projects/site",
        json!({ "desc": "modifié" }),
        Some(&ed),
    )
    .await;
    a.login("alice", "faux").await;
    let audit = a.s.audit(Some(&admin)).await;
    let text = |e: &Value| e["text"].as_str().unwrap_or_default().to_string();
    assert!(
        audit
            .iter()
            .any(|e| e["user"] == "ed" && e["id"] == "site" && text(e).contains("Réglages modifiés : desc"))
    );
    assert!(
        audit
            .iter()
            .any(|e| e["user"] == "alice" && text(e).contains("Protection par mot de passe activée"))
    );
    assert!(audit.iter().any(|e| text(e).contains("Échec de connexion")));
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › verrouillage après inactivité, commun à tous
#[tokio::test]
async fn verrouillage_apres_inactivite() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let s = &a.s;
    let settings = |lock: i64| {
        s.call_json(
            "PATCH",
            "/api/auth/settings",
            json!({ "lockAfter": lock }),
            Some(&admin),
        )
    };
    let projects = || s.call_as("GET", "/api/projects", Some(&admin));
    let unlock = |password: &'static str| {
        s.call_json(
            "POST",
            "/api/auth/unlock",
            json!({ "password": password }),
            Some(&admin),
        )
    };
    assert_eq!(settings(45).await.status, 400);
    assert_eq!(settings(120).await.status, 200);
    a.advance(100_000);
    assert_eq!(
        s.call_as("POST", "/api/auth/activity", Some(&admin)).await.status,
        200
    );
    a.advance(100_000);
    assert_eq!(projects().await.status, 200, "activité il y a 100 s");
    a.advance(140_000);
    let locked = projects().await;
    assert_eq!(locked.status, 401);
    assert_eq!(locked.json()["auth"], "locked");
    assert_eq!(unlock("faux").await.status, 401);
    assert_eq!(unlock("mot de passe 1").await.status, 200);
    assert_eq!(projects().await.status, 200);
    assert_eq!(
        s.call_as("POST", "/api/auth/lock", Some(&admin)).await.status,
        200,
        "verrouiller tout de suite"
    );
    assert_eq!(projects().await.json()["auth"], "locked");
    unlock("mot de passe 1").await;
    settings(0).await;
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › dernier administrateur protégé
#[tokio::test]
async fn dernier_administrateur_protege() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    a.s.join(&admin, &["edit"], "ed", "mot de passe 3").await;
    let users = a.s.call_as("GET", "/api/auth/users", Some(&admin)).await.json()["users"].clone();
    let find = |name: &str| {
        users
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == name)
            .cloned()
            .unwrap()
    };
    let me = find("alice")["id"].as_str().unwrap().to_string();
    assert_eq!(
        a.s.call_as("DELETE", &format!("/api/auth/users/{me}"), Some(&admin))
            .await
            .status,
        409
    );
    assert_eq!(
        a.s.call_json(
            "PATCH",
            &format!("/api/auth/users/{me}"),
            json!({ "permissions": ["operate"] }),
            Some(&admin)
        )
        .await
        .status,
        409
    );
    let ed = find("ed")["id"].as_str().unwrap().to_string();
    let res =
        a.s.call_json(
            "PATCH",
            &format!("/api/auth/users/{ed}"),
            json!({ "permissions": ["commands", "instance"] }),
            Some(&admin),
        )
        .await;
    assert_eq!(
        res.json()["user"]["permissions"],
        json!(["view", "operate", "edit", "commands", "instance"])
    );
    assert_eq!(res.json()["user"]["role"], "custom");
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › réinitialisation par code (npm run auth) : sessions fermées
#[tokio::test]
async fn reinitialisation_par_code() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    a.s.join(&admin, &["operate"], "op", "mot de passe 3").await;
    let old = a.login("op", "mot de passe 3").await;
    let code = a.s.app.create_reset_code("op", a.now());
    let reset = |code: String, password: &'static str| {
        a.s.call_json(
            "POST",
            "/api/auth/reset",
            json!({ "username": "op", "code": code, "password": password }),
            None,
        )
    };
    assert_eq!(reset("AAAA-AAAA".into(), "nouveau mdp 1").await.status, 401);
    assert_eq!(reset(code.to_lowercase(), "nouveau mdp 1").await.status, 200);
    assert_eq!(
        a.s.call_as("GET", "/api/projects", old.as_deref()).await.status,
        401,
        "ancienne session fermée"
    );
    assert_eq!(
        reset(code, "encore autre 1").await.status,
        401,
        "code à usage unique"
    );
    a.advance(16 * 60_000);
    let late = a.s.app.create_reset_code("op", a.now() - 16 * 60_000);
    assert_eq!(
        reset(late, "encore autre 1").await.status,
        401,
        "expiré après 15 min"
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › clé de secours : une fois, puis une nouvelle
#[tokio::test]
async fn cle_de_secours() {
    let a = make_app().await;
    let (_, recovery) = with_admin(&a).await;
    let recover = |key: &str, password: &str| {
        a.s.call_json(
            "POST",
            "/api/auth/recover",
            json!({ "username": "alice", "key": key, "password": password }),
            None,
        )
    };
    let res = recover(&recovery, "mot de passe 2").await;
    assert_eq!(res.status, 200);
    assert_ne!(res.json()["recovery"].as_str().unwrap(), recovery);
    assert!(res.cookie("servym_session").is_some());
    assert_eq!(recover(&recovery, "mot de passe 3").await.status, 401);
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › mon compte : mot de passe, sessions, préférences
#[tokio::test]
async fn mon_compte() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let s = &a.s;
    let other = a.login("alice", "mot de passe 1").await;
    let sessions = s
        .call_as("GET", "/api/auth/me/sessions", Some(&admin))
        .await
        .json()["sessions"]
        .clone();
    let sessions = sessions.as_array().unwrap();
    assert!(sessions.len() >= 2 && sessions.iter().any(|x| x["current"] == true));
    let password = |current: &str| {
        s.call_json(
            "POST",
            "/api/auth/me/password",
            json!({ "current": current, "password": "mot de passe 4" }),
            Some(&admin),
        )
    };
    assert_eq!(password("faux").await.status, 401);
    let changed = password("mot de passe 1").await;
    assert!(changed.json()["closed"].as_u64().unwrap() >= 1);
    assert_eq!(
        s.call_as("GET", "/api/projects", other.as_deref()).await.status,
        401,
        "autres sessions fermées"
    );
    assert_eq!(
        s.call_as("GET", "/api/projects", Some(&admin)).await.status,
        200,
        "la session courante reste"
    );
    let prefs = s
        .call_json(
            "PATCH",
            "/api/auth/me",
            json!({ "prefs": { "theme": "solarized-dark" } }),
            Some(&admin),
        )
        .await;
    assert_eq!(prefs.json()["user"]["prefs"]["theme"], "solarized-dark");
    assert_eq!(
        s.call_as("POST", "/api/instance/onboarded", Some(&admin))
            .await
            .status,
        200
    );
    assert_eq!(
        s.call_as("GET", "/api/projects", Some(&admin)).await.json()["instance"]["onboarded"],
        true,
        "présentation vue : par compte"
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › nom de l'instance
#[tokio::test]
async fn nom_de_l_instance() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    assert_eq!(
        a.s.call_json(
            "PATCH",
            "/api/instance",
            json!({ "name": "Serveur maison" }),
            Some(&admin)
        )
        .await
        .status,
        200
    );
    assert_eq!(
        a.s.call_as("GET", "/api/projects", Some(&admin)).await.json()["instance"]["name"],
        "Serveur maison"
    );
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › supprimer : permission « Commandes » et mot de passe ; réordonner : permission « Modifier »
#[tokio::test]
async fn supprimer_et_reordonner_permissions() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let s = &a.s;
    let pilote = s.join(&admin, &["operate"], "pilote", "mot de passe 5").await;
    let editeur = s.join(&admin, &["edit"], "editeur", "mot de passe 5").await;
    let commandes = s.join(&admin, &["commands"], "commandes", "mot de passe 5").await;
    let ids = || {
        read_json(s.file())["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let order = json!({ "order": ["extra", "site"] });

    assert_eq!(
        s.call_json("PUT", "/api/projects", order.clone(), Some(&pilote))
            .await
            .status,
        403
    );
    assert_eq!(
        s.call_json("PUT", "/api/projects", order, Some(&editeur))
            .await
            .status,
        200
    );
    assert_eq!(ids(), ["extra", "site"]);

    let delete = |body: Value, cookie: &str| s.call_json("DELETE", "/api/projects/extra", body, Some(cookie));
    assert_eq!(
        delete(json!({ "password": "mot de passe 5" }), &editeur)
            .await
            .status,
        403,
        "permission Commandes"
    );
    assert_eq!(
        delete(json!({ "confirm": "Extra" }), &commandes).await.status,
        403,
        "protection active : le nom ne suffit pas"
    );
    assert_eq!(
        delete(json!({ "password": "faux" }), &commandes).await.status,
        403
    );
    assert_eq!(ids(), ["extra", "site"], "rien de supprimé");
    assert_eq!(
        delete(json!({ "password": "mot de passe 5" }), &commandes)
            .await
            .status,
        200
    );
    assert_eq!(ids(), ["site"]);
    let audit = s.audit(Some(&admin)).await;
    assert!(audit.iter().any(|e| e["user"] == "commandes"
        && e["id"] == "extra"
        && e["text"].as_str().unwrap().contains("Projet supprimé")));
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › désactiver puis réactiver : comptes conservés
#[tokio::test]
async fn desactiver_puis_reactiver() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    let s = &a.s;
    s.join(&admin, &["edit"], "ed", "mot de passe 3").await;
    let disable = |password: &str| {
        s.call_json(
            "POST",
            "/api/auth/disable",
            json!({ "password": password }),
            Some(&admin),
        )
    };
    assert_eq!(disable("faux").await.status, 401);
    assert_eq!(disable("mot de passe 1").await.status, 200);
    assert_eq!(
        s.call("GET", "/api/projects").await.status,
        200,
        "de nouveau ouvert"
    );
    let enable = |username: &str, password: &str| {
        s.call_json(
            "POST",
            "/api/auth/enable",
            json!({ "username": username, "password": password }),
            None,
        )
    };
    assert_eq!(
        enable("ed", "mot de passe 3").await.status,
        401,
        "un administrateur seulement"
    );
    assert_eq!(enable("alice", "mot de passe 1").await.status, 200);
    assert_eq!(s.call("GET", "/api/projects").await.status, 401);
    a.s.close(&[]).await;
}

/// JS : auth.test.js › authentification › fichier des comptes illisible : tout est refusé
#[tokio::test]
async fn fichier_des_comptes_illisible() {
    let a = make_app().await;
    let (admin, _) = with_admin(&a).await;
    write(a.s.path().join("runtime").join("auth.json"), "{ cassé");
    let res = a.s.call_as("GET", "/api/projects", Some(&admin)).await;
    assert_eq!(res.status, 503);
    assert_match!(res.error(), "illisible");
    a.s.close(&[]).await;
}
