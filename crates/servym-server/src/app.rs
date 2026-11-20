//! État partagé de l'application : configuration (écritures, relecture), journal de Servym,
//! historique et notifications tirés des évènements du gestionnaire, flux ouverts par session.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use servym_core::config::{describe_changes, load_config, save_config};
use servym_core::i18n::{Lang, LocalError, Params, msg, tr};
use servym_core::manager::{Event, Manager, State};
use servym_core::params;
use servym_core::stats::Sampler;
use servym_sys::notify::{Notification, Notifier};
use servym_sys::{Autostart, NetworkProfile, Platform};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::auth::{AuthCore, Provider};
use crate::{Clock, Listening};

pub struct AppInner {
    pub config_file: PathBuf,
    pub runtime_dir: PathBuf,
    pub own_log: PathBuf,
    pub own_prev_log: PathBuf,
    pub ui_file: PathBuf,
    pub valid_copy: PathBuf,
    /// Dossier de Servym (ses données, `{servym}` des arguments).
    pub data_dir: PathBuf,
    pub manager: Manager,
    pub auth: tokio::sync::Mutex<AuthCore>,
    /// Erreur de projects.json, affichée par la page (mode dégradé).
    pub config_error: Mutex<Option<LocalError>>,
    /// Sérialise les lectures-modifications-écritures de projects.json.
    pub config_write: Mutex<()>,
    /// Contenu de projects.json déjà connu (lu ou écrit par Servym) : pas à relire.
    pub known: Mutex<Option<String>>,
    pub own_log_size: Mutex<u64>,
    /// Auteur de la dernière action sur chaque projet, déjà traduit (« par alice », « via la zone
    /// de notification »), et son heure.
    pub actors: Mutex<HashMap<String, (String, i64)>>,
    /// Flux SSE ouverts par session : fermés quand la session est révoquée ou verrouillée.
    pub streams: Mutex<HashMap<String, Vec<CancellationToken>>>,
    /// Changement de l'erreur de configuration, pour les flux SSE.
    pub instance_events: broadcast::Sender<()>,
    /// Changement du démarrage avec la session, pour la zone de notification.
    pub autostart_events: broadcast::Sender<()>,
    pub sampler: Sampler,
    pub git_cache: tokio::sync::Mutex<HashMap<String, (Instant, Value)>>,
    pub network: tokio::sync::Mutex<Option<(Instant, Vec<NetworkProfile>)>>,
    pub restart: Option<Arc<dyn Fn() + Send + Sync>>,
    pub autostart: Arc<dyn Autostart>,
    pub clock: Clock,
    pub notifier: Arc<dyn Notifier>,
    pub listening: Mutex<Listening>,
    pub hostnames: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    pub local_viewer: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>,
    pub platform_name: String,
    /// Ligne « Moteur » des Paramètres.
    pub engine: String,
    pub desktop: bool,
    /// Application de bureau autour du serveur (absent : `servym` sans écran).
    pub shell: Option<Arc<dyn crate::Shell>>,
    pub system_lang: Lang,
    pub platform: Arc<dyn Platform>,
    pub providers: Vec<Provider>,
}

pub type App = Arc<AppInner>;

/// Contenu d'un fichier, ou `None` s'il est illisible (absent, en cours d'écriture…).
pub fn read_text(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok()
}

impl AppInner {
    pub fn lang(&self) -> Lang {
        self.manager.lang()
    }

    pub fn tr(&self, key: &str, params: Params) -> String {
        tr(key, &params, self.lang())
    }

