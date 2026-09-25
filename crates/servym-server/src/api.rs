//! Routes `/api/*` (hors comptes) : portage de la fonction `api()` de `server.js`.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{Method, header};
use axum::response::Response;
use serde_json::{Value, json};
use servym_core::config::{
    create_project, describe_changes, duplicate_project, patch_project, port_of, remove_project,
    reorder_projects, validate,
};
use servym_core::i18n::{Lang, LocalError};
use servym_core::logs::{Follow, Follower, history, tail_lines};
use servym_core::manager::{Event, State};
use servym_core::params;
use servym_sys::notify::Notification;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::app::{App, read_text};
use crate::http::{Ctx, err, permission_for, send};

/// Champs dont la modification revient à choisir ce qui s'exécute (ou s'expose).
const SENSITIVE: [&str; 7] = ["type", "cwd", "cmd", "args", "env", "stop", "expose"];
/// Ouvrent quelque chose sur cette machine : réservées à qui est devant elle.
const HOST_ONLY: [&str; 9] = [
    "open",
    "folder",
    "code",
    "terminal",
    "config",
    "log",
    "opener",
    "browser",
    "reset-code",
];

fn ok() -> Response {
    send(200, &json!({ "ok": true }))
}

fn raw(text: impl Into<String>) -> LocalError {
    LocalError::new(500, "raw", params![text = text.into()])
}

fn who_of(user: &Option<Value>) -> String {
    user.as_ref()
        .and_then(|u| u["username"].as_str())
        .unwrap_or("-")
        .to_string()
}

impl crate::app::AppInner {
    fn viewer_local(&self, ctx: &Ctx) -> bool {
        self.desktop && (self.local_viewer)(ctx.peer.ip())
    }

    pub(crate) fn host_only(&self, ctx: &Ctx, action: &str) -> Result<(), LocalError> {
        if !HOST_ONLY.contains(&action) {
            return Ok(());
        }
        if !self.desktop {
            return Err(err(403, "http.noDesktop"));
        }
        if !(self.local_viewer)(ctx.peer.ip()) {
            return Err(err(403, "http.hostOnly"));
        }
        Ok(())
    }

    fn read_ui(&self) -> Value {
        read_text(&self.ui_file)
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(json!({}))
    }

