//! Dossier des données et import depuis une autre installation (phase 6).

use std::path::PathBuf;

use serde_json::{Value, json};
use servynex_desktop::data::{data_arg, data_dir, import, old_install, task_dir};
use servynex_fixtures::{read, temp_dir, write};

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

#[test]
fn anciennes_installations() {
    assert_eq!(
        data_arg(r#""C:\p\servynex.exe" --supervised --data "C:\Servynex""#),
        Some(PathBuf::from(r"C:\Servynex"))
    );
    assert_eq!(data_arg(r#""C:\p\Servynex.exe" --background"#), None);
    assert_eq!(
        task_dir(
            "<Exec><Command>wscript.exe</Command><WorkingDirectory>C:\\Servynex</WorkingDirectory></Exec>"
        ),
        Some(PathBuf::from(r"C:\Servynex"))
    );
    assert_eq!(task_dir("<Exec></Exec>"), None);

    let old = temp_dir("servynex-d-");
    let empty = temp_dir("servynex-d-");
    let data = temp_dir("servynex-d-");
    write(old.path().join("projects.json"), "{}");
    let candidates = [empty.path().to_path_buf(), old.path().to_path_buf()];
    assert_eq!(
        old_install(&candidates, data.path()),
        Some(old.path().to_path_buf())
    );
    // Le dossier des données lui-même n'est pas une ancienne installation.
    assert_eq!(old_install(&candidates, old.path()), None);
}

#[test]
fn import_des_donnees() {
    let old = temp_dir("servynex-d-");
    let new = temp_dir("servynex-d-");
    let to = new.path().join("Servynex");
    write(old.path().join("projects.json"), r#"{"projects":[]}"#);
    write(old.path().join("runtime/auth.json"), r#"{"enabled":true}"#);
    write(
        old.path().join("runtime/pids.json"),
        json!({ "api": { "pid": 42 }, "db": { "pid": 43, "log": "X:\\ailleurs.log" } }).to_string(),
    );
    write(old.path().join("logs/api.log"), "ligne\n");
    write(old.path().join("config/mysql.ini"), "[mysqld]\n");
    import(old.path(), &to).unwrap();

    assert_eq!(read(to.join("projects.json")), r#"{"projects":[]}"#);
    assert_eq!(read(to.join("runtime/auth.json")), r#"{"enabled":true}"#);
    assert_eq!(read(to.join("logs/api.log")), "ligne\n");
    assert_eq!(read(to.join("config/mysql.ini")), "[mysqld]\n");
    let pids: Value = serde_json::from_str(&read(to.join("runtime/pids.json"))).unwrap();
    assert_eq!(pids["api"]["pid"], 42);
    assert_eq!(pids["api"]["log"], json!(old.path().join("logs").join("api.log")));
    assert_eq!(pids["db"]["log"], "X:\\ailleurs.log", "déjà ailleurs : gardé");
    // L'ancien dossier n'est pas modifié.
    let before: Value = serde_json::from_str(&read(old.path().join("runtime/pids.json"))).unwrap();
    assert!(before["api"].get("log").is_none());
}
