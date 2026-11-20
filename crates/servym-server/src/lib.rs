//! Serveur HTTP (axum) : API, flux SSE, interface embarquée, sites statiques, GitHub.
//!
//! [`App::builder`] reprend les points d'injection de `createApp()` de la version Node : horloge,
//! relevés, notificateur, démarrage avec la session, cadences, visiteur distant…

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::Method;
use axum::response::Response;
use serde_json::Value;
use servym_core::auth::{AuthStore, create_reset_code};
use servym_core::config::{default_config, load_config, save_config};
use servym_core::i18n::{Lang, LocalError, detect_lang};
use servym_core::manager::{Manager, Timings};
use servym_core::params;
use servym_core::stats::{SampleFn, Sampler};
use servym_sys::notify::{Notifier, Os, notifier_for};
use servym_sys::{Autostart, Platform};
use tokio_util::sync::CancellationToken;

mod api;
mod app;
mod auth;
pub mod cli;
pub mod http;
pub mod supervisor;
pub mod testing;
mod ui;

use app::{App as AppRef, AppInner};

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

/// Adresse d'écoute : tout le réseau seulement si l'accès réseau est demandé ET la protection par
/// mot de passe active ; sinon n'importe qui sur le réseau aurait la main.
pub fn bind_host(instance: &Value, protected_by_password: bool) -> &'static str {
    if instance["access"] == "lan" && protected_by_password {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    }
}

/// Servym est-il lancé par un superviseur qui le relancera (code 75) ?
/// `SERVYM_SUPERVISOR` : `servym` (mode `--supervised`) ou `systemd`.
pub fn is_supervised(supervisor: Option<&str>) -> bool {
    matches!(supervisor, Some("servym" | "systemd"))
}

/// Code de sortie qui demande au superviseur une relance immédiate.
pub const RESTART_CODE: i32 = 75;
/// Même chose, en rouvrant la fenêtre de l'application de bureau (elle était ouverte).
pub const RESTART_WINDOW_CODE: i32 = 76;

/// Horloge de l'authentification (ms).
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Application de bureau autour du serveur. Sa présence ajoute à l'interface le choix « fenêtre
/// ou navigateur » (réglage de la machine, `runtime/ui.json`).
pub trait Shell: Send + Sync {
    /// Ouvre Servym dans le navigateur par défaut.
    fn open_browser(&self);
    /// Ferme la fenêtre de l'application (l'application reste dans la zone de notification).
    fn close_window(&self) {}
    /// Ferme l'application de bureau (les projets lancés continuent de tourner).
    fn quit(&self) {}
}

