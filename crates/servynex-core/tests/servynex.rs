//! Partie « cœur » de tests/servynex.test.js : configuration, exemple publié, processeur et
//! mémoire, fichiers statiques, journaux. La partie HTTP est dans servynex-server/tests/servynex.rs.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use servynex_core::config::{
    create_project, describe_changes, format_config, load_config, patch_project, remove_project,
    reorder_projects, validate,
};
use servynex_core::i18n::{Lang, LocalError};
use servynex_core::logs::{last_error, startup_reason};
use servynex_core::static_files::resolve_inside;
use servynex_core::stats::{ProcSample, ProcTable, Sampler, Snapshot, Usage, tree_of, usage_of};
use servynex_fixtures::{assert_match, temp_dir, write};

/// Message français d'une erreur attendue.
fn fr<T: std::fmt::Debug>(result: Result<T, LocalError>) -> String {
    result
        .expect_err("une erreur était attendue")
        .message_in(Lang::Fr)
}

fn base() -> Value {
    json!({
        "instance": { "port": 9000 },
        "projects": [
            { "id": "a", "name": "A", "cwd": "C:/a", "cmd": "node", "args": ["a.js"], "port": 3001, "hostControl": "env" },
            { "id": "b", "name": "B", "cwd": "C:/b", "type": "static", "port": 3002, "fixedPort": "fixe" },
            { "id": "c", "name": "C", "cwd": "C:/c", "cmd": "node", "args": [], "port": 3003, "hostControl": "local" },
        ],
    })
}

fn remove(value: &mut Value, key: &str) {
    value.as_object_mut().unwrap().remove(key);
}

// ---- configuration ----

/// JS : servynex.test.js › configuration › refuse les ports en double, y compris celui de Servynex
#[test]
#[ignore = "phase 2"]
fn refuse_ports_en_double() {
    let mut conf = base();
    conf["projects"][1]["port"] = json!(3001);
    assert_match!(fr(validate(&conf)), "Port 3001");
    conf["projects"][1]["port"] = json!(9000);
    assert_match!(fr(validate(&conf)), "Servynex");
}

/// JS : servynex.test.js › configuration › refuse guillemets, % et métacaractères de cmd.exe dans les arguments
#[test]
#[ignore = "phase 2"]
fn refuse_metacaracteres_de_cmd() {
    let mut conf = base();
    for arg in [
        "a.js\" & calc",
        "%PATH%",
        "a & calc",
        "a | calc",
        "a > b",
        "a ^& b",
        "(a)",
    ] {
        conf["projects"][0]["args"] = json!([arg]);
        assert_match!(fr(validate(&conf)), "args", "{arg}");
        assert_match!(
            fr(patch_project(&base(), "a", &json!({ "args": [arg] }))),
            "args",
            "interface : {arg}"
        );
    }
    conf["projects"][0]["args"] = json!([
        "--port={port}",
        "--defaults-file={servynex}/config/mysql.ini",
        "server/server.js"
    ]);
    validate(&conf).unwrap();
}

/// JS : servynex.test.js › configuration › stop et env validés comme la commande
#[test]
#[ignore = "phase 2"]
fn stop_et_env_valides() {
    let mut conf = base();
    conf["projects"][0]["stop"] = json!({ "cmd": "mysqladmin", "args": ["--port={port}", "shutdown"] });
    validate(&conf).unwrap();
    for args in [json!([3306]), json!(["a & b"]), json!("shutdown")] {
        conf["projects"][0]["stop"]["args"] = args.clone();
        assert_match!(fr(validate(&conf)), "stop", "{args}");
    }
    remove(&mut conf["projects"][0], "stop");
    conf["projects"][0]["env"] = json!({ "SHELL_VERBOSITY": "1" });
    validate(&conf).unwrap();
    for env in [
        json!({ "A B": "1" }),
        json!({ "A": 1 }),
        json!({ "A": "%PATH%" }),
        json!(["A=1"]),
    ] {
        conf["projects"][0]["env"] = env.clone();
        assert_match!(fr(validate(&conf)), "env", "{env}");
    }
}

