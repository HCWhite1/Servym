//! Aides communes aux tests du serveur : un Servym de test dans un dossier temporaire.

#![allow(dead_code)]

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use servym_core::i18n::Lang;
use servym_core::manager::{ProjectState, Timings};
use servym_fixtures::{Req, Res, TempDir, fixture_exe, path_str, read_json, temp_dir, write};
use servym_server::testing::{FakeAutostart, FakeNotifier, Reply};
use servym_server::{App, AppBuilder, RunningApp};

/// Projet lancé par l'application factice (`servym-fixture <mode>`).
pub fn program(id: &str, name: &str, cwd: &Path, mode: &str, port: u16) -> Value {
    json!({
        "id": id,
        "name": name,
        "cwd": path_str(cwd),
        "cmd": path_str(fixture_exe()),
        "args": [mode],
        "port": port,
        "hostControl": "env",
    })
}

/// Ajoute des champs à un projet.
pub fn with(mut project: Value, extra: Value) -> Value {
    for (k, v) in extra.as_object().unwrap() {
        project[k] = v.clone();
    }
    project
}

/// Cadences courtes des tests (`tickMs: 500`…).
pub fn fast() -> Timings {
    Timings {
        tick: Duration::from_millis(500),
        running_tick: Duration::from_millis(500),
        start_timeout: Duration::from_secs(8),
        ..Timings::default()
    }
}

/// Servym de test.
pub struct Servym {
    pub app: RunningApp,
    pub dir: TempDir,
    pub port: u16,
    pub notifier: Arc<FakeNotifier>,
    pub autostart: Arc<FakeAutostart>,
}

/// Réglages communs : dossier de test, français, faux démarrage avec la session et faux
/// notificateur (qui répond « envoyée »), aucune relecture automatique de projects.json.
pub fn builder(dir: &Path, notifier: &Arc<FakeNotifier>, autostart: &Arc<FakeAutostart>) -> AppBuilder {
    App::builder()
        .data_dir(dir)
        .system_lang(Lang::Fr)
        .notifier(notifier.clone())
        .autostart(autostart.clone())
        .watch(Duration::ZERO)
        // Machine avec écran, comme un PC de bureau (la CI Linux n'en a pas) ; les tests du
        // serveur sans écran le demandent eux-mêmes.
        .desktop(true)
}

impl Servym {
    /// Écrit `projects.json`, démarre le serveur ; `tweak` complète les réglages.
    pub async fn start(prefix: &str, config: Value, tweak: impl FnOnce(AppBuilder) -> AppBuilder) -> Servym {
        let dir = temp_dir(prefix);
        Self::start_in(dir, config, tweak).await
    }

    pub async fn start_in(
        dir: TempDir,
        config: Value,
        tweak: impl FnOnce(AppBuilder) -> AppBuilder,
    ) -> Servym {
        write(dir.path().join("projects.json"), config.to_string());
        let notifier = FakeNotifier::new(Reply::Sent);
        let autostart = Arc::new(FakeAutostart::default());
        let app = tweak(builder(dir.path(), &notifier, &autostart))
            .listen("127.0.0.1:0")
            .await
            .unwrap();
        let port = app.port;
        Servym {
            app,
            dir,
            port,
            notifier,
            autostart,
        }
    }

    /// Comme `start`, puis initialise le gestionnaire.
    pub async fn init(prefix: &str, config: Value, tweak: impl FnOnce(AppBuilder) -> AppBuilder) -> Servym {
        let s = Self::start(prefix, config, tweak).await;
        s.app.manager.init().await;
        s
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn file(&self) -> PathBuf {
        self.path().join("projects.json")
    }

    /// `projects.json` tel qu'il est sur le disque.
    pub fn disk(&self) -> Value {
        read_json(self.file())
    }

    pub fn state(&self, id: &str) -> ProjectState {
        self.app.manager.state(id)
    }

    // Les requêtes sont construites tout de suite : le futur rendu n'emprunte rien.

    /// Requête vers ce Servym.
    pub fn send(&self, req: Req) -> impl Future<Output = Res> + use<> {
        let port = self.port;
        async move { req.send(port).await }
    }

    /// Requête de l'interface (avec `X-Servym-Request`).
    pub fn call(&self, method: &str, path: &str) -> impl Future<Output = Res> + use<> {
        self.send(Req::new(method, path).servym())
    }

    /// Requête de l'interface avec un corps JSON et un cookie éventuel.
    pub fn call_json(
        &self,
        method: &str,
        path: &str,
        body: Value,
        cookie: Option<&str>,
    ) -> impl Future<Output = Res> + use<> {
        self.send(Req::new(method, path).servym().cookie(cookie).json(&body))
    }

    /// Requête de l'interface avec un cookie, sans corps.
    pub fn call_as(
        &self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
    ) -> impl Future<Output = Res> + use<> {
        self.send(Req::new(method, path).servym().cookie(cookie))
    }

    /// Entrées du journal d'audit (textes).
    pub async fn audit(&self, cookie: Option<&str>) -> Vec<Value> {
        let history = self.call_as("GET", "/api/history", cookie).await.json();
        history["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "audit")
            .cloned()
            .collect()
    }

    pub fn audit_texts(entries: &[Value]) -> Vec<String> {
        entries
            .iter()
            .map(|e| e["text"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// Active la protection : compte administrateur « alice », rend son cookie.
    pub async fn setup_admin(&self) -> String {
        let res = self
            .call_json(
                "POST",
                "/api/auth/setup",
                json!({ "username": "alice", "password": "mot de passe 1" }),
                None,
            )
            .await;
        res.cookie("servym_session").expect("session de l'administrateur")
    }

    /// Invite un compte avec ces permissions ; rend son cookie.
    pub async fn join(&self, admin: &str, permissions: &[&str], username: &str, password: &str) -> String {
        let invite = self
            .call_json(
                "POST",
                "/api/auth/invites",
                json!({ "permissions": permissions }),
                Some(admin),
            )
            .await
            .json();
        let token = invite["token"].as_str().expect("jeton d'invitation").to_string();
        let res = self
            .call_json(
                "POST",
                "/api/auth/invite",
                json!({ "token": token, "username": username, "password": password }),
                None,
            )
            .await;
        res.cookie("servym_session").expect("session du compte invité")
    }

    /// Arrête les projets encore lancés, puis ferme le serveur.
    pub async fn close(self, running: &[&str]) {
        for id in running {
            if self.state(id).managed {
                let _ = self.app.manager.stop(id).await;
            }
        }
        self.app.close().await;
    }
}

/// Attend qu'un projet vérifie une condition (20 s au plus, vérifiée toutes les 100 ms).
pub async fn until_state(
    manager: &servym_core::manager::Manager,
    id: &str,
    check: impl Fn(&ProjectState) -> bool,
) {
    until_state_for(manager, id, check, 20_000).await;
}

pub async fn until_state_for(
    manager: &servym_core::manager::Manager,
    id: &str,
    check: impl Fn(&ProjectState) -> bool,
    timeout_ms: u64,
) {
    let end = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let state = manager.state(id);
        if check(&state) {
            return;
        }
        assert!(
            std::time::Instant::now() < end,
            "délai dépassé : {id} est {state:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Serveur factice minimal pour occuper un port.
pub async fn block_port(port: u16) -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap()
}
