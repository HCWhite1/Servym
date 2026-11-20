//! Linux : ports (`ss`), processus (`/proc`), arrêt par signaux, lancement sans shell, outils du
//! bureau, démarrage avec la session (service utilisateur systemd).
//!
//! Les fonctions d'analyse et celles qui prennent leurs appels système en paramètre se compilent
//! partout (leurs tests tournent aussi sous Windows) ; la plateforme réelle est réservée à Unix.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::{Autostart, BoxFuture, Listener, NetworkProfile, ProcSample, ProcTable, Tools, tree_of};

pub const SIGKILL: i32 = 9;
pub const SIGTERM: i32 = 15;

// ---- Ports ----

/// Adresse de ss ramenée à la forme de netstat sous Windows : « 127.0.0.53%lo » → « 127.0.0.53 »,
/// « [::ffff:127.0.0.1] » → « 127.0.0.1 », « * » (toutes) → « 0.0.0.0 ».
fn address(raw: &str) -> String {
    let plain = match raw.find('%') {
        Some(start) => {
            let end = raw[start..].find(']').map_or(raw.len(), |e| start + e);
            format!("{}{}", &raw[..start], &raw[end..])
        }
        None => raw.to_string(),
    };
    if plain == "*" {
        return "0.0.0.0".into();
    }
    if let Some(v4) = plain.strip_prefix("[::ffff:").and_then(|r| r.strip_suffix(']'))
        && v4.parse::<std::net::Ipv4Addr>().is_ok()
    {
        return v4.to_string();
    }
    plain
}

/// Lignes de `ss -Hltnp` (ou avec la ligne d'en-tête) : adresses ramenées à la forme de Windows.
/// PID absent quand le processus appartient à un autre compte (ss ne le montre qu'à root).
pub fn parse_ss(text: &str) -> Vec<Listener> {
    let mut list = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.first() != Some(&"LISTEN") || cols.len() < 5 {
            continue;
        }
        let local = cols[3];
        let Some(i) = local.rfind(':') else { continue };
        let Ok(port) = local[i + 1..].parse() else {
            continue;
        };
        let pid = line.find("pid=").and_then(|at| {
            let digits: String = line[at + 4..].chars().take_while(char::is_ascii_digit).collect();
            digits.parse().ok()
        });
        list.push(Listener {
            address: address(&local[..i]),
            port,
            pid,
        });
    }
    list
}

// ---- Processus (/proc) ----

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

/// Analyse une ligne `stat`. Le nom (2e champ) peut contenir espaces et parenthèses : on repart
/// de la dernière « ) ».
pub fn parse_stat(line: &str) -> Option<Stat> {
    let pid = line.split(' ').next()?.parse().ok()?;
    let close = line.rfind(')')?;
    let rest: Vec<&str> = line.get(close + 2..)?.split(' ').collect();
    let num = |i: usize| rest.get(i).and_then(|v| v.parse::<u64>().ok());
    Some(Stat {
        pid,
        ppid: num(1)? as u32,
        ticks: num(11)? + num(12)?,
        start_ticks: num(19)?,
        rss: num(21)?,
    })
}

fn pid_dirs(proc: &Path) -> Vec<u32> {
    std::fs::read_dir(proc)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.parse().ok()))
        .collect()
}

fn boot_time(proc: &Path) -> u64 {
    std::fs::read_to_string(proc.join("stat"))
        .ok()
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("btime ").and_then(|v| v.trim().parse().ok()))
        })
        .unwrap_or(0)
}

fn rounded(ticks: u64, per_second: u64) -> u64 {
    (ticks * 1000 + per_second / 2) / per_second.max(1)
}

/// Relevé de `/proc` (ou d'un faux `/proc`) : début (ms), temps processeur (ms) et mémoire
/// (octets). Un processus terminé pendant la lecture est ignoré.
pub fn sample_processes(proc: &Path, ticks_per_second: u64, page_size: u64) -> ProcTable {
    let btime = boot_time(proc);
    let mut table = ProcTable::new();
    for pid in pid_dirs(proc) {
        let Ok(text) = std::fs::read_to_string(proc.join(pid.to_string()).join("stat")) else {
            continue;
        };
        let Some(s) = parse_stat(text.trim()) else {
            continue;
        };
        table.insert(
            pid,
            ProcSample {
                pid,
                ppid: s.ppid,
                start: (btime * 1000 + rounded(s.start_ticks, ticks_per_second)) as i64,
                cpu: rounded(s.ticks, ticks_per_second),
                memory: s.rss * page_size,
            },
        );
    }
    table
}

