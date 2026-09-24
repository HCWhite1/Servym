//! Essai 1 : lancer un projet comme launch.ps1, sans PowerShell.
//!
//! `cmd.exe /d /s /c "chcp 65001 >nul & <commande>"` avec CREATE_NO_WINDOW (aucune console
//! visible, les sous-processus en mode console en héritent) et CREATE_BREAKAWAY_FROM_JOB quand
//! c'est permis (le projet survit à la fin de Servynex). Sorties standard et d'erreur : le
//! journal, ouvert en ajout. Le code de sortie vient du handle du processus.
//!
//! Usage :
//!   hidden-launch [--job|--job-strict] [--detach] [--watch <s>] --cwd <dir> --log <fichier> -- <argv…>
//!   hidden-launch jobinfo <pid>…
//!   hidden-launch exitcode <pid>     (adopte un processus déjà lancé et attend son code)

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, HWND, LPARAM, TRUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, GetCurrentProcess, GetExitCodeProcess, INFINITE,
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

const ERROR_ACCESS_DENIED: i32 = 5;

type Windows = BTreeMap<isize, (u32, String, String)>;

/// Fenêtres visibles de premier niveau : hwnd → (pid, classe, titre).
fn visible_windows() -> Windows {
    unsafe extern "system" fn each(hwnd: HWND, acc: LPARAM) -> i32 {
        let acc = unsafe { &mut *(acc as *mut Windows) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return TRUE;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        let mut class = [0u16; 256];
        let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), 256) } as usize;
        let mut title = [0u16; 256];
        let m = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), 256) } as usize;
        acc.insert(
            hwnd as isize,
            (pid, String::from_utf16_lossy(&class[..n]), String::from_utf16_lossy(&title[..m])),
        );
        TRUE
    }
    let mut acc = Windows::new();
    unsafe { EnumWindows(Some(each), &mut acc as *mut _ as LPARAM) };
    acc
}