    /// Ligne du journal de Servym (servym.log, gardé à 1 Mo : le précédent dans
    /// servym.prev.log), recopiée sur la sortie standard.
    pub fn log(&self, text: &str) {
        let clean = text.replace(['\r', '\n'], " ");
        let line = format!(
            "{} {clean}\n",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ")
        );
        print!("{line}");
        let mut size = self.own_log_size.lock().unwrap_or_else(|e| e.into_inner());
        *size += line.len() as u64;
        if *size > 1_000_000 {
            let _ = std::fs::rename(&self.own_log, &self.own_prev_log);
            *size = line.len() as u64;
        }
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.own_log)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }

    /// Action demandée sur un projet : « [start] api · par alice » au journal, et l'auteur retenu
    /// pour la ligne d'historique de la transition qui suit.
    pub fn note_actor(&self, id: &str, action: &str, by: Option<String>) {
        let suffix = by.as_ref().map(|b| format!(" · {b}")).unwrap_or_default();
        self.log(&format!("[{action}] {id}{suffix}"));
        if let Some(by) = by {
            self.actors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.to_string(), (by, (self.clock)()));
        }
    }

    /// Journal d'audit : « [audit] <projet ou -> <compte ou -> · <texte> ».
    pub fn audit(&self, project: &str, who: &str, key: &str, params: Params) {
        self.log(&format!("[audit] {project} {who} · {}", self.tr(key, params)));
    }

    pub fn config(&self) -> Value {
        self.manager.config()
    }

    pub fn set_config_error(&self, error: Option<LocalError>) {
        let mut current = self.config_error.lock().unwrap_or_else(|e| e.into_inner());
        let same = current.as_ref().map(|e| e.message_in(Lang::En))
            == error.as_ref().map(|e| e.message_in(Lang::En));
        if same {
            return;
        }
        *current = error;
        drop(current);
        let _ = self.instance_events.send(());
    }

    pub fn config_error_in(&self, lang: Lang) -> Option<String> {
        self.config_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|e| e.message_in(lang))
    }

    fn remember_valid(&self, config: &Value) {
        let _ = save_config(&self.valid_copy, config);
    }

    /// Langue des journaux et des états : `instance.lang`, sinon celle du système.
    pub fn apply_lang(&self, config: &Value) {
        let lang = config["instance"]["lang"]
            .as_str()
            .and_then(Lang::pick)
            .unwrap_or(self.system_lang);
        self.manager.set_lang(lang);
    }

    /// Toute écriture part du fichier tel qu'il est sur le disque (et non de la copie en mémoire) :
    /// une modification faite à la main n'est jamais écrasée.
    pub fn disk_config(&self) -> Result<Value, LocalError> {
        load_config(&self.config_file).map_err(|error| {
            let fix = LocalError::new(
                400,
                "config.fixFirst",
                params![error = msg(&error.key, error.params.clone())],
            );
            self.set_config_error(Some(error));
            fix
        })
    }

    pub fn apply_config(&self, next: Value) -> Result<(), LocalError> {
        self.manager.check_config(&next)?;
        save_config(&self.config_file, &next)
            .map_err(|e| LocalError::new(500, "raw", params![text = e.to_string()]))?;
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = read_text(&self.config_file);
        self.manager.set_config(next.clone())?;
        self.apply_lang(&next);
        self.remember_valid(&next);
        self.set_config_error(None);
        Ok(())
    }

    pub fn edit_config<T>(
        &self,
        f: impl FnOnce(Value) -> Result<(Value, T), LocalError>,
    ) -> Result<T, LocalError> {
        let _guard = self.config_write.lock().unwrap_or_else(|e| e.into_inner());
        let current = self.disk_config()?;
        let (next, out) = f(current)?;
        self.apply_config(next)?;
        Ok(out)
    }

    /// Relit projects.json (bouton « Relire » ou relecture automatique) ; `who` : compte qui l'a
    /// demandé, « - » sinon. Fichier invalide ou projet en marche retiré : l'erreur est affichée
    /// par la page et la configuration actuelle reste en place.
    pub fn reload_config(&self, who: &str, log_key: &str) -> Result<(), LocalError> {
        let _guard = self.config_write.lock().unwrap_or_else(|e| e.into_inner());
        *self.known.lock().unwrap_or_else(|e| e.into_inner()) = read_text(&self.config_file);
        let next =
            load_config(&self.config_file).and_then(|next| self.manager.check_config(&next).map(|()| next));
        let next = match next {
            Ok(next) => next,
            Err(error) => {
                self.set_config_error(Some(error.clone()));
                return Err(error);
            }
        };
        let previous = self.manager.config();
        self.manager.set_config(next.clone())?;
        self.apply_lang(&next);
        self.remember_valid(&next);
        self.set_config_error(None);
        self.log(&format!("[config] {}", self.tr(log_key, params![])));
        // Modifications faites à la main dans le fichier : inscrites comme les autres.
        let empty = Vec::new();
        let old_list = previous["projects"].as_array().unwrap_or(&empty);
        let new_list = next["projects"].as_array().unwrap_or(&empty);
        for p in new_list {
            let id = p["id"].as_str().unwrap_or_default();
            match old_list.iter().find(|o| o["id"] == p["id"]) {
                None => self.audit(id, who, "audit.addedInFile", params![]),
                Some(old) => {
                    let changes = describe_changes(old, p);
                    if !changes.is_empty() {
                        self.audit(id, who, "audit.changedInFile", params![changes = changes]);
                    }
                }
            }
        }
        for old in old_list {
            if !new_list.iter().any(|p| p["id"] == old["id"]) {
                let name = old["name"].as_str().unwrap_or_default().to_string();
                self.audit(
                    old["id"].as_str().unwrap_or_default(),
                    who,
                    "audit.removedInFile",
                    params![name = name],
                );
            }
        }
        Ok(())
    }

    /// Ferme les flux SSE de ces sessions.
    pub fn revoke_streams(&self, sessions: &[String]) {
        let mut streams = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        for id in sessions {
            for token in streams.remove(id).unwrap_or_default() {
                token.cancel();
            }
        }
    }

    /// Nom affiché comme émetteur des notifications.
    pub fn source(&self) -> String {
        self.config()["instance"]["name"]
            .as_str()
            .unwrap_or("Servym")
            .to_string()
    }

    /// Adresses IPv4 de cette machine sur les réseaux (hors boucle locale et auto-configuration).
    pub fn lan_addresses(&self) -> Vec<Value> {
        let mut list = Vec::new();
        for iface in if_addrs::get_if_addrs().unwrap_or_default() {
            if let IpAddr::V4(ip) = iface.ip()
                && !ip.is_loopback()
                && !ip.is_link_local()
            {
                list.push(serde_json::json!({ "name": iface.name, "address": ip.to_string() }));
            }
        }
        list
    }
}