/// Nom de chaque processus (`/proc/<pid>/comm`).
pub fn process_names(proc: &Path) -> HashMap<u32, String> {
    pid_dirs(proc)
        .into_iter()
        .filter_map(|pid| {
            let name = std::fs::read_to_string(proc.join(pid.to_string()).join("comm")).ok()?;
            Some((pid, name.trim().to_string()))
        })
        .collect()
}

/// Pas de profils réseau comme sous Windows : le pare-feu (firewalld…) se règle à part.
pub fn network_profiles() -> Vec<NetworkProfile> {
    Vec::new()
}

// ---- Arrêt ----

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

/// SIGTERM au groupe du projet (le lancement le crée) et à tous ses descendants, qui peuvent
/// s'arrêter proprement ; SIGKILL à ceux qui restent. Erreur (avec le PID) si le projet survit.
pub async fn kill_tree(pid: u32, options: KillOptions) -> Result<(), String> {
    if pid == 0 {
        return Ok(());
    }
    let snapshot = sample_processes(&options.proc, 100, 4096);
    let mut tree: Vec<u32> = tree_of(&snapshot, &[pid]).into_iter().collect();
    if !tree.contains(&pid) {
        tree.push(pid);
    }
    let living = || {
        tree.iter()
            .copied()
            .filter(|p| (options.alive)(*p))
            .collect::<Vec<_>>()
    };
    if pid > 1 && pid <= i32::MAX as u32 {
        (options.kill)(-(pid as i32), SIGTERM);
    }
    for p in &tree {
        if *p > 0 && *p <= i32::MAX as u32 {
            (options.kill)(*p as i32, SIGTERM);
        }
    }
    let step = Duration::from_millis(50);
    let mut waited = Duration::ZERO;
    while waited < options.grace && !living().is_empty() {
        tokio::time::sleep(step).await;
        waited += step;
    }
    let remaining = living();
    if !remaining.is_empty() {
        if pid > 1 && pid <= i32::MAX as u32 {
            (options.kill)(-(pid as i32), SIGKILL);
        }
        for p in remaining {
            if p > 0 && p <= i32::MAX as u32 {
                (options.kill)(p as i32, SIGKILL);
            }
        }
    }
    let mut waited = Duration::ZERO;
    while waited < options.wait && (options.alive)(pid) {
        tokio::time::sleep(step).await;
        waited += step;
    }
    if (options.alive)(pid) {
        Err(pid.to_string())
    } else {
        Ok(())
    }
}

/// Code de sortie à la manière d'un shell : tué par un signal, 128 + numéro (SIGKILL → 137).
pub fn exit_code(code: Option<i32>, signal: Option<i32>) -> Option<i64> {
    match (code, signal) {
        (Some(code), _) => Some(i64::from(code)),
        (None, Some(signal)) => Some(128 + i64::from(signal)),
        (None, None) => None,
    }
}

// ---- Outils du bureau ----

/// Terminaux reconnus, dans l'ordre de préférence : commande, nom.
const TERMINALS: [(&str, &str); 7] = [
    ("ptyxis", "Ptyxis"), // Fedora 41 et suivantes
    ("gnome-terminal", "GNOME Terminal"),
    ("konsole", "Konsole"),
    ("xfce4-terminal", "Xfce Terminal"),
    ("kitty", "kitty"),
    ("alacritty", "Alacritty"),
    ("xterm", "XTerm"),
];

/// Commande qui ouvre ce terminal dans un dossier.
pub fn terminal_command(name: &str, dir: &str) -> Option<(String, Vec<String>)> {
    let (cmd, _) = TERMINALS.iter().find(|(_, n)| *n == name)?;
    let args: Vec<String> = match *cmd {
        "ptyxis" => vec!["--new-window".into(), format!("--working-directory={dir}")],
        "gnome-terminal" | "xfce4-terminal" => vec![format!("--working-directory={dir}")],
        "konsole" => vec!["--workdir".into(), dir.into()],
        "kitty" => vec!["--directory".into(), dir.into()],
        "alacritty" => vec!["--working-directory".into(), dir.into()],
        _ => Vec::new(),
    };
    Some((cmd.to_string(), args))
}

/// Outils du bureau : aucun sans écran (serveur) ; sinon VS Code et le premier terminal trouvé.
pub fn detect_tools(env: &HashMap<String, String>, has: &dyn Fn(&str) -> bool) -> Tools {
    let set = |k: &str| env.get(k).is_some_and(|v| !v.is_empty());
    if !set("DISPLAY") && !set("WAYLAND_DISPLAY") {
        return Tools {
            desktop: false,
            code: false,
            terminal: None,
        };
    }
    Tools {
        desktop: true,
        code: has("code"),
        terminal: TERMINALS
            .iter()
            .find(|(cmd, _)| has(cmd))
            .map(|(_, name)| name.to_string()),
    }
}

