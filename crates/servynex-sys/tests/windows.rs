//! Windows natif, sur la vraie machine (tests ajoutés par la migration).
#![cfg(windows)]

use std::collections::BTreeSet;
use std::process::Command;
use std::time::Duration;

use servynex_fixtures::{Req, fixture_exe, free_port, listening, read, temp_dir, until};
use servynex_sys::{LaunchOptions, Platform, clean_env, native};
use tokio::sync::oneshot;

/// Même analyse que `listeners()` de la version Node (windows.js).
fn netstat() -> BTreeSet<(String, u16, u32)> {
    let mut set = BTreeSet::new();
    for proto in ["TCP", "TCPv6"] {
        let out = Command::new("netstat")
            .args(["-ano", "-p", proto])
            .output()
            .unwrap();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() != 5 || !cols[2].ends_with(":0") {
                continue;
            }
            let i = cols[1].rfind(':').unwrap();
            set.insert((
                cols[1][..i].to_string(),
                cols[1][i + 1..].parse().unwrap(),
                cols[4].parse().unwrap(),
            ));
        }
    }
    set
}

#[tokio::test]
async fn ports_au_format_de_netstat() {
    // Un port à nous, pour qu'il y ait au moins une ligne connue.
    let own = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = own.local_addr().unwrap().port();
    let native: BTreeSet<(String, u16, u32)> = native()
        .listeners()
        .await
        .into_iter()
        .map(|l| (l.address, l.port, l.pid.unwrap()))
        .collect();
    assert!(native.contains(&("127.0.0.1".into(), port, std::process::id())));
    // Des ports s'ouvrent et se ferment entre les deux lectures : on compare les ports stables.
    let reference = netstat();
    let common = native.intersection(&reference).count();
    assert!(
        common * 10 >= native.len().max(reference.len()) * 9,
        "natif {native:?}\nnetstat {reference:?}"
    );
    let v6 = native.iter().filter(|(a, _, _)| a.starts_with('[')).count();
    let first_v6 = native_order_first_v6().await;
    assert!(v6 == 0 || first_v6, "IPv4 avant IPv6");
}

async fn native_order_first_v6() -> bool {
    let list = native().listeners().await;
    let first = list.iter().position(|l| l.address.starts_with('['));
    first.is_none_or(|i| list[i..].iter().all(|l| l.address.starts_with('[')))
}

fn options(dir: &std::path::Path, port: u16) -> (LaunchOptions, oneshot::Receiver<(u32, Option<i64>)>) {
    let (tx, rx) = oneshot::channel();
    let mut env = clean_env();
    env.insert("PORT".into(), port.to_string());
    env.insert("HOST".into(), "127.0.0.1".into());
    env.insert("SERVYNEX_TEST_ENV".into(), "accents éèà ✓".into());
    let options = LaunchOptions {
        cwd: dir.to_path_buf(),
        env,
        log: dir.join("p.log"),
        on_exit: Box::new(move |pid, code| {
            let _ = tx.send((pid, code));
        }),
    };
    (options, rx)
}

/// Fenêtres visibles de premier niveau.
fn visible_windows() -> BTreeSet<isize> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM, TRUE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, IsWindowVisible};
    unsafe extern "system" fn each(hwnd: HWND, acc: LPARAM) -> i32 {
        // SAFETY : `acc` pointe sur l'ensemble passé à EnumWindows, vivant pendant l'appel.
        unsafe {
            if IsWindowVisible(hwnd) != 0 {
                (*(acc as *mut BTreeSet<isize>)).insert(hwnd as isize);
            }
        }
        TRUE
    }
    let mut set = BTreeSet::new();
    // SAFETY : rappel appelé pendant EnumWindows seulement.
    unsafe { EnumWindows(Some(each), &mut set as *mut _ as LPARAM) };
    set
}

fn window_pid(hwnd: isize) -> u32 {
    let mut pid = 0;
    // SAFETY : lecture du processus propriétaire d'une fenêtre.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd as _, &mut pid) };
    pid
}