    /// Fenêtre (par défaut) ou navigateur.
    pub fn opener(&self) -> &'static str {
        if self.read_ui()["opener"] == "browser" {
            "browser"
        } else {
            "window"
        }
    }

    /// Premier port libre à partir de `from` (ni Servym, ni un projet, ni occupé).
    async fn free_port(&self, from: u32) -> Result<u16, LocalError> {
        let config = self.config();
        let mut taken: Vec<u16> = config["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(port_of)
            .collect();
        taken.extend(port_of(&config["instance"]));
        let start = from.max(1024);
        for port in start..(from + 500).min(65536) {
            let port = port as u16;
            if !taken.contains(&port) && !servym_core::health::port_in_use(port).await {
                return Ok(port);
            }
        }
        Err(err(404, "http.noFreePort"))
    }

    /// Profils réseau, gardés une minute.
    async fn network_profiles(&self) -> Vec<servym_sys::NetworkProfile> {
        let mut cache = self.network.lock().await;
        if let Some((at, profiles)) = cache.as_ref()
            && at.elapsed() < Duration::from_secs(60)
        {
            return profiles.clone();
        }
        let profiles = self.platform.network_profiles().await;
        *cache = Some((Instant::now(), profiles.clone()));
        profiles
    }

    /// Tous les ports en écoute sur la machine, rattachés aux projets quand c'est le cas.
    async fn ports(&self) -> Value {
        let (list, names, profiles) = tokio::join!(
            self.platform.listeners(),
            self.platform.process_names(),
            self.network_profiles()
        );
        let mut rows: Vec<Value> = Vec::new();
        for l in list {
            let pid = l.pid.map_or(Value::Null, |p| json!(p));
            match rows.iter_mut().find(|r| r["port"] == l.port && r["pid"] == pid) {
                Some(row) => {
                    let addresses = row["addresses"].as_array_mut().unwrap();
                    if !addresses.iter().any(|a| a == l.address.as_str()) {
                        addresses.push(json!(l.address));
                    }
                }
                None => rows.push(json!({
                    "port": l.port, "pid": pid, "process": l.pid.and_then(|p| names.get(&p).cloned()), "addresses": [l.address],
                })),
            }
        }
        // Rattachement par port, puis par PID : les autres ports ouverts par le même processus (un
        // second serveur HTTPS, par exemple) appartiennent au projet.
        let projects: Vec<Value> = self
            .manager
            .snapshot()
            .into_iter()
            .filter(|p| !p["port"].is_null())
            .collect();
        let mut by_pid: BTreeMap<u64, Value> = BTreeMap::new();
        for r in &mut rows {
            let found = projects
                .iter()
                .find(|p| p["activePort"].as_u64().or_else(|| p["port"].as_u64()) == r["port"].as_u64())
                .map(|p| p["id"].clone());
            r["project"] = found.clone().unwrap_or(Value::Null);
            if let (Some(project), Some(pid)) = (found, r["pid"].as_u64()) {
                by_pid.insert(pid, project);
            }
        }
        for r in &mut rows {
            let exposed = r["addresses"].as_array().unwrap().iter().any(|a| {
                let a = a.as_str().unwrap_or_default();
                !a.starts_with("127.") && a != "[::1]"
            });
            r["exposed"] = json!(exposed);
            if r["project"].is_null()
                && let Some(project) = r["pid"].as_u64().and_then(|p| by_pid.get(&p))
            {
                r["project"] = project.clone();
            }
        }
        rows.sort_by_key(|r| (r["port"].as_u64().unwrap_or(0), r["pid"].as_u64().unwrap_or(0)));
        json!({
            "instance": self.config()["instance"]["port"],
            "lan": self.lan_addresses(),
            "networks": profiles,
            "ports": rows,
        })
    }

    async fn git(&self, project: &Value, fresh: bool) -> Value {
        let id = project["id"].as_str().unwrap_or_default().to_string();
        {
            let cache = self.git_cache.lock().await;
            if !fresh
                && let Some((at, info)) = cache.get(&id)
                && at.elapsed() < Duration::from_secs(20)
            {
                return info.clone();
            }
        }
        let info = git_info(project["cwd"].as_str().unwrap_or_default()).await;
        self.git_cache
            .lock()
            .await
            .insert(id, (Instant::now(), info.clone()));
        info
    }
}

/// État git d'un dossier (branche, avance et retard, fichiers modifiés, dernier commit).
async fn git_info(dir: &str) -> Value {
    let run = servym_sys::run_command();
    let git = |args: &[&str]| {
        let mut all = vec!["-C".to_string(), dir.to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        run("git".into(), all)
    };
    let Ok(status) = git(&["status", "--porcelain=v2", "--branch"]).await else {
        return json!({ "repo": false });
    };
    let mut info =
        json!({ "repo": true, "branch": null, "ahead": 0, "behind": 0, "changes": 0, "upstream": false });
    let mut changes = 0;
    for line in status.lines() {
        if let Some(branch) = line.strip_prefix("# branch.head ") {
            info["branch"] = json!(branch);
        } else if line.starts_with("# branch.upstream ") {
            info["upstream"] = json!(true);
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut parts = ab.split_whitespace();
            let ahead = parts
                .next()
                .and_then(|a| a.trim_start_matches('+').parse::<i64>().ok())
                .unwrap_or(0);
            let behind = parts
                .next()
                .and_then(|b| b.trim_start_matches('-').parse::<i64>().ok())
                .unwrap_or(0);
            info["ahead"] = json!(ahead);
            info["behind"] = json!(behind);
        } else if !line.is_empty() && !line.starts_with('#') {
            changes += 1;
        }
    }
    info["changes"] = json!(changes);
    if let Ok(last) = git(&["log", "-1", "--format=%h%x09%ct%x09%s"]).await {
        let last = last.trim();
        if !last.is_empty() {
            let mut parts = last.splitn(3, '\t');
            let hash = parts.next().unwrap_or_default();
            let time: i64 = parts.next().and_then(|t| t.parse().ok()).unwrap_or(0);
            let subject = parts.next().unwrap_or_default();
            info["last"] = json!({ "hash": hash, "date": time * 1000, "subject": subject });
        }
    }
    info
}

// ---- Flux SSE ----

type Tx = mpsc::Sender<Result<Bytes, Infallible>>;

async fn emit(tx: &Tx, event: &str, data: &Value) -> bool {
    tx.send(Ok(Bytes::from(format!("event: {event}\ndata: {data}\n\n"))))
        .await
        .is_ok()
}

/// Flux ouvert : ping toutes les 25 s, fermé quand la page part ou que la session est révoquée.
fn stream(
    app: &App,
    session: Option<String>,
    run: impl FnOnce(Tx, CancellationToken) -> tokio::task::JoinHandle<()>,
) -> Response {
    let (tx, rx) = mpsc::channel(64);
    let token = CancellationToken::new();
    // Sans session (protection désactivée) : rangé sous « - », fermé seulement à l'arrêt du serveur.
    let key = session.unwrap_or_else(|| "-".into());
    app.streams
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .push(token.clone());
    let ping_tx = tx.clone();
    let ping_token = token.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(25));
        every.tick().await;
        loop {
            tokio::select! {
                _ = ping_token.cancelled() => return,
                _ = every.tick() => if ping_tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() { ping_token.cancel(); return },
            }
        }
    });
    let task = run(tx, token.clone());
    tokio::spawn(async move {
        token.cancelled().await;
        task.abort();
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)))
        .unwrap()
}

