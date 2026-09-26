//! Ce qui diffère entre Windows et Linux : message d'erreur, notifications, démarrage avec la
//! session, dossier des données.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use servym_sys::Autostart;
use servym_sys::notify::Notifier;

/// Libellé « Démarrer avec Windows » ou « Démarrer avec la session » du menu de l'icône.
pub const AUTOSTART_LABEL: &str = if cfg!(windows) {
    "tray.autostart"
} else {
    "tray.autostartSession"
};

/// Moteur web de la fenêtre.
pub const WEBVIEW: &str = if cfg!(windows) { "WebView2" } else { "WebKitGTK" };

/// Dossier des données par défaut (voir [`crate::data::default_dir`]).
pub fn default_data_dir() -> Option<PathBuf> {
    crate::data::default_dir(cfg!(windows), |name: &str| std::env::var_os(name))
}

/// Programme à lancer à l'ouverture de session. Linux : une AppImage s'exécute depuis un dossier
/// monté le temps de sa vie ; c'est le fichier `.AppImage` qu'il faut relancer (`$APPIMAGE`).
pub fn program(exe: &Path) -> PathBuf {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|a| !a.is_empty()) {
        return PathBuf::from(appimage);
    }
    exe.to_path_buf()
}

/// Démarrage avec la session, sans fenêtre (`--background`) ; `data` : dossier des données choisi
/// explicitement (`--data` ou `SERVYM_HOME`), à redonner au lancement.
pub fn autostart(program: &Path, data: &Path, explicit: bool) -> Arc<dyn Autostart> {
    let abs = std::path::absolute(data).unwrap_or_else(|_| data.to_path_buf());
    #[cfg(windows)]
    {
        let mut command = format!("\"{}\" --background", program.display());
        if explicit {
            command.push_str(&format!(" --data \"{}\"", abs.display()));
        }
        Arc::new(servym_sys::windows::autostart::RunKey { command })
    }
    #[cfg(target_os = "linux")]
    {
        use servym_sys::linux::{SystemdTask, Unit};
        let mut args = vec!["--background".to_string()];
        if explicit {
            args.extend(["--data".to_string(), abs.to_string_lossy().into_owned()]);
        }
        let unit = Unit {
            exe: program.to_path_buf(),
            args,
            workdir: abs,
            graphical: true,
        };
        Arc::new(SystemdTask::for_user(unit, servym_sys::run_command()))
    }
}

/// Notifications au nom de l'application. Windows : seulement une fois installée (le raccourci du
/// menu Démarrer déclare son identifiant), sinon celui de Windows PowerShell (`None` : celui par
/// défaut). Linux : `notify-send`, celui par défaut.
pub fn notifier(identifier: &str) -> Option<Arc<dyn Notifier>> {
    #[cfg(windows)]
    {
        let installed = std::env::var_os("APPDATA").is_some_and(|appdata| {
            Path::new(&appdata)
                .join(r"Microsoft\Windows\Start Menu\Programs\Servym.lnk")
                .is_file()
        });
        installed.then(|| {
            Arc::from(servym_sys::notify::app_notifier(
                identifier,
                servym_sys::run_command(),
            ))
        })
    }
    #[cfg(target_os = "linux")]
    {
        let _ = identifier;
        None
    }
}

/// Sorties vers le terminal qui a lancé la commande. Windows : l'application n'a pas de console
/// à elle (sous-système « windows »), elle emprunte celle du terminal parent.
pub fn attach_console() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
        // SAFETY : sans effet si le parent n'a pas de console.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}

/// Message d'erreur (avant la fenêtre, ou sans elle).
#[cfg(windows)]
pub fn error_box(text: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONERROR, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
    };
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let (title, text) = (wide("Servym"), wide(text));
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

/// Message d'erreur (avant la fenêtre, ou sans elle) : boîte de dialogue du bureau (zenity pour
/// GNOME et les autres, kdialog pour KDE), sinon une notification ; toujours sur la sortie
/// d'erreur. Programmes lancés sans shell : le texte n'est jamais interprété.
#[cfg(target_os = "linux")]
pub fn error_box(text: &str) {
    use std::process::{Command, Stdio};
    eprintln!("Servym : {text}");
    let attempts: [(&str, Vec<&str>); 3] = [
        (
            "zenity",
            vec!["--error", "--no-markup", "--title=Servym", "--text", text],
        ),
        ("kdialog", vec!["--title", "Servym", "--error", text]),
        (
            "notify-send",
            vec!["--app-name=Servym", "--urgency=critical", "Servym", text],
        ),
    ];
    for (cmd, args) in attempts {
        let status = Command::new(cmd)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // 255 : zenity trop ancien pour --no-markup, ou pas d'écran ; on essaie le suivant.
        if status.is_ok_and(|s| s.code() != Some(255)) {
            return;
        }
    }
}
