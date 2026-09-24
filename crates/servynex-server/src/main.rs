//! `servynex` : le serveur sans écran.
//!
//! ```text
//! servynex [--data <dossier>] [--supervised]     démarre Servynex
//! servynex auth list                             comptes existants
//! servynex auth reset-password <nom>             code de réinitialisation valable 15 minutes
//! ```
//!
//! Données (`projects.json`, `logs/`, `runtime/`, `config/`) : `--data`, sinon `SERVYNEX_HOME`,
//! sinon le dossier courant. `--supervised` : un superviseur relance Servynex après un plantage
//! (5 plantages rapides : abandon) et tout de suite quand l'interface le demande (code 75), comme
//! `scripts/start-hidden.vbs`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use servynex_core::auth::{AuthStore, create_reset_code, role_of};
use servynex_core::config::load_config;
use servynex_core::i18n::{Lang, detect_lang, tr};
use servynex_core::params;
use servynex_server::{App, Listening, RESTART_CODE, bind_host, is_supervised};

fn data_dir(args: &[String]) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--data")
        && let Some(dir) = args.get(i + 1)
    {
        return PathBuf::from(dir);
    }
    std::env::var_os("SERVYNEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// Même langue que le serveur : `instance.lang`, sinon celle du système.
fn cli_lang(data: &Path) -> Lang {
    let system = detect_lang(
        servynex_sys::system_locale().as_deref(),
        std::env::var("SERVYNEX_LANG").ok().as_deref(),
    );
    load_config(&data.join("projects.json"))
        .ok()
        .and_then(|c| c["instance"]["lang"].as_str().and_then(Lang::pick))
        .unwrap_or(system)
}

/// Secours en ligne de commande pour les comptes, à lancer sur la machine. Avoir accès à cette
/// machine donne de toute façon la main sur Servynex : ces commandes n'ouvrent aucune porte
/// nouvelle. Dernier recours : supprimer runtime/auth.json retire la protection (et les comptes).
fn auth_command(data: &Path, args: &[String]) -> ExitCode {
    let lang = cli_lang(data);
    let t = |key: &str, p| tr(key, &p, lang);
    let mut store = AuthStore::new(&data.join("runtime").join("auth.json"));
    if let Some(error) = store.error() {
        eprintln!("{}", t("auth.brokenFile", params![error = error.to_string()]));
        return ExitCode::FAILURE;
    }
    let command = args.first().map(String::as_str);
    match (command, args.get(1)) {
        (Some("list"), _) => {
            let data = store.load().clone();
            let users = data["users"].as_array().cloned().unwrap_or_default();
            if users.is_empty() {
                println!("{}", t("cli.noAccounts", params![]));
            } else {
                let key = if data["enabled"] == true {
                    "cli.summaryOn"
                } else {
                    "cli.summaryOff"
                };
                println!("{}", t(key, params![count = users.len()]));
                for u in users {
                    let username = u["username"].as_str().unwrap_or_default();
                    let perms: Vec<String> = u["permissions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect();
                    let role = t(&format!("role.{}", role_of(&perms)), params![]);
                    let name = u["name"].as_str().filter(|n| !n.is_empty() && *n != username);
                    let name = name.map(|n| format!(" ({n})")).unwrap_or_default();
                    println!("  {username:<20} {role}{name}");
                }
            }
            ExitCode::SUCCESS
        }
        (Some("reset-password"), Some(name)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            match create_reset_code(&mut store, &name.to_lowercase(), now) {
                Ok(code) => {
                    println!(
                        "{}",
                        t("cli.resetCode", params![name = name.as_str(), code = code])
                    );
                    println!("{}", t("cli.resetHelp", params![]));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{}", error.message_in(lang));
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            println!("{}", t("cli.usage", params![]));
            if command.is_some() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}

/// Superviseur : relance Servynex après un plantage, mais abandonne s'il plante en boucle (port
/// déjà pris par une autre instance, projects.json invalide…) ; code 75 : redémarrage demandé
/// depuis l'interface, relancé aussitôt et non compté comme un plantage.
fn supervise(args: &[String]) -> ExitCode {
    #[cfg(windows)]
    {
        // Lancé à l'ouverture de session : pas de fenêtre de console.
        // SAFETY : détache le processus de sa console, sans autre effet.
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    }
    let exe = std::env::current_exe().expect("chemin de servynex");
    let child_args: Vec<&String> = args.iter().filter(|a| *a != "--supervised").collect();
    let mut failures = 0;
    loop {
        let started = Instant::now();
        let mut command = std::process::Command::new(&exe);
        command.args(&child_args).env("SERVYNEX_SUPERVISOR", "servynex");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let code = match command.status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(_) => return ExitCode::FAILURE,
        };
        if code == 0 {
            return ExitCode::SUCCESS;
        }
        if code != RESTART_CODE {
            failures = if started.elapsed() > Duration::from_secs(60) {
                0
            } else {
                failures + 1
            };
            if failures >= 5 {
                return ExitCode::FAILURE;
            }
            std::thread::sleep(Duration::from_secs(5));
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let data = data_dir(&args);
    if args.first().map(String::as_str) == Some("auth") {
        return auth_command(&data, &args[1..]);
    }
    if args.iter().any(|a| a == "--supervised") {
        return supervise(&args);
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("environnement asynchrone");
    runtime.block_on(serve(data))
}

async fn serve(data: PathBuf) -> ExitCode {
    let _ = std::fs::create_dir_all(&data);
    // Redémarrage depuis l'interface : seulement sous un superviseur, qui relance Servynex ; lancé à
    // la main, personne ne le relancerait.
    let supervised = is_supervised(std::env::var("SERVYNEX_SUPERVISOR").ok().as_deref());
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut builder = App::builder().data_dir(&data);
    if supervised {
        builder = builder.restart(Arc::new(move || {
            let _ = restart_tx.send(());
        }));
    }
    let app = builder.build();
    let config = app.manager().config();
    let port: u16 = std::env::var("SERVYNEX_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| config["instance"]["port"].as_u64().map(|p| p as u16))
        .unwrap_or(9000);
    // Ce PC seulement, sauf accès réseau demandé avec la protection active : le panneau lance des
    // programmes.
    let host = bind_host(&config["instance"], app.protected().await);
    app.set_listening(if host == "0.0.0.0" {
        Listening::Lan
    } else {
        Listening::Local
    });
    if config["instance"]["access"] == "lan" && host != "0.0.0.0" {
        app.log(&format!("[listen] {}", app.tr("log.lanRefused")));
    }
    let manager = app.manager().clone();
    let lang = manager.lang();
    let log_line = |text: &str| app.log(text);
    let running = match app.listen(&format!("{host}:{port}")).await {
        Ok(running) => running,
        Err(error) => {
            let text = if error.kind() == std::io::ErrorKind::AddrInUse {
                tr("log.portInUse", &params![port = port], lang)
            } else {
                error.to_string()
            };
            log_line(&format!("[listen] {text}"));
            return ExitCode::FAILURE;
        }
    };
    running.log(&format!("Servynex → http://localhost:{port}"));
    manager.init().await;
    // Les projets lancés continuent de tourner : la prochaine instance les reprendra.
    let code = tokio::select! {
        _ = tokio::signal::ctrl_c() => 0,
        _ = restart_rx.recv() => RESTART_CODE,
    };
    running.close().await;
    ExitCode::from(code as u8)
}
