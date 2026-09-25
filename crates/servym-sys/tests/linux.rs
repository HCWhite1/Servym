//! Partie « système » de tests/linux.test.js : sorties enregistrées (ss, /proc) et appels système
//! remplacés ; tourne aussi sous Windows, sauf le lancement réel.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use servym_fixtures::{TempDir, temp_dir, write};
use servym_sys::linux::{
    self, KillOptions, SIGKILL, SIGTERM, Stat, SystemdTask, detect_tools, exit_code, parse_ss, parse_stat,
    terminal_command, unit_file,
};
use servym_sys::{Autostart, BoxFuture, Listener, ProcSample, Tools};

struct Fake {
    comm: &'static str,
    ppid: u32,
    utime: u64,
    stime: u64,
    start: u64,
    rss: u64,
}

fn p(comm: &'static str, ppid: u32) -> Fake {
    Fake {
        comm,
        ppid,
        utime: 0,
        stime: 0,
        start: 0,
        rss: 0,
    }
}

/// Faux /proc : un dossier par processus (stat, comm), btime en secondes.
fn fake_proc(processes: Vec<(u32, Fake)>, btime: u64) -> TempDir {
    let dir = temp_dir("servym-proc-");
    write(
        dir.path().join("stat"),
        format!("cpu  1 2 3 4\ncpu0 1 2 3 4\nbtime {btime}\nprocesses 1234\n"),
    );
    std::fs::create_dir(dir.path().join("self")).unwrap();
    std::fs::create_dir(dir.path().join("sys")).unwrap();
    for (pid, f) in processes {
        let fields = format!(
            "S {} {pid} {pid} 0 -1 4194560 100 0 0 0 {} {} 0 0 20 0 1 0 {} 1000000 {} 18446744073709551615",
            f.ppid, f.utime, f.stime, f.start, f.rss
        );
        write(
            dir.path().join(pid.to_string()).join("stat"),
            format!("{pid} ({}) {fields}\n", f.comm),
        );
        write(
            dir.path().join(pid.to_string()).join("comm"),
            format!("{}\n", f.comm),
        );
    }
    dir
}

/// JS : linux.test.js › Linux : mêmes fonctions que Windows › chaque fonction du module Windows existe sous Linux
/// (en Rust, le trait `Platform` le garantit à la compilation ; il reste à vérifier le système natif.)
#[test]
fn meme_interface_que_windows() {
    let platform = servym_sys::native();
    let _ = platform.tools();
}

