//! Où vivent les données, dans l'ordre :
//! 1. `--data <dossier>` ;
//! 2. la variable `SERVYNEX_HOME` ;
//! 3. mode portable : le dossier de l'exécutable s'il contient un `projects.json` ;
//! 4. `%LOCALAPPDATA%\Servynex` (local, non itinérant : chemins et ports sont propres à la machine).

use std::path::{Path, PathBuf};

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
