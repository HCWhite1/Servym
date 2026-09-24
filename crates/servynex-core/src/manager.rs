//! Gestionnaire des projets : lance, surveille et arrête.
//!
//! États : stopped, starting, running, stopping, failed, conflict. « managed » : Servynex connaît
//! le PID (lancé ou repris par lui). Un projet qui répond sans être managed a été lancé ailleurs.
//!
//! Structures d'état (indexées par projet), toutes dans [`St`] derrière un verrou jamais tenu
//! pendant une attente :
//! - `states` : état affiché ; écrit uniquement par `set`, qui diffuse `Change` (et `Transition`
//!   si l'état nommé change).
//! - `ops` : action en cours, sous forme de numéro. Posé par `start` et `stop` ; chaque étape
//!   asynchrone vérifie que le numéro est toujours le sien, sinon elle abandonne. Tant qu'il
//!   existe, la sonde ne touche pas au projet.
//! - `relaunch` : fenêtre d'attente après une sortie propre non demandée (projet qui se relance
//!   lui-même), plus courte si elle survient pendant le lancement (lanceur qui laisse un serveur en
//!   arrière-plan) ; la sonde y cherche le nouveau PID par le port. `start` est refusé pendant ce
//!   temps.
//! - `checked_at` : dernière sonde, pour la cadence (3 s arrêté, 15 s en ligne).
//! - `statics` : sites statiques servis par Servynex.
//! - `watched` : PID dont la sortie arrivera avec son code ; la sonde l'attend un peu avant de
//!   conclure sans lui.
//! - `retries` : relance automatique après un plantage (restartOnCrash).
//! - `ready` : faux jusqu'à la première sonde ; l'état trouvé au démarrage n'est pas une transition.
//!
//! Seuls les projets managed sont gardés dans `runtime/pids.json`, avec l'heure de création de
//! leur processus : un PID réutilisé par un autre programme (après un redémarrage du PC) n'est
//! jamais repris.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use servynex_sys::{LaunchError, LaunchOptions, Platform};
use tokio::sync::broadcast;
use tokio::task::{AbortHandle, JoinHandle};

use crate::config::{expand, host_of, port_of, url_of};
use crate::health::{Health, http_health, port_in_use};
use crate::i18n::{Lang, LocalError, Params, msg, tr};
use crate::logs::{last_error, startup_reason};
use crate::params;
use crate::static_site::{self, StaticSite};

/// Cadences et délais du gestionnaire (raccourcis dans les tests).
#[derive(Clone, Debug)]
pub struct Timings {
    /// Sonde d'un projet arrêté ou en cours de lancement.
    pub tick: Duration,
    /// Sonde d'un projet en ligne.
    pub running_tick: Duration,
    /// Délai de lancement par défaut (réglable par projet : `startTimeout`).
    pub start_timeout: Duration,
    /// Attente d'une nouvelle instance après une sortie propre (projet qui se relance lui-même).
    pub relaunch: Duration,
    /// Lanceur sorti sans erreur (symfony) : attente du port avant l'échec.
    pub launcher: Duration,
    /// Délais des relances automatiques successives.
    pub restart_delays: Vec<Duration>,
    /// Fenêtre dans laquelle les relances sont comptées.
    pub restart_window: Duration,
    /// Délai de grâce avant de conclure à une sortie sans son code.
    pub exit_grace: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            tick: Duration::from_secs(3),
            running_tick: Duration::from_secs(15),
            start_timeout: Duration::from_secs(60),
            relaunch: Duration::from_secs(30),
            launcher: Duration::from_secs(10),
            restart_delays: vec![
                Duration::from_secs(2),
                Duration::from_secs(10),
                Duration::from_secs(30),
            ],
            restart_window: Duration::from_secs(300),
            exit_grace: Duration::from_millis(1500),
        }
    }
}

/// État d'un projet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
    Conflict,
    /// Projet « file » : jamais lancé.
    File,
}

impl State {
    fn active(self) -> bool {
        matches!(self, State::Running | State::Starting | State::Stopping)
    }
}

/// Relance automatique après un plantage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoRestart {
    Scheduled,
    GaveUp,
}

/// État en direct d'un projet (valeur par défaut : arrêté).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub state: State,
    /// Lancé (ou repris) par Servynex : arrêtable.
    pub managed: bool,
    pub pid: Option<u32>,
    /// Début du lancement (ms).
    pub started_at: Option<i64>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub detail: Option<String>,
    /// Port et adresse du lancement en cours (ceux de la configuration ont pu changer depuis).
    pub active_port: Option<u16>,
    pub active_host: Option<String>,
    /// Lancé comme dépendance d'un autre projet (arrêté avec le dernier qui l'utilisait).
    pub dependency: bool,
    pub auto_restart: Option<AutoRestart>,
}

/// Changement d'état, avec l'état complet d'après (historique, notifications).
#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub id: String,
    pub from: State,
    pub to: State,
    pub after: ProjectState,
}

/// Évènements diffusés aux flux SSE et au serveur.
#[derive(Clone, Debug)]
pub enum Event {
    /// État d'un projet changé (vue complète du projet).
    Change(Value),
    Transition(Transition),
    /// Configuration remplacée.
    Config,
    /// Avertissement à inscrire au journal de Servynex.
    Warning(String),
}

struct Relaunch {
    until: Instant,
    code: Option<i64>,
    starting: bool,
    active_port: Option<u16>,
    active_host: Option<String>,
}

#[derive(Default)]
struct Retry {
    crashes: Vec<Instant>,
    timer: Option<AbortHandle>,
    dependency: bool,
}

#[derive(Default)]
struct St {
    config: Value,
    states: HashMap<String, ProjectState>,
    ops: HashMap<String, u64>,
    next_op: u64,
    statics: HashMap<String, StaticSite>,
    checked_at: HashMap<String, Instant>,
    relaunch: HashMap<String, Relaunch>,
    watched: HashSet<u32>,
    retries: HashMap<String, Retry>,
    /// Sorties arrivées avant que le PID soit enregistré (PID → code).
    early_exits: HashMap<u32, Option<i64>>,
    /// Heure de création du processus de chaque projet managed (pids.json).
    created: HashMap<String, i64>,
    ready: bool,
}

struct Inner {
    st: Mutex<St>,
    events: broadcast::Sender<Event>,
    platform: Arc<dyn Platform>,
    http: reqwest::Client,
    timings: Timings,
    logs_dir: PathBuf,
    pids_file: PathBuf,
    boot_file: PathBuf,
    boot_time: i64,
    lang: RwLock<Lang>,
    servynex_dir: String,
    ticking: AtomicBool,
    ticker: Mutex<Option<JoinHandle<()>>>,
    /// Environnement asynchrone : les fils qui attendent la fin des projets y programment la suite.
    runtime: tokio::runtime::Handle,
}

/// Gestionnaire (clonable : les clones partagent le même état).
#[derive(Clone)]
pub struct Manager {
    inner: Arc<Inner>,
}

