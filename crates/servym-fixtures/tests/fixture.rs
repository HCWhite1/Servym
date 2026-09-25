//! Le harnais lui-même : applications factices et client HTTP brut.

use std::process::{Command, Stdio};

use servym_fixtures::{Req, fixture_exe, free_port, listening, sleep, temp_dir, until};

/// Programme factice lancé directement (sans Servym).
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn(mode: &str, port: u16, cwd: &std::path::Path) -> Running {
    Running(
        Command::new(fixture_exe())
            .arg(mode)
            .current_dir(cwd)
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("SERVYM_TEST_ENV", "bonjour")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn echo_sorties_et_code() {
    let out = Command::new(fixture_exe())
        .args(["echo", "sortie $HOME", "erreur", "3"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "sortie $HOME");
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), "erreur");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn crash_et_launcher() {
    let crash = Command::new(fixture_exe()).arg("crash").output().unwrap();
    assert_eq!(crash.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&crash.stderr).contains("Error: base introuvable"));
    let launcher = Command::new(fixture_exe()).arg("launcher").output().unwrap();
    assert_eq!(launcher.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&launcher.stdout)
            .contains("[WARNING] The local web server is already running")
    );
}

#[tokio::test]
async fn serveur_factice_et_client_brut() {
    let dir = temp_dir("servym-fixture-");
    let port = free_port();
    let _server = spawn("server", port, dir.path());
    until(|| listening(port), 10_000).await;
    let res = Req::get("/").send(port).await;
    assert_eq!(res.status, 200);
    assert_eq!(res.body, "<title>Factice</title>");
    assert_eq!(res.header("Content-Type"), Some("text/html; charset=utf-8"));
}

#[tokio::test]
async fn relance_d_elle_meme() {
    let dir = temp_dir("servym-fixture-");
    let port = free_port();
    let _first = spawn("respawn", port, dir.path());
    until(|| listening(port), 10_000).await;
    assert_eq!(Req::get("/").send(port).await.body, "<title>Relance</title>");
    assert_eq!(Req::get("/restart").send(port).await.body, "ok");
    // La nouvelle instance reprend le port.
    sleep(300).await;
    until(
        || async move {
            Req::get("/")
                .try_send(port)
                .await
                .is_ok_and(|r| r.body.contains("Relance"))
        },
        10_000,
    )
    .await;
    // Arrêt de la nouvelle instance (détachée) : par son propre /restart impossible, donc on la tue.
    stop_listener(port);
}

#[tokio::test]
async fn flaky_compte_ses_lancements() {
    let dir = temp_dir("servym-fixture-");
    let port = free_port();
    let server = Running(
        Command::new(fixture_exe())
            .arg("flaky")
            .current_dir(dir.path())
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("CRASHES", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    until(|| listening(port), 10_000).await;
    assert_eq!(std::fs::read_to_string(dir.path().join("runs.txt")).unwrap(), "1");
    let mut child = server;
    let status = tokio::task::spawn_blocking(move || {
        let status = child.0.wait().unwrap();
        std::mem::forget(child);
        status
    })
    .await
    .unwrap();
    assert_eq!(status.code(), Some(3), "plante 1,5 s après son premier lancement");
}

/// Tue le processus qui écoute sur ce port (nouvelle instance détachée d'un test).
fn stop_listener(port: u16) {
    #[cfg(windows)]
    {
        let out = Command::new("netstat")
            .args(["-ano", "-p", "TCP"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        let pid = text.lines().find_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            (cols.len() == 5 && cols[1] == format!("127.0.0.1:{port}") && cols[2].ends_with(":0"))
                .then(|| cols[4].to_string())
        });
        if let Some(pid) = pid {
            let _ = Command::new("taskkill").args(["/PID", &pid, "/F"]).output();
        }
    }
    #[cfg(unix)]
    {
        let _ = Command::new("fuser")
            .args(["-k", &format!("{port}/tcp")])
            .output();
    }
}