fn events_stream(app: &App, session: Option<String>, lang: Lang) -> Response {
    let app2 = app.clone();
    stream(app, session, move |tx, token| {
        tokio::spawn(async move {
            let app = app2;
            let mut events = app.manager.subscribe();
            let mut instance = app.instance_events.subscribe();
            let snapshot = |app: &App| json!(app.manager.snapshot());
            if !emit(&tx, "snapshot", &snapshot(&app)).await
                || !emit(
                    &tx,
                    "instance",
                    &json!({ "configError": app.config_error_in(lang) }),
                )
                .await
            {
                token.cancel();
                return;
            }
            loop {
                let sent = tokio::select! {
                    event = events.recv() => match event {
                        Ok(Event::Change(view)) => emit(&tx, "state", &view).await,
                        Ok(Event::Config) => emit(&tx, "snapshot", &snapshot(&app)).await,
                        Ok(_) => true,
                        // Flux en retard : un instantané complet remplace ce qui a été perdu.
                        Err(broadcast::error::RecvError::Lagged(_)) => emit(&tx, "snapshot", &snapshot(&app)).await,
                        Err(broadcast::error::RecvError::Closed) => false,
                    },
                    changed = instance.recv() => match changed {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => emit(&tx, "instance", &json!({ "configError": app.config_error_in(lang) })).await,
                        Err(broadcast::error::RecvError::Closed) => false,
                    },
                };
                if !sent {
                    token.cancel();
                    return;
                }
            }
        })
    })
}

