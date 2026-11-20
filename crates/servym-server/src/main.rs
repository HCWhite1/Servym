//! `servym` : le serveur sans écran.
//!
//! ```text
//! servym [--data <dossier>] [--supervised]     démarre Servym
//! servym auth list                             comptes existants
//! servym auth reset-password <nom>             code de réinitialisation valable 15 minutes
//! ```
//!
//! Données (`projects.json`, `logs/`, `runtime/`, `config/`) : `--data`, sinon `SERVYM_HOME`,
//! sinon le dossier courant. `--supervised` : un superviseur relance Servym après un plantage
//! (5 plantages rapides : abandon) et tout de suite quand l'interface le demande (code 75).

use servym_core::i18n::tr;
use servym_core::params;
use servym_server::cli::auth_command;
use servym_server::{App, Listening, RESTART_CODE, bind_host, is_supervised, supervisor};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

fn data_dir(args: &[String]) -> PathBuf {
    if let Some(i) = args.iter().position(|a| a == "--data")
        && let Some(dir) = args.get(i + 1)
    {
        return PathBuf::from(dir);
    }
    std::env::var_os("SERVYM_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// Mode `--supervised` : lancé à l'ouverture de session, sans fenêtre de console.
fn supervise(args: &[String]) -> ExitCode {
    #[cfg(windows)]
    {
        // SAFETY : détache le processus de sa console, sans autre effet.
        unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    }
    let exe = std::env::current_exe().expect("chemin de servym");
    let child_args: Vec<String> = args.iter().filter(|a| *a != "--supervised").cloned().collect();
    supervisor::supervise(&exe, &data_dir(args), |_| child_args.clone())
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

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = sigterm.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn serve(data: PathBuf) -> ExitCode {
    let _ = std::fs::create_dir_all(&data);
    // Redémarrage depuis l'interface : seulement sous un superviseur, qui relance Servym ; lancé à
    // la main, personne ne le relancerait.
    let supervised = is_supervised(std::env::var("SERVYM_SUPERVISOR").ok().as_deref());
    let (restart_tx, mut restart_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let mut builder = App::builder().data_dir(&data);
    if supervised {
        builder = builder.restart(Arc::new(move || {
            let _ = restart_tx.send(());
        }));
    }
    let app = builder.build();
    let config = app.manager().config();
    let port: u16 = std::env::var("SERVYM_PORT")
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
    running.log(&format!("Servym → http://localhost:{port}"));
    manager.init().await;
    // Les projets lancés continuent de tourner : la prochaine instance les reprendra.
    let code = tokio::select! {
        _ = shutdown_signal() => 0,
        _ = restart_rx.recv() => RESTART_CODE,
    };
    running.close().await;
    ExitCode::from(code as u8)
}
