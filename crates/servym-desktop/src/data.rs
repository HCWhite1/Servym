//! Où vivent les données, dans l'ordre :
//! 1. `--data <dossier>` ;
//! 2. la variable `SERVYM_HOME` ;
//! 3. mode portable : le dossier de l'exécutable s'il contient un `projects.json` ;
//! 4. le dossier par défaut ([`default_dir`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Dossier des données (voir le module). `default` : [`default_dir`].
pub fn data_dir(
    args: &[String],
    home: Option<PathBuf>,
    exe_dir: Option<&Path>,
    default: Option<PathBuf>,
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
    default.unwrap_or_else(|| PathBuf::from("."))
}

/// Dossier par défaut, local à la machine (chemins et ports lui sont propres) :
/// `%LOCALAPPDATA%\Servym` sous Windows (non itinérant) ; ailleurs `$XDG_DATA_HOME/servym`, sinon
/// `~/.local/share/servym`. `var` lit une variable d'environnement.
pub fn default_dir(windows: bool, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let var = |name: &str| var(name).filter(|v| !v.is_empty());
    if windows {
        return var("LOCALAPPDATA").map(|l| PathBuf::from(l).join("Servym"));
    }
    var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))
        .map(|d| d.join("servym"))
}