/// Action lancée depuis la zone de notification : sur la machine hôte, sans compte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayAction {
    Start,
    Restart,
    Stop,
}

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
    engine: Option<String>,
    desktop: Option<bool>,
    shell: Option<Arc<dyn Shell>>,
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
    /// Arrêt de Servym pour qu'un superviseur le relance (absent : impossible).
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
    /// Ligne « Moteur » des Paramètres (par défaut : « Rust »).
    pub fn engine(mut self, engine: &str) -> Self {
        self.engine = Some(engine.into());
        self
    }
    /// Système annoncé à la page (« win32 », « linux »).
    pub fn platform_name(mut self, name: &str) -> Self {
        self.platform_name = Some(name.into());
        self
    }
    /// Session avec écran : sinon rien ne peut s'ouvrir sur cette machine.
    pub fn desktop(mut self, desktop: bool) -> Self {
        self.desktop = Some(desktop);
        self
    }
    /// Lancé par l'application de bureau.
    pub fn shell(mut self, shell: Arc<dyn Shell>) -> Self {
        self.shell = Some(shell);
        self
    }
    /// Relecture automatique de `projects.json` (zéro : aucune ; par défaut : chaque seconde).
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
    /// Langue du système (sans `instance.lang`) : `SERVYM_LANG`, sinon celle de la machine.
    pub fn system_lang(mut self, lang: Lang) -> Self {
        self.system_lang = Some(lang);
        self
    }
    pub fn platform(mut self, platform: Arc<dyn Platform>) -> Self {
        self.platform = Some(platform);
        self
    }

    /// Construit l'application (configuration lue, gestionnaire prêt mais pas initialisé).
    pub fn build(self) -> App {
        let config_file = self.config_file.unwrap_or_else(|| PathBuf::from("projects.json"));
        let data_dir = config_file.parent().map(Path::to_path_buf).unwrap_or_default();
        let logs_dir = self.logs_dir.unwrap_or_else(|| data_dir.join("logs"));
        let runtime_dir = self.runtime_dir.unwrap_or_else(|| data_dir.join("runtime"));
        let _ = std::fs::create_dir_all(&logs_dir);
        let _ = std::fs::create_dir_all(&runtime_dir);
        let valid_copy = runtime_dir.join("projects.valid.json");
        // projects.json invalide au démarrage : Servym démarre quand même, avec la dernière
        // configuration valide, et l'interface affiche l'erreur (mode dégradé).
        let (initial, config_error) = match load_config(&config_file) {
            Ok(config) => (config, None),
            Err(error) => (
                load_config(&valid_copy).unwrap_or_else(|_| default_config()),
                Some(error),
            ),
        };
        let system_lang = self.system_lang.unwrap_or_else(|| {
            detect_lang(
                servym_sys::system_locale().as_deref(),
                std::env::var("SERVYM_LANG").ok().as_deref(),
            )
        });
        let lang = initial["instance"]["lang"]
            .as_str()
            .and_then(Lang::pick)
            .unwrap_or(system_lang);
        let platform = self.platform.unwrap_or_else(servym_sys::native);
        let mut manager = Manager::builder(initial.clone())
            .logs_dir(&logs_dir)
            .runtime_dir(&runtime_dir)
            .timings(self.timings)
            .lang(lang)
            .platform(platform.clone())
            .servym_dir(&data_dir);
        if let Some(boot) = self.boot_time {
            manager = manager.boot_time(boot);
        }
        let manager = manager.build();
        let clock: Clock = self.clock.unwrap_or_else(|| {
            Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64)
            })
        });
        let sample: SampleFn = self.sample.unwrap_or_else(|| {
            let p = platform.clone();
            Arc::new(move || {
                let p = p.clone();
                Box::pin(async move { p.sample_processes().await })
            })
        });
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
        let now: servym_core::stats::NowFn = Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64)
        });
        let own_log = logs_dir.join("servym.log");
        let own_log_size = std::fs::metadata(&own_log).map_or(0, |m| m.len());
        let (instance_events, _) = tokio::sync::broadcast::channel(16);
        let (autostart_events, _) = tokio::sync::broadcast::channel(16);
        let hostnames = self.hostnames.unwrap_or_else(|| Arc::new(default_hostnames));
        let desktop = self.desktop.unwrap_or_else(|| platform.tools().desktop);
        let inner = AppInner {
            known: Mutex::new(std::fs::read_to_string(&config_file).ok()),
            ui_file: runtime_dir.join("ui.json"),
            own_prev_log: logs_dir.join("servym.prev.log"),
            own_log,
            valid_copy,
            auth: tokio::sync::Mutex::new(auth::AuthCore::new(AuthStore::new(
                &runtime_dir.join("auth.json"),
            ))),
            config_error: Mutex::new(config_error.clone()),
            config_write: Mutex::default(),
            own_log_size: Mutex::new(own_log_size),
            actors: Mutex::default(),
            streams: Mutex::default(),
            instance_events,
            autostart_events,
            sampler: Sampler::new(sample, now, cores, Duration::from_secs(2)),
            git_cache: tokio::sync::Mutex::default(),
            network: tokio::sync::Mutex::default(),
            restart: self.restart,
            autostart: self.autostart.unwrap_or_else(|| default_autostart(&data_dir)),
            clock,
            notifier: self
                .notifier
                .unwrap_or_else(|| Arc::from(notifier_for(Os::current(), servym_sys::run_command()))),
            listening: Mutex::new(self.listening),
            hostnames,
            local_viewer: self
                .local_viewer
                .unwrap_or_else(|| Arc::new(|ip: IpAddr| ip.is_loopback() || is_mapped_loopback(ip))),
            engine: self.engine.unwrap_or_else(|| "Rust".into()),
            platform_name: self.platform_name.unwrap_or_else(|| {
                if cfg!(windows) {
                    "win32".into()
                } else {
                    std::env::consts::OS.into()
                }
            }),
            desktop,
            shell: self.shell,
            system_lang,
            providers: auth::providers(self.github.as_ref()),
            platform,
            manager,
            config_file,
            runtime_dir,
            data_dir,
        };
        let app = Arc::new(inner);
        app::spawn_event_log(&app);
        app::spawn_watcher(&app, self.watch.unwrap_or(Duration::from_secs(1)));
        match &config_error {
            Some(error) => {
                let text = app.tr("log.degraded", params![error = error.message_in(app.lang())]);
                app.log(&format!("[config] {text}"));
            }
            None => {
                let _ = save_config(&app.valid_copy, &initial);
            }
        }
        App { inner: app }
    }

    /// Construit l'application et démarre le serveur sur `addr` (« 127.0.0.1:0 » : port libre).
    /// Le gestionnaire n'est pas encore initialisé : voir [`Manager::init`].
    pub async fn listen(self, addr: &str) -> std::io::Result<RunningApp> {
        self.build().listen(addr).await
    }
}