/// Commande présente dans le PATH ?
pub fn in_path(env: &HashMap<String, String>, cmd: &str) -> bool {
    env.get("PATH").is_some_and(|path| {
        path.split(':')
            .any(|dir| !dir.is_empty() && Path::new(dir).join(cmd).exists())
    })
}

// ---- Démarrage avec la session : service utilisateur systemd ----

/// Ce que lance le service « servym ».
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub workdir: PathBuf,
    /// Application de bureau : démarre avec la session graphique (il lui faut l'écran) et se
    /// supervise elle-même, comme sous Windows. Sinon, `servym` sans écran, supervisé par systemd.
    pub graphical: bool,
}

impl Unit {
    /// `servym` sans écran, avec ses données.
    pub fn headless(exe: &Path, data: &Path) -> Unit {
        Unit {
            exe: exe.to_path_buf(),
            args: vec!["--data".into(), data.to_string_lossy().into_owned()],
            workdir: data.to_path_buf(),
            graphical: false,
        }
    }
}

/// Argument de `ExecStart` : entre guillemets, `%` et `$` protégés (variables de systemd).
fn exec_arg(arg: &str) -> String {
    let escaped = arg
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// Ligne `ExecStart` du service : l'exécutable et ses arguments.
pub fn exec_start(unit: &Unit) -> String {
    let mut line = format!("ExecStart={}", exec_arg(&unit.exe.to_string_lossy()));
    for arg in &unit.args {
        line.push(' ');
        line.push_str(&exec_arg(arg));
    }
    line
}

/// Service « servym » : démarre à l'ouverture de session, relancé s'il plante (5 échecs rapides :
/// abandon). `KillMode=process` : arrêter Servym laisse tourner les projets, repris au démarrage
/// suivant.
pub fn unit_file(unit: &Unit) -> String {
    let q = |p: &Path| p.to_string_lossy().replace('%', "%%"); // % introduit une variable pour systemd
    let mut lines = vec![
        "[Unit]".to_string(),
        format!("Description=Servym ({})", q(&unit.workdir)),
    ];
    if unit.graphical {
        // Il faut l'écran : après l'ouverture de la session graphique, arrêté avec elle.
        lines.push("After=graphical-session.target".into());
        lines.push("PartOf=graphical-session.target".into());
    } else {
        lines.push("After=network.target".into());
    }
    lines.extend([
        "StartLimitIntervalSec=300".into(),
        "StartLimitBurst=5".into(),
        String::new(),
        "[Service]".into(),
        "Type=simple".into(),
        format!("WorkingDirectory={}", q(&unit.workdir)),
        exec_start(unit),
    ]);
    if unit.graphical {
        lines.push("# L’application se supervise elle-même (redémarrage demandé, fenêtre rouverte).".into());
    } else {
        lines.push("Environment=SERVYM_SUPERVISOR=systemd".into());
        lines.push("# Code 75 : redémarrage demandé depuis l’interface.".into());
    }
    let target = if unit.graphical {
        "graphical-session.target"
    } else {
        "default.target"
    };
    lines.extend([
        "Restart=on-failure".into(),
        "RestartSec=2".into(),
        "KillMode=process".into(),
        String::new(),
        "[Install]".into(),
        format!("WantedBy={target}"),
        String::new(),
    ]);
    lines.join("\n")
}

/// Exécution d'une commande : sortie standard, ou erreur (code non nul) avec sa sortie standard.
pub type Runner =
    Arc<dyn Fn(String, Vec<String>) -> BoxFuture<'static, Result<String, String>> + Send + Sync>;

/// Démarrage avec la session par un service utilisateur systemd.
pub struct SystemdTask {
    /// Dossier des services utilisateur (`~/.config/systemd/user`).
    pub dir: PathBuf,
    pub run: Runner,
    pub unit: Unit,
}

impl SystemdTask {
    /// Service de l'utilisateur courant (`$XDG_CONFIG_HOME/systemd/user`).
    pub fn for_user(unit: Unit, run: Runner) -> SystemdTask {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_default();
        SystemdTask {
            dir: config.join("systemd").join("user"),
            run,
            unit,
        }
    }

    fn systemctl(&self, args: &[&str]) -> BoxFuture<'static, Result<String, String>> {
        let mut all = vec!["--user".to_string()];
        all.extend(args.iter().map(|a| a.to_string()));
        (self.run)("systemctl".into(), all)
    }
}