/// Tous les processus : pid → (parent, nom).
fn processes() -> BTreeMap<u32, (u32, String)> {
    let mut map = BTreeMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            map.insert(e.th32ProcessID, (e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    map
}

fn descendants(root: u32, all: &BTreeMap<u32, (u32, String)>) -> BTreeSet<u32> {
    let mut tree = BTreeSet::from([root]);
    loop {
        let before = tree.len();
        for (&pid, (parent, _)) in all {
            if tree.contains(parent) && pid != *parent {
                tree.insert(pid);
            }
        }
        if tree.len() == before {
            return tree;
        }
    }
}

fn in_job(handle: HANDLE) -> bool {
    let mut result = FALSE;
    unsafe { IsProcessInJob(handle, std::ptr::null_mut(), &mut result) };
    result != 0
}

/// Simule la tâche planifiée : un job qui tue ses processus à sa fermeture.
fn enter_kill_on_close_job(breakaway_ok: bool) -> HANDLE {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | if breakaway_ok { JOB_OBJECT_LIMIT_BREAKAWAY_OK } else { 0 };
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        let ok = AssignProcessToJobObject(job, GetCurrentProcess());
        println!("dans un job « kill on close » (breakaway permis : {breakaway_ok}) : {}", ok != 0);
        job
    }
}

fn exit_code_of(pid: u32) -> Option<u32> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, FALSE, pid);
        if h.is_null() {
            return None;
        }
        let waited = WaitForSingleObject(h, INFINITE);
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        (waited == WAIT_OBJECT_0 && ok != 0).then_some(code)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("jobinfo") {
        let all = processes();
        for pid in &args[1..] {
            let pid: u32 = pid.parse().unwrap();
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
            let name = all.get(&pid).map_or("?", |p| p.1.as_str());
            if h.is_null() {
                println!("{pid} {name} : inaccessible");
            } else {
                println!("{pid} {name} : dans un job = {}", in_job(h));
                unsafe { CloseHandle(h) };
            }
        }
        return;
    }
    if args.first().map(String::as_str) == Some("kill") {
        // Remplace taskkill /T /F : l'arborescence, des feuilles vers la racine.
        let root: u32 = args[1].parse().unwrap();
        let all = processes();
        let tree = descendants(root, &all);
        let t = Instant::now();
        let mut killed = 0;
        for pid in tree.iter().rev() {
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE, FALSE, *pid);
                if !h.is_null() {
                    killed += i32::from(TerminateProcess(h, 1) != 0);
                    CloseHandle(h);
                }
            }
        }
        let names: Vec<_> = tree.iter().filter_map(|p| all.get(p).map(|x| x.1.as_str())).collect();
        println!("{killed}/{} arrêtés en {:.1} ms : {names:?}", tree.len(), t.elapsed().as_secs_f64() * 1000.0);
        return;
    }
    if args.first().map(String::as_str) == Some("exitcode") {
        let pid: u32 = args[1].parse().unwrap();
        println!("code de sortie de {pid} (adopté) : {:?}", exit_code_of(pid));
        return;
    }

    let mut job = None;
    let (mut detach, mut watch, mut cwd, mut log) = (false, 5u64, String::from("."), String::new());
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--job" => job = Some(enter_kill_on_close_job(true)),
            "--job-strict" => job = Some(enter_kill_on_close_job(false)),
            "--detach" => detach = true,
            "--watch" => {
                i += 1;
                watch = args[i].parse().unwrap();
            }
            "--cwd" => {
                i += 1;
                cwd = args[i].clone();
            }
            "--log" => {
                i += 1;
                log = args[i].clone();
            }
            "--" => {
                i += 1;
                break;
            }
            other => panic!("option inconnue : {other}"),
        }
        i += 1;
    }
    let argv = &args[i..];
    println!("essai dans un job : {}", in_job(unsafe { GetCurrentProcess() }));

    // Même ligne que processes.js + launch.ps1 : chaque argument entre guillemets.
    let cmdline = argv.iter().map(|a| format!("\"{a}\"")).collect::<Vec<_>>().join(" ");
    let inner = format!("/d /s /c \"chcp 65001 >nul & {cmdline}\"");
    let comspec = std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".into());

    let before = visible_windows();
    let file = OpenOptions::new().create(true).append(true).open(&log).unwrap();
    let spawn = |flags: u32| {
        Command::new(&comspec)
            .raw_arg(&inner)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(file.try_clone().unwrap())
            .stderr(file.try_clone().unwrap())
            .creation_flags(flags)
            .spawn()
    };
    let t = Instant::now();
    let mut child = match spawn(CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(c) => {
            println!("lancé avec CREATE_BREAKAWAY_FROM_JOB");
            c
        }
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) => {
            println!("breakaway refusé ({e}) : lancement dans le job");
            spawn(CREATE_NO_WINDOW).unwrap()
        }
        Err(e) => panic!("lancement impossible : {e}"),
    };
    drop(file);
    let pid = child.id();
    println!(
        "PID {pid} en {:.1} ms · projet dans un job : {}",
        t.elapsed().as_secs_f64() * 1000.0,
        in_job(child.as_raw_handle() as HANDLE)
    );

    // Surveillance : nouvelles fenêtres visibles (de n'importe quel processus, pour voir aussi
    // une délégation à Windows Terminal) et arborescence du projet.
    let deadline = Instant::now() + Duration::from_secs(watch);
    let mut seen = BTreeSet::new();
    let mut tree = BTreeMap::new();
    let mut finished = false;
    while Instant::now() < deadline {
        for (hwnd, w) in visible_windows() {
            if !before.contains_key(&hwnd) && seen.insert(hwnd) {
                println!("  NOUVELLE FENÊTRE visible : pid {} classe {:?} titre {:?}", w.0, w.1, w.2);
            }
        }
        let all = processes();
        for p in descendants(pid, &all) {
            if let Some((_, name)) = all.get(&p) {
                tree.entry(p).or_insert_with(|| name.clone());
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            println!("fini : code {:?}", status.code());
            finished = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    println!("arborescence vue : {tree:?}");
    println!("nouvelles fenêtres visibles : {}", seen.len());
    if detach || finished {
        if detach {
            println!("l'essai s'arrête et laisse le projet tourner (PID {pid})");
        }
        drop(job);
        return;
    }
    match child.wait() {
        Ok(s) => println!("code de sortie : {:?}", s.code()),
        Err(e) => println!("attente impossible : {e}"),
    }
}