fn is_mapped_loopback(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()))
}

/// Noms de cette machine sur le réseau : ses adresses IPv4 et son nom d'ordinateur.
fn default_hostnames() -> Vec<String> {
    let mut names: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| i.ip().is_ipv4() && !i.is_loopback())
        .map(|i| i.ip().to_string())
        .collect();
    if let Some(host) = servym_sys::hostname() {
        names.push(host);
    }
    names
}

/// L'application, prête à écouter (clonable : les clones partagent le même état).
#[derive(Clone)]
pub struct App {
    inner: AppRef,
}

impl App {
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }

    pub fn manager(&self) -> &Manager {
        &self.inner.manager
    }

    /// Protection par mot de passe active (choix de l'adresse d'écoute).
    pub async fn protected(&self) -> bool {
        self.inner.auth_enabled().await
    }

    /// Adresse d'écoute choisie au démarrage (`bind_host`), une fois la configuration lue.
    pub fn set_listening(&self, listening: Listening) {
        *self.inner.listening.lock().unwrap_or_else(|e| e.into_inner()) = listening;
    }

    /// Ligne du journal de Servym.
    pub fn log(&self, text: &str) {
        self.inner.log(text);
    }

    pub fn tr(&self, key: &str) -> String {
        self.inner.tr(key, params![])
    }

    /// Dossier des données (`projects.json`, `logs/`, `runtime/`…).
    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    /// Où ouvrir l'interface : `window` (par défaut) ou `browser`.
    pub fn opener(&self) -> &'static str {
        self.inner.opener()
    }

    pub fn autostart(&self) -> Arc<dyn Autostart> {
        self.inner.autostart.clone()
    }

    /// Abonnement aux changements du démarrage avec la session.
    pub fn subscribe_autostart(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.inner.autostart_events.subscribe()
    }

    /// Signale à la zone de notification de relire l'état du démarrage avec la session.
    pub fn notify_autostart(&self) {
        let _ = self.inner.autostart_events.send(());
    }

    /// Démarrer, redémarrer ou arrêter depuis la zone de notification : même chemin que l'interface,
    /// inscrit au journal et à l'historique « via la zone de notification ». Erreur : message dans
    /// la langue de Servym.
    pub async fn tray_action(&self, id: &str, action: TrayAction) -> Result<(), String> {
        let app = &self.inner;
        let name = match action {
            TrayAction::Start => "start",
            TrayAction::Restart => "restart",
            TrayAction::Stop => "stop",
        };
        app.note_actor(id, name, Some(app.tr("history.viaTray", params![])));
        let result = match action {
            TrayAction::Start => app.manager.start(id),
            TrayAction::Restart => app.manager.restart(id).await,
            TrayAction::Stop => app.manager.stop_cascade(id).await,
        };
        result.map_err(|e| e.message_in(app.lang()))
    }

    /// Texte du serveur avec paramètres, dans sa langue.
    pub fn tr_with(&self, key: &str, params: servym_core::i18n::Params) -> String {
        self.inner.tr(key, params)
    }

    pub async fn listen(&self, addr: &str) -> std::io::Result<RunningApp> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let port = listener.local_addr()?.port();
        let router = axum::Router::new()
            .fallback(handle)
            .with_state(self.inner.clone());
        let shutdown = CancellationToken::new();
        let stop = shutdown.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move { stop.cancelled().await })
            .await;
        });
        Ok(RunningApp {
            port,
            manager: self.inner.manager.clone(),
            app: self.inner.clone(),
            shutdown,
            task,
        })
    }
}

