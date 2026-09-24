//! Où vivent les données, et import depuis une autre installation.
//!
//! Dossier des données, dans l'ordre :
//! 1. `--data <dossier>` ;
//! 2. la variable `SERVYNEX_HOME` ;
//! 3. mode portable : le dossier de l'exécutable s'il contient un `projects.json` ;
//! 4. `%LOCALAPPDATA%\Servynex` (local, non itinérant : chemins et ports sont propres à la machine).
//!
//! Import (premier lancement, dossier sans `projects.json`) : `projects.json`, `runtime/`, `logs/`
//! et `config/` sont copiés ; l'ancien dossier n'est pas modifié. Les projets en marche gardent leur
//! journal ouvert dans l'ancien dossier : `pids.json` retient ce chemin (`log`), suivi jusqu'à leur
//! prochain lancement.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Dossier des données (voir le module).
pub fn data_dir(
    args: &[String],
    home: Option<PathBuf>,
    exe_dir: Option<&Path>,
    local: Option<PathBuf>,
) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--data")
        && let Some(dir) = args.get(i + 1)
    {
        return PathBuf::from(dir);
    }
    if let Some(home) = home.filter(|h| !h.as_os_str().is_empty()) {
        return home;
    }
    if let Some(dir) = exe_dir.filter(|d| d.join("projects.json").is_file()) {
        return dir.to_path_buf();
    }
    local.map_or_else(|| PathBuf::from("."), |l| l.join("Servynex"))
}

/// Dossier des données donné par une commande de démarrage (`… --data "C:\Servynex"`).
pub fn data_arg(command: &str) -> Option<PathBuf> {
    let rest = command.split_once("--data")?.1.trim_start();
    let dir = match rest.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => rest.split_whitespace().next()?,
    };
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// Dossier de travail d'une tâche planifiée (sortie XML de `schtasks /query /xml`).
pub fn task_dir(xml: &str) -> Option<PathBuf> {
    let start = xml.find("<WorkingDirectory>")? + "<WorkingDirectory>".len();
    let end = start + xml[start..].find("</WorkingDirectory>")?;
    let dir = xml[start..end].trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Première installation trouvée parmi `candidates` (dans l'ordre), autre que `data`.
pub fn old_install(candidates: &[PathBuf], data: &Path) -> Option<PathBuf> {
    candidates
        .iter()
        .find(|dir| dir.join("projects.json").is_file() && !same_dir(dir, data))
        .cloned()
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copie les données de `from` dans `to` (voir le module). À faire une fois l'ancienne instance
/// arrêtée : `pids.json` et `auth.json` sont alors dans leur état final.
pub fn import(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for dir in ["runtime", "logs", "config"] {
        copy_dir(&from.join(dir), &to.join(dir))?;
    }
    let pids_file = to.join("runtime").join("pids.json");
    if let Some(mut pids) = std::fs::read_to_string(&pids_file)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        && let Some(entries) = pids.as_object_mut()
    {
        for (id, entry) in entries.iter_mut() {
            if entry.get("log").is_none() && entry.is_object() {
                entry["log"] = json!(from.join("logs").join(format!("{id}.log")));
            }
        }
        std::fs::write(&pids_file, serde_json::to_string_pretty(&pids)?)?;
    }
    // En dernier : sa présence marque l'import comme fait.
    std::fs::copy(from.join("projects.json"), to.join("projects.json"))?;
    Ok(())
}