/// JS : servynex.test.js › configuration › l'interface modifie commande, dossier et environnement, jamais l'identifiant
#[test]
#[ignore = "phase 2"]
fn interface_modifie_commande_dossier_env() {
    let patch = json!({ "cmd": "npm", "args": ["run", "dev"], "cwd": "D:/app", "env": { "A": "1" }, "stop": { "cmd": "x", "args": [] } });
    let next = patch_project(&base(), "a", &patch).unwrap();
    assert_eq!(next["projects"][0]["args"], json!(["run", "dev"]));
    assert_eq!(next["projects"][0]["cwd"], "D:/app");
    let cleared = patch_project(&next, "a", &json!({ "env": null, "stop": null })).unwrap();
    assert!(
        cleared["projects"][0].get("env").is_none(),
        "null retire le champ"
    );
    assert_match!(
        fr(patch_project(&base(), "a", &json!({ "id": "z" }))),
        "non modifiable"
    );
    assert_match!(
        fr(patch_project(&base(), "a", &json!({ "cwd": null }))),
        "obligatoire"
    );
    assert_match!(
        fr(patch_project(&base(), "a", &json!({ "cwd": "relatif" }))),
        "absolu"
    );
    assert_match!(
        fr(patch_project(
            &base(),
            "a",
            &json!({ "__proto__": { "cmd": "calc" } })
        )),
        "non modifiable"
    );
}

/// JS : servynex.test.js › configuration › projet « file » : sans port, chemin relatif ; un changement de type retire les champs sans objet
#[test]
#[ignore = "phase 2"]
fn projet_file() {
    let mut conf = base();
    conf["projects"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "f", "name": "F", "type": "file", "cwd": "C:/f", "open": "index.html" }));
    validate(&conf).unwrap();
    assert_eq!(
        patch_project(&conf, "f", &json!({ "name": "G" })).unwrap()["projects"][3]["name"],
        "G"
    );
    for open in ["../secret.html", "C:/Windows/win.ini", "a/../../b.html", ""] {
        conf["projects"][3]["open"] = json!(open);
        assert_match!(fr(validate(&conf)), "fichier à ouvrir", "{open}");
    }
    conf["projects"][3]["open"] = json!("index.html");
    conf["projects"][3]["port"] = json!(4000);
    assert_match!(fr(validate(&conf)), "pas de port");
    let as_file = patch_project(&base(), "a", &json!({ "type": "file", "open": "index.html" })).unwrap();
    assert!(as_file["projects"][0].get("cmd").is_none());
    assert!(as_file["projects"][0].get("port").is_none());
}

/// JS : servynex.test.js › configuration › autostart : booléen, interdit pour un fichier
#[test]
#[ignore = "phase 2"]
fn autostart_booleen() {
    let mut conf = base();
    conf["projects"][0]["autostart"] = json!("oui");
    assert_match!(fr(validate(&conf)), "autostart");
    assert_eq!(
        patch_project(&base(), "a", &json!({ "autostart": true })).unwrap()["projects"][0]["autostart"],
        true
    );
}

/// JS : servynex.test.js › configuration › icône : nom d'icône ou court texte
#[test]
#[ignore = "phase 2"]
fn icone() {
    assert_eq!(
        patch_project(&base(), "a", &json!({ "icon": "graduation-cap" })).unwrap()["projects"][0]["icon"],
        "graduation-cap"
    );
    assert_eq!(
        patch_project(&base(), "a", &json!({ "icon": "🎯" })).unwrap()["projects"][0]["icon"],
        "🎯"
    );
    assert_match!(
        fr(patch_project(
            &base(),
            "a",
            &json!({ "icon": "un texte beaucoup trop long" })
        )),
        "icon"
    );
    assert_match!(fr(patch_project(&base(), "a", &json!({ "icon": 42 }))), "icon");
}

/// JS : servynex.test.js › configuration › port fixe, port pris, exposition impossible
#[test]
#[ignore = "phase 2"]
fn port_fixe_pris_exposition() {
    let base = base();
    assert_match!(fr(patch_project(&base, "b", &json!({ "port": 4000 }))), "fixe");
    assert_match!(
        fr(patch_project(&base, "a", &json!({ "port": 3003 }))),
        "Port 3003"
    );
    assert_match!(fr(patch_project(&base, "c", &json!({ "expose": true }))), "local");
    assert_match!(fr(patch_project(&base, "a", &json!({ "port": 70000 }))), "port");
    let next = patch_project(
        &base,
        "a",
        &json!({ "port": 4000, "expose": true, "tags": [" x "] }),
    )
    .unwrap();
    assert_eq!(next["projects"][0]["port"], 4000);
    assert_eq!(next["projects"][0]["tags"], json!(["x"]));
    assert_eq!(base["projects"][0]["port"], 3001, "l'original n'est pas modifié");
}