impl Autostart for SystemdTask {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            // Activé seulement s'il lance bien ce programme, avec ces données : un service laissé
            // par l'autre version de Servym (sans écran ou de bureau) ne compte pas.
            let file = std::fs::read_to_string(self.dir.join("servym.service")).unwrap_or_default();
            let expected = exec_start(&self.unit);
            if !file.lines().any(|l| l.trim() == expected) {
                return false;
            }
            // is-enabled répond « disabled » avec un code d'erreur : sa sortie suffit.
            let out = self
                .systemctl(&["is-enabled", "servym.service"])
                .await
                .unwrap_or_else(|e| e);
            out.trim() == "enabled"
        })
    }

    fn set(&self, on: bool) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let file = self.dir.join("servym.service");
            if on {
                std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
                // Un service de l'autre version est remplacé : un seul Servym par session.
                std::fs::write(&file, unit_file(&self.unit)).map_err(|e| e.to_string())?;
                self.systemctl(&["daemon-reload"]).await?;
                // Les liens de l'ancien service (autre cible) sont refaits pour celui-ci.
                self.systemctl(&["reenable", "servym.service"]).await?;
            } else {
                self.systemctl(&["disable", "servym.service"]).await?;
                let _ = std::fs::remove_file(&file);
                self.systemctl(&["daemon-reload"]).await?;
            }
            Ok(())
        })
    }
}

// ---- Plateforme réelle ----

#[cfg(unix)]
pub use real::LinuxPlatform;

#[cfg(unix)]
mod real {
    use std::collections::HashMap;
    use std::fs::OpenOptions;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::time::Duration;

    use super::{KillOptions, detect_tools, exit_code, in_path, parse_ss, parse_stat, terminal_command};
    use crate::{
        BoxFuture, LaunchError, LaunchOptions, Listener, NetworkProfile, OnExit, Platform, ProcTable, Tools,
        WATCH_STACK, clean_env, run_command,
    };

    const PROC: &str = "/proc";

    /// Linux (et autres Unix disposant de /proc).
    pub struct LinuxPlatform {
        ticks: u64,
        page_size: u64,
        tools: Tools,
    }

    fn sysconf(name: libc::c_int, default: u64) -> u64 {
        // SAFETY : sysconf lit une valeur du système, sans effet de bord.
        let value = unsafe { libc::sysconf(name) };
        if value > 0 { value as u64 } else { default }
    }

    impl Default for LinuxPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    impl LinuxPlatform {
        pub fn new() -> Self {
            let env: HashMap<String, String> = std::env::vars().collect();
            let tools = detect_tools(&env, &|cmd| in_path(&env, cmd));
            LinuxPlatform {
                ticks: sysconf(libc::_SC_CLK_TCK, 100),
                page_size: sysconf(libc::_SC_PAGESIZE, 4096),
                tools,
            }
        }
    }

    fn is_zombie(pid: u32) -> bool {
        let Ok(text) = std::fs::read_to_string(Path::new(PROC).join(pid.to_string()).join("stat")) else {
            return false;
        };
        let Some(close) = text.rfind(')') else {
            return false;
        };
        text.as_bytes().get(close + 2) == Some(&b'Z')
    }

    fn alive(pid: u32) -> bool {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY : le signal 0 ne fait que vérifier l'existence du processus.
        let ok = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        if ok {
            return !is_zombie(pid);
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// Programme du bureau détaché (xdg-open, code, terminal), sans lien avec Servym.
    fn detach(cmd: &str, args: &[String], cwd: Option<&Path>) -> Result<(), String> {
        let mut command = Command::new(cmd);
        command.args(args).env_clear().envs(clean_env());
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        if let Some(dir) = cwd {
            command.current_dir(dir);
        }
        let mut child = command.spawn().map_err(|e| format!("{cmd} : {e}"))?;
        std::thread::spawn(move || child.wait());
        Ok(())
    }

    impl Platform for LinuxPlatform {
        fn listeners(&self) -> BoxFuture<'_, Vec<Listener>> {
            Box::pin(async {
                let out = run_command()("ss".into(), vec!["-Hltnp".into()])
                    .await
                    .unwrap_or_default();
                parse_ss(&out)
            })
        }

        fn process_names(&self) -> BoxFuture<'_, HashMap<u32, String>> {
            Box::pin(async {
                tokio::task::spawn_blocking(|| super::process_names(Path::new(PROC)))
                    .await
                    .unwrap_or_default()
            })
        }

        fn network_profiles(&self) -> BoxFuture<'_, Vec<NetworkProfile>> {
            Box::pin(async { super::network_profiles() })
        }

