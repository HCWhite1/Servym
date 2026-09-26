//! Windows : tout en appels natifs, sans PowerShell ni netstat, tasklist ou taskkill.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{
    BoxFuture, LaunchError, LaunchOptions, Listener, NetworkProfile, OnExit, Platform, ProcTable, Tools,
    clean_env,
};

pub mod autostart;
pub mod launch;
pub mod network;
pub mod procs;
pub mod tcp;
pub mod toast;

/// VS Code, installé pour l'utilisateur ou pour la machine.
fn vscode() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join(r"Programs\Microsoft VS Code\Code.exe"));
    let machine =
        std::env::var_os("ProgramFiles").map(|d| PathBuf::from(d).join(r"Microsoft VS Code\Code.exe"));
    [local, machine].into_iter().flatten().find(|p| p.exists())
}

/// Windows Terminal : alias d'application (lien d'exécution) que `exists()` ne voit pas, mais
/// `symlink_metadata` si.
fn windows_terminal() -> Option<PathBuf> {
    let wt = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Microsoft\WindowsApps\wt.exe");
    std::fs::symlink_metadata(&wt).is_ok().then_some(wt)
}

/// Chemin avec des barres obliques inverses (Explorer n'accepte que celles-ci).
fn native_path(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\")
}

/// Windows.
pub struct WindowsPlatform {
    code: Option<PathBuf>,
    terminal: Option<PathBuf>,
}

impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsPlatform {
    pub fn new() -> Self {
        WindowsPlatform {
            code: vscode(),
            terminal: windows_terminal(),
        }
    }
}

fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static, fallback: T) -> BoxFuture<'static, T> {
    Box::pin(async move { tokio::task::spawn_blocking(f).await.unwrap_or(fallback) })
}

impl Platform for WindowsPlatform {
    fn listeners(&self) -> BoxFuture<'_, Vec<Listener>> {
        Box::pin(async { tcp::listeners() })
    }

    fn process_names(&self) -> BoxFuture<'_, HashMap<u32, String>> {
        blocking(procs::names, HashMap::new())
    }

    fn network_profiles(&self) -> BoxFuture<'_, Vec<NetworkProfile>> {
        blocking(network::network_profiles, Vec::new())
    }

    fn sample_processes(&self) -> BoxFuture<'_, ProcTable> {
        blocking(procs::sample, ProcTable::new())
    }

    fn process_start(&self, pid: u32) -> Option<i64> {
        procs::start_of(pid)
    }

    fn is_alive(&self, pid: u32) -> bool {
        procs::alive(pid)
    }

    fn kill_tree(&self, pid: u32) -> BoxFuture<'_, Result<(), String>> {
        blocking(move || procs::kill_tree(pid), Err(pid.to_string()))
    }

    fn launch(&self, argv: Vec<String>, options: LaunchOptions) -> BoxFuture<'_, Result<u32, LaunchError>> {
        Box::pin(async move { launch::launch(&argv, options) })
    }

    fn watch_exit(&self, pid: u32, on_exit: OnExit) {
        procs::watch_exit(pid, on_exit);
    }

    /// Terminal : toujours disponible (PowerShell à défaut de Windows Terminal) ; on indique lequel
    /// s'ouvrira. Session avec écran : toujours sous Windows.
    fn tools(&self) -> Tools {
        Tools {
            desktop: true,
            code: self.code.is_some(),
            terminal: Some(
                if self.terminal.is_some() {
                    "Windows Terminal"
                } else {
                    "PowerShell"
                }
                .into(),
            ),
        }
    }

    fn open_folder(&self, dir: &Path) -> Result<(), String> {
        launch::detach("explorer.exe", &[native_path(dir)], None, None, false)
    }

    /// Explorer ouvre le fichier avec l'application par défaut (le navigateur pour un .html).
    fn open_file(&self, file: &Path) -> Result<(), String> {
        launch::detach("explorer.exe", &[native_path(file)], None, None, false)
    }

    fn open_in_code(&self, dir: &Path) -> Result<(), String> {
        let code = self.code.as_ref().ok_or("VS Code introuvable")?;
        launch::detach(
            &code.to_string_lossy(),
            &[native_path(dir)],
            None,
            Some(&clean_env()),
            false,
        )
    }

    /// Fichier texte (projects.json, servym.log) : VS Code, sinon le Bloc-notes.
    fn open_in_editor(&self, file: &Path) -> Result<(), String> {
        match &self.code {
            Some(code) => launch::detach(
                &code.to_string_lossy(),
                &[native_path(file)],
                None,
                Some(&clean_env()),
                false,
            ),
            None => launch::detach("notepad.exe", &[native_path(file)], None, None, false),
        }
    }

    fn open_terminal(&self, dir: &Path) -> Result<(), String> {
        match &self.terminal {
            Some(wt) => launch::detach(
                &wt.to_string_lossy(),
                &["-d".into(), native_path(dir)],
                None,
                Some(&clean_env()),
                false,
            ),
            None => launch::detach(
                "powershell.exe",
                &["-NoExit".into()],
                Some(dir),
                Some(&clean_env()),
                true,
            ),
        }
    }
}
