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
pub mod net;
pub mod notify;
#[cfg(windows)]
pub mod windows;

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
pub struct NetworkProfile {
    pub name: String,
    /// Nom de la carte réseau (« Wi-Fi », « Ethernet »…).
    pub interface: String,
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

/// Démarrage de Servym avec la session (Windows : clé Run ; Linux : service systemd).
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
    /// Attend la fin d'un processus déjà lancé (repris après un redémarrage de Servym).
    fn watch_exit(&self, pid: u32, on_exit: OnExit);
    fn tools(&self) -> Tools;
    fn open_folder(&self, dir: &Path) -> Result<(), String>;
    fn open_file(&self, file: &Path) -> Result<(), String>;
    fn open_in_code(&self, dir: &Path) -> Result<(), String>;
    fn open_in_editor(&self, file: &Path) -> Result<(), String>;
    fn open_terminal(&self, dir: &Path) -> Result<(), String>;
}

/// Le système de cette machine (le même objet à chaque appel).
pub fn native() -> Arc<dyn Platform> {
    static NATIVE: std::sync::OnceLock<Arc<dyn Platform>> = std::sync::OnceLock::new();
    NATIVE
        .get_or_init(|| {
            #[cfg(windows)]
            return Arc::new(windows::WindowsPlatform::new());
            #[cfg(unix)]
            return Arc::new(linux::LinuxPlatform::new());
        })
        .clone()
}

/// PID de l'arborescence de chaque racine. Un enfant démarré avant son parent désigne un ancien
/// processus au PID réutilisé : il n'en fait pas partie.
pub fn tree_of(processes: &ProcTable, roots: &[u32]) -> std::collections::BTreeSet<u32> {
    let mut children: HashMap<u32, Vec<&ProcSample>> = HashMap::new();
    for p in processes.values() {
        children.entry(p.ppid).or_default().push(p);
    }
    let mut tree = std::collections::BTreeSet::new();
    let mut stack: Vec<u32> = roots.iter().rev().copied().collect();
    while let Some(pid) = stack.pop() {
        let Some(parent) = processes.get(&pid) else {
            continue;
        };
        if !tree.insert(pid) {
            continue;
        }
        for child in children.get(&pid).into_iter().flatten() {
            if child.pid != pid && child.start >= parent.start {
                stack.push(child.pid);
            }
        }
    }
    tree
}

/// Variables d'environnement de Servym, sans celles de l'éditeur ou du terminal qui l'a démarré
/// (VS Code, Electron, Claude) ni celle de son superviseur : pour les programmes qu'il lance.
pub fn clean_env() -> HashMap<String, String> {
    std::env::vars()
        .filter(|(k, _)| {
            let upper = k.to_ascii_uppercase();
            !(upper.starts_with("ELECTRON_")
                || upper.starts_with("VSCODE_")
                || upper.starts_with("CLAUDE")
                || upper == "SERVYM_SUPERVISOR")
        })
        .collect()
}

async fn exec_command(mut command: tokio::process::Command, cmd: &str) -> Result<String, String> {
    command.stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| format!("{cmd} : délai dépassé"))?
        .map_err(|e| format!("{cmd} : {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let err = stderr.trim();
        if err.is_empty() {
            Err(stdout)
        } else {
            Err(err.to_string())
        }
    }
}

/// Exécute une commande (sans shell, sans fenêtre) : sortie standard si elle réussit, sinon sa
/// sortie d'erreur (ou standard si elle est vide). Délai : 15 s.
pub fn run_command() -> linux::Runner {
    Arc::new(
        |cmd: String, args: Vec<String>| -> BoxFuture<'static, Result<String, String>> {
            Box::pin(async move {
                let mut command = tokio::process::Command::new(&cmd);
                command.args(&args);
                exec_command(command, &cmd).await
            })
        },
    )
}

/// Exécute une commande dans un dossier et un environnement précis (sans shell, sans fenêtre).
/// Délai : 15 s.
pub async fn run_command_in(
    cmd: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: &HashMap<String, String>,
) -> Result<String, String> {
    let mut command = tokio::process::Command::new(cmd);
    command.args(args).env_clear().envs(env);
    if let Some(dir) = cwd.filter(|d| d.is_dir()) {
        command.current_dir(dir);
    }
    exec_command(command, cmd).await
}

/// Heure de démarrage du PC (ms depuis l'époque Unix).
pub fn boot_time_ms() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    #[cfg(windows)]
    {
        // SAFETY : lecture du temps écoulé depuis le démarrage.
        let uptime = unsafe { ::windows_sys::Win32::System::SystemInformation::GetTickCount64() };
        now - uptime as i64
    }
    #[cfg(unix)]
    {
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| t.split_whitespace().next().and_then(|s| s.parse::<f64>().ok()))
            .unwrap_or(0.0);
        now - (uptime * 1000.0) as i64
    }
}

/// Pile des fils qui attendent la fin d'un projet : réservée, pas engagée ; le traitement de la
/// sortie (journal, état, évènements) y tient même sans optimisation.
pub(crate) const WATCH_STACK: usize = 1024 * 1024;

/// Langue de l'utilisateur (« fr-FR »), pour la langue du serveur par défaut.
pub fn system_locale() -> Option<String> {
    #[cfg(windows)]
    {
        let mut buf = [0u16; 85];
        // SAFETY : tampon de LOCALE_NAME_MAX_LENGTH caractères.
        let n = unsafe {
            ::windows_sys::Win32::Globalization::GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32)
        };
        (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
    }
    #[cfg(unix)]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C"))
    }
}

/// Nom de cette machine sur le réseau (pour l'en-tête `Host` en accès réseau local).
pub fn hostname() -> Option<String> {
    #[cfg(windows)]
    {
        std::env::var("COMPUTERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        // SAFETY : tampon de 256 octets rempli par gethostname.
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) } == 0;
        if ok {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let name = String::from_utf8_lossy(&buf[..len]).trim().to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
        std::env::var("HOSTNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::fs::read_to_string("/etc/hostname")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            })
    }
}
