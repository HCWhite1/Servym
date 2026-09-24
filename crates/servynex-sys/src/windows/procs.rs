//! Processus : relevé (Toolhelp + temps et mémoire de chaque processus), noms, heure de
//! démarrage, arrêt de l'arborescence, attente de la fin et code de sortie.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::{OnExit, ProcSample, ProcTable, tree_of};

/// Handle fermé à la fin de sa portée.
pub struct Owned(pub HANDLE);

// SAFETY : un handle de processus s'utilise depuis n'importe quel thread.
unsafe impl Send for Owned {}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY : handle obtenu du système et fermé une seule fois.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Ouvre un processus ; `None` s'il n'existe plus ou reste inaccessible.
pub fn open(pid: u32, access: u32) -> Option<Owned> {
    // SAFETY : simple ouverture ; le handle est fermé par `Owned`.
    let h = unsafe { OpenProcess(access, FALSE, pid) };
    (!h.is_null()).then_some(Owned(h))
}

/// Liste des processus : PID, parent, nom de l'exécutable.
pub fn list() -> Vec<(u32, u32, String)> {
    let mut out = Vec::new();
    // SAFETY : instantané Toolhelp parcouru avec une structure dont la taille est renseignée.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let snap = Owned(snap);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap.0, &mut e);
        while ok != 0 {
            let len = e
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(e.szExeFile.len());
            out.push((
                e.th32ProcessID,
                e.th32ParentProcessID,
                String::from_utf16_lossy(&e.szExeFile[..len]),
            ));
            ok = Process32NextW(snap.0, &mut e);
        }
    }
    out
}

fn filetime(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Écart entre 1601 (FILETIME) et 1970 (Unix), en centaines de nanosecondes.
const UNIX_EPOCH_FILETIME: u64 = 116_444_736_000_000_000;

/// Début (ms Unix) et temps processeur (ms) d'un processus ouvert.
fn times(h: &Owned) -> Option<(i64, u64)> {
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY : quatre structures valides à remplir.
    let ok = unsafe { GetProcessTimes(h.0, &mut created, &mut exited, &mut kernel, &mut user) };
    if ok == 0 {
        return None;
    }
    let start = (filetime(created).saturating_sub(UNIX_EPOCH_FILETIME) / 10_000) as i64;
    Some((start, (filetime(kernel) + filetime(user)) / 10_000))
}

fn working_set(h: &Owned) -> u64 {
    // SAFETY : structure dont la taille est renseignée.
    unsafe {
        let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        m.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(h.0, &mut m, m.cb) != 0 {
            m.WorkingSetSize as u64
        } else {
            0
        }
    }
}

/// Relevé de tous les processus. Ceux qu'on ne peut pas ouvrir (protégés, autre session) gardent
/// leur parent mais un début, un temps et une mémoire nuls.
pub fn sample() -> ProcTable {
    list()
        .into_iter()
        .map(|(pid, ppid, _)| {
            let (start, cpu, memory) = match open(pid, PROCESS_QUERY_LIMITED_INFORMATION) {
                Some(h) => {
                    let (start, cpu) = times(&h).unwrap_or((0, 0));
                    (start, cpu, working_set(&h))
                }
                None => (0, 0, 0),
            };
            (
                pid,
                ProcSample {
                    pid,
                    ppid,
                    start,
                    cpu,
                    memory,
                },
            )
        })
        .collect()
}

pub fn names() -> HashMap<u32, String> {
    list().into_iter().map(|(pid, _, name)| (pid, name)).collect()
}

/// Heure de démarrage d'un processus (ms Unix).
pub fn start_of(pid: u32) -> Option<i64> {
    open(pid, PROCESS_QUERY_LIMITED_INFORMATION)
        .and_then(|h| times(&h))
        .map(|(start, _)| start)
}

/// Le processus existe-t-il encore ? Inaccessible mais présent : oui (comme EPERM).
pub fn alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    match open(pid, PROCESS_SYNCHRONIZE) {
        // SAFETY : attente nulle sur un handle valide.
        Some(h) => (unsafe { WaitForSingleObject(h.0, 0) }) == WAIT_TIMEOUT,
        None => {
            let denied = std::io::Error::last_os_error().raw_os_error() == Some(5); // ERROR_ACCESS_DENIED
            denied
        }
    }
}

/// Arrête un processus et toute son arborescence (même règle que le relevé : un PID réutilisé
/// n'en fait pas partie), des plus récents aux plus anciens. Erreur si la racine survit 2 s.
pub fn kill_tree(pid: u32) -> Result<(), String> {
    let table = sample();
    let mut tree: Vec<u32> = tree_of(&table, &[pid]).into_iter().collect();
    if tree.is_empty() {
        tree.push(pid);
    }
    tree.sort_by_key(|p| std::cmp::Reverse(table.get(p).map_or(0, |s| s.start)));
    for p in &tree {
        if let Some(h) = open(*p, PROCESS_TERMINATE) {
            // SAFETY : handle ouvert avec le droit d'arrêt.
            unsafe { TerminateProcess(h.0, 1) };
        }
    }
    let end = Instant::now() + Duration::from_secs(2);
    while alive(pid) {
        if Instant::now() > end {
            return Err(pid.to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Attend la fin d'un processus (handle avec SYNCHRONIZE et QUERY_LIMITED) puis appelle `on_exit`
/// avec son code de sortie, sur un petit thread dédié.
pub fn watch_handle(handle: Owned, pid: u32, on_exit: OnExit) {
    let spawned = std::thread::Builder::new()
        .name(format!("sortie {pid}"))
        .stack_size(64 * 1024)
        .spawn(move || {
            // SAFETY : attente puis lecture du code sur un handle valide, fermé par `Owned`.
            let code = unsafe {
                let mut code = 0u32;
                (WaitForSingleObject(handle.0, INFINITE) == WAIT_OBJECT_0
                    && GetExitCodeProcess(handle.0, &mut code) != 0)
                    .then_some(i64::from(code as i32))
            };
            drop(handle);
            on_exit(pid, code);
        });
    if let Err(e) = spawned {
        eprintln!("surveillance du processus {pid} impossible : {e}");
    }
}

/// Comme `watch_handle`, pour un processus repris par son PID. Déjà terminé ou inaccessible :
/// `on_exit` tout de suite, code inconnu.
pub fn watch_exit(pid: u32, on_exit: OnExit) {
    match open(pid, PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION) {
        Some(h) => watch_handle(h, pid, on_exit),
        None => on_exit(pid, None),
    }
}
