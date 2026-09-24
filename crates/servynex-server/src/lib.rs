//! Serveur HTTP (axum) : API, flux SSE, interface embarquée, sites statiques, GitHub.
//!
//! [`App::builder`] reprend les points d'injection de `createApp()` de la version Node : horloge,
//! relevés, notificateur, démarrage avec la session, cadences, visiteur distant…

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use servynex_core::i18n::Lang;
use servynex_core::manager::{Manager, Timings};
use servynex_core::stats::SampleFn;
use servynex_sys::notify::Notifier;
use servynex_sys::{Autostart, Platform};

pub mod testing;

/// Adresse d'écoute réelle : ce PC seulement, ou le réseau local.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Listening {
    #[default]
    Local,
    Lan,
}

/// Adresses d'un fournisseur de connexion (tests : faux GitHub local).
#[derive(Clone, Debug)]
pub struct ProviderUrls {
    pub authorize: String,
    pub token: String,
    pub user: String,
}

/// Adresse d'écoute : le réseau seulement avec la protection par mot de passe.
pub fn bind_host(instance: &Value, protected_by_password: bool) -> &'static str {
    let _ = (instance, protected_by_password);
    unimplemented!("phase 5")
}

/// Servynex est-il lancé par un superviseur qui le relancera (code 75) ?
/// `SERVYNEX_SUPERVISOR` : `servynex` (mode `--supervised`) ou `systemd`.
pub fn is_supervised(supervisor: Option<&str>) -> bool {
    let _ = supervisor;
    unimplemented!("phase 5")
}

/// Horloge de l'authentification (ms).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Construction de l'application.
#[derive(Default)]
pub struct AppBuilder {
    config_file: Option<PathBuf>,
    logs_dir: Option<PathBuf>,
    runtime_dir: Option<PathBuf>,
    restart: Option<Arc<dyn Fn() + Send + Sync>>,
    autostart: Option<Arc<dyn Autostart>>,
    clock: Option<Clock>,
    sample: Option<SampleFn>,
    github: Option<ProviderUrls>,
    listening: Listening,
    hostnames: Option<Arc<dyn Fn() -> Vec<String> + Send + Sync>>,
    local_viewer: Option<Arc<dyn Fn(IpAddr) -> bool + Send + Sync>>,
    notifier: Option<Arc<dyn Notifier>>,
    platform_name: Option<String>,
    desktop: Option<bool>,
    watch: Option<Duration>,
    timings: Timings,
    boot_time: Option<i64>,
    system_lang: Option<Lang>,
    platform: Option<Arc<dyn Platform>>,
}

impl AppBuilder {
    pub fn config_file(mut self, file: &Path) -> Self {
        self.config_file = Some(file.into());
        self
    }
    pub fn logs_dir(mut self, dir: &Path) -> Self {
        self.logs_dir = Some(dir.into());
        self
    }
    pub fn runtime_dir(mut self, dir: &Path) -> Self {
        self.runtime_dir = Some(dir.into());
        self
    }
    /// Données dans un seul dossier : `projects.json`, `logs/`, `runtime/`.
    pub fn data_dir(self, dir: &Path) -> Self {
        self.config_file(&dir.join("projects.json"))
            .logs_dir(&dir.join("logs"))
            .runtime_dir(&dir.join("runtime"))
    }
    /// Arrêt de Servynex pour qu'un superviseur le relance (absent : impossible).
    pub fn restart(mut self, restart: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.restart = Some(restart);
        self
    }
    pub fn autostart(mut self, autostart: Arc<dyn Autostart>) -> Self {
        self.autostart = Some(autostart);
        self
    }
    pub fn clock(mut self, clock: Clock) -> Self {
        self.clock = Some(clock);
        self
    }
    /// Relevé des processus (processeur et mémoire).
    pub fn sample(mut self, sample: SampleFn) -> Self {
        self.sample = Some(sample);
        self
    }
    pub fn github(mut self, urls: ProviderUrls) -> Self {
        self.github = Some(urls);
        self
    }
    pub fn listening(mut self, listening: Listening) -> Self {
        self.listening = listening;
        self
    }
    /// Noms de cette machine sur le réseau (adresses acceptées dans `Host`).
    pub fn hostnames(mut self, hostnames: Arc<dyn Fn() -> Vec<String> + Send + Sync>) -> Self {
        self.hostnames = Some(hostnames);
        self
    }
    /// La page est-elle ouverte sur cette machine ?
    pub fn local_viewer(mut self, local: Arc<dyn Fn(IpAddr) -> bool + Send + Sync>) -> Self {
        self.local_viewer = Some(local);
        self
    }
    pub fn notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }
    /// Système annoncé à la page (« windows », « linux »).
    pub fn platform_name(mut self, name: &str) -> Self {
        self.platform_name = Some(name.into());
        self
    }
    /// Session avec écran : sinon rien ne peut s'ouvrir sur cette machine.
    pub fn desktop(mut self, desktop: bool) -> Self {
        self.desktop = Some(desktop);
        self
    }
    /// Relecture automatique de `projects.json` (zéro : aucune).
    pub fn watch(mut self, interval: Duration) -> Self {
        self.watch = Some(interval);
        self
    }
    pub fn timings(mut self, timings: Timings) -> Self {
        self.timings = timings;
        self
    }
    pub fn boot_time(mut self, ms: i64) -> Self {
        self.boot_time = Some(ms);
        self
    }
    /// Langue du système (sans `instance.lang`) : `SERVYNEX_LANG`, sinon celle de la machine.
    pub fn system_lang(mut self, lang: Lang) -> Self {
        self.system_lang = Some(lang);
        self
    }
    pub fn platform(mut self, platform: Arc<dyn Platform>) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Démarre le serveur sur `addr` (« 127.0.0.1:0 » : port libre). Le gestionnaire n'est pas
    /// encore initialisé : voir [`Manager::init`].
    pub async fn listen(self, addr: &str) -> std::io::Result<RunningApp> {
        let _ = addr;
        let _ = (
            self.config_file,
            self.logs_dir,
            self.runtime_dir,
            self.restart,
            self.autostart,
            self.clock,
            self.sample,
            self.github,
            self.listening,
            self.hostnames,
            self.local_viewer,
            self.notifier,
            self.platform_name,
            self.desktop,
            self.watch,
            self.timings,
            self.boot_time,
            self.system_lang,
            self.platform,
        );
        unimplemented!("phase 5")
    }
}

/// L'application.
pub struct App;

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }
}

/// Application en service.
pub struct RunningApp {
    pub port: u16,
    pub manager: Manager,
}

impl RunningApp {
    /// Code de réinitialisation d'un compte (comme `servynex auth reset-password`).
    pub fn create_reset_code(&self, username: &str, now: i64) -> String {
        let _ = (username, now);
        unimplemented!("phase 5")
    }

    /// Ferme le serveur et ses connexions, puis le gestionnaire (les projets continuent).
    pub async fn close(self) {
        unimplemented!("phase 5")
    }
}