        fn sample_processes(&self) -> BoxFuture<'_, ProcTable> {
            let (ticks, page) = (self.ticks, self.page_size);
            Box::pin(async move {
                tokio::task::spawn_blocking(move || super::sample_processes(Path::new(PROC), ticks, page))
                    .await
                    .unwrap_or_default()
            })
        }

        fn process_start(&self, pid: u32) -> Option<i64> {
            let text = std::fs::read_to_string(Path::new(PROC).join(pid.to_string()).join("stat")).ok()?;
            let stat = parse_stat(text.trim())?;
            let btime = super::boot_time(Path::new(PROC));
            Some((btime * 1000 + super::rounded(stat.start_ticks, self.ticks)) as i64)
        }

        fn is_alive(&self, pid: u32) -> bool {
            alive(pid)
        }

        fn kill_tree(&self, pid: u32) -> BoxFuture<'_, Result<(), String>> {
            let options = KillOptions {
                proc: PathBuf::from(PROC),
                kill: Arc::new(|pid, signal| {
                    // SAFETY : envoi d'un signal à un processus (ou au groupe si pid < 0).
                    unsafe { libc::kill(pid, signal) };
                }),
                alive: Arc::new(alive),
                grace: Duration::from_secs(3),
                wait: Duration::from_secs(2),
            };
            Box::pin(super::kill_tree(pid, options))
        }

        fn launch(
            &self,
            argv: Vec<String>,
            options: LaunchOptions,
        ) -> BoxFuture<'_, Result<u32, LaunchError>> {
            Box::pin(async move { launch(argv, options) })
        }

        fn watch_exit(&self, pid: u32, on_exit: OnExit) {
            // Processus repris : pas enfant de Servym, son code de sortie est inconnu.
            std::thread::Builder::new()
                .stack_size(WATCH_STACK)
                .spawn(move || {
                    while alive(pid) {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    on_exit(pid, None);
                })
                .expect("surveillance d'un processus");
        }

        fn tools(&self) -> Tools {
            self.tools.clone()
        }

        fn open_folder(&self, dir: &Path) -> Result<(), String> {
            detach("xdg-open", &[dir.to_string_lossy().into_owned()], None)
        }

        fn open_file(&self, file: &Path) -> Result<(), String> {
            detach("xdg-open", &[file.to_string_lossy().into_owned()], None)
        }

        fn open_in_code(&self, dir: &Path) -> Result<(), String> {
            if !self.tools.code {
                return Err("VS Code introuvable".into());
            }
            detach("code", &[dir.to_string_lossy().into_owned()], None)
        }

        /// Fichier texte (projects.json, servym.log) : VS Code, sinon l'éditeur par défaut.
        fn open_in_editor(&self, file: &Path) -> Result<(), String> {
            if self.tools.code {
                detach("code", &[file.to_string_lossy().into_owned()], None)
            } else {
                self.open_file(file)
            }
        }

        fn open_terminal(&self, dir: &Path) -> Result<(), String> {
            let name = self.tools.terminal.as_deref().ok_or("system.noTerminal")?;
            let (cmd, args) = terminal_command(name, &dir.to_string_lossy()).ok_or("system.noTerminal")?;
            detach(&cmd, &args, Some(dir))
        }
    }

    /// Lance `argv` directement, sans shell : aucun caractère n'est interprété. Nouveau groupe de
    /// processus : le projet survit à un redémarrage de Servym et l'arrêt vise tout le groupe.
    pub fn launch(argv: Vec<String>, options: LaunchOptions) -> Result<u32, LaunchError> {
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| LaunchError::Other("commande vide".into()))?;
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&options.log)
            .map_err(|e| LaunchError::Other(e.to_string()))?;
        let err = log.try_clone().map_err(|e| LaunchError::Other(e.to_string()))?;
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(&options.env)
            .current_dir(&options.cwd)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(err)
            .process_group(0);
        let mut child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound && options.cwd.is_dir() {
                LaunchError::NotFound(program.clone())
            } else {
                LaunchError::Other(e.to_string())
            }
        })?;
        let pid = child.id();
        let on_exit = options.on_exit;
        std::thread::Builder::new()
            .stack_size(WATCH_STACK)
            .spawn(move || {
                let code = child.wait().ok().and_then(|s| exit_code(s.code(), s.signal()));
                on_exit(pid, code);
            })
            .map_err(|e| LaunchError::Other(e.to_string()))?;
        Ok(pid)
    }
}

/// Lance `argv` sans shell, sorties ajoutées au journal ; rend le PID.
#[cfg(unix)]
pub async fn launch(argv: Vec<String>, options: crate::LaunchOptions) -> Result<u32, crate::LaunchError> {
    real::launch(argv, options)
}