/// JS : servynex.test.js › configuration › dépendances : inconnues, fichier ou en boucle refusées
#[test]
#[ignore = "phase 2"]
fn dependances_refusees() {
    assert_match!(
        fr(patch_project(&base(), "a", &json!({ "requires": ["zz"] }))),
        "inconnu"
    );
    assert_match!(
        fr(patch_project(&base(), "a", &json!({ "requires": ["a"] }))),
        "lui-même"
    );
    let with_loop = patch_project(&base(), "a", &json!({ "requires": ["c"] })).unwrap();
    assert_match!(
        fr(patch_project(&with_loop, "c", &json!({ "requires": ["a"] }))),
        "boucle"
    );
    let mut conf = base();
    conf["projects"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "f", "name": "F", "type": "file", "cwd": "C:/f", "open": "index.html" }));
    assert_match!(
        fr(patch_project(&conf, "a", &json!({ "requires": ["f"] }))),
        "fichier"
    );
}

/// JS : servynex.test.js › configuration › ajout : identifiant tiré du nom, champs par défaut, validation complète
#[test]
#[ignore = "phase 2"]
fn ajout() {
    let base = base();
    let (config, id) = create_project(
        &base,
        &json!({ "name": "Mon Été", "cwd": "C:/m", "cmd": "node", "args": ["s.js"], "port": 3500 }),
    )
    .unwrap();
    assert_eq!(id, "mon-ete");
    assert_eq!(config["projects"][3]["hostControl"], "env");
    let (_, second) = create_project(
        &config,
        &json!({ "name": "Mon été", "type": "file", "cwd": "C:/m", "open": "index.html" }),
    )
    .unwrap();
    assert_eq!(second, "mon-ete-2");
    assert_match!(
        fr(create_project(
            &base,
            &json!({ "name": "X", "cwd": "C:/x", "cmd": "node", "port": 3001 })
        )),
        "Port 3001"
    );
    assert_match!(
        fr(create_project(
            &base,
            &json!({ "name": "X", "cwd": "C:/x", "port": 3600 })
        )),
        "commande"
    );
    assert_match!(
        fr(create_project(
            &base,
            &json!({ "name": "X", "cwd": "C:/x", "cmd": "a & calc", "port": 3600 })
        )),
        "commande"
    );
    assert_eq!(
        base["projects"].as_array().unwrap().len(),
        3,
        "l'original n'est pas modifié"
    );
}