/// Historique : chaque changement d'état est écrit dans servym.log, pour pouvoir expliquer
/// après coup un arrêt survenu en l'absence de l'utilisateur. Notification du système quand un
/// projet passe en échec (réglage `instance.notify`) ; ni pour un arrêt demandé, ni pour un
/// plantage que la relance automatique va réparer (seulement son abandon).
pub fn spawn_event_log(app: &App) {
    let mut events = app.manager.subscribe();
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let Some(app) = weak.upgrade() else { return };
            match event {
                Event::Transition(t) => {
                    log_transition(&app, &t);
                    notify_failure(&app, &t);
                }
                Event::Warning(text) => app.log(&text),
                _ => {}
            }
        }
    });
}

fn state_name(state: State) -> String {
    serde_json::to_value(state)
        .unwrap()
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn log_transition(app: &AppInner, t: &servym_core::manager::Transition) {
    let after = &t.after;
    let mut details = Vec::new();
    // Raison d'un arrêt, relance automatique (après l'erreur si elle est prévue ou abandonnée).
    let moving = matches!(t.to, State::Stopping | State::Starting);
    if moving && let Some(detail) = &after.detail {
        details.push(detail.clone());
    }
    let actor = app
        .actors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&t.id)
        .cloned();
    if let Some((by, at)) = actor
        && moving
        && (app.clock)() - at < 10_000
    {
        details.push(by);
    }
    if t.to == State::Running && !after.managed {
        details.push(app.tr("history.external", params![]));
    }
    if let Some(code) = after.exit_code
        && matches!(t.to, State::Stopped | State::Failed)
    {
        details.push(app.tr("history.exitCode", params![code = code]));
    }
    if let Some(error) = &after.error {
        details.push(error.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    if t.to == State::Failed
        && let Some(detail) = &after.detail
    {
        details.push(detail.clone());
    }
    let suffix = if details.is_empty() {
        String::new()
    } else {
        format!(" · {}", details.join(" · "))
    };
    app.log(&format!(
        "[state] {} {} → {}{suffix}",
        t.id,
        state_name(t.from),
        state_name(t.to)
    ));
}

fn notify_failure(app: &App, t: &servym_core::manager::Transition) {
    use servym_core::manager::AutoRestart;
    let config = app.config();
    if t.to != State::Failed
        || t.after.auto_restart == Some(AutoRestart::Scheduled)
        || config["instance"]["notify"] != true
    {
        return;
    }
    let name = config["projects"]
        .as_array()
        .and_then(|l| l.iter().find(|p| p["id"] == t.id.as_str()))
        .and_then(|p| p["name"].as_str())
        .unwrap_or(&t.id)
        .to_string();
    let body = t.after.error.clone().unwrap_or_else(|| {
        t.after
            .exit_code
            .map(|code| app.tr("history.exitCode", params![code = code]))
            .unwrap_or_default()
    });
    let title = if t.after.auto_restart == Some(AutoRestart::GaveUp) {
        "notify.gaveUp"
    } else if t.from == State::Starting {
        "notify.failedStart"
    } else {
        "notify.crashed"
    };
    let notification = Notification {
        title: app.tr(title, params![name = name]),
        body,
        source: Some(app.source()),
    };
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = app.notifier.notify(notification).await {
            app.log(&format!(
                "[notify] {}",
                app.tr("log.notifyFailed", params![error = error])
            ));
        }
    });
}

/// Relecture automatique : projects.json modifié à la main (éditeur, git pull…) est relu dès qu'il
/// ne bouge plus (même contenu deux tours de suite). Refusé (fichier invalide, projet en marche
/// retiré) : réessayé à chaque tour, pour s'appliquer dès que possible, et signalé une seule fois.
pub fn spawn_watcher(app: &App, every: Duration) {
    if every.is_zero() {
        return;
    }
    let weak = Arc::downgrade(app);
    tokio::spawn(async move {
        let mut pending: Option<String> = None;
        let mut failure: Option<String> = None;
        let mut interval = tokio::time::interval(every);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let Some(app) = weak.upgrade() else { return };
            let text = read_text(&app.config_file);
            let known = app.known.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if text.is_none() || text == known {
                pending = None;
                continue;
            }
            if pending != text {
                pending = text;
                continue;
            }
            match app.reload_config("-", "log.autoReloaded") {
                Ok(()) => {
                    pending = None;
                    failure = None;
                }
                Err(error) => {
                    *app.known.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    let message = error.message_in(app.lang());
                    if failure.as_deref() != Some(message.as_str()) {
                        app.log(&format!(
                            "[config] {}",
                            app.tr("log.autoReloadFailed", params![error = message.clone()])
                        ));
                    }
                    failure = Some(message);
                }
            }
        }
    });
}
