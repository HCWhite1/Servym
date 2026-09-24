//! Dossier des données (phase 6).

use std::path::PathBuf;

use servynex_desktop::data::data_dir;
use servynex_fixtures::{temp_dir, write};

#[test]
fn dossier_des_donnees() {
    let exe = temp_dir("servynex-d-");
    let local = PathBuf::from("L");
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Par défaut : %LOCALAPPDATA%\Servynex.
    assert_eq!(
        data_dir(&[], None, Some(exe.path()), Some(local.clone())),
        local.join("Servynex")
    );
    // SERVYNEX_HOME, puis --data qui prime.
    assert_eq!(
        data_dir(&[], Some("H".into()), Some(exe.path()), Some(local.clone())),
        PathBuf::from("H")
    );
    assert_eq!(
        data_dir(
            &args(&["--data", "D"]),
            Some("H".into()),
            None,
            Some(local.clone())
        ),
        PathBuf::from("D")
    );
    // Mode portable : projects.json à côté de l'exécutable.
    write(exe.path().join("projects.json"), "{}");
    assert_eq!(data_dir(&[], None, Some(exe.path()), Some(local)), exe.path());
}
