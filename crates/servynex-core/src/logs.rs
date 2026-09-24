//! Journaux des projets : un fichier par projet, lu par la fin et suivi en direct.

use std::path::Path;

/// Dernières lignes d'un journal (codes ANSI retirés).
pub fn tail_lines(file: &Path, count: usize) -> Vec<String> {
    let _ = (file, count);
    unimplemented!("phase 2")
}

/// Ligne la plus parlante pour expliquer un arrêt : la dernière erreur, sinon la dernière ligne.
pub fn last_error(file: &Path) -> Option<String> {
    let _ = file;
    unimplemented!("phase 2")
}

/// Programme sorti sans erreur pendant son lancement : sa dernière erreur, sinon son dernier
/// avertissement, sinon rien (la dernière ligne seule n'expliquerait rien).
pub fn startup_reason(file: &Path) -> Option<String> {
    let _ = file;
    unimplemented!("phase 2")
}
