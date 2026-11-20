//! Lancement sans PowerShell.
//!
//! Un projet : `cmd.exe /d /s /c "chcp 65001 >nul & "arg" "arg"…"` (cmd.exe résout PATHEXT :
//! npm.cmd, symfony…), sans fenêtre (`CREATE_NO_WINDOW` : les sous-processus en mode console en
//! héritent), hors du « job » de Servym quand c'est permis (le projet survit à Servym).
//! Seuls le journal (sorties standard et d'erreur, en ajout) et l'entrée vide sont transmis :
//! `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` empêche toute autre fuite de handle vers le projet.
//!
//! Invariant : chaque argument est entre guillemets, et la configuration garantit qu'il ne
//! contient ni guillemet, ni %, ni retour à la ligne, ni & | < > ^ ( ). Ne pas relâcher l'un sans
//! l'autre.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, GENERIC_READ, GetLastError, HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_APPEND_DATA, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_ALWAYS, OPEN_EXISTING, SYNCHRONIZE,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
};

use super::procs::{Owned, watch_handle};
use crate::{LaunchError, LaunchOptions};

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Chemin de cmd.exe (`ComSpec`).
pub fn comspec() -> String {
    std::env::var("ComSpec").unwrap_or_else(|_| {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        format!(r"{root}\System32\cmd.exe")
    })
}

/// Ligne de commande de cmd.exe pour un projet (voir l'invariant en tête du module).
///
/// Deux cmd.exe : le premier passe la console en UTF-8 (`chcp 65001`), le second, démarré
/// après, lance le projet. cmd.exe choisit l'encodage de ses propres messages (« commande
/// introuvable ») à son démarrage quand sa sortie est un fichier : seul un cmd.exe démarré après
/// `chcp` les écrit en UTF-8 dans le journal. Les programmes lancés partagent la console, déjà en
/// UTF-8. Les barres obliques inversées en fin d'argument sont doublées pour que le guillemet
/// fermant ne soit pas échappé par l'analyseur C (`CommandLineToArgvW`).
pub fn project_command_line(argv: &[String]) -> String {
    let quoted: Vec<String> = argv
        .iter()
        .map(|a| {
            let trailing = a.bytes().rev().take_while(|&b| b == b'\\').count();
            format!("\"{a}{}\"", "\\".repeat(trailing))
        })
        .collect();
    let cmd = comspec();
    format!(
        "\"{cmd}\" /d /s /c \"chcp 65001 >nul & \"{cmd}\" /d /s /c \"{}\"\"",
        quoted.join(" ")
    )
}

/// Argument cité selon les règles de la bibliothèque C de Microsoft (programmes du bureau).
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// Bloc d'environnement UTF-16, trié sans tenir compte de la casse (comme l'attend Windows).
fn env_block(env: &HashMap<String, String>) -> Vec<u16> {
    let mut vars: Vec<(&String, &String)> = env
        .iter()
        .filter(|(k, _)| !k.is_empty() && !k.contains('='))
        .collect();
    vars.sort_by_key(|(k, _)| k.to_uppercase());
    let mut block = Vec::new();
    for (k, v) in vars {
        block.extend(OsStr::new(&format!("{k}={v}")).encode_wide());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

/// Liste d'attributs de lancement ne contenant que la liste des handles transmis.
struct HandleList {
    buf: Vec<u64>,
    // Les handles doivent rester en place tant que la liste sert.
    _handles: Box<[HANDLE]>,
}

impl HandleList {
    fn new(handles: &[HANDLE]) -> std::io::Result<HandleList> {
        let handles: Box<[HANDLE]> = handles.into();
        let mut size = 0usize;
        // SAFETY : premier appel pour connaître la taille (échoue exprès avec un tampon nul).
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
        let mut buf = vec![0u64; size.div_ceil(8)];
        let list = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        // SAFETY : tampon de la taille demandée ; les handles vivent aussi longtemps que la liste.
        unsafe {
            if InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(&*handles),
                null_mut(),
                null(),
            ) == 0
            {
                let err = std::io::Error::last_os_error();
                DeleteProcThreadAttributeList(list);
                return Err(err);
            }
        }
        Ok(HandleList {
            buf,
            _handles: handles,
        })
    }

    fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST
    }
}

impl Drop for HandleList {
    fn drop(&mut self) {
        // SAFETY : liste initialisée par `new`.
        unsafe { DeleteProcThreadAttributeList(self.ptr()) };
    }
}

/// Handle héritable (transmis au projet par la liste des handles).
fn inheritable_file(path: &Path, access: u32, disposition: u32) -> std::io::Result<Owned> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: TRUE,
    };
    let name = wide(path);
    // SAFETY : nom terminé par un zéro, attributs de sécurité valides.
    let h = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &sa,
            disposition,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(Owned(h))
    }
}

