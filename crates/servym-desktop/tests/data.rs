//! Dossier des données (phase 6 ; Linux : phase 7).

use std::ffi::OsString;
use std::path::PathBuf;

use servym_desktop::data::{data_dir, default_dir};
use servym_fixtures::{temp_dir, write};

#[test]
fn dossier_des_donnees() {
    let exe = temp_dir("servym-d-");
    let local = PathBuf::from("L").join("Servym");
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Par défaut : le dossier du système.
    assert_eq!(data_dir(&[], None, Some(exe.path()), Some(local.clone())), local);
    // SERVYM_HOME, puis --data qui prime.
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

#[test]
fn dossier_par_defaut() {
    let vars = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| OsString::from(v))
        }
    };
    assert_eq!(
        default_dir(true, vars(&[("LOCALAPPDATA", "C:/U/AppData/Local")])),
        Some(PathBuf::from("C:/U/AppData/Local").join("Servym"))
    );
    assert_eq!(
        default_dir(false, vars(&[("HOME", "/home/a")])),
        Some(PathBuf::from("/home/a/.local/share").join("servym"))
    );
    assert_eq!(
        default_dir(false, vars(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "/d")])),
        Some(PathBuf::from("/d").join("servym"))
    );
    assert_eq!(
        default_dir(false, vars(&[("HOME", "/home/a"), ("XDG_DATA_HOME", "")])),
        Some(PathBuf::from("/home/a/.local/share").join("servym")),
        "XDG_DATA_HOME vide : ignoré"
    );
    assert_eq!(default_dir(false, vars(&[])), None);
}