/// Construction d'un [`Manager`].
pub struct ManagerBuilder {
    config: Value,
    logs_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    timings: Timings,
    boot_time: Option<i64>,
    lang: Lang,
    platform: Option<Arc<dyn Platform>>,
    servynex_dir: Option<PathBuf>,
}

impl ManagerBuilder {
    pub fn logs_dir(mut self, dir: &Path) -> Self {
        self.logs_dir = Some(dir.to_path_buf());
        self
    }
    pub fn runtime_dir(mut self, dir: &Path) -> Self {
        self.runtime_dir = Some(dir.to_path_buf());
        self
    }
    pub fn timings(mut self, timings: Timings) -> Self {
        self.timings = timings;
        self
    }
    /// Heure de démarrage du PC (ms) : « autostart » une fois par démarrage.
    pub fn boot_time(mut self, ms: i64) -> Self {
        self.boot_time = Some(ms);
        self
    }
    /// Langue des états et des erreurs.
    pub fn lang(mut self, lang: Lang) -> Self {
        self.lang = lang;
        self
    }
    /// Système (par défaut : celui de la machine).
    pub fn platform(mut self, platform: Arc<dyn Platform>) -> Self {
        self.platform = Some(platform);
        self
    }
    /// Dossier désigné par `{servynex}` dans les arguments (fichiers de config/ de MySQL…).
    pub fn servynex_dir(mut self, dir: &Path) -> Self {
        self.servynex_dir = Some(dir.to_path_buf());
        self
    }
    pub fn build(self) -> Manager {
        let logs_dir = self.logs_dir.unwrap_or_else(|| PathBuf::from("logs"));
        let runtime_dir = self.runtime_dir.unwrap_or_else(|| PathBuf::from("runtime"));
        let _ = std::fs::create_dir_all(&logs_dir);
        let _ = std::fs::create_dir_all(&runtime_dir);
        let servynex_dir = self
            .servynex_dir
            .or_else(|| logs_dir.parent().map(Path::to_path_buf))
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (events, _) = broadcast::channel(1024);
        Manager {
            inner: Arc::new(Inner {
                st: Mutex::new(St {
                    config: self.config,
                    ..St::default()
                }),
                events,
                platform: self.platform.unwrap_or_else(servynex_sys::native),
                http: crate::health::client(),
                timings: self.timings,
                pids_file: runtime_dir.join("pids.json"),
                boot_file: runtime_dir.join("autostart.json"),
                logs_dir,
                boot_time: self.boot_time.unwrap_or_else(servynex_sys::boot_time_ms),
                lang: RwLock::new(self.lang),
                servynex_dir,
                ticking: AtomicBool::new(false),
                ticker: Mutex::new(None),
                runtime: tokio::runtime::Handle::current(),
            }),
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn id_of(p: &Value) -> String {
    p["id"].as_str().unwrap_or_default().to_string()
}

fn name_of(p: &Value) -> String {
    p["name"].as_str().unwrap_or_default().to_string()
}

fn kind_of(p: &Value) -> Option<&str> {
    p.get("type").and_then(Value::as_str)
}

fn requires_of(p: &Value) -> Vec<String> {
    p.get("requires")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |l| {
            l.iter().filter_map(Value::as_str).map(str::to_string).collect()
        })
}

impl Manager {
    pub fn builder(config: Value) -> ManagerBuilder {
        ManagerBuilder {
            config,
            logs_dir: None,
            runtime_dir: None,
            timings: Timings::default(),
            boot_time: None,
            lang: Lang::default(),
            platform: None,
            servynex_dir: None,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, St> {
        self.inner.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn t(&self, key: &str, params: Params) -> String {
        tr(key, &params, self.lang())
    }

    pub fn lang(&self) -> Lang {
        *self.inner.lang.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_lang(&self, lang: Lang) {
        *self.inner.lang.write().unwrap_or_else(|e| e.into_inner()) = lang;
    }

    pub fn platform(&self) -> &Arc<dyn Platform> {
        &self.inner.platform
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    fn emit(&self, event: Event) {
        let _ = self.inner.events.send(event);
    }

    fn warn(&self, message: String) {
        self.emit(Event::Warning(message));
    }

    // ---- Lecture ----

    pub fn config(&self) -> Value {
        self.lock().config.clone()
    }

    pub fn state(&self, id: &str) -> ProjectState {
        self.lock().states.get(id).cloned().unwrap_or_default()
    }

    /// Projet de la configuration (erreur 404 s'il n'existe pas).
    pub fn project(&self, id: &str) -> Result<Value, LocalError> {
        project_in(&self.lock().config, id)
    }

    pub fn log_file(&self, id: &str) -> PathBuf {
        self.inner.logs_dir.join(format!("{id}.log"))
    }

    /// Journal du lancement précédent : le journal est renommé ainsi à chaque lancement.
    pub fn prev_log_file(&self, id: &str) -> PathBuf {
        self.inner.logs_dir.join(format!("{id}.prev.log"))
    }

    /// Vue d'un projet pour l'interface (réglages et état en direct).
    pub fn view(&self, project: &Value) -> Value {
        let state = self.state(project["id"].as_str().unwrap_or_default());
        self.view_with(project, &state)
    }

    fn view_with(&self, p: &Value, state: &ProjectState) -> Value {
        let id = id_of(p);
        let kind = kind_of(p);
        let command = match kind {
            Some("static") => self.t("process.builtinServer", params![]),
            Some("file") => self.t("process.openedInBrowser", params![]),
            _ => {
                let mut argv = vec![p["cmd"].as_str().unwrap_or_default().to_string()];
                argv.extend(
                    p["args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_string),
                );
                argv.iter()
                    .map(|a| expand(a, p, &self.inner.servynex_dir))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        };
        let file = (kind == Some("file")).then(|| {
            Path::new(p["cwd"].as_str().unwrap_or_default())
                .join(p["open"].as_str().unwrap_or_default())
                .to_string_lossy()
                .into_owned()
        });
        let mut view = json!({
            "id": id,
            "name": p["name"],
            "icon": p.get("icon").cloned().unwrap_or(json!("box")),
            "color": p.get("color").cloned().unwrap_or(json!("#8b5cf6")),
            "desc": p.get("desc").cloned().unwrap_or(json!("")),
            "tags": p.get("tags").cloned().unwrap_or(json!([])),
            "type": kind.unwrap_or("process"),
            "service": p.get("service").and_then(Value::as_bool).unwrap_or(false),
            "autostart": p.get("autostart").and_then(Value::as_bool).unwrap_or(false),
            "restartOnCrash": p.get("restartOnCrash").and_then(Value::as_bool).unwrap_or(false),
            "port": p.get("port").cloned().unwrap_or(Value::Null),
            "expose": p.get("expose").and_then(Value::as_bool).unwrap_or(false),
            "host": host_of(p),
            "hostControl": match kind { Some("static") => json!("internal"), Some("file") => json!("none"), _ => p.get("hostControl").cloned().unwrap_or(Value::Null) },
            "fixedPort": p.get("fixedPort").cloned().unwrap_or(Value::Null),
            "requires": p.get("requires").cloned().unwrap_or(json!([])),
            "cwd": p["cwd"],
            "url": url_of(p, &self.inner.servynex_dir),
            "command": command,
            "env": p.get("env").cloned().unwrap_or(json!({})),
            "open": p.get("open").cloned().unwrap_or(Value::Null),
            "file": file,
            "hasLog": self.log_file(&id).exists(),
            "hasPrevLog": self.prev_log_file(&id).exists(),
            "source": p,
        });
        let state = if kind == Some("file") {
            ProjectState {
                state: State::File,
                ..ProjectState::default()
            }
        } else {
            state.clone()
        };
        if let (Value::Object(view), Value::Object(fields)) =
            (&mut view, serde_json::to_value(state).unwrap())
        {
            view.extend(fields);
            // Comme JSON.stringify d'un champ indéfini : une page locale n'a pas de port.
            if p.get("port").is_none() {
                view.shift_remove("port");
            }
        }
        view
    }

    /// Vue de tous les projets, dans l'ordre de la configuration.
    pub fn snapshot(&self) -> Vec<Value> {
        let (projects, states) = {
            let st = self.lock();
            (
                st.config["projects"].as_array().cloned().unwrap_or_default(),
                st.states.clone(),
            )
        };
        projects
            .iter()
            .map(|p| self.view_with(p, &states.get(&id_of(p)).cloned().unwrap_or_default()))
            .collect()
    }

    // ---- Écriture de l'état ----

    fn set(&self, st: &mut St, id: &str, change: impl FnOnce(&mut ProjectState)) {
        let before = st.states.get(id).cloned().unwrap_or_default();
        let mut after = before.clone();
        change(&mut after);
        if after == before {
            return;
        }
        st.states.insert(id.to_string(), after.clone());
        let Ok(p) = project_in(&st.config, id) else { return };
        self.emit(Event::Change(self.view_with(&p, &after)));
        // Avant la première sonde, « stopped » n'est qu'une valeur par défaut : trouver un projet
        // déjà en ligne au démarrage de Servynex n'est pas un changement. Une action lancée
        // entre-temps en est bien un.
        let real = st.ready || st.ops.contains_key(id);
        if before.state != after.state && real {
            self.emit(Event::Transition(Transition {
                id: id.to_string(),
                from: before.state,
                to: after.state,
                after,
            }));
        }
    }

    fn save_pids(&self, st: &St) {
        let mut pids = serde_json::Map::new();
        for p in st.config["projects"].as_array().into_iter().flatten() {
            let id = id_of(p);
            let Some(s) = st.states.get(&id) else { continue };
            if s.managed && kind_of(p) != Some("static") {
                pids.insert(
                    id.clone(),
                    json!({
                        "pid": s.pid,
                        "startedAt": s.started_at,
                        "activePort": s.active_port,
                        "activeHost": s.active_host,
                        "dependency": s.dependency,
                        "createdAt": st.created.get(&id),
                    }),
                );
            }
        }
        let _ = std::fs::write(
            &self.inner.pids_file,
            serde_json::to_string_pretty(&Value::Object(pids)).unwrap(),
        );
    }

    // ---- Configuration ----

    /// Refuse de retirer un projet que Servynex fait tourner : il continuerait sans surveillance,
    /// en gardant son port.
    fn assert_can_apply(&self, st: &St, config: &Value) -> Result<(), LocalError> {
        let kept: HashSet<String> = config["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .map(id_of)
            .collect();
        let orphans: Vec<String> = st.config["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| {
                let id = id_of(p);
                // Une action en cours (lancement) compte aussi : le projet serait perdu de vue.
                !kept.contains(&id)
                    && (st.statics.contains_key(&id)
                        || st.ops.contains_key(&id)
                        || st.states.get(&id).is_some_and(|s| s.managed && s.state.active()))
            })
            .map(name_of)
            .collect();
        if orphans.is_empty() {
            Ok(())
        } else {
            Err(LocalError::new(
                409,
                "process.stillRunning",
                params![names = orphans.join(", ")],
            ))
        }
    }

    /// Vérifie qu'une configuration peut remplacer l'actuelle (aucun projet en marche retiré).
    pub fn check_config(&self, config: &Value) -> Result<(), LocalError> {
        self.assert_can_apply(&self.lock(), config)
    }

    /// Remplace la configuration ; refusé (409) si un projet en marche en serait retiré.
    pub fn set_config(&self, config: Value) -> Result<(), LocalError> {
        let mut st = self.lock();
        self.assert_can_apply(&st, &config)?;
        let kept: HashSet<String> = config["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .map(id_of)
            .collect();
        st.config = config;
        st.states.retain(|id, _| kept.contains(id));
        // Relance automatique désactivée (ou projet retiré) : celle qui était prévue n'a plus lieu.
        let ids: Vec<String> = st.retries.keys().cloned().collect();
        for id in ids {
            let still = project_in(&st.config, &id)
                .ok()
                .is_some_and(|p| p.get("restartOnCrash") == Some(&Value::Bool(true)));
            if still {
                continue;
            }
            cancel_restart(&mut st, &id);
            if st
                .states
                .get(&id)
                .is_some_and(|s| s.auto_restart == Some(AutoRestart::Scheduled))
            {
                self.set(&mut st, &id, |s| {
                    s.detail = None;
                    s.auto_restart = None;
                });
            }
        }
        drop(st);
        self.emit(Event::Config);
        Ok(())
    }

    // ---- Surveillance ----

    /// Reprise des projets en marche (`pids.json`), première sonde, démarrage automatique.
    pub async fn init(&self) {
        let saved: serde_json::Map<String, Value> = std::fs::read_to_string(&self.inner.pids_file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let platform = self.inner.platform.clone();
        let mut adopted = Vec::new();
        {
            let mut st = self.lock();
            let ids: Vec<String> = st.config["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .map(id_of)
                .collect();
            for id in &ids {
                let Some(s) = saved.get(id) else { continue };
                let Some(pid) = s["pid"].as_u64().map(|p| p as u32) else {
                    continue;
                };
                if !platform.is_alive(pid) || !same_process(platform.as_ref(), pid, s) {
                    continue;
                }
                let created = platform.process_start(pid);
                if let Some(c) = created {
                    st.created.insert(id.clone(), c);
                }
                self.set(&mut st, id, |state| {
                    *state = ProjectState {
                        state: State::Running,
                        managed: true,
                        pid: Some(pid),
                        started_at: s["startedAt"].as_i64(),
                        active_port: s["activePort"].as_u64().map(|p| p as u16),
                        active_host: s["activeHost"].as_str().map(str::to_string),
                        dependency: s["dependency"].as_bool().unwrap_or(false),
                        ..ProjectState::default()
                    }
                });
                adopted.push((id.clone(), pid));
            }
            // Projet retiré du fichier pendant que Servynex était arrêté : il tourne toujours.
            for (id, s) in &saved {
                let pid = s["pid"].as_u64().unwrap_or(0) as u32;
                if !ids.contains(id) && platform.is_alive(pid) {
                    let text = self.t("process.orphan", params![id = id.as_str(), pid = pid]);
                    self.warn(format!("[config] {text}"));
                }
            }
            // Réécrit tout de suite, avec l'heure de création des processus repris (fichier de la
            // version Node, ou projets disparus depuis).
            if !saved.is_empty() {
                self.save_pids(&st);
            }
        }
        // Projets repris : leur fin est surveillée (sous Windows, avec leur code de sortie).
        for (id, pid) in adopted {
            self.watch(&id, pid);
        }
        self.tick().await;
        self.lock().ready = true;
        let weak = Arc::downgrade(&self.inner);
        let every = self.inner.timings.tick;
        let ticker = tokio::spawn(async move {
            let mut interval = tokio::time::interval(every);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { return };
                let manager = Manager { inner };
                tokio::spawn(async move { manager.tick().await });
            }
        });
        *self.inner.ticker.lock().unwrap_or_else(|e| e.into_inner()) = Some(ticker);
        self.autostart();
    }

    /// Surveille la fin d'un processus managed.
    fn watch(&self, id: &str, pid: u32) {
        self.lock().watched.insert(pid);
        let weak = Arc::downgrade(&self.inner);
        let id = id.to_string();
        self.inner.platform.watch_exit(
            pid,
            Box::new(move |pid, code| {
                if let Some(inner) = weak.upgrade() {
                    Manager { inner }.on_exit(&id, pid, code);
                }
            }),
        );
    }

    fn on_exit(&self, id: &str, pid: u32, code: Option<i64>) {
        let mut st = self.lock();
        st.watched.remove(&pid);
        let current = st.states.get(id).cloned().unwrap_or_default();
        if current.pid != Some(pid) {
            // Sortie arrivée avant que le lancement ait enregistré le PID (plantage immédiat) :
            // traitée dès qu'il le sera.
            if current.state == State::Starting && current.pid.is_none() {
                st.early_exits.insert(pid, code);
            }
            return;
        }
        // Arrêt demandé en cours : c'est lui qui conclut.
        if current.state == State::Stopping {
            return;
        }
        self.exited(&mut st, id, pid, code);
    }

    /// Lance les projets « autostart » une fois par démarrage du PC : un simple redémarrage de
    /// Servynex ne relance pas un projet arrêté volontairement.
    fn autostart(&self) {
        let last = std::fs::read_to_string(&self.inner.boot_file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["bootTime"].as_i64());
        if last.is_some_and(|last| (last - self.inner.boot_time).abs() < 120_000) {
            return;
        }
        let _ = std::fs::write(
            &self.inner.boot_file,
            json!({ "bootTime": self.inner.boot_time }).to_string(),
        );
        let candidates: Vec<String> = {
            let st = self.lock();
            st.config["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| p.get("autostart") == Some(&Value::Bool(true)) && kind_of(p) != Some("file"))
                .map(id_of)
                .filter(|id| {
                    st.states.get(id).map_or(State::Stopped, |s| s.state) == State::Stopped
                        && !st.ops.contains_key(id)
                })
                .collect()
        };
        for id in candidates {
            let _ = self.start(&id);
        }
    }

    /// Arrête la surveillance ; les projets continuent de tourner (sites statiques exceptés).
    pub async fn close(&self) {
        if let Some(ticker) = self.inner.ticker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            ticker.abort();
        }
        let statics: Vec<StaticSite> = {
            let mut st = self.lock();
            let ids: Vec<String> = st.retries.keys().cloned().collect();
            for id in ids {
                cancel_restart(&mut st, &id);
            }
            st.statics.drain().map(|(_, s)| s).collect()
        };
        for site in statics {
            site.stop().await;
        }
    }

    /// Une sonde : chaque projet dû est vérifié (3 s arrêté, 15 s en ligne).
    pub async fn tick(&self) {
        if self.inner.ticking.swap(true, Ordering::SeqCst) {
            return;
        }
        let due: Vec<Value> = {
            let mut st = self.lock();
            let now = Instant::now();
            let running_every = self.inner.timings.running_tick;
            let due: Vec<Value> = st.config["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| {
                    let id = id_of(p);
                    if st.ops.contains_key(&id) || matches!(kind_of(p), Some("static" | "file")) {
                        return false;
                    }
                    let every = if st.states.get(&id).is_some_and(|s| s.state == State::Running) {
                        running_every
                    } else {
                        Duration::ZERO
                    };
                    st.checked_at
                        .get(&id)
                        .is_none_or(|at| now.duration_since(*at) >= every)
                })
                .cloned()
                .collect();
            for p in &due {
                st.checked_at.insert(id_of(p), now);
            }
            due
        };
        let mut tasks = tokio::task::JoinSet::new();
        for p in due {
            let manager = self.clone();
            tasks.spawn(async move { manager.refresh(&p).await });
        }
        while tasks.join_next().await.is_some() {}
        self.inner.ticking.store(false, Ordering::SeqCst);
    }

    async fn health(&self, p: &Value, port: u16) -> Health {
        let health = p.get("health");
        if health.and_then(|h| h.get("type")).and_then(Value::as_str) == Some("tcp") {
            return if port_in_use(port).await {
                Health::Ok
            } else {
                Health::Down
            };
        }
        let path = health
            .and_then(|h| h.get("path"))
            .and_then(Value::as_str)
            .unwrap_or("/");
        let expect = health
            .and_then(|h| h.get("expect"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let tls = url_of(p, &self.inner.servynex_dir).is_some_and(|u| u.starts_with("https:"));
        http_health(&self.inner.http, port, path, expect, tls).await
    }

    async fn pid_on_port(&self, port: u16) -> Option<u32> {
        self.inner
            .platform
            .listeners()
            .await
            .into_iter()
            .find(|l| l.port == port)
            .and_then(|l| l.pid)
    }

    async fn refresh(&self, p: &Value) {
        let id = id_of(p);
        let st0 = self.state(&id);
        let port = st0.active_port.or_else(|| port_of(p)).unwrap_or(0);
        if st0.managed && st0.pid.is_some_and(|pid| !self.inner.platform.is_alive(pid)) {
            let pid = st0.pid.unwrap();
            // Lancé par cette instance : sa sortie, avec son code, arrive juste après. Sans elle, un
            // plantage passerait pour une sortie propre (code inconnu).
            if self.lock().watched.contains(&pid) {
                tokio::time::sleep(self.inner.timings.exit_grace).await;
                if self.state(&id).pid != Some(pid) {
                    return;
                }
            }
            let mut st = self.lock();
            self.exited(&mut st, &id, pid, None);
            return;
        }
        let health = self.health(p, port).await;
        let relaunch_port = {
            let st = self.lock();
            if st.ops.contains_key(&id) {
                return; // une action a commencé pendant le contrôle
            }
            st.relaunch
                .get(&id)
                .map(|r| r.active_port.or_else(|| port_of(p)).unwrap_or(0))
        };
        if let Some(rport) = relaunch_port {
            // Le nouveau processus hérite de la sortie de l'ancien : on le reprend (PID trouvé par
            // le port), avec son journal.
            let pid = if health == Health::Ok {
                self.pid_on_port(rport).await
            } else {
                None
            };
            let mut adopted = None;
            {
                let mut st = self.lock();
                if st.ops.contains_key(&id) {
                    return;
                }
                let Some(relaunch) = st.relaunch.get(&id) else {
                    return;
                };
                if let Some(pid) = pid {
                    let (active_port, active_host) = (relaunch.active_port, relaunch.active_host.clone());
                    st.relaunch.remove(&id);
                    if let Some(c) = self.inner.platform.process_start(pid) {
                        st.created.insert(id.clone(), c);
                    }
                    self.set(&mut st, &id, |s| {
                        *s = ProjectState {
                            state: State::Running,
                            managed: true,
                            pid: Some(pid),
                            started_at: Some(now_ms()),
                            active_port,
                            active_host,
                            ..ProjectState::default()
                        }
                    });
                    self.save_pids(&st);
                    adopted = Some(pid);
                } else if Instant::now() >= relaunch.until {
                    let relaunch = st.relaunch.remove(&id).unwrap();
                    if !relaunch.starting {
                        self.set(&mut st, &id, |s| {
                            *s = ProjectState {
                                exit_code: relaunch.code,
                                ..ProjectState::default()
                            }
                        });
                    } else {
                        // Sorti pendant son lancement sans que rien ne réponde : c'est un échec, et son
                        // journal dit souvent pourquoi (« The local web server is already running »).
                        let reason = startup_reason(&self.log_file(&id));
                        let port = relaunch.active_port.or_else(|| port_of(p)).unwrap_or(0);
                        let error = match reason {
                            Some(reason) => self.t(
                                "process.exitedDuringStartWhy",
                                params![name = name_of(p), port = port, reason = reason],
                            ),
                            None => self.t(
                                "process.exitedDuringStart",
                                params![name = name_of(p), port = port],
                            ),
                        };
                        self.set(&mut st, &id, |s| {
                            *s = ProjectState {
                                state: State::Failed,
                                exit_code: relaunch.code,
                                error: Some(error),
                                ..ProjectState::default()
                            }
                        });
                    }
                }
            }
            if let Some(pid) = adopted {
                self.watch(&id, pid);
            }
            return;
        }
        match health {
            Health::Ok => {
                let mut st = self.lock();
                self.set(&mut st, &id, |s| {
                    s.state = State::Running;
                    s.error = None;
                    s.detail = None;
                });
            }
            Health::Mismatch => {
                let error = self.t("process.wrongService", params![port = port, name = name_of(p)]);
                let mut st = self.lock();
                self.set(&mut st, &id, |s| {
                    s.state = State::Conflict;
                    s.error = Some(error);
                });
            }
            Health::Down if st0.managed => {
                // Un projet qui plante ferme son port un peu avant que sa sortie soit connue : on
                // laisse à celle-ci le temps d'arriver (code de sortie, relance automatique).
                if st0.state == State::Running {
                    tokio::time::sleep(self.inner.timings.exit_grace).await;
                    let now = self.state(&id);
                    let busy = self.lock().ops.contains_key(&id);
                    if busy
                        || now.pid != st0.pid
                        || st0.pid.is_none_or(|pid| !self.inner.platform.is_alive(pid))
                    {
                        return;
                    }
                }
                let error = self.t("process.notResponding", params![port = port]);
                let mut st = self.lock();
                self.set(&mut st, &id, |s| {
                    s.state = State::Failed;
                    s.error = Some(error);
                });
            }
            Health::Down if st0.state != State::Failed => {
                let mut st = self.lock();
                self.set(&mut st, &id, |s| {
                    *s = ProjectState {
                        exit_code: st0.exit_code,
                        ..ProjectState::default()
                    }
                });
            }
            Health::Down => {}
        }
    }

    /// Fin d'un processus managed (code connu ou non).
    fn exited(&self, st: &mut St, id: &str, pid: u32, code: Option<i64>) {
        let Some(before) = st.states.get(id).cloned() else {
            return;
        };
        if before.pid != Some(pid) || !before.managed {
            return;
        }
        let Ok(p) = project_in(&st.config, id) else { return };
        if before.state != State::Stopping && (code == Some(0) || code.is_none()) {
            // Sortie propre sans demande d'arrêt : le projet se relance peut-être lui-même (bouton
            // « Redémarrer » d'une application). Pendant le lancement, c'est un lanceur qui a
            // peut-être laissé tourner le serveur en arrière-plan : attente plus courte, échec
            // expliqué ensuite.
            let starting = before.state == State::Starting;
            let port = before.active_port.or_else(|| port_of(&p)).unwrap_or(0);
            let wait = if starting {
                self.inner.timings.launcher
            } else {
                self.inner.timings.relaunch
            };
            st.relaunch.insert(
                id.to_string(),
                Relaunch {
                    until: Instant::now() + wait,
                    code,
                    starting,
                    active_port: before.active_port,
                    active_host: before.active_host.clone(),
                },
            );
            let detail = if starting {
                self.t("process.launcherDone", params![port = port])
            } else {
                self.t("process.restarting", params![])
            };
            self.set(st, id, |s| {
                *s = ProjectState {
                    state: State::Starting,
                    detail: Some(detail),
                    active_port: before.active_port,
                    active_host: before.active_host.clone(),
                    ..ProjectState::default()
                }
            });
            self.save_pids(st);
            return;
        }
        let expected = before.state == State::Stopping || code == Some(0);
        // Plantage d'un projet en ligne : relancé si demandé. Pas après un échec au lancement (port
        // pris, commande introuvable…) : relancer ne corrigerait rien.
        let retry = if !expected && before.state == State::Running {
            self.plan_restart(st, id, before.dependency)
        } else {
            None
        };
        let error = if expected {
            None
        } else {
            Some(last_error(&self.log_file(id)).unwrap_or_else(|| match code {
                None => self.t("process.unexpectedExit", params![]),
                Some(code) => self.t("process.unexpectedExitCode", params![code = code]),
            }))
        };
        self.set(st, id, |s| {
            *s = ProjectState {
                state: if expected { State::Stopped } else { State::Failed },
                exit_code: code,
                error,
                detail: retry.as_ref().map(|r| r.1.clone()),
                auto_restart: retry.as_ref().map(|r| r.0),
                ..ProjectState::default()
            }
        });
        // Mort pendant le lancement : celui-ci est fini, le projet peut être relancé tout de suite.
        if before.state == State::Starting {
            st.ops.remove(id);
        }
        st.created.remove(id);
        self.save_pids(st);
    }

    // ---- Relance automatique (restartOnCrash) ----

    /// Prévoit la relance d'un projet qui vient de planter, avec un délai qui grandit. Autant de
    /// relances dans la fenêtre que de délais : au plantage suivant, abandon.
    fn plan_restart(&self, st: &mut St, id: &str, dependency: bool) -> Option<(AutoRestart, String)> {
        let p = project_in(&st.config, id).ok()?;
        if p.get("restartOnCrash") != Some(&Value::Bool(true)) {
            return None;
        }
        let timings = &self.inner.timings;
        let now = Instant::now();
        let max = timings.restart_delays.len();
        let mut retry = st.retries.remove(id).unwrap_or_default();
        retry
            .crashes
            .retain(|at| now.duration_since(*at) < timings.restart_window);
        if retry.crashes.len() >= max {
            if let Some(timer) = retry.timer.take() {
                timer.abort();
            }
            let minutes = (timings.restart_window.as_secs_f64() / 60.0).round() as u64;
            return Some((
                AutoRestart::GaveUp,
                self.t("process.restartGaveUp", params![count = max, minutes = minutes]),
            ));
        }
        retry.crashes.push(now);
        let attempt = retry.crashes.len();
        let delay = timings.restart_delays[attempt - 1];
        if let Some(timer) = retry.timer.take() {
            timer.abort();
        }
        let weak: Weak<Inner> = Arc::downgrade(&self.inner);
        let target = id.to_string();
        retry.timer = Some(
            self.inner
                .runtime
                .spawn(async move {
                    tokio::time::sleep(delay).await;
                    if let Some(inner) = weak.upgrade() {
                        Manager { inner }.auto_restart(&target, attempt, max);
                    }
                })
                .abort_handle(),
        );
        retry.dependency = dependency;
        st.retries.insert(id.to_string(), retry);
        let seconds = delay.as_millis().div_ceil(1000) as u64;
        Some((
            AutoRestart::Scheduled,
            self.t(
                "process.restartIn",
                params![seconds = seconds, attempt = attempt, max = max],
            ),
        ))
    }

    fn auto_restart(&self, id: &str, attempt: usize, max: usize) {
        let dependency = {
            let mut st = self.lock();
            let Some(retry) = st.retries.get_mut(id) else {
                return;
            };
            retry.timer = None;
            let dependency = retry.dependency;
            let scheduled = st
                .states
                .get(id)
                .is_some_and(|s| s.auto_restart == Some(AutoRestart::Scheduled));
            if !scheduled || st.ops.contains_key(id) {
                return;
            }
            dependency
        };
        let detail = self.t("process.autoRestarted", params![attempt = attempt, max = max]);
        if let Err(err) = self.start_with(id, dependency, Some(detail)) {
            let mut st = self.lock();
            self.set(&mut st, id, |s| {
                s.detail = None;
                s.auto_restart = None;
            });
            drop(st);
            self.warn(format!("[restart] {id} : {}", err.message_in(self.lang())));
        }
    }

    // ---- Actions ----

    fn refuse(&self, p: &Value, start: bool, why: &str, params: Params) -> LocalError {
        LocalError::new(
            409,
            if start {
                "process.cannotStart"
            } else {
                "process.cannotStop"
            },
            params![name = name_of(p), why = msg(why, params)],
        )
    }

    /// Lance en arrière-plan : la suite (démarrage, échec) arrive par les évènements. Les
    /// vérifications immédiates (projet inconnu, déjà lancé…) renvoient une erreur tout de suite.
    pub fn start(&self, id: &str) -> Result<(), LocalError> {
        self.start_with(id, false, None)
    }

    /// `as_dependency` : lancé pour un projet qui en dépend. `auto` : relance automatique après un
    /// plantage (texte affiché et inscrit à l'historique) ; un lancement demandé remet à zéro le
    /// compte des plantages.
    fn start_with(&self, id: &str, as_dependency: bool, auto: Option<String>) -> Result<(), LocalError> {
        let (p, op) = {
            let mut st = self.lock();
            let p = project_in(&st.config, id)?;
            if kind_of(&p) == Some("file") {
                return Err(self.refuse(&p, true, "process.why.file", params![]));
            }
            let state = st.states.get(id).map_or(State::Stopped, |s| s.state);
            if st.ops.contains_key(id)
                || !matches!(state, State::Stopped | State::Failed)
                || st.relaunch.contains_key(id)
            {
                let why = if state == State::Running {
                    "process.why.running"
                } else {
                    "process.why.busy"
                };
                return Err(self.refuse(&p, true, why, params![]));
            }
            st.relaunch.remove(id);
            if auto.is_none() {
                cancel_restart(&mut st, id);
            }
            st.next_op += 1;
            let op = st.next_op;
            st.ops.insert(id.to_string(), op);
            // « starting » tout de suite, comme en JavaScript : dès que start() rend la main,
            // l'état du projet dit qu'il se lance.
            self.set(&mut st, id, |s| {
                *s = ProjectState {
                    state: State::Starting,
                    dependency: as_dependency,
                    detail: auto.clone(),
                    ..ProjectState::default()
                }
            });
            (p, op)
        };
        let manager = self.clone();
        let id = id.to_string();
        self.inner.runtime.spawn(async move {
            let result = manager.run_start(&p, op).await;
            let mut st = manager.lock();
            if st.ops.get(&id) == Some(&op) {
                if let Err(error) = result {
                    manager.set(&mut st, &id, |s| {
                        s.state = State::Failed;
                        s.error = Some(error);
                        s.detail = None;
                    });
                }
                st.ops.remove(&id);
            }
        });
        Ok(())
    }

    fn has_op(&self, id: &str, op: u64) -> bool {
        self.lock().ops.get(id) == Some(&op)
    }

    async fn run_start(&self, p: &Value, op: u64) -> Result<(), String> {
        let id = id_of(p);
        for dep_id in requires_of(p) {
            let dep = self.project(&dep_id).map_err(|e| e.message_in(self.lang()))?;
            if self.state(&dep_id).state == State::Running {
                continue;
            }
            let waiting = self.t("process.waiting", params![name = name_of(&dep)]);
            {
                let mut st = self.lock();
                self.set(&mut st, &id, |s| s.detail = Some(waiting));
            }
            let busy = self.lock().ops.contains_key(&dep_id);
            if !busy {
                self.start_with(&dep_id, true, None)
                    .map_err(|e| e.message_in(self.lang()))?;
            }
            while self.lock().ops.contains_key(&dep_id) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if !self.has_op(&id, op) {
                return Ok(());
            }
            let dep_state = self.state(&dep_id);
            if dep_state.state != State::Running {
                let reason = dep_state
                    .error
                    .unwrap_or_else(|| self.t("process.unknownReason", params![]));
                return Err(self.t(
                    "process.depFailed",
                    params![name = name_of(&dep), reason = reason],
                ));
            }
        }
        {
            let mut st = self.lock();
            self.set(&mut st, &id, |s| s.detail = None);
        }

        let port = port_of(p).unwrap_or(0);
        if port_in_use(port).await {
            if self.health(p, port).await == Health::Ok {
                let mut st = self.lock();
                self.set(&mut st, &id, |s| s.state = State::Running);
                return Ok(());
            }
            return Err(match self.pid_on_port(port).await {
                Some(pid) => self.t("process.portTakenBy", params![port = port, pid = pid]),
                None => self.t("process.portTaken", params![port = port]),
            });
        }
        let cwd = PathBuf::from(p["cwd"].as_str().unwrap_or_default());
        if !cwd.exists() {
            return Err(self.t(
                "process.noFolder",
                params![path = cwd.to_string_lossy().into_owned()],
            ));
        }

        let host = host_of(p).to_string();
        let started_at = now_ms();
        if kind_of(p) == Some("static") {
            let header = format!(
                "[servynex] {} · {} · {host}:{port}",
                self.stamp(),
                self.t(
                    "process.staticFiles",
                    params![root = cwd.to_string_lossy().into_owned()]
                )
            );
            let site = static_site::start(&cwd, port, &host, &self.log_file(&id), &header)
                .await
                .map_err(|e| e.to_string())?;
            // Arrêt demandé pendant l'ouverture du port : le site est refermé.
            let site = {
                let mut st = self.lock();
                if st.ops.get(&id) == Some(&op) {
                    st.statics.insert(id.clone(), site);
                    None
                } else {
                    Some(site)
                }
            };
            if let Some(site) = site {
                site.stop().await;
                return Ok(());
            }
            let mut st = self.lock();
            self.set(&mut st, &id, |s| {
                s.state = State::Running;
                s.managed = true;
                s.pid = Some(std::process::id());
                s.active_port = Some(port);
                s.active_host = Some(host.clone());
                s.started_at = Some(started_at);
            });
            return Ok(());
        }
        if !self.has_op(&id, op) {
            return Ok(());
        }
        let pid = self.spawn(p).await?;
        if !self.has_op(&id, op) {
            // Arrêt demandé pendant le lancement.
            let _ = self.inner.platform.kill_tree(pid).await;
            return Ok(());
        }
        {
            let mut st = self.lock();
            if let Some(c) = self.inner.platform.process_start(pid) {
                st.created.insert(id.clone(), c);
            }
            self.set(&mut st, &id, |s| {
                s.active_port = Some(port);
                s.active_host = Some(host.clone());
                s.started_at = Some(started_at);
                s.managed = true;
                s.pid = Some(pid);
            });
            self.save_pids(&st);
            if let Some(code) = st.early_exits.remove(&pid) {
                self.exited(&mut st, &id, pid, code);
            }
        }
        self.wait_healthy(p, pid).await
    }

    /// Date lisible de l'en-tête des journaux, dans la langue du serveur.
    fn stamp(&self) -> String {
        let now = chrono::Local::now();
        match self.lang() {
            Lang::Fr => now.format("%d/%m/%Y %H:%M:%S").to_string(),
            Lang::En => now.format("%-m/%-d/%Y, %-I:%M:%S %p").to_string(),
        }
    }

    async fn spawn(&self, p: &Value) -> Result<u32, String> {
        let id = id_of(p);
        let log = self.log_file(&id);
        let _ = std::fs::rename(&log, self.prev_log_file(&id));
        let dir = &self.inner.servynex_dir;
        let mut argv = vec![expand(p["cmd"].as_str().unwrap_or_default(), p, dir)];
        argv.extend(
            p["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|a| expand(a, p, dir)),
        );
        let port = port_of(p).unwrap_or(0);
        let header = format!(
            "[servynex] {} · {} · {}:{port}\n",
            self.stamp(),
            argv.join(" "),
            host_of(p)
        );
        std::fs::write(&log, header).map_err(|e| e.to_string())?;

        let mut env = servynex_sys::clean_env();
        for (k, v) in p.get("env").and_then(Value::as_object).into_iter().flatten() {
            env.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
        }
        env.insert("PORT".into(), port.to_string());
        env.insert("NO_COLOR".into(), "1".into());
        env.retain(|k, _| !k.eq_ignore_ascii_case("HOST"));
        // HOST vide = toutes les interfaces (IPv4 et IPv6) pour les projets Node.
        if p.get("hostControl").and_then(Value::as_str) == Some("env")
            && p.get("expose") != Some(&Value::Bool(true))
        {
            env.insert("HOST".into(), "127.0.0.1".into());
        }

        let weak = Arc::downgrade(&self.inner);
        let target = id.clone();
        let options = LaunchOptions {
            cwd: PathBuf::from(p["cwd"].as_str().unwrap_or_default()),
            env,
            log,
            on_exit: Box::new(move |pid, code| {
                if let Some(inner) = weak.upgrade() {
                    Manager { inner }.on_exit(&target, pid, code);
                }
            }),
        };
        // Sortie surveillée dès le lancement : la fin la plus rapide (commande introuvable) peut
        // arriver avant que le PID soit connu ici.
        let pid = self
            .inner
            .platform
            .launch(argv, options)
            .await
            .map_err(|e| match e {
                LaunchError::NotFound(cmd) => self.t("process.commandNotFound", params![cmd = cmd]),
                LaunchError::Other(text) => text,
            })?;
        if self.inner.platform.is_alive(pid) {
            self.lock().watched.insert(pid);
        }
        Ok(pid)
    }

    async fn wait_healthy(&self, p: &Value, pid: u32) -> Result<(), String> {
        let id = id_of(p);
        let timeout = p
            .get("startTimeout")
            .and_then(Value::as_u64)
            .map_or(self.inner.timings.start_timeout, Duration::from_secs);
        let deadline = Instant::now() + timeout;
        let port = port_of(p).unwrap_or(0);
        while Instant::now() < deadline {
            let s = self.state(&id);
            if s.pid != Some(pid) || s.state != State::Starting {
                return Ok(()); // arrêté ou mort entre-temps
            }
            match self.health(p, port).await {
                Health::Ok => {
                    let mut st = self.lock();
                    if st
                        .states
                        .get(&id)
                        .is_some_and(|s| s.pid == Some(pid) && s.state == State::Starting)
                    {
                        self.set(&mut st, &id, |s| s.state = State::Running);
                    }
                    return Ok(());
                }
                Health::Mismatch => {
                    return Err(self.t("process.wrongService", params![port = port, name = name_of(p)]));
                }
                Health::Down => tokio::time::sleep(Duration::from_millis(600)).await,
            }
        }
        let seconds = timeout.as_secs_f64();
        Err(self.t(
            "process.timeout",
            params![name = name_of(p), port = port, seconds = seconds],
        ))
    }

    /// Arrête un projet (et, `cascade`, les dépendances lancées pour lui et devenues inutiles) :
    /// commande d'arrêt propre si le projet en déclare une, sinon (ou si elle ne suffit pas) arrêt
    /// forcé de toute son arborescence.
    pub async fn stop(&self, id: &str) -> Result<(), LocalError> {
        self.stop_with(id, false, None).await
    }

    pub async fn stop_cascade(&self, id: &str) -> Result<(), LocalError> {
        self.stop_with(id, true, None).await
    }

    pub fn stop_with<'a>(
        &'a self,
        id: &'a str,
        cascade: bool,
        reason: Option<String>,
    ) -> servynex_sys::BoxFuture<'a, Result<(), LocalError>> {
        Box::pin(async move {
            let (p, before, op) = {
                let mut st = self.lock();
                let p = project_in(&st.config, id)?;
                if kind_of(&p) == Some("file") {
                    return Err(self.refuse(&p, false, "process.why.noProcess", params![]));
                }
                let before = st.states.get(id).cloned().unwrap_or_default();
                if st.ops.contains_key(id) && before.state != State::Starting {
                    return Err(self.refuse(&p, false, "process.why.busy", params![]));
                }
                // Projet planté dont la relance est prévue : l'arrêter, c'est l'annuler.
                cancel_restart(&mut st, id);
                if before.auto_restart == Some(AutoRestart::Scheduled) {
                    self.set(&mut st, id, |s| {
                        s.detail = None;
                        s.auto_restart = None;
                    });
                    return Ok(());
                }
                if before.state == State::Stopped || (before.state == State::Failed && !before.managed) {
                    return Err(self.refuse(&p, false, "process.why.stopped", params![]));
                }
                // Sans PID connu, le port désigne le processus : pas question de tuer un programme étranger.
                if before.state == State::Conflict && !before.managed {
                    let port = port_of(&p).unwrap_or(0);
                    return Err(self.refuse(&p, false, "process.why.portTaken", params![port = port]));
                }
                st.next_op += 1;
                let op = st.next_op;
                st.ops.insert(id.to_string(), op); // remplace un démarrage en cours, qui s'interrompt
                st.relaunch.remove(id);
                self.set(&mut st, id, |s| {
                    s.state = State::Stopping;
                    s.error = None;
                    s.detail = reason.clone();
                });
                (p, before, op)
            };
            let result = self.stop_inner(id, &p, &before).await;
            {
                let mut st = self.lock();
                if let Err(error) = &result {
                    let state = if before.state == State::Starting {
                        State::Failed
                    } else {
                        before.state
                    };
                    self.set(&mut st, id, |s| {
                        s.state = state;
                        s.error = Some(error.clone());
                    });
                }
                if st.ops.get(id) == Some(&op) {
                    st.ops.remove(id);
                }
            }
            match result {
                Ok(()) => {
                    if cascade {
                        self.stop_unused_dependencies(&p).await;
                    }
                    Ok(())
                }
                Err(error) => Err(LocalError::new(500, "raw", params![text = error])),
            }
        })
    }

    async fn stop_inner(&self, id: &str, p: &Value, before: &ProjectState) -> Result<(), String> {
        if kind_of(p) == Some("static") {
            let site = self.lock().statics.remove(id);
            if let Some(site) = site {
                site.stop().await;
            }
            let mut st = self.lock();
            self.set(&mut st, id, |s| *s = ProjectState::default());
            return Ok(());
        }
        let platform = &self.inner.platform;
        let pid = if before.managed {
            before.pid
        } else {
            self.pid_on_port(before.active_port.or_else(|| port_of(p)).unwrap_or(0))
                .await
        };
        if let (Some(pid), Some(stop)) = (pid, p.get("stop")) {
            let dir = &self.inner.servynex_dir;
            let cmd = expand(stop["cmd"].as_str().unwrap_or_default(), p, dir);
            let args: Vec<String> = stop["args"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|a| expand(a, p, dir))
                .collect();
            let _ = servynex_sys::run_command()(cmd, args).await;
            for _ in 0..60 {
                if !platform.is_alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
        if let Some(pid) = pid
            && platform.is_alive(pid)
        {
            platform
                .kill_tree(pid)
                .await
                .map_err(|_| self.t("system.killFailed", params![pid = pid]))?;
        }
        let mut st = self.lock();
        if let (true, Some(pid)) = (before.managed, pid) {
            self.exited(&mut st, id, pid, None);
        }
        self.set(&mut st, id, |s| *s = ProjectState::default());
        st.created.remove(id);
        self.save_pids(&st);
        Ok(())
    }

    /// Dépendances lancées automatiquement pour `p`, qu'aucun autre projet actif n'utilise.
    async fn stop_unused_dependencies(&self, p: &Value) {
        let own = id_of(p);
        for dep_id in requires_of(p) {
            let unused = {
                let st = self.lock();
                let s = st.states.get(&dep_id).cloned().unwrap_or_default();
                let exists = project_in(&st.config, &dep_id).is_ok();
                let used = st.config["projects"].as_array().into_iter().flatten().any(|x| {
                    let xid = id_of(x);
                    xid != own
                        && requires_of(x).contains(&dep_id)
                        && st.states.get(&xid).is_some_and(|xs| xs.state.active())
                });
                exists
                    && s.dependency
                    && s.managed
                    && s.state == State::Running
                    && !st.ops.contains_key(&dep_id)
                    && !used
            };
            if !unused {
                continue;
            }
            let reason = self.t("process.unusedBy", params![name = name_of(p)]);
            if let Err(err) = self.stop_with(&dep_id, true, Some(reason)).await {
                self.warn(format!("[stop] {dep_id} : {}", err.message_in(self.lang())));
            }
        }
    }

    /// Ouvre un projet « file » avec l'application par défaut (fichier vérifié d'abord).
    pub fn open_file(&self, id: &str) -> Result<(), LocalError> {
        let p = self.project(id)?;
        if kind_of(&p) != Some("file") {
            return Err(LocalError::new(
                400,
                "process.notAFile",
                params![name = name_of(&p)],
            ));
        }
        let file =
            Path::new(p["cwd"].as_str().unwrap_or_default()).join(p["open"].as_str().unwrap_or_default());
        if !file.exists() {
            return Err(LocalError::new(
                404,
                "process.fileMissing",
                params![path = file.to_string_lossy().into_owned()],
            ));
        }
        self.inner
            .platform
            .open_file(&file)
            .map_err(|e| LocalError::new(500, "raw", params![text = e]))
    }

    /// Arrête (si besoin) puis relance ; la suite arrive par les évènements.
    pub async fn restart(&self, id: &str) -> Result<(), LocalError> {
        let s = self.state(id);
        if s.state != State::Stopped && !(s.state == State::Failed && !s.managed) {
            self.stop(id).await?;
        }
        self.start(id)
    }
}

/// Projet d'une configuration (erreur 404 s'il n'existe pas).
fn project_in(config: &Value, id: &str) -> Result<Value, LocalError> {
    config["projects"]
        .as_array()
        .and_then(|l| l.iter().find(|p| p["id"] == id))
        .cloned()
        .ok_or_else(|| LocalError::new(404, "project.unknown", params![]))
}

/// Oublie la relance prévue et les plantages comptés (lancement ou arrêt demandé, réglage retiré).
fn cancel_restart(st: &mut St, id: &str) {
    if let Some(retry) = st.retries.remove(id)
        && let Some(timer) = retry.timer
    {
        timer.abort();
    }
}

/// Le processus vivant sous ce PID est-il bien celui qui a été enregistré ? Avec `createdAt`
/// (version Rust) : même heure de création, à une seconde près. Sans (fichier de la version Node) :
/// créé au plus 10 s après le lancement enregistré. Heure illisible : repris seulement sans
/// `createdAt`, comme la version Node le faisait.
fn same_process(platform: &dyn Platform, pid: u32, saved: &Value) -> bool {
    let created = platform.process_start(pid);
    match (saved["createdAt"].as_i64(), created) {
        (Some(expected), Some(actual)) => (expected - actual).abs() <= 1000,
        (Some(_), None) => false,
        (None, Some(actual)) => saved["startedAt"]
            .as_i64()
            .is_none_or(|started| actual <= started + 10_000),
        (None, None) => true,
    }
}