const SS: &str = "LISTEN 0      511          127.0.0.1:3990       0.0.0.0:*    users:((\"node\",pid=4242,fd=21))
LISTEN 0      4096     127.0.0.53%lo:53         0.0.0.0:*
LISTEN 0      511                  *:9000             *:*    users:((\"node\",pid=1111,fd=18))
LISTEN 0      128               [::]:22            [::]:*
LISTEN 0      511              [::1]:5173          [::]:*    users:((\"node\",pid=5555,fd=30),(\"node\",pid=5556,fd=30))
LISTEN 0      80  [::ffff:127.0.0.1]:3306             *:*    users:((\"mysqld\",pid=777,fd=12))
LISTEN 0      511            0.0.0.0:8080       0.0.0.0:*    users:((\"php\",pid=888,fd=4))";

fn l(address: &str, port: u16, pid: Option<u32>) -> Listener {
    Listener {
        address: address.into(),
        port,
        pid,
    }
}

/// JS : linux.test.js › Linux : ports en écoute (ss) › adresse, port et PID ; adresses ramenées à la forme de Windows
#[test]
fn ss_adresses_forme_windows() {
    assert_eq!(
        parse_ss(SS),
        [
            l("127.0.0.1", 3990, Some(4242)),
            l("127.0.0.53", 53, None),
            l("0.0.0.0", 9000, Some(1111)),
            l("[::]", 22, None),
            l("[::1]", 5173, Some(5555)),
            l("127.0.0.1", 3306, Some(777)),
            l("0.0.0.0", 8080, Some(888)),
        ]
    );
}

/// JS : linux.test.js › Linux : ports en écoute (ss) › avec la ligne d’en-tête (ancien ss sans -H)
#[test]
fn ss_avec_en_tete() {
    let with_header = format!("State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n{SS}");
    assert_eq!(parse_ss(&with_header).len(), 7);
}

/// JS : linux.test.js › Linux : processus (/proc) › ligne stat : un nom avec espaces et parenthèses ne décale rien
#[test]
fn ligne_stat() {
    let line = "4242 (node (worker) 2) S 4000 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 7 0 12345 1000000 2048 18446744073709551615";
    assert_eq!(
        parse_stat(line),
        Some(Stat {
            pid: 4242,
            ppid: 4000,
            ticks: 300,
            start_ticks: 12345,
            rss: 2048
        })
    );
}

/// JS : linux.test.js › Linux : processus (/proc) › relevé : début, temps processeur et mémoire en ms et octets
#[test]
fn releve_proc() {
    let proc = fake_proc(
        vec![
            (
                1,
                Fake {
                    comm: "systemd",
                    ppid: 0,
                    utime: 10,
                    stime: 5,
                    start: 1,
                    rss: 1000,
                },
            ),
            (
                4242,
                Fake {
                    comm: "node",
                    ppid: 1,
                    utime: 250,
                    stime: 50,
                    start: 12345,
                    rss: 2048,
                },
            ),
        ],
        1_700_000_000,
    );
    let sample = linux::sample_processes(proc.path(), 100, 4096);
    assert_eq!(
        sample[&4242],
        ProcSample {
            pid: 4242,
            ppid: 1,
            start: 1_700_000_000_000 + 123_450,
            cpu: 3000,
            memory: 2048 * 4096
        }
    );
    assert_eq!(
        sample.len(),
        2,
        "les dossiers self, sys… ne sont pas des processus"
    );
}

/// JS : linux.test.js › Linux : processus (/proc) › un processus disparu pendant la lecture est ignoré
#[test]
fn processus_disparu() {
    let proc = fake_proc(vec![(10, p("a", 1))], 1_700_000_000);
    std::fs::create_dir(proc.path().join("11")).unwrap(); // plus de stat : terminé entre-temps
    let sample = linux::sample_processes(proc.path(), 100, 4096);
    assert_eq!(sample.keys().copied().collect::<Vec<_>>(), [10]);
}

/// JS : linux.test.js › Linux : processus (/proc) › pas de profils réseau comme sous Windows
#[test]
fn pas_de_profils_reseau() {
    assert!(linux::network_profiles().is_empty());
}

/// JS : linux.test.js › Linux : processus (/proc) › noms des processus
#[test]
fn noms_des_processus() {
    let proc = fake_proc(vec![(4242, p("node", 1)), (777, p("mysqld", 1))], 1_700_000_000);
    let names = linux::process_names(proc.path());
    assert_eq!(names[&4242], "node");
    assert_eq!(names[&777], "mysqld");
}

fn started(comm: &'static str, ppid: u32, start: u64) -> Fake {
    Fake {
        start,
        ..p(comm, ppid)
    }
}

/// JS : linux.test.js › Linux : arrêt de toute l’arborescence › SIGTERM au groupe et aux descendants, SIGKILL à ceux qui résistent, rien aux autres
#[tokio::test]
async fn arret_de_l_arborescence() {
    let proc = fake_proc(
        vec![
            (100, started("npm", 1, 10)),
            (101, started("node", 100, 11)),
            (102, started("esbuild", 101, 12)),
            (200, started("bash", 1, 5)),
        ],
        1_700_000_000,
    );
    let calls = Arc::new(Mutex::new(Vec::<(i32, i32)>::new()));
    let alive = Arc::new(Mutex::new(BTreeSet::from([100u32, 101, 102, 200])));
    let (c, a) = (calls.clone(), alive.clone());
    let kill = Arc::new(move |pid: i32, signal: i32| {
        c.lock().unwrap().push((pid, signal));
        // esbuild ignore SIGTERM
        if pid > 0 && (signal == SIGKILL || pid != 102) {
            a.lock().unwrap().remove(&(pid as u32));
        }
    });
    let a = alive.clone();
    let options = KillOptions {
        proc: proc.path().to_path_buf(),
        kill,
        alive: Arc::new(move |pid| a.lock().unwrap().contains(&pid)),
        grace: Duration::from_millis(50),
        wait: Duration::from_millis(2000),
    };
    linux::kill_tree(100, options).await.unwrap();
    let calls = calls.lock().unwrap();
    let mut terms: Vec<i32> = calls
        .iter()
        .filter(|(_, s)| *s == SIGTERM)
        .map(|(p, _)| *p)
        .collect();
    terms.sort();
    assert_eq!(terms, [-100, 100, 101, 102]);
    assert_eq!(
        calls
            .iter()
            .filter(|(_, s)| *s == SIGKILL)
            .copied()
            .collect::<Vec<_>>(),
        [(102, SIGKILL)]
    );
    assert!(alive.lock().unwrap().contains(&200), "processus étranger intact");
}

/// JS : linux.test.js › Linux : arrêt de toute l’arborescence › un processus impossible à arrêter : erreur
#[tokio::test]
async fn processus_impossible_a_arreter() {
    let proc = fake_proc(vec![(100, p("bloqué", 1))], 1_700_000_000);
    let options = KillOptions {
        proc: proc.path().to_path_buf(),
        kill: Arc::new(|_, _| {}),
        alive: Arc::new(|_| true),
        grace: Duration::from_millis(20),
        wait: Duration::from_millis(20),
    };
    let err = linux::kill_tree(100, options).await.unwrap_err();
    assert!(err.contains("100"), "{err}");
}

/// JS : linux.test.js › Linux : lancement d’un projet › sans shell, sortie dans le journal, code de sortie transmis
#[cfg(unix)]
#[tokio::test]
async fn lancement_sans_shell() {
    let dir = temp_dir("servym-launch-");
    let log = dir.path().join("p.log");
    write(&log, "[servym] en-tête\n");
    let (tx, rx) = tokio::sync::oneshot::channel();
    let argv = [
        servym_fixtures::fixture_exe().to_string_lossy().into_owned(),
        "echo".into(),
        "sortie $HOME".into(),
        "erreur".into(),
        "3".into(),
    ];
    let options = servym_sys::LaunchOptions {
        cwd: dir.path().to_path_buf(),
        env: std::env::vars().collect(),
        log: log.clone(),
        on_exit: Box::new(move |pid, code| {
            let _ = tx.send((pid, code));
        }),
    };
    let pid = linux::launch(argv.to_vec(), options).await.unwrap();
    assert!(pid > 0);
    let exit = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(exit, (pid, Some(3)));
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(
        text.starts_with("[servym] en-tête\n"),
        "journal complété, pas remplacé"
    );
    assert!(
        text.contains("sortie $HOME"),
        "aucun shell : $HOME reste tel quel"
    );
    assert!(text.contains("erreur"));
}

/// JS : linux.test.js › Linux : lancement d’un projet › tué par un signal : code 128 + numéro du signal
#[test]
fn code_de_sortie_signal() {
    assert_eq!(exit_code(None, Some(SIGKILL)), Some(137));
    assert_eq!(exit_code(None, Some(SIGTERM)), Some(143));
    assert_eq!(exit_code(Some(2), None), Some(2));
    assert_eq!(exit_code(None, None), None);
}

/// JS : linux.test.js › Linux : lancement d’un projet › commande introuvable : message clair
#[cfg(unix)]
#[tokio::test]
async fn commande_introuvable() {
    let dir = temp_dir("servym-launch-");
    let options = servym_sys::LaunchOptions {
        cwd: dir.path().to_path_buf(),
        env: std::env::vars().collect(),
        log: dir.path().join("p.log"),
        on_exit: Box::new(|_, _| {}),
    };
    let err = linux::launch(vec!["commande-qui-n-existe-pas".into(), "x".into()], options)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        servym_sys::LaunchError::NotFound("commande-qui-n-existe-pas".into())
    );
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn has(list: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |cmd| list.contains(&cmd)
}

fn tools(desktop: bool, code: bool, terminal: Option<&str>) -> Tools {
    Tools {
        desktop,
        code,
        terminal: terminal.map(str::to_string),
    }
}

/// JS : linux.test.js › Linux : outils du bureau › sans écran (serveur) : rien à ouvrir
#[test]
fn sans_ecran_rien_a_ouvrir() {
    assert_eq!(
        detect_tools(&env(&[]), &has(&["code", "gnome-terminal"])),
        tools(false, false, None)
    );
}

/// JS : linux.test.js › Linux : outils du bureau › avec écran : VS Code et le premier terminal trouvé (Ptyxis sur Fedora)
#[test]
fn avec_ecran_code_et_terminal() {
    assert_eq!(
        detect_tools(
            &env(&[("WAYLAND_DISPLAY", "wayland-0")]),
            &has(&["code", "ptyxis", "gnome-terminal"])
        ),
        tools(true, true, Some("Ptyxis"))
    );
    assert_eq!(
        detect_tools(&env(&[("DISPLAY", ":0")]), &has(&["konsole"])),
        tools(true, false, Some("Konsole"))
    );
    assert_eq!(
        detect_tools(&env(&[("DISPLAY", ":0")]), &has(&[])),
        tools(true, false, None)
    );
}

/// JS : linux.test.js › Linux : outils du bureau › terminal ouvert dans le dossier du projet
#[test]
fn terminal_dans_le_dossier() {
    let cmd = |name, args: &[&str]| {
        Some((
            String::from(name),
            args.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
        ))
    };
    assert_eq!(
        terminal_command("Ptyxis", "/home/hugo/app"),
        cmd("ptyxis", &["--new-window", "--working-directory=/home/hugo/app"])
    );
    assert_eq!(
        terminal_command("GNOME Terminal", "/srv/a b"),
        cmd("gnome-terminal", &["--working-directory=/srv/a b"])
    );
    assert_eq!(
        terminal_command("Konsole", "/srv/app"),
        cmd("konsole", &["--workdir", "/srv/app"])
    );
}

/// JS : linux.test.js › Linux : démarrage avec la session (systemd) › service utilisateur : relancé s’il plante, les projets survivent à son arrêt
#[test]
fn service_utilisateur() {
    let unit = unit_file(
        Path::new("/home/hugo/Servym 100%/servym"),
        Path::new("/home/hugo/Servym 100%"),
    );
    let lines: Vec<&str> = unit.lines().collect();
    assert!(
        lines.contains(&"ExecStart=\"/home/hugo/Servym 100%%/servym\""),
        "% doublé pour systemd :\n{unit}"
    );
    assert!(
        lines.contains(&"WorkingDirectory=/home/hugo/Servym 100%%"),
        "{unit}"
    );
    assert!(lines.contains(&"Environment=SERVYM_SUPERVISOR=systemd"), "{unit}");
    assert!(lines.contains(&"Restart=on-failure"), "{unit}");
    assert!(lines.contains(&"KillMode=process"), "{unit}");
    assert!(lines.contains(&"WantedBy=default.target"), "{unit}");
}

/// JS : linux.test.js › Linux : démarrage avec la session (systemd) › activer, lire l’état, désactiver
#[tokio::test]
async fn activer_lire_desactiver() {
    let dir = temp_dir("servym-systemd-");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let enabled = Arc::new(Mutex::new(false));
    let (c, e) = (calls.clone(), enabled.clone());
    let run = Arc::new(
        move |cmd: String, args: Vec<String>| -> BoxFuture<'static, Result<String, String>> {
            c.lock().unwrap().push(format!("{cmd} {}", args.join(" ")));
            let mut on = e.lock().unwrap();
            let result = if args.iter().any(|a| a == "is-enabled") {
                if *on {
                    Ok("enabled\n".to_string())
                } else {
                    Err("disabled\n".to_string())
                }
            } else {
                if args.iter().any(|a| a == "enable") {
                    *on = true;
                }
                if args.iter().any(|a| a == "disable") {
                    *on = false;
                }
                Ok(String::new())
            };
            Box::pin(async move { result })
        },
    );
    let task = SystemdTask {
        dir: dir.path().to_path_buf(),
        run,
        exe: "/opt/servym/servym".into(),
        workdir: "/opt/servym".into(),
    };
    assert!(!task.enabled().await);
    task.set(true).await.unwrap();
    assert!(dir.path().join("servym.service").exists());
    assert!(task.enabled().await);
    task.set(false).await.unwrap();
    assert!(!task.enabled().await);
    assert!(!dir.path().join("servym.service").exists(), "fichier retiré");
    let calls: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| !c.contains("is-enabled"))
        .cloned()
        .collect();
    assert_eq!(
        calls,
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable servym.service",
            "systemctl --user disable servym.service",
            "systemctl --user daemon-reload",
        ]
    );
}