/// JS : servynex.test.js › configuration › suppression : retirée des dépendances des autres projets
#[test]
#[ignore = "phase 2"]
fn suppression() {
    let mut conf = base();
    let projects = conf["projects"].as_array_mut().unwrap();
    projects.push(json!({ "id": "d", "name": "D", "cwd": "C:/d", "cmd": "node", "args": [], "port": 3004, "hostControl": "env", "requires": ["a", "c"] }));
    projects.push(json!({ "id": "e", "name": "E", "cwd": "C:/e", "cmd": "node", "args": [], "port": 3005, "hostControl": "env", "requires": ["a"] }));
    let next = remove_project(&conf, "a").unwrap();
    let ids: Vec<&str> = next["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["b", "c", "d", "e"]);
    assert_eq!(next["projects"][2]["requires"], json!(["c"]));
    assert!(
        next["projects"][3].get("requires").is_none(),
        "liste vide : champ retiré"
    );
    assert_eq!(
        conf["projects"].as_array().unwrap().len(),
        5,
        "l'original n'est pas modifié"
    );
    assert_eq!(remove_project(&conf, "absent").unwrap_err().status, 404);
}

/// JS : servynex.test.js › configuration › différences de réglages : valeurs lisibles, jamais celles de env
#[test]
#[ignore = "phase 2"]
fn differences_de_reglages() {
    let before = json!({ "id": "a", "name": "A", "port": 1, "env": { "K": "secret" }, "tags": ["x"] });
    let after = json!({ "id": "a", "name": "B", "port": 1, "env": { "K": "autre", "L": "2" }, "tags": ["x", "y"], "url": "http://a" });
    assert_eq!(
        describe_changes(&before, &after),
        "name \"A\" → \"B\" · env K=••• → K=•••, L=••• · tags [\"x\"] → [\"x\",\"y\"] · url — → \"http://a\""
    );
    assert_eq!(describe_changes(&before, &before), "");
    let long = describe_changes(&json!({ "desc": "x".repeat(200) }), &json!({}));
    assert_match!(long, "^desc \"x{40,}…\" → —$", "valeurs longues coupées");
}

/// JS : servynex.test.js › configuration › réordonner : tous les identifiants, chacun une fois
#[test]
#[ignore = "phase 2"]
fn reordonner() {
    let base = base();
    let next = reorder_projects(&base, &json!(["c", "a", "b"])).unwrap();
    let ids = |c: &Value| {
        c["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&next), ["c", "a", "b"]);
    assert_eq!(ids(&base), ["a", "b", "c"], "l'original n'est pas modifié");
    for order in [
        json!(["c", "a"]),
        json!(["c", "a", "b", "x"]),
        json!(["a", "a", "b"]),
        json!("a,b,c"),
        json!(null),
        json!(["a", "b", 3]),
    ] {
        assert_eq!(
            reorder_projects(&base, &order).unwrap_err().status,
            400,
            "{order}"
        );
    }
}

/// JS : servynex.test.js › configuration › langue des messages du serveur (instance.lang) : fr ou en
#[test]
#[ignore = "phase 2"]
fn langue_du_serveur_fr_ou_en() {
    for lang in ["fr", "en"] {
        let mut conf = base();
        conf["instance"] = json!({ "port": 9000, "lang": lang });
        validate(&conf).unwrap();
    }
    let mut conf = base();
    conf["instance"] = json!({ "port": 9000, "lang": "de" });
    assert_match!(fr(validate(&conf)), "instance.lang");
}

/// JS : servynex.test.js › configuration › fichier absent : configuration vide ; mise en forme compacte conservée
#[test]
#[ignore = "phase 2"]
fn fichier_absent_et_mise_en_forme() {
    let dir = temp_dir("servynex-absent-");
    assert_eq!(
        load_config(&dir.path().join("absent.json")).unwrap(),
        json!({ "instance": { "port": 9000 }, "projects": [] })
    );
    let text = format_config(
        &json!({ "instance": { "port": 9000 }, "projects": [{ "id": "a", "name": "A", "tags": ["x", "y"], "health": { "path": "/" } }] }),
    );
    assert_match!(text, r#""tags": \["x", "y"\]"#);
    assert_match!(text, r#""health": \{ "path": "/" \}"#);
    assert_match!(text, r#""instance": \{ "port": 9000 \}"#);
}

// ---- exemple publié ----

/// JS : servynex.test.js › exemple publié › projects.example.json : valide, sans fichier de config/ (non publié)
#[test]
#[ignore = "phase 2"]
fn exemple_publie() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../projects.example.json");
    load_config(&file).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains("{servynex}/config/"), "config/ n’est pas publié");
}

// ---- processeur et mémoire ----

fn proc(pid: u32, ppid: u32, start: i64, cpu: u64, memory: u64) -> (u32, ProcSample) {
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
}

fn snap(at: i64, list: Vec<(u32, ProcSample)>) -> Snapshot {
    Snapshot {
        at,
        processes: list.into_iter().collect(),
    }
}

/// JS : servynex.test.js › processeur et mémoire › arborescence : enfants et petits-enfants, jamais un PID réutilisé
#[test]
#[ignore = "phase 2"]
fn arborescence() {
    let s = snap(
        0,
        vec![
            proc(10, 1, 100, 0, 0),
            proc(11, 10, 110, 0, 0),
            proc(12, 11, 120, 0, 0),
            proc(13, 10, 50, 0, 0),
            proc(20, 1, 100, 0, 0),
        ],
    );
    assert_eq!(
        tree_of(&s.processes, &[10]).into_iter().collect::<Vec<_>>(),
        [10, 11, 12],
        "13 a démarré avant 10 : son parent n'est pas ce 10-là"
    );
    assert_eq!(
        tree_of(&s.processes, &[10, 11]).into_iter().collect::<Vec<_>>(),
        [10, 11, 12],
        "racines qui se recouvrent : chaque processus une fois"
    );
    assert!(tree_of(&s.processes, &[99]).is_empty());
}

/// JS : servynex.test.js › processeur et mémoire › somme de l’arborescence, en part de toute la machine
#[test]
#[ignore = "phase 2"]
fn somme_de_l_arborescence() {
    let prev = snap(
        10_000,
        vec![
            proc(10, 1, 100, 1_000, 0),
            proc(11, 10, 110, 3_000, 0),
            proc(12, 11, 120, 500, 0),
        ],
    );
    let next = snap(
        15_000,
        vec![
            proc(10, 1, 100, 1_200, 10_000_000),
            proc(11, 10, 110, 3_600, 50_000_000),
            proc(12, 11, 12_000, 200, 5_000_000),
            proc(14, 11, 13_000, 200, 1_000_000),
        ],
    );
    // 12 a été remplacé (autre début) et 14 est nouveau : comptés depuis leur démarrage.
    // (200 + 600 + 200 + 200) ms sur 5 000 ms × 4 cœurs = 6 %.
    assert_eq!(
        usage_of(Some(&prev), &next, &[10], 4),
        Some(Usage {
            cpu: Some(6.0),
            memory: 66_000_000,
            processes: 4
        })
    );
    assert_eq!(
        usage_of(None, &next, &[10], 4).unwrap().cpu,
        None,
        "premier relevé : processeur inconnu"
    );
    assert_eq!(usage_of(None, &next, &[10], 4).unwrap().memory, 66_000_000);
    assert_eq!(
        usage_of(Some(&prev), &next, &[99], 4),
        None,
        "processus introuvable"
    );
}

fn roots(list: &[(&str, &[u32])]) -> BTreeMap<String, Vec<u32>> {
    list.iter()
        .map(|(id, pids)| (id.to_string(), pids.to_vec()))
        .collect()
}

/// JS : servynex.test.js › processeur et mémoire › relevés partagés : des lectures rapprochées ne relancent pas la mesure
#[tokio::test]
#[ignore = "phase 2"]
async fn releves_partages() {
    let calls = Arc::new(AtomicU64::new(0));
    let now = Arc::new(AtomicI64::new(0));
    let counter = calls.clone();
    let sample = Arc::new(move || {
        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
        let table: ProcTable = [proc(10, 1, 0, n * 1000, 5_000_000)].into_iter().collect();
        Box::pin(async move { table }) as servynex_core::BoxFuture<'static, ProcTable>
    });
    let clock = now.clone();
    let sampler = Sampler::new(
        sample,
        Arc::new(move || clock.load(Ordering::SeqCst)),
        1,
        Duration::from_millis(2000),
    );
    let first = sampler.measure(&roots(&[("a", &[10]), ("b", &[99])])).await;
    assert_eq!(
        first,
        BTreeMap::from([(
            "a".to_string(),
            Usage {
                cpu: None,
                memory: 5_000_000,
                processes: 1
            }
        )]),
        "b introuvable : absent"
    );
    now.store(1000, Ordering::SeqCst);
    assert_eq!(
        sampler.measure(&roots(&[("a", &[10])])).await,
        first,
        "moins de 2 s : même relevé"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    now.store(5000, Ordering::SeqCst);
    let a = roots(&[("a", &[10])]);
    tokio::join!(sampler.measure(&a), sampler.measure(&a));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "deux onglets en même temps : une seule mesure"
    );
    assert_eq!(
        sampler.measure(&a).await["a"].cpu,
        Some(20.0),
        "(2 000 − 1 000) ms sur 5 000 ms"
    );
}

/// JS : servynex.test.js › total : Servynex compris, chaque processus compté une fois
#[tokio::test]
#[ignore = "phase 2"]
async fn total_servynex_compris() {
    // Servynex (1) lance le projet par un intermédiaire (2) → cmd.exe (3, racine du projet) → node (4) ;
    // un relevé (5) est aussi son enfant.
    let table: ProcTable = [(1, 0, 100), (2, 1, 110), (3, 2, 120), (4, 3, 130), (5, 1, 140)]
        .into_iter()
        .map(|(pid, ppid, start)| proc(pid, ppid, start, 0, u64::from(pid) * 1_000_000))
        .collect();
    let sample = Arc::new(move || {
        let table = table.clone();
        Box::pin(async move { table }) as servynex_core::BoxFuture<'static, ProcTable>
    });
    let sampler = Sampler::new(sample, Arc::new(|| 0), 1, Duration::from_millis(2000));
    let res = sampler.measure_all(&roots(&[("self", &[3])]), 1).await;
    assert_eq!(
        res.projects["self"].processes, 2,
        "un projet peut s’appeler « self »"
    );
    assert_eq!(res.projects["self"].memory, 7_000_000);
    assert_eq!(
        res.own,
        Usage {
            cpu: None,
            memory: 8_000_000,
            processes: 3
        },
        "Servynex sans ses projets"
    );
    assert_eq!(
        res.total,
        Usage {
            cpu: None,
            memory: 15_000_000,
            processes: 5
        },
        "chacun une fois"
    );
}

// ---- fichiers statiques ----

fn site_root() -> std::path::PathBuf {
    std::env::temp_dir().join("site")
}

/// JS : servynex.test.js › fichiers statiques › reste dans le dossier
#[test]
#[ignore = "phase 2"]
fn reste_dans_le_dossier() {
    let root = site_root();
    assert_eq!(
        resolve_inside(&root, "/css/a.css"),
        Some(root.join("css").join("a.css"))
    );
    assert_eq!(resolve_inside(&root, "/"), Some(root.clone()));
    assert_eq!(resolve_inside(&root, "/../secret"), None);
    assert_eq!(resolve_inside(&root, "/%2e%2e/%2e%2e/Windows/win.ini"), None);
    assert_eq!(resolve_inside(&root, "/..%5c..%5cWindows"), None);
    assert_eq!(
        resolve_inside(&root, "//serveur/partage/x"),
        Some(root.join("serveur").join("partage").join("x"))
    );
    assert_eq!(
        resolve_inside(&root, "/%5c%5cserveur%5cpartage"),
        Some(root.join("serveur").join("partage"))
    );
}

/// JS : servynex.test.js › fichiers statiques › refuse fichiers cachés, flux NTFS et octets nuls
#[test]
#[ignore = "phase 2"]
fn refuse_caches_flux_ntfs_octets_nuls() {
    let root = site_root();
    assert_eq!(resolve_inside(&root, "/.git/config"), None);
    assert_eq!(resolve_inside(&root, "/.env"), None);
    assert_eq!(resolve_inside(&root, "/index.html::$DATA"), None);
    assert_eq!(resolve_inside(&root, "/a%00.html"), None);
    assert_eq!(resolve_inside(&root, "/%E0%A4%A"), None);
}

// ---- journaux ----

/// JS : servynex.test.js › startupReason : erreur, sinon avertissement, jamais une ligne quelconque
#[test]
#[ignore = "phase 2"]
fn startup_reason_erreur_sinon_avertissement() {
    let dir = temp_dir("servynex-startup-");
    let file = dir.path().join("p.log");
    write(&file, "[servynex] lancement\n [WARNING] already running\nNone\n");
    assert_eq!(
        startup_reason(&file).as_deref(),
        Some("[WARNING] already running")
    );
    write(&file, "Warning: deprecated\nError: port pris\nNone\n");
    assert_eq!(startup_reason(&file).as_deref(), Some("Error: port pris"));
    write(&file, "None\n");
    assert_eq!(startup_reason(&file), None);
}

/// JS : servynex.test.js › lastError choisit la ligne qui explique l'arrêt, pas un avertissement
#[test]
#[ignore = "phase 2"]
fn last_error_explique_l_arret() {
    let dir = temp_dir("servynex-test-");
    let file = dir.path().join("p.log");
    write(
        &file,
        "[servynex] lancement\nprêt\nError: listen EADDRINUSE :::3000\n    at Server.listen\n(node) Warning: deprecated\n",
    );
    assert_eq!(
        last_error(&file).as_deref(),
        Some("Error: listen EADDRINUSE :::3000")
    );
    write(&file, "Connexion impossible à la base\nfin\n");
    assert_eq!(
        last_error(&file).as_deref(),
        Some("Connexion impossible à la base")
    );
}