fn log_stream(app: &App, session: Option<String>, file: std::path::PathBuf) -> Response {
    stream(app, session, move |tx, token| {
        tokio::spawn(async move {
            let (mut follower, first) = Follower::new(&file);
            let mut next = Some(first);
            loop {
                if let Some(event) = next.take() {
                    let sent = match event {
                        Follow::Reset(lines) => emit(&tx, "reset", &json!(lines)).await,
                        Follow::Lines(lines) => emit(&tx, "lines", &json!(lines)).await,
                    };
                    if !sent {
                        token.cancel();
                        return;
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                next = follower.poll();
            }
        })
    })
}

// ---- Routes ----

pub async fn api(app: &App, ctx: &mut Ctx) -> Result<Response, LocalError> {
    let rest = ctx.path.strip_prefix("/api/").unwrap_or_default().to_string();
    let parts: Vec<&str> = rest.split('/').collect();
    let section = parts.first().copied().unwrap_or_default();
    let id = parts.get(1).copied().filter(|s| !s.is_empty());
    let action = parts.get(2).copied().filter(|s| !s.is_empty());
    let method = ctx.method.clone();
    let get = method == Method::GET || method == Method::HEAD;

    // Connexion, comptes, sessions : routes à part, publiques ou non.
    if section == "auth" {
        return app.auth_handle(ctx, &parts[1..]).await;
    }
    // Tout le reste : compte connecté et permission de la route (None : protection désactivée).
    let user = app
        .require(ctx, permission_for(section, id, action, &method))
        .await?;
    let who = who_of(&user);
    let audit = |project: &str, key: &str, params| app.audit(project, &who, key, params);
    let lang = ctx.lang(app.lang());
    let config = app.config();
    let instance = &config["instance"];

    if section == "projects" && id.is_none() && get {
        let tools = if app.desktop {
            serde_json::to_value(app.platform.tools()).unwrap()
        } else {
            json!({ "desktop": false, "code": false, "terminal": null })
        };
        // Présentation vue : par compte si la protection est active, sinon pour Servym.
        let onboarded = match &user {
            Some(u) => u["prefs"]["onboarded"] == true,
            None => app.read_ui()["onboarded"] == true,
        };
        let listening = *app.listening.lock().unwrap_or_else(|e| e.into_inner());
        let info = json!({
            "port": instance["port"],
            "version": env!("CARGO_PKG_VERSION"),
            "node": null,
            "dir": app.data_dir,
            "config": app.config_file,
            "tools": tools,
            "lan": app.lan_addresses(),
            "restartable": app.restart.is_some(),
            "name": instance.get("name").cloned().unwrap_or(Value::Null),
            "onboarded": onboarded,
            "access": instance.get("access").cloned().unwrap_or(json!("local")),
            "listening": if listening == crate::Listening::Lan { "lan" } else { "local" },
            "viewerLocal": app.viewer_local(ctx),
            "platform": app.platform_name,
            "lang": instance.get("lang").cloned().unwrap_or(Value::Null),
            "serverLang": app.lang().code(),
            "notify": instance["notify"] == true,
            "configError": app.config_error_in(lang),
            // Application de bureau : où « Ouvrir Servym » ouvre l'interface (réglage de la machine).
            "app": app.shell.as_ref().map(|_| json!({ "opener": app.opener() })),
        });
        return Ok(send(
            200,
            &json!({ "instance": info, "projects": app.manager.snapshot() }),
        ));
    }
    if section == "projects" && id.is_none() && method == Method::POST {
        let input = ctx.read_json(16_000).await?;
        let (next, created) = create_project(&app.disk_config()?, &input)?;
        app.apply_config(next)?;
        app.log(&format!(
            "[config] {}",
            app.tr("log.added", params![id = created.as_str()])
        ));
        audit(&created, "audit.added", params![]);
        return Ok(send(201, &app.manager.view(&app.manager.project(&created)?)));
    }
    if section == "projects" && id.is_none() && method == Method::PUT {
        let body = ctx.read_json(16_000).await?;
        app.apply_config(reorder_projects(
            &app.disk_config()?,
            body.get("order").unwrap_or(&Value::Null),
        )?)?;
        app.log(&format!("[config] {}", app.tr("log.reordered", params![])));
        audit("-", "audit.reordered", params![]);
        return Ok(ok());
    }
    if section == "history" && id.is_none() && get {
        let entries = history(&[app.own_prev_log.clone(), app.own_log.clone()], None, 300);
        return Ok(send(200, &json!({ "entries": entries })));
    }
    if section == "autostart" && id.is_none() {
        if get {
            return Ok(send(200, &json!({ "enabled": app.autostart.enabled().await })));
        }
        if method == Method::PUT {
            let body = ctx.read_json(16_000).await?;
            let Some(enabled) = body["enabled"].as_bool() else {
                return Err(err(400, "http.badEnabled"));
            };
            app.autostart.set(enabled).await.map_err(raw)?;
            let key = if enabled {
                "log.autostartOn"
            } else {
                "log.autostartOff"
            };
            app.log(&format!("[autostart] {}", app.tr(key, params![])));
            return Ok(send(200, &json!({ "enabled": app.autostart.enabled().await })));
        }
    }
    if section == "instance"
        && method == Method::POST
        && id == Some("onboarded")
        && let Some(u) = &user
    {
        app.set_prefs(&u["id"], json!({ "onboarded": true })).await;
        return Ok(ok());
    }
    if section == "instance"
        && method == Method::POST
        && matches!(id, Some("folder" | "config" | "log" | "onboarded"))
    {
        let id = id.unwrap();
        app.host_only(ctx, id)?;
        // Chemins fixes : rien ne vient de la requête.
        match id {
            "folder" => app.platform.open_folder(&app.data_dir).map_err(raw)?,
            "config" => app.platform.open_in_editor(&app.config_file).map_err(raw)?,
            "log" => app.platform.open_in_editor(&app.own_log).map_err(raw)?,
            _ => {
                let _ = std::fs::create_dir_all(&app.runtime_dir);
                let mut ui = app.read_ui();
                ui["onboarded"] = json!(true);
                std::fs::write(&app.ui_file, ui.to_string()).map_err(|e| raw(e.to_string()))?;
            }
        }
        return Ok(ok());
    }
    // Application de bureau, devant la machine : fenêtre ou navigateur.
    if section == "instance" && matches!(id, Some("opener" | "browser")) {
        let id = id.unwrap();
        let Some(shell) = app.shell.clone() else {
            return Err(err(404, "http.unknownRoute"));
        };
        app.host_only(ctx, id)?;
        if id == "browser" && method == Method::POST {
            // closeWindow : la page qui demande est dans la fenêtre, qui se ferme ensuite.
            let body = ctx.read_json_if_any(16_000).await?;
            shell.open_browser();
            if body["closeWindow"] == true {
                shell.close_window();
            }
            return Ok(ok());
        }
        if id == "opener" && method == Method::PUT {
            let body = ctx.read_json(16_000).await?;
            let Some(opener) = body["opener"]
                .as_str()
                .filter(|o| ["window", "browser"].contains(o))
            else {
                return Err(err(400, "http.badOpener"));
            };
            let _ = std::fs::create_dir_all(&app.runtime_dir);
            let mut ui = app.read_ui();
            ui["opener"] = json!(opener);
            std::fs::write(&app.ui_file, ui.to_string()).map_err(|e| raw(e.to_string()))?;
            return Ok(send(200, &json!({ "opener": opener })));
        }
    }
    if section == "instance" && id == Some("lang") && method == Method::PUT {
        // Langue des journaux, de l'historique et des états : appliquée tout de suite. null :
        // celle du système.
        let body = ctx.read_json(16_000).await?;
        let chosen = &body["lang"];
        if !(chosen.is_null() || matches!(chosen.as_str(), Some("en" | "fr"))) {
            return Err(err(400, "http.badLang"));
        }
        let mut next = app.disk_config()?;
        match chosen.as_str() {
            Some(l) => next["instance"]["lang"] = json!(l),
            None => {
                next["instance"].as_object_mut().unwrap().shift_remove("lang");
            }
        }
        app.apply_config(next)?;
        audit(
            "-",
            "audit.serverLang",
            params![lang = chosen.as_str().unwrap_or("auto")],
        );
        return Ok(send(
            200,
            &json!({ "lang": chosen, "serverLang": app.lang().code() }),
        ));
    }
    if section == "instance" && id == Some("notify") && method == Method::PUT {
        // Notifications du système quand un projet plante : désactivées par défaut.
        let body = ctx.read_json(16_000).await?;
        let Some(enabled) = body["notify"].as_bool() else {
            return Err(err(400, "http.badNotify"));
        };
        let mut next = app.disk_config()?;
        if enabled {
            next["instance"]["notify"] = json!(true);
        } else {
            next["instance"].as_object_mut().unwrap().shift_remove("notify");
        }
        app.apply_config(next)?;
        audit(
            "-",
            if enabled {
                "audit.notifyOn"
            } else {
                "audit.notifyOff"
            },
            params![],
        );
        return Ok(send(200, &json!({ "notify": enabled })));
    }
    if section == "instance" && id == Some("notify-test") && method == Method::POST {
        // Essai depuis les paramètres, même désactivées. sent false : pas sur ce système.
        let notification = Notification {
            title: app.tr("notify.testTitle", params![]),
            body: app.tr("notify.testBody", params![]),
            source: Some(app.source()),
        };
        let sent = app
            .notifier
            .notify(notification)
            .await
            .map_err(|error| LocalError::new(502, "http.notifyFailed", params![error = error]))?;
        return Ok(send(200, &json!({ "sent": sent })));
    }
    if section == "instance" && id == Some("access") && method == Method::PUT {
        // Ouvrir au réseau : protection active et mot de passe redemandé. Revenir à ce PC seulement
        // ne demande rien. Appliqué au prochain démarrage.
        let body = ctx.read_json(16_000).await?;
        let access = body["access"].as_str().unwrap_or_default();
        if access != "local" && access != "lan" {
            return Err(err(400, "http.badAccess"));
        }
        if access == "lan" {
            if !app.auth_enabled().await {
                return Err(err(409, "http.lanNeedsProtection"));
            }
            if let Some(u) = &user {
                app.confirm_password(ctx, u, &body["password"]).await?;
            }
        }
        let mut next = app.disk_config()?;
        if access == "lan" {
            next["instance"]["access"] = json!("lan");
        } else {
            next["instance"].as_object_mut().unwrap().shift_remove("access");
        }
        app.apply_config(next)?;
        audit(
            "-",
            if access == "lan" {
                "audit.accessLan"
            } else {
                "audit.accessLocal"
            },
            params![],
        );
        let listening = *app.listening.lock().unwrap_or_else(|e| e.into_inner());
        return Ok(send(
            200,
            &json!({ "access": access, "listening": if listening == crate::Listening::Lan { "lan" } else { "local" } }),
        ));
    }
    if section == "instance" && id.is_none() && method == Method::PATCH {
        // Nom de l'instance, affiché dans la barre latérale (« Servym · Serveur maison »).
        let body = ctx.read_json(16_000).await?;
        let name = &body["name"];
        let valid = name.is_null()
            || name
                .as_str()
                .is_some_and(|n| n.trim().encode_utf16().count() <= 40);
        if !valid {
            return Err(err(400, "http.nameTooLong"));
        }
        let trimmed = name.as_str().map(str::trim).filter(|n| !n.is_empty());
        let mut next = app.disk_config()?;
        match trimmed {
            Some(n) => next["instance"]["name"] = json!(n),
            None => {
                next["instance"].as_object_mut().unwrap().shift_remove("name");
            }
        }
        app.apply_config(next)?;
        audit(
            "-",
            "audit.instanceName",
            params![name = trimmed.unwrap_or("Servym")],
        );
        return Ok(send(200, &json!({ "name": trimmed })));
    }
    if section == "events" && get {
        let session = app.session_id(ctx).await;
        return Ok(events_stream(app, session, lang));
    }
    if section == "git" && get {
        let fresh = ctx.has_query("fresh");
        let mut out = serde_json::Map::new();
        for p in config["projects"].as_array().into_iter().flatten() {
            out.insert(
                p["id"].as_str().unwrap_or_default().to_string(),
                app.git(p, fresh).await,
            );
        }
        return Ok(send(200, &Value::Object(out)));
    }
    if section == "free-port" && get {
        let from = ctx
            .query("from")
            .and_then(|f| f.parse::<u32>().ok())
            .filter(|f| *f > 0)
            .unwrap_or(3500);
        return Ok(send(200, &json!({ "port": app.free_port(from).await? })));
    }
    if section == "ports" && get {
        return Ok(send(200, &app.ports().await));
    }
    if section == "stats" && get {
        // Racines de chaque projet en marche : le PID lancé par Servym et celui qui écoute sur son
        // port (un projet qui se relance lui-même n'est plus sous le premier).
        let running: Vec<Value> = app
            .manager
            .snapshot()
            .into_iter()
            .filter(|p| {
                p["type"] != "file"
                    && matches!(p["state"].as_str(), Some("running" | "starting" | "stopping"))
            })
            .collect();
        let listening = if running.iter().any(|p| p["type"] != "static") {
            app.platform.listeners().await
        } else {
            Vec::new()
        };
        let mut roots = BTreeMap::new();
        let mut in_process = serde_json::Map::new(); // sites statiques : servis par Servym lui-même
        for p in &running {
            let pid_id = p["id"].as_str().unwrap_or_default().to_string();
            if p["type"] == "static" {
                in_process.insert(pid_id, json!({ "inProcess": true }));
                continue;
            }
            let port = p["activePort"]
                .as_u64()
                .or_else(|| p["port"].as_u64())
                .unwrap_or(0);
            let mut pids: Vec<u32> = Vec::new();
            if p["managed"] == true
                && let Some(pid) = p["pid"].as_u64()
            {
                pids.push(pid as u32);
            }
            pids.extend(
                listening
                    .iter()
                    .filter(|l| u64::from(l.port) == port)
                    .filter_map(|l| l.pid),
            );
            roots.insert(pid_id, pids);
        }
        let measured = app.sampler.measure_all(&roots, std::process::id()).await;
        let mut projects = serde_json::to_value(&measured.projects).unwrap();
        projects.as_object_mut().unwrap().extend(in_process);
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        return Ok(send(
            200,
            &json!({ "cores": cores, "projects": projects, "self": measured.own, "total": measured.total }),
        ));
    }
    if section == "network" && get {
        return Ok(send(
            200,
            &json!({ "lan": app.lan_addresses(), "networks": app.network_profiles().await }),
        ));
    }
    if section == "restart" && method == Method::POST {
        let Some(restart) = app.restart.clone() else {
            return Err(err(409, "http.notRestartable"));
        };
        let by = if user.is_some() {
            format!(" · {}", app.tr("history.by", params![user = who.as_str()]))
        } else {
            String::new()
        };
        app.log(&format!("[restart] Servym{by}"));
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            restart();
        });
        return Ok(ok());
    }
    if section == "reload" && method == Method::POST {
        app.reload_config(&who, "log.reloaded")?;
        return Ok(ok());
    }
    if section == "config" && id == Some("export") && get {
        // Le fichier tel qu'il est sur le disque, à télécharger.
        let day = chrono::Utc::now().format("%Y-%m-%d");
        let text = std::fs::read_to_string(&app.config_file).map_err(|e| raw(e.to_string()))?;
        return Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"servym-projects-{day}.json\""),
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(text))
            .unwrap());
    }
    if section == "config" && id == Some("import") && method == Method::POST {
        // Remplace les projets par ceux d'un fichier exporté ; la section instance (port, accès,
        // nom…) reste celle de cette machine. Sans apply : aperçu seulement.
        let body = ctx.read_json(512_000).await?;
        let imported = &body["config"]["projects"];
        if !imported.is_array() {
            return Err(err(400, "config.noProjects"));
        }
        let current = app.disk_config()?;
        let mut next = current.clone();
        next["projects"] = imported.clone();
        validate(&next)?;
        let (mut added, mut removed, mut changed, mut unchanged) = (Vec::new(), Vec::new(), Vec::new(), 0);
        let empty = Vec::new();
        let olds = current["projects"].as_array().unwrap_or(&empty);
        for p in next["projects"].as_array().unwrap() {
            match olds.iter().find(|o| o["id"] == p["id"]) {
                None => added.push(json!({ "id": p["id"], "name": p["name"] })),
                Some(old) => {
                    let changes = describe_changes(old, p);
                    if changes.is_empty() {
                        unchanged += 1;
                    } else {
                        changed.push(json!({ "id": p["id"], "name": p["name"], "changes": changes }));
                    }
                }
            }
        }
        for old in olds {
            if !next["projects"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["id"] == old["id"])
            {
                removed.push(json!({ "id": old["id"], "name": old["name"] }));
            }
        }
        let apply = body["apply"] == true;
        if apply {
            app.apply_config(next)?;
            audit(
                "-",
                "audit.imported",
                params![
                    added = added.len(),
                    changed = changed.len(),
                    removed = removed.len()
                ],
            );
            let s = |v: &Value, k: &str| v[k].as_str().unwrap_or_default().to_string();
            for p in &added {
                audit(&s(p, "id"), "audit.addedByImport", params![name = s(p, "name")]);
            }
            for p in &changed {
                audit(
                    &s(p, "id"),
                    "audit.changedByImport",
                    params![changes = s(p, "changes")],
                );
            }
            for p in &removed {
                audit(&s(p, "id"), "audit.removedByImport", params![name = s(p, "name")]);
            }
        }
        return Ok(send(
            200,
            &json!({ "added": added, "removed": removed, "changed": changed, "unchanged": unchanged, "applied": apply }),
        ));
    }
    let (true, Some(id)) = (section == "projects", id) else {
        return Err(err(404, "http.unknownRoute"));
    };

    let project = app.manager.project(id)?;
    let name = project["name"].as_str().unwrap_or_default().to_string();
    if action.is_none() && method == Method::PATCH {
        let patch = ctx.read_json(16_000).await?;
        let sensitive = patch
            .as_object()
            .is_some_and(|m| m.keys().any(|k| SENSITIVE.contains(&k.as_str())));
        if sensitive && !crate::app::AppInner::can(user.as_ref(), "commands") {
            return Err(err(403, "auth.needsCommands"));
        }
        let current = app.disk_config()?;
        let next = patch_project(&current, id, &patch)?;
        app.apply_config(next.clone())?;
        app.log(&format!("[config] {}", app.tr("log.changed", params![id = id])));
        let find = |c: &Value| {
            c["projects"]
                .as_array()
                .and_then(|l| l.iter().find(|p| p["id"] == id).cloned())
                .unwrap_or_default()
        };
        let changes = describe_changes(&find(&current), &find(&next));
        if !changes.is_empty() {
            audit(id, "audit.changed", params![changes = changes]);
        }
        return Ok(send(200, &app.manager.view(&app.manager.project(id)?)));
    }
    if action.is_none() && method == Method::DELETE {
        // Confirmation : mot de passe du compte si la protection est active, sinon le nom exact du
        // projet.
        let body = ctx.read_json_if_any(16_000).await?;
        match &user {
            Some(u) => app.confirm_password(ctx, u, &body["password"]).await?,
            None if body["confirm"].as_str() != Some(name.as_str()) => {
                return Err(LocalError::new(
                    400,
                    "http.confirmName",
                    params![name = name.as_str()],
                ));
            }
            None => {}
        }
        // Arrêté d'abord (avec les dépendances lancées pour lui) : il ne tournerait plus sous aucun
        // contrôle.
        let st = app.manager.state(id);
        if (st.managed || project["type"] == "static") && st.state != State::Stopped {
            let reason = app.tr("history.deleted", params![]);
            app.manager.stop_with(id, true, Some(reason)).await?;
        }
        app.apply_config(remove_project(&app.disk_config()?, id)?)?;
        app.log(&format!("[config] {}", app.tr("log.deleted", params![id = id])));
        audit(id, "audit.deleted", params![name = name.as_str()]);
        return Ok(ok());
    }
    if action == Some("logs") && get {
        let file = app.manager.log_file(id);
        if ctx.has_query("tail") {
            return Ok(send(200, &json!({ "lines": tail_lines(&file, 30) })));
        }
        // Lancement précédent : terminé, envoyé en une fois (sa fin, comme le journal en direct).
        if ctx.has_query("prev") {
            return Ok(send(
                200,
                &json!({ "lines": tail_lines(&app.manager.prev_log_file(id), servym_core::logs::MAX_LINES) }),
            ));
        }
        let session = app.session_id(ctx).await;
        return Ok(log_stream(app, session, file));
    }
    if action == Some("duplicate") && method == Method::POST {
        // Port libre après celui de l'original ; nom dans la langue du serveur.
        let port = if project["type"] == "file" {
            None
        } else {
            Some(
                app.free_port(u32::from(port_of(&project).unwrap_or(3000)) + 1)
                    .await?,
            )
        };
        let copy_name = app.tr("project.copyName", params![name = name.as_str()]);
        let (next, created) = duplicate_project(&app.disk_config()?, id, &copy_name, port)?;
        app.apply_config(next)?;
        audit(&created, "audit.duplicated", params![name = name.as_str()]);
        return Ok(send(201, &app.manager.view(&app.manager.project(&created)?)));
    }
    if action == Some("history") && get {
        let entries = history(&[app.own_prev_log.clone(), app.own_log.clone()], Some(id), 100);
        return Ok(send(200, &json!({ "entries": entries })));
    }
    if method == Method::POST
        && let Some(action) =
            action.filter(|a| ["start", "restart", "stop", "open", "folder", "code", "terminal"].contains(a))
    {
        app.host_only(ctx, action)?;
        let by = user
            .is_some()
            .then(|| app.tr("history.by", params![user = who.as_str()]));
        app.note_actor(id, action, by);
        let cwd = std::path::PathBuf::from(project["cwd"].as_str().unwrap_or_default());
        match action {
            "start" => app.manager.start(id)?,
            "restart" => app.manager.restart(id).await?,
            "stop" => app.manager.stop_cascade(id).await?,
            "open" => app.manager.open_file(id)?,
            "folder" => app.platform.open_folder(&cwd).map_err(raw)?,
            "code" => app.platform.open_in_code(&cwd).map_err(raw)?,
            _ => app.platform.open_terminal(&cwd).map_err(|e| {
                if e.starts_with("system.") {
                    err(500, &e)
                } else {
                    raw(e)
                }
            })?,
        }
        return Ok(ok());
    }
    Err(err(404, "http.unknownRoute"))
}
