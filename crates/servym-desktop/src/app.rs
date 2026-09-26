//! Démarrage : superviseur, puis application (serveur, zone de notification, fenêtre).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use servym_core::i18n::tr;
use servym_core::params;
use servym_server::supervisor::{self, Launch};
use servym_server::{App, Listening, RESTART_CODE, RESTART_WINDOW_CODE, RunningApp, Shell, bind_host};
use tauri::{AppHandle, Manager, RunEvent};

use crate::data;
use crate::system::{self, error_box};

/// État de l'application, rangé dans Tauri.
pub struct Desktop {
    pub app: App,
    pub port: u16,
    pub runtime: tokio::runtime::Handle,
    running: Mutex<Option<RunningApp>>,
}

impl Desktop {
    pub fn url(&self) -> String {
        format!("http://localhost:{}/", self.port)
    }

    /// « Ouvrir Servym » : dans la fenêtre ou le navigateur, selon le réglage de la machine.
    pub fn open_ui(&self, handle: &AppHandle) {
        if self.app.opener() == "browser" {
            crate::window::to_browser(&self.url());
        } else if let Err(error) = crate::window::show_main(handle, self.port) {
            self.app.log(&format!("[window] {error}"));
        }
    }
}

struct DesktopShell {
    port: Arc<AtomicU16>,
    handle: AppHandle,
}

impl Shell for DesktopShell {
    fn open_browser(&self) {
        crate::window::to_browser(&format!("http://localhost:{}/", self.port.load(Ordering::SeqCst)));
    }

    fn close_window(&self) {
        if let Some(window) = self.handle.get_webview_window("main") {
            let _ = window.close();
        }
    }
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Servym.exe"));
    let home = std::env::var_os("SERVYM_HOME").map(PathBuf::from);
    let explicit = args.iter().any(|a| a == "--data") || home.is_some();
    let data = data::data_dir(&args, home, exe.parent(), system::default_data_dir());
    let background = args.iter().any(|a| a == "--background");

    // `auth list`, `auth reset-password <nom>` : mêmes commandes que `servym`, sans interface.
    if args.first().map(String::as_str) == Some("auth") {
        system::attach_console();
        return servym_server::cli::auth_command(&data, &args[1..]);
    }

    // Lancé par l'utilisateur ou la clé Run : ce processus devient le superviseur.
    if std::env::var_os("SERVYM_SUPERVISOR").is_none() {
        let base: Vec<String> = args.iter().filter(|a| *a != "--background").cloned().collect();
        let code = supervisor::supervise(&exe, &data, move |launch| {
            // Relancé après un plantage ou un redémarrage : sans rouvrir l'interface, sauf si la
            // fenêtre était ouverte.
            let hidden = match launch {
                Launch::First => background,
                Launch::Restart(code) => code != RESTART_WINDOW_CODE,
                Launch::Crash => true,
            };
            let mut args = base.clone();
            if hidden {
                args.push("--background".into());
            }
            args
        });
        // Abandon après des plantages en boucle : sans ce message, rien ne se passerait à l'écran.
        if code == ExitCode::FAILURE {
            let lang = servym_core::i18n::detect_lang(
                servym_sys::system_locale().as_deref(),
                std::env::var("SERVYM_LANG").ok().as_deref(),
            );
            let logs = std::path::absolute(data.join("logs")).unwrap_or_else(|_| data.join("logs"));
            error_box(&tr(
                "app.gaveUp",
                &params![logs = logs.display().to_string()],
                lang,
            ));
        }
        return code;
    }

    // Une panique est écrite dans le journal : la sortie d'erreur n'est souvent visible par personne.
    let panic_data = data.clone();
    std::panic::set_hook(Box::new(move |info| {
        supervisor::supervisor_log(&panic_data, &format!("[panic] {info}"));
    }));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("environnement asynchrone");
    tauri::async_runtime::set(runtime.handle().clone());
    let autostart = system::autostart(&system::program(&exe), &data, explicit);
    let start = Start {
        data,
        background,
        autostart,
        runtime: runtime.handle().clone(),
    };

