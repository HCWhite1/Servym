//! Accès au système : ports, processus, lancement, arrêt, réseau, ouverture de dossiers.
//!
//! Tout le code `unsafe` (Win32, COM, appels Linux) vit dans ce crate, derrière le trait
//! [`Platform`]. Le module `linux` se compile partout : ses fonctions d'analyse sont testées
//! sur des sorties enregistrées, sous Windows aussi.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde::Serialize;

pub mod linux;
pub mod notify;

/// Future en boîte, pour les traits utilisés derrière `dyn`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Un processus dans un relevé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcSample {
    pub pid: u32,
    pub ppid: u32,
    /// Heure de démarrage (ms depuis l'époque Unix).
    pub start: i64,
    /// Temps processeur cumulé (ms).
    pub cpu: u64,
    /// Mémoire vive utilisée (octets).
    pub memory: u64,
}

/// Relevé de tous les processus : PID → processus.
pub type ProcTable = HashMap<u32, ProcSample>;

/// Port TCP en écoute. L'adresse a la forme de netstat (`0.0.0.0`, `[::1]`) : la colonne
/// « exposé » de l'interface en dépend.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Listener {
    pub address: String,
    pub port: u16,
    /// Absent quand le système ne le donne pas (processus d'un autre compte sous Linux).
    pub pid: Option<u32>,
}

/// Réseau connecté et son profil (Windows : le pare-feu applique ses règles par profil).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkProfile {
    pub name: String,
    pub interface_alias: String,
    /// `public`, `private` ou `domain`.
    pub category: String,
}

/// Outils du bureau disponibles pour ouvrir un projet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tools {
    /// Session avec écran : sinon rien ne peut s'ouvrir sur cette machine.
    pub desktop: bool,
    pub code: bool,
    /// Nom du terminal trouvé.
    pub terminal: Option<String>,
}

/// Échec d'un lancement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// La commande n'existe pas (Linux : exec impossible).
    NotFound(String),
    /// Autre erreur du système.
    Other(String),
}

/// Fin d'un projet lancé : PID et code de sortie (`None` si inconnu).
pub type OnExit = Box<dyn FnOnce(u32, Option<i64>) + Send>;

/// Ce qu'il faut pour lancer un projet.
pub struct LaunchOptions {
    pub cwd: PathBuf,
    pub env: HashMap<String, String>,
    /// Journal, complété (jamais remplacé) par les sorties standard et d'erreur.
    pub log: PathBuf,
    pub on_exit: OnExit,
}

/// Démarrage de Servynex avec la session (Windows : clé Run ; Linux : service systemd).
pub trait Autostart: Send + Sync {
    fn enabled(&self) -> BoxFuture<'_, bool>;
    fn set(&self, on: bool) -> BoxFuture<'_, Result<(), String>>;
}

/// Le système, derrière une interface commune (Windows, Linux, faux système des tests).
pub trait Platform: Send + Sync {
    fn listeners(&self) -> BoxFuture<'_, Vec<Listener>>;
    fn process_names(&self) -> BoxFuture<'_, HashMap<u32, String>>;
    fn network_profiles(&self) -> BoxFuture<'_, Vec<NetworkProfile>>;
    fn sample_processes(&self) -> BoxFuture<'_, ProcTable>;
    /// Heure de démarrage d'un processus (ms), pour refuser un PID réutilisé.
    fn process_start(&self, pid: u32) -> Option<i64>;
    fn is_alive(&self, pid: u32) -> bool;
    /// Arrête un processus et toute son arborescence.
    fn kill_tree(&self, pid: u32) -> BoxFuture<'_, Result<(), String>>;
    /// Lance `argv` (Windows : par cmd.exe, console cachée) ; rend le PID racine.
    fn launch(&self, argv: Vec<String>, options: LaunchOptions) -> BoxFuture<'_, Result<u32, LaunchError>>;
    /// Attend la fin d'un processus déjà lancé (repris après un redémarrage de Servynex).
    fn watch_exit(&self, pid: u32, on_exit: OnExit);
    fn tools(&self) -> Tools;
    fn open_folder(&self, dir: &Path) -> Result<(), String>;
    fn open_file(&self, file: &Path) -> Result<(), String>;
    fn open_in_code(&self, dir: &Path) -> Result<(), String>;
    fn open_in_editor(&self, file: &Path) -> Result<(), String>;
    fn open_terminal(&self, dir: &Path) -> Result<(), String>;
    fn autostart(&self) -> Arc<dyn Autostart>;
}

/// Le système de cette machine.
pub fn native() -> Arc<dyn Platform> {
    unimplemented!("phase 3")
}