#[tokio::test]
async fn lancement_cache_journal_et_arret() {
    let dir = temp_dir("servynex-win-");
    let port = free_port();
    let before = visible_windows();
    let platform = native();
    let (opts, rx) = options(dir.path(), port);
    let exe = fixture_exe().to_string_lossy().into_owned();
    let pid = platform.launch(vec![exe, "server".into()], opts).await.unwrap();
    until(|| listening(port), 10_000).await;
    assert_eq!(Req::get("/").send(port).await.body, "<title>Factice</title>");
    // Seules comptent les fenêtres des processus du projet (d'autres programmes de la machine
    // peuvent en ouvrir pendant le test).
    let table = native().sample_processes().await;
    let tree = servynex_sys::tree_of(&table, &[pid]);
    let new: Vec<(isize, u32)> = visible_windows()
        .difference(&before)
        .map(|h| (*h, window_pid(*h)))
        .filter(|(_, p)| tree.contains(p))
        .collect();
    assert!(new.is_empty(), "aucune fenêtre ne doit s'ouvrir : {new:?}");
    until(
        || async { read(dir.path().join("p.log")).contains("démarrage sur 127.0.0.1") },
        5_000,
    )
    .await;
    assert!(
        read(dir.path().join("p.log")).contains("accents éèà ✓"),
        "UTF-8 intact"
    );

    // L'arborescence : cmd.exe puis le programme.
    let table = platform.sample_processes().await;
    let tree = servynex_sys::tree_of(&table, &[pid]);
    assert!(tree.len() >= 2, "{tree:?}");
    assert!(table[&pid].start > 0 && table[&pid].memory > 0);
    let start = platform.process_start(pid).unwrap();
    assert_eq!(start, table[&pid].start);

    platform.kill_tree(pid).await.unwrap();
    assert!(!platform.is_alive(pid));
    for p in &tree {
        assert!(!platform.is_alive(*p), "{p} arrêté avec l'arborescence");
    }
    let (exited, code) = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (exited, code),
        (pid, Some(1)),
        "arrêt forcé : code 1, comme taskkill /F"
    );
    assert!(!listening(port).await);
}

#[tokio::test]
async fn code_de_sortie_transmis() {
    let dir = temp_dir("servynex-win-");
    let (opts, rx) = options(dir.path(), free_port());
    let exe = fixture_exe().to_string_lossy().into_owned();
    let pid = native()
        .launch(
            vec![exe, "echo".into(), "sortie".into(), "erreur".into(), "3".into()],
            opts,
        )
        .await
        .unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit, (pid, Some(3)));
    let log = read(dir.path().join("p.log"));
    assert!(log.contains("sortie") && log.contains("erreur"), "{log}");
}

#[tokio::test]
async fn commande_introuvable_dans_le_journal() {
    let dir = temp_dir("servynex-win-");
    let (opts, rx) = options(dir.path(), free_port());
    native()
        .launch(vec!["commande-qui-n-existe-pas".into()], opts)
        .await
        .unwrap();
    let (_, code) = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(code, Some(1), "cmd.exe : commande introuvable");
    // Message de cmd.exe, lisible grâce à chcp 65001 (sa langue dépend de Windows).
    let log = read(dir.path().join("p.log"));
    assert!(log.contains("commande-qui-n-existe-pas"), "{log}");
    assert!(!log.contains('\u{fffd}'), "UTF-8 valide : {log}");
}

/// Un projet repris après un redémarrage de Servynex : son code de sortie est lu par son PID.
#[tokio::test]
async fn code_de_sortie_d_un_processus_repris() {
    let port = free_port();
    let mut child = Command::new(fixture_exe())
        .arg("server")
        .env("PORT", port.to_string())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (tx, rx) = oneshot::channel();
    native().watch_exit(
        pid,
        Box::new(move |p, code| {
            let _ = tx.send((p, code));
        }),
    );
    until(|| listening(port), 10_000).await;
    child.kill().unwrap();
    let exit = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit, (pid, Some(1)));
    let _ = child.wait();
}

#[tokio::test]
async fn releve_noms_et_reseaux() {
    let platform = native();
    let own = std::process::id();
    let table = platform.sample_processes().await;
    let me = &table[&own];
    assert!(me.start > 1_600_000_000_000 && me.memory > 1_000_000);
    assert!(platform.process_names().await[&own].ends_with(".exe"));
    assert!(platform.is_alive(own));
    assert!(!platform.is_alive(0));
    // Profils réseau : pas d'erreur (la liste peut être vide sur une machine sans réseau).
    for n in platform.network_profiles().await {
        assert!(
            ["public", "private", "domain"].contains(&n.category.as_str()),
            "{n:?}"
        );
    }
    assert!(platform.tools().desktop);
}