/// Crée un processus. `std_handles` : entrée, sortie (et erreur), transmis seuls ; sans eux, aucun
/// handle n'est transmis. Rend le handle du processus et son PID.
pub fn create_process(
    application: Option<&str>,
    command_line: &str,
    cwd: Option<&Path>,
    env: Option<&HashMap<String, String>>,
    flags: u32,
    std_handles: Option<(HANDLE, HANDLE)>,
) -> std::io::Result<(Owned, u32)> {
    let app = application.map(wide);
    let mut cmdline = wide(command_line);
    let dir = cwd.map(wide);
    let block = env.map(env_block);
    // SAFETY : structures initialisées à zéro puis renseignées ; tous les pointeurs restent
    // valides pendant l'appel ; les handles rendus sont fermés par `Owned`.
    unsafe {
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = if std_handles.is_some() {
            size_of::<STARTUPINFOEXW>() as u32
        } else {
            size_of::<STARTUPINFOW>() as u32
        };
        let mut list = None;
        let mut flags = flags | CREATE_UNICODE_ENVIRONMENT;
        if let Some((input, output)) = std_handles {
            si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = input;
            si.StartupInfo.hStdOutput = output;
            si.StartupInfo.hStdError = output;
            let mut l = HandleList::new(&[input, output])?;
            si.lpAttributeList = l.ptr();
            list = Some(l);
            flags |= EXTENDED_STARTUPINFO_PRESENT;
        }
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ref().map_or(null(), |a| a.as_ptr()),
            cmdline.as_mut_ptr(),
            null(),
            null(),
            i32::from(std_handles.is_some()),
            flags,
            block.as_ref().map_or(null(), |b| b.as_ptr().cast()),
            dir.as_ref().map_or(null(), |d| d.as_ptr()),
            &si.StartupInfo,
            &mut pi,
        );
        let err = GetLastError();
        drop(list);
        if ok == 0 {
            return Err(std::io::Error::from_raw_os_error(err as i32));
        }
        drop(Owned(pi.hThread));
        Ok((Owned(pi.hProcess), pi.dwProcessId))
    }
}

fn has_cmd_metachar(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            '"' | '%' | '\r' | '\n' | '&' | '|' | '<' | '>' | '^' | '(' | ')'
        )
    })
}

/// Lance un projet ; rend le PID de cmd.exe, racine de son arborescence.
pub fn launch(argv: &[String], options: LaunchOptions) -> Result<u32, LaunchError> {
    if argv.iter().any(|a| has_cmd_metachar(a)) {
        return Err(LaunchError::Other("caractère refusé dans la commande".into()));
    }
    let other = |e: std::io::Error| LaunchError::Other(e.to_string());
    let log = inheritable_file(&options.log, FILE_APPEND_DATA | SYNCHRONIZE, OPEN_ALWAYS).map_err(other)?;
    let nul = inheritable_file(Path::new("NUL"), GENERIC_READ, OPEN_EXISTING).map_err(other)?;
    let command_line = project_command_line(argv);
    let app = comspec();
    let start = |flags| {
        create_process(
            Some(&app),
            &command_line,
            Some(&options.cwd),
            Some(&options.env),
            flags,
            Some((nul.0, log.0)),
        )
    };
    let (process, pid) = match start(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB) {
        // Job qui interdit d'en sortir (rare) : le projet y reste, et s'arrêtera avec Servym.
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => start(CREATE_NO_WINDOW),
        other => other,
    }
    .map_err(other)?;
    drop((log, nul));
    watch_handle(process, pid, options.on_exit);
    Ok(pid)
}

/// Programme du bureau détaché (Explorer, VS Code, terminal) : aucun handle transmis.
pub fn detach(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    env: Option<&HashMap<String, String>>,
    console: bool,
) -> Result<(), String> {
    let mut line = quote_arg(program);
    for a in args {
        line.push(' ');
        line.push_str(&quote_arg(a));
    }
    let flags = if console { CREATE_NEW_CONSOLE } else { 0 };
    create_process(None, &line, cwd, env, flags, None)
        .map(|_| ())
        .map_err(|e| format!("{program} : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn citation_des_arguments() {
        assert_eq!(quote_arg(r"C:\a"), r"C:\a");
        assert_eq!(quote_arg(r"C:\Program Files\a"), r#""C:\Program Files\a""#);
        assert_eq!(quote_arg(r"C:\dossier\"), r"C:\dossier\");
        assert_eq!(quote_arg(r"C:\mon dossier\"), r#""C:\mon dossier\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(""), r#""""#);
    }

    #[test]
    fn ligne_de_cmd() {
        let line = project_command_line(&["symfony".into(), "server:start".into(), "--port=8000".into()]);
        assert!(
            line.ends_with(r#"/d /s /c ""symfony" "server:start" "--port=8000""""#),
            "{line}"
        );
        assert!(line.contains(r#"/d /s /c "chcp 65001 >nul & ""#), "{line}");

        let slashed = project_command_line(&["tool".into(), r"C:\mon dossier\".into(), r"C:\autre\\".into()]);
        assert!(
            slashed.ends_with(r#"/d /s /c ""tool" "C:\mon dossier\\" "C:\autre\\\\""""#),
            "{slashed}"
        );
    }

    #[test]
    fn lance_refuse_metacaracteres() {
        let dir = std::env::temp_dir();
        for bad in ["a & calc", "%PATH%"] {
            let options = LaunchOptions {
                cwd: dir.clone(),
                env: HashMap::new(),
                log: dir.join("servym-unused.log"),
                on_exit: Box::new(|_, _| {}),
            };
            assert!(
                matches!(launch(&[bad.into()], options), Err(LaunchError::Other(_))),
                "{bad}"
            );
        }
    }
}
