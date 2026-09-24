//! Démarrage : superviseur, puis application (serveur, zone de notification, fenêtre).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicI32, AtomicU16, Ordering};
use std::sync::{Arc, Mutex};

use servynex_core::i18n::tr;
use servynex_core::params;
use servynex_server::supervisor::{self, Launch};
use servynex_server::{App, Listening, RESTART_CODE, RESTART_WINDOW_CODE, RunningApp, Shell, bind_host};
use tauri::{AppHandle, Manager, RunEvent};

use crate::data;

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

    /// « Ouvrir Servynex » : dans la fenêtre ou le navigateur, selon le réglage de la machine.
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

/// Boîte d'erreur de Windows (avant la fenêtre, ou sans elle).
fn error_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (title, text) = (wide("Servynex"), wide(text));
    // SAFETY : textes UTF-16 terminés par un zéro, vivants pendant l'appel.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

/// Raccourci du menu Démarrer posé par l'installateur : il déclare l'identifiant de l'application,
/// au nom duquel les notifications peuvent s'afficher.
fn installed() -> bool {
    std::env::var_os("APPDATA").is_some_and(|appdata| {
        Path::new(&appdata)
            .join(r"Microsoft\Windows\Start Menu\Programs\Servynex.lnk")
            .is_file()
    })
}

pub fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("Servynex.exe"));
    let home = std::env::var_os("SERVYNEX_HOME").map(PathBuf::from);
    let explicit = args.iter().any(|a| a == "--data") || home.is_some();
    let data = data::data_dir(
        &args,
        home,
        exe.parent(),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    );
    let background = args.iter().any(|a| a == "--background");

    // Lancé par l'utilisateur ou la clé Run : ce processus devient le superviseur.
    if std::env::var_os("SERVYNEX_SUPERVISOR").is_none() {
        let base: Vec<String> = args.iter().filter(|a| *a != "--background").cloned().collect();
        return supervisor::supervise(&exe, &data, move |launch| {
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
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("environnement asynchrone");
    tauri::async_runtime::set(runtime.handle().clone());
    let mut autostart = format!("\"{}\" --background", exe.display());
    if explicit {
        let abs = std::path::absolute(&data).unwrap_or_else(|_| data.clone());
        autostart.push_str(&format!(" --data \"{}\"", abs.display()));
    }
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
        // Dernière fenêtre fermée : Servynex reste dans la zone de notification.
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
    autostart: String,
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
    let run_key = Arc::new(servynex_sys::windows::autostart::RunKey { command: autostart });
    let mut builder = App::builder()
        .data_dir(&data)
        .restart(restart)
        .shell(Arc::new(DesktopShell {
            port: port_cell.clone(),
            handle: handle.clone(),
        }))
        .autostart(run_key);
    if installed() {
        builder = builder.notifier(Arc::from(servynex_sys::notify::app_notifier(
            identifier,
            servynex_sys::run_command(),
        )));
    }
    let app = builder.build();

    let config = app.manager().config();
    let port: u16 = std::env::var("SERVYNEX_PORT")
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
    running.log(&format!("Servynex → http://localhost:{port}"));
    port_cell.store(port, Ordering::SeqCst);
    let manager = app.manager().clone();
    runtime.spawn(async move { manager.init().await });

    tauri_app.manage(Desktop {
        app,
        port,
        runtime,
        running: Mutex::new(Some(running)),
    });
    crate::tray::install(&handle).map_err(|e| e.to_string())?;
    if !background {
        handle.state::<Desktop>().open_ui(&handle);
    }
    Ok(())
}
