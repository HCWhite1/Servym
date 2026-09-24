//! Gestionnaire des projets : lancement, surveillance, arrêt, dépendances, relance après
//! plantage, reprise après un redémarrage de Servynex.
//!
//! L'état est partagé derrière un verrou jamais tenu pendant une attente (`.await`) ; chaque
//! action porte un numéro de génération, et les changements sont diffusés par [`Event`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::i18n::{Lang, LocalError};

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
    /// Délai de grâce avant de lire la fin d'un journal après une sortie.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
    Conflict,
    /// Projet « file » : jamais lancé.
    File,
}

/// Relance automatique après un plantage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoRestart {
    Scheduled,
    GaveUp,
}

/// État en direct d'un projet.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectState {
    pub state: State,
    /// Lancé (ou repris) par Servynex : arrêtable.
    pub managed: bool,
    pub pid: Option<u32>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub detail: Option<String>,
    /// Lancé comme dépendance d'un autre projet (arrêté avec lui).
    pub dependency: bool,
    pub auto_restart: Option<AutoRestart>,
}

/// Changement d'état inscrit à l'historique.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Transition {
    pub id: String,
    pub from: State,
    pub to: State,
    pub detail: Option<String>,
}

/// Évènements diffusés aux flux SSE et au serveur.
#[derive(Clone, Debug)]
pub enum Event {
    /// État d'un projet changé (vue complète du projet).
    Change(Value),
    Transition(Transition),
    /// Configuration remplacée.
    Config,
    /// Avertissement à afficher.
    Warning(String),
}

/// Gestionnaire (clonable : les clones partagent le même état).
#[derive(Clone)]
pub struct Manager {
    _inner: Arc<()>,
}

/// Construction d'un [`Manager`].
pub struct ManagerBuilder {
    config: Value,
    logs_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    timings: Timings,
    boot_time: Option<i64>,
    lang: Lang,
    platform: Option<Arc<dyn servynex_sys::Platform>>,
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
    pub fn platform(mut self, platform: Arc<dyn servynex_sys::Platform>) -> Self {
        self.platform = Some(platform);
        self
    }
    pub fn build(self) -> Manager {
        let _ = (
            self.config,
            self.logs_dir,
            self.runtime_dir,
            self.timings,
            self.boot_time,
            self.lang,
            self.platform,
        );
        unimplemented!("phase 4")
    }
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
        }
    }

    /// Reprise des projets en marche (`pids.json`), première sonde, démarrage automatique.
    pub async fn init(&self) {
        unimplemented!("phase 4")
    }

    /// Arrête la surveillance ; les projets continuent de tourner.
    pub async fn close(&self) {
        unimplemented!("phase 4")
    }

    pub async fn start(&self, id: &str) -> Result<(), LocalError> {
        let _ = id;
        unimplemented!("phase 4")
    }

    pub async fn stop(&self, id: &str) -> Result<(), LocalError> {
        let _ = id;
        unimplemented!("phase 4")
    }

    pub fn state(&self, id: &str) -> ProjectState {
        let _ = id;
        unimplemented!("phase 4")
    }

    pub fn config(&self) -> Value {
        unimplemented!("phase 4")
    }

    /// Remplace la configuration ; refusé (409) si un projet en marche en serait retiré.
    pub fn set_config(&self, config: Value) -> Result<(), LocalError> {
        let _ = config;
        unimplemented!("phase 4")
    }

    /// Projet de la configuration (erreur 404 s'il n'existe pas).
    pub fn project(&self, id: &str) -> Result<Value, LocalError> {
        let _ = id;
        unimplemented!("phase 4")
    }

    /// Vue d'un projet pour l'interface (réglages et état en direct).
    pub fn view(&self, project: &Value) -> Value {
        let _ = project;
        unimplemented!("phase 4")
    }

    pub fn log_file(&self, id: &str) -> PathBuf {
        let _ = id;
        unimplemented!("phase 4")
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        let _ = &self._inner;
        unimplemented!("phase 4")
    }
}