    let context = tauri::generate_context!();
    let identifier = context.config().identifier.clone();
    let built = tauri::Builder::default()
        // En premier : un 2ᵉ lancement s'arrête ici et ouvre l'interface de la première instance.
        .plugin(tauri_plugin_single_instance::init(|handle, _args, _cwd| {
            if let Some(desktop) = handle.try_state::<Desktop>() {
                desktop.open_ui(handle);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // Lancé dans la boucle d'événements, où une erreur rendue ferait paniquer Tauri : le
            // problème a déjà été expliqué (port occupé…), on s'arrête sans relance.
            if setup(app, start, &identifier).is_err() {
                std::process::exit(0);
            }
            Ok(())
        })
        .build(context);
    let tauri_app = match built {
        Ok(app) => app,
        Err(error) => {
            error_box(&error.to_string());
            return ExitCode::FAILURE;
        }
    };
    // Tauri rend toujours 0 : le code demandé (redémarrage pour le superviseur) est gardé ici.
    let requested = Arc::new(AtomicI32::new(0));
    let code = requested.clone();
    tauri_app.run_return(move |handle, event| match event {
        // Dernière fenêtre fermée : Servym reste dans la zone de notification.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::ExitRequested { code: Some(c), .. } => requested.store(c, Ordering::SeqCst),
        RunEvent::Exit => {
            let desktop = handle.state::<Desktop>();
            let running = desktop.running.lock().unwrap_or_else(|e| e.into_inner()).take();
            // Les projets continuent de tourner : la prochaine instance les reprendra.
            if let Some(running) = running {
                desktop.runtime.block_on(running.close());
            }
        }
        _ => {}
    });
    drop(runtime);
    ExitCode::from(u8::try_from(code.load(Ordering::SeqCst)).unwrap_or(1))
}

struct Start {
    data: PathBuf,
    background: bool,
    autostart: Arc<dyn servym_sys::Autostart>,
    runtime: tokio::runtime::Handle,
}

fn setup(tauri_app: &mut tauri::App, start: Start, identifier: &str) -> Result<(), String> {
    let handle = tauri_app.handle().clone();
    let Start {
        data,
        background,
        autostart,
        runtime,
    } = start;
    let _ = std::fs::create_dir_all(&data);
    let _enter = runtime.enter();

    let port_cell = Arc::new(AtomicU16::new(0));
    let restart_handle = handle.clone();
    let restart: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let code = if restart_handle.get_webview_window("main").is_some() {
            RESTART_WINDOW_CODE
        } else {
            RESTART_CODE
        };
        restart_handle.exit(code);
    });
    // Ligne « Moteur » des Paramètres : « Rust · Tauri 2.11.6 · WebView2 140.0… ».
    let webview = tauri::webview_version()
        .map(|v| format!(" · {} {v}", system::WEBVIEW))
        .unwrap_or_default();
    let mut builder = App::builder()
        .data_dir(&data)
        .engine(&format!("Rust · Tauri {}{webview}", tauri::VERSION))
        .restart(restart)
        .shell(Arc::new(DesktopShell {
            port: port_cell.clone(),
            handle: handle.clone(),
        }))
        .autostart(autostart);
    if let Some(notifier) = system::notifier(identifier) {
        builder = builder.notifier(notifier);
    }
    let app = builder.build();

    let config = app.manager().config();
    let port: u16 = std::env::var("SERVYM_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| {
            config["instance"]["port"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
        })
        .unwrap_or(9000);
    let host = bind_host(&config["instance"], runtime.block_on(app.protected()));
    app.set_listening(if host == "0.0.0.0" {
        Listening::Lan
    } else {
        Listening::Local
    });
    if config["instance"]["access"] == "lan" && host != "0.0.0.0" {
        app.log(&format!("[listen] {}", app.tr("log.lanRefused")));
    }
    let running = match runtime.block_on(app.listen(&format!("{host}:{port}"))) {
        Ok(running) => running,
        Err(error) => {
            let lang = app.manager().lang();
            let text = if error.kind() == std::io::ErrorKind::AddrInUse {
                app.log(&format!(
                    "[listen] {}",
                    tr("log.portInUse", &params![port = port], lang)
                ));
                tr("app.portInUse", &params![port = port], lang)
            } else {
                app.log(&format!("[listen] {error}"));
                tr("app.startFailed", &params![error = error.to_string()], lang)
            };
            error_box(&text);
            return Err(text);
        }
    };
    running.log(&format!("Servym → http://localhost:{port}"));
    port_cell.store(port, Ordering::SeqCst);
    let manager = app.manager().clone();
    runtime.spawn(async move { manager.init().await });

    tauri_app.manage(Desktop {
        app,
        port,
        runtime,
        running: Mutex::new(Some(running)),
    });
    // Sans zone de notification (Linux sans AppIndicator…), Servym reste utilisable : fenêtre,
    // navigateur, et un 2ᵉ lancement rouvre l'interface.
    let tray = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::tray::install(&handle)));
    match tray {
        Ok(Ok(())) => {}
        Ok(Err(error)) => handle.state::<Desktop>().app.log(&format!("[tray] {error}")),
        Err(_) => handle
            .state::<Desktop>()
            .app
            .log("[tray] zone de notification indisponible"),
    }
    if !background {
        handle.state::<Desktop>().open_ui(&handle);
    }
    Ok(())
}
