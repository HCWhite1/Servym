//! Linux : ports (`ss`), processus (`/proc`), arrêt par signaux, lancement sans shell, outils du
//! bureau, démarrage avec la session (service utilisateur systemd).
//!
//! Les fonctions d'analyse et celles qui prennent leurs appels système en paramètre se compilent
//! partout ; le lancement réel est réservé à Unix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::{Autostart, BoxFuture, Listener, NetworkProfile, ProcTable, Tools};

pub const SIGKILL: i32 = 9;
pub const SIGTERM: i32 = 15;

/// Lignes de `ss -Hltnp` (ou avec la ligne d'en-tête) : adresses ramenées à la forme de Windows.
pub fn parse_ss(text: &str) -> Vec<Listener> {
    let _ = text;
    unimplemented!("phase 3")
}

/// Champs utiles d'une ligne `/proc/<pid>/stat`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub pid: u32,
    pub ppid: u32,
    /// utime + stime.
    pub ticks: u64,
    pub start_ticks: u64,
    /// Pages.
    pub rss: u64,
}

/// Analyse une ligne `stat` : un nom avec espaces et parenthèses ne décale rien.
pub fn parse_stat(line: &str) -> Option<Stat> {
    let _ = line;
    unimplemented!("phase 3")
}

/// Relevé de `/proc` (ou d'un faux `/proc`) : début, temps processeur et mémoire.
pub fn sample_processes(proc: &Path, ticks_per_second: u64, page_size: u64) -> ProcTable {
    let _ = (proc, ticks_per_second, page_size);
    unimplemented!("phase 3")
}

/// Nom de chaque processus (`/proc/<pid>/comm`).
pub fn process_names(proc: &Path) -> HashMap<u32, String> {
    let _ = proc;
    unimplemented!("phase 3")
}

/// Pas de profils réseau comme sous Windows.
pub fn network_profiles() -> Vec<NetworkProfile> {
    Vec::new()
}

/// Appels système de l'arrêt, remplaçables dans les tests.
pub struct KillOptions {
    pub proc: PathBuf,
    /// `kill(pid, signal)` ; un PID négatif vise le groupe.
    pub kill: Arc<dyn Fn(i32, i32) + Send + Sync>,
    pub alive: Arc<dyn Fn(u32) -> bool + Send + Sync>,
    /// Attente après SIGTERM avant SIGKILL.
    pub grace: Duration,
    /// Attente après SIGKILL avant de conclure à l'échec.
    pub wait: Duration,
}

/// SIGTERM au groupe et à chaque descendant, SIGKILL à ceux qui résistent, rien aux autres.
/// Erreur (avec le PID) si un processus survit.
pub async fn kill_tree(pid: u32, options: KillOptions) -> Result<(), String> {
    let _ = (pid, options);
    unimplemented!("phase 3")
}

/// Code de sortie à la manière d'un shell : tué par un signal, 128 + numéro du signal.
pub fn exit_code(code: Option<i32>, signal: Option<i32>) -> Option<i64> {
    let _ = (code, signal);
    unimplemented!("phase 3")
}

/// Lance `argv` sans shell, sorties ajoutées au journal ; rend le PID.
#[cfg(unix)]
pub async fn launch(argv: Vec<String>, options: crate::LaunchOptions) -> Result<u32, crate::LaunchError> {
    let _ = (argv, options);
    unimplemented!("phase 3")
}

/// Outils du bureau : aucun sans écran ; sinon VS Code et le premier terminal trouvé.
pub fn detect_tools(env: &HashMap<String, String>, has: &dyn Fn(&str) -> bool) -> Tools {
    let _ = (env, has);
    unimplemented!("phase 3")
}

/// Commande qui ouvre ce terminal dans un dossier.
pub fn terminal_command(name: &str, dir: &str) -> Option<(String, Vec<String>)> {
    let _ = (name, dir);
    unimplemented!("phase 3")
}

/// Service utilisateur systemd : relancé s'il plante, les projets survivent à son arrêt.
pub fn unit_file(exe: &Path, workdir: &Path) -> String {
    let _ = (exe, workdir);
    unimplemented!("phase 3")
}

/// Exécution d'une commande : sortie standard, ou erreur (code non nul) avec sa sortie standard.
pub type Runner =
    Arc<dyn Fn(String, Vec<String>) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

/// Démarrage avec la session par un service utilisateur systemd.
pub struct SystemdTask {
    pub dir: PathBuf,
    pub run: Runner,
    pub exe: PathBuf,
    pub workdir: PathBuf,
}

impl Autostart for SystemdTask {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        unimplemented!("phase 3")
    }
    fn set(&self, on: bool) -> BoxFuture<'_, Result<(), String>> {
        let _ = (on, &self.dir, &self.run, &self.exe, &self.workdir);
        unimplemented!("phase 3")
    }
}