async fn handle(
    State(app): State<AppRef>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request,
) -> Response {
    let mut ctx = http::Ctx::new(req, peer);
    let mut res = match route(&app, &mut ctx).await {
        Ok(res) => res,
        Err(error) => {
            if error.status == 500 {
                app.log(&format!(
                    "[error] {} {} {}",
                    ctx.method,
                    ctx.url,
                    error.message_in(app.lang())
                ));
            }
            // Message dans la langue de la page (Accept-Language), sinon celle du serveur.
            http::error_response(&error, ctx.lang(app.lang()))
        }
    };
    http::security_headers(res.headers_mut());
    res
}

async fn route(app: &AppRef, ctx: &mut http::Ctx) -> Result<Response, LocalError> {
    let lan = *app.listening.lock().unwrap_or_else(|e| e.into_inner()) == Listening::Lan;
    let names = lan.then(|| (app.hostnames)());
    http::guard(ctx, names.as_deref())?;
    if ctx.path.starts_with("/api/") {
        return api::api(app, ctx).await;
    }
    // Retour d'un fournisseur de connexion : navigation venue de son site, hors de /api/.
    let segments: Vec<&str> = ctx.path.split('/').collect();
    if segments.get(1) == Some(&"auth") && segments.get(3) == Some(&"callback") && ctx.method == Method::GET {
        let provider = segments[2].to_string();
        return Ok(app.oauth_callback(ctx, &provider).await);
    }
    if let Some(res) = ui::serve(&ctx.method, &ctx.path) {
        return Ok(res);
    }
    Err(http::err(404, "http.notFound"))
}

/// Application en service.
pub struct RunningApp {
    pub port: u16,
    pub manager: Manager,
    app: AppRef,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl RunningApp {
    /// Code de réinitialisation d'un compte (comme `servym auth reset-password`).
    pub fn create_reset_code(&self, username: &str, now: i64) -> String {
        let mut core = self.app.auth.blocking_lock_or_try();
        create_reset_code(&mut core.store, username, now).expect("compte connu")
    }

    /// Ligne du journal de Servym.
    pub fn log(&self, text: &str) {
        self.app.log(text);
    }

    /// Attend la fin du serveur.
    pub async fn wait(&mut self) {
        let _ = (&mut self.task).await;
    }

    /// Ferme le serveur et ses connexions (flux compris), puis le gestionnaire (les projets
    /// continuent).
    pub async fn close(self) {
        let sessions: Vec<String> = self
            .app
            .streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        self.app.revoke_streams(&sessions);
        self.shutdown.cancel();
        let mut task = self.task;
        if tokio::time::timeout(Duration::from_secs(2), &mut task)
            .await
            .is_err()
        {
            task.abort();
        }
        self.app.manager.close().await;
    }
}

/// Verrou du fichier des comptes pris depuis un contexte synchrone (tests, commande `auth`).
trait LockOrTry<T> {
    fn blocking_lock_or_try(&self) -> tokio::sync::MutexGuard<'_, T>;
}

impl<T> LockOrTry<T> for tokio::sync::Mutex<T> {
    fn blocking_lock_or_try(&self) -> tokio::sync::MutexGuard<'_, T> {
        loop {
            if let Ok(guard) = self.try_lock() {
                return guard;
            }
            std::thread::yield_now();
        }
    }
}

/// Démarrage avec la session, avec le dossier des données : sous Windows, la clé Run lance
/// `servym --supervised` (depuis la clé Run, le dossier courant est System32) ; sous Linux, un
/// service systemd utilisateur lance `servym`, supervisé par systemd.
fn default_autostart(data_dir: &Path) -> Arc<dyn Autostart> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("servym.exe"));
        let data = std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
        Arc::new(servym_sys::windows::autostart::RunKey {
            command: format!("\"{}\" --supervised --data \"{}\"", exe.display(), data.display()),
        })
    }
    #[cfg(not(windows))]
    {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("servym"));
        let data = std::path::absolute(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
        Arc::new(servym_sys::linux::SystemdTask::for_user(
            servym_sys::linux::Unit::headless(&exe, &data),
            servym_sys::run_command(),
        ))
    }
}
