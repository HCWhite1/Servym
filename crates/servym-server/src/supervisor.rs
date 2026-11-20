//! Superviseur : relance Servym après un plantage, mais abandonne s'il plante en boucle (port
//! déjà pris par une autre instance, projects.json invalide…). Codes de redémarrage demandé depuis
//! l'interface ([`RESTART_CODE`], [`RESTART_WINDOW_CODE`]) : relancé aussitôt, sans compter comme
//! un plantage.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::{RESTART_CODE, RESTART_WINDOW_CODE};

/// Pourquoi le superviseur lance Servym.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    First,
    /// Redémarrage demandé, avec le code de sortie qui l'a demandé.
    Restart(i32),
    /// Après un plantage.
    Crash,
}

/// Lance `exe` avec les arguments de `args_for` tant qu'il ne sort pas avec le code 0. Servym
/// écrit son journal dans `logs/servym.log` ; sa sortie d'erreur (et celle des bibliothèques qu'il
/// charge : GTK, WebKit…) va dans `logs/servym.stderr.log` (conservé avec rotation à 1 Mo vers
/// `servym.stderr.prev.log`). Sans console, les sorties du superviseur lui-même ne sont pas valides.
pub fn supervise(exe: &Path, data: &Path, mut args_for: impl FnMut(Launch) -> Vec<String>) -> ExitCode {
    let _ = std::fs::create_dir_all(data.join("logs"));
    let stderr_file = data.join("logs").join("servym.stderr.log");
    let prev_stderr = data.join("logs").join("servym.stderr.prev.log");
    let mut failures = 0;
    let mut launch = Launch::First;
    loop {
        if std::fs::metadata(&stderr_file).is_ok_and(|m| m.len() > 1_000_000) {
            let _ = std::fs::rename(&stderr_file, &prev_stderr);
        }
        let started = Instant::now();
        let mut command = std::process::Command::new(exe);
        command
            .args(args_for(launch))
            .env("SERVYM_SUPERVISOR", "servym")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&stderr_file)
                    .map_or_else(|_| std::process::Stdio::null(), std::process::Stdio::from),
            );
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let code = match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(error) => {
                supervisor_log(data, &format!("[supervisor] {error}"));
                return ExitCode::FAILURE;
            }
        };
        if code == 0 {
            return ExitCode::SUCCESS;
        }
        if code == RESTART_CODE || code == RESTART_WINDOW_CODE {
            launch = Launch::Restart(code);
            continue;
        }
        supervisor_log(
            data,
            &format!("[supervisor] Servym s'est arrêté (code {code}) : voir logs/servym.stderr.log"),
        );
        failures = if started.elapsed() > Duration::from_secs(60) {
            0
        } else {
            failures + 1
        };
        if failures >= 5 {
            supervisor_log(
                data,
                "[supervisor] 5 plantages rapides : abandon (voir les lignes précédentes)",
            );
            return ExitCode::FAILURE;
        }
        launch = Launch::Crash;
        std::thread::sleep(Duration::from_secs(5));
    }
}

/// Ligne dans logs/servym.log, écrite par le superviseur (il n'a pas de console).
pub fn supervisor_log(data: &Path, text: &str) {
    use std::io::Write;
    let dir = data.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("servym.log"))
    {
        let _ = writeln!(
            f,
            "{} {text}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ")
        );
    }
}
