//! Premier lancement : import d'une installation existante de Servynex (version Node lancée par la
//! tâche planifiée, ou `servynex.exe` sans écran lancé par la clé Run).
//!
//! 1. Chercher l'ancienne installation : dossier `--data` de la clé Run, dossier de travail de la
//!    tâche planifiée « Servynex », puis `C:\Servynex`.
//! 2. Demander. Sans accord : Servynex démarre sans projets (et ne redemande pas).
//! 3. Arrêter l'ancienne instance : son superviseur (wscript ou servynex) puis son serveur, et eux
//!    seuls (pas leur arborescence) : les projets continuent de tourner. Désactiver la tâche
//!    planifiée (sans « Arrêter la tâche », qui arrêterait ce qu'elle a lancé).
//! 4. Copier les données ([`data::import`]) ; l'ancien dossier n'est pas modifié.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use servynex_core::config::{default_config, load_config, save_config};
use servynex_core::i18n::tr;
use servynex_core::params;
use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_ICONWARNING, MB_YESNO};

use crate::app::{dialog_lang, message};
use crate::data;

/// Import fait : lignes pour le journal de Servynex, démarrage avec la session à reprendre.
pub struct Imported {
    pub notes: Vec<String>,
    pub autostart: bool,
}

/// Commande sans fenêtre de console ; sa sortie standard, ou `None` si elle échoue.
fn output(program: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new(program)
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Tâche planifiée « Servynex » de la version Node : XML de sa définition.
fn task_xml() -> Option<String> {
    output("schtasks", &["/query", "/tn", "Servynex", "/xml"])
}

fn candidates(task: Option<&str>) -> Vec<PathBuf> {
    let mut list = Vec::new();
    if let Some(dir) = servynex_sys::windows::autostart::registered().and_then(|c| data::data_arg(&c)) {
        list.push(dir);
    }
    if let Some(dir) = task.and_then(data::task_dir) {
        list.push(dir);
    }
    list.push(PathBuf::from(r"C:\Servynex"));
    list
}

pub fn first_run(data_dir: &Path) -> Option<Imported> {
    let task = task_xml();
    let old = data::old_install(&candidates(task.as_deref()), data_dir)?;
    let lang = dialog_lang(&old);
    let asked = message(
        &tr("import.title", &params![], lang),
        &tr("import.body", &params![dir = old.display().to_string()], lang),
        MB_YESNO | MB_ICONQUESTION,
    );
    if asked != IDYES {
        // Refusé : ne pas redemander au prochain lancement.
        let _ = save_config(&data_dir.join("projects.json"), &default_config());
        return None;
    }
    let mut notes = Vec::new();
    // Démarrage avec la session : repris s'il était actif (clé Run ou tâche planifiée activée).
    let task_enabled = task
        .as_deref()
        .is_some_and(|xml| !xml.contains("<Enabled>false</Enabled>"));
    let autostart = servynex_sys::windows::autostart::registered().is_some() || task_enabled;
    if task.is_some() && output("schtasks", &["/change", "/tn", "Servynex", "/disable"]).is_some() {
        notes.push(format!("[import] {}", tr("log.importTask", &params![], lang)));
    }
    let port = load_config(&old.join("projects.json"))
        .ok()
        .and_then(|c| c["instance"]["port"].as_u64())
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(9000);
    if let Some(what) = stop_old(port) {
        notes.push(format!(
            "[import] {}",
            tr("log.importStopped", &params![what = what], lang)
        ));
    }
    match data::import(&old, data_dir) {
        Ok(()) => {
            notes.push(format!(
                "[import] {}",
                tr("log.imported", &params![dir = old.display().to_string()], lang)
            ));
            Some(Imported { notes, autostart })
        }
        Err(error) => {
            message(
                "Servynex",
                &tr(
                    "import.failed",
                    &params![dir = old.display().to_string(), error = error.to_string()],
                    lang,
                ),
                MB_ICONWARNING,
            );
            let _ = save_config(&data_dir.join("projects.json"), &default_config());
            None
        }
    }
}

fn terminate(pid: u32) {
    use windows_sys::Win32::System::Threading::{PROCESS_TERMINATE, TerminateProcess};
    if let Some(h) = servynex_sys::windows::procs::open(pid, PROCESS_TERMINATE) {
        // SAFETY : handle ouvert avec le droit d'arrêt.
        unsafe { TerminateProcess(h.0, 0) };
    }
}

/// Arrête l'ancienne instance qui écoute sur `port` : son superviseur d'abord (sinon il la
/// relancerait), puis elle. Seulement si ce sont bien Servynex (node ou servynex) et son
/// superviseur (wscript ou servynex). Rend ce qui a été arrêté, pour le journal.
fn stop_old(port: u16) -> Option<String> {
    let server = servynex_sys::windows::tcp::listeners()
        .into_iter()
        .find(|l| l.port == port)?
        .pid?;
    let processes = servynex_sys::windows::procs::list();
    let name_of = |pid: u32| {
        processes
            .iter()
            .find(|(p, _, _)| *p == pid)
            .map(|(_, parent, name)| (*parent, name.to_lowercase()))
    };
    let (parent, name) = name_of(server)?;
    if !matches!(name.as_str(), "node.exe" | "servynex.exe") {
        return None;
    }
    let mut stopped = Vec::new();
    if let Some((_, parent_name)) = name_of(parent)
        && matches!(parent_name.as_str(), "wscript.exe" | "servynex.exe")
    {
        terminate(parent);
        stopped.push(format!("{parent_name} {parent}"));
    }
    terminate(server);
    stopped.push(format!("{name} {server}"));
    // Port libéré avant que le nouveau serveur ne le prenne.
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end
        && servynex_sys::windows::tcp::listeners()
            .iter()
            .any(|l| l.port == port)
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    Some(stopped.join(", "))
}
