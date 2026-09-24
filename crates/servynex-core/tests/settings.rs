//! Parties « configuration » des autres fichiers de tests JS : relance après plantage,
//! duplication, accès réseau, notifications ; et le fichier des comptes.

use serde_json::{Value, json};
use servynex_core::auth::AuthStore;
use servynex_core::config::{duplicate_project, patch_project, validate};
use servynex_core::i18n::{Lang, LocalError};
use servynex_fixtures::{assert_match, temp_dir, write};

fn fr<T: std::fmt::Debug>(result: Result<T, LocalError>) -> String {
    result
        .expect_err("une erreur était attendue")
        .message_in(Lang::Fr)
}

// ---- restart.test.js ----

fn one_program() -> Value {
    json!({ "instance": { "port": 9000 }, "projects": [{ "id": "a", "name": "A", "cwd": "C:/a", "cmd": "node", "args": [], "port": 3001, "hostControl": "env" }] })
}

/// JS : restart.test.js › relancer s’il plante : configuration › booléen, pour un programme seulement
#[test]
fn relance_booleen_programme_seulement() {
    assert_eq!(
        patch_project(&one_program(), "a", &json!({ "restartOnCrash": true })).unwrap()["projects"][0]["restartOnCrash"],
        true
    );
    let mut conf = one_program();
    conf["projects"][0]["restartOnCrash"] = json!("oui");
    assert_match!(fr(validate(&conf)), "restartOnCrash");
    conf["projects"][0] = json!({ "id": "a", "name": "A", "cwd": "C:/a", "type": "static", "port": 3001, "restartOnCrash": true });
    assert_match!(fr(validate(&conf)), "restartOnCrash");
}

/// JS : restart.test.js › relancer s’il plante : configuration › passer en site statique ou en fichier retire le réglage
#[test]
fn relance_retiree_au_changement_de_type() {
    let on = patch_project(&one_program(), "a", &json!({ "restartOnCrash": true })).unwrap();
    assert!(
        patch_project(&on, "a", &json!({ "type": "static" })).unwrap()["projects"][0]
            .get("restartOnCrash")
            .is_none()
    );
    let as_file = patch_project(&on, "a", &json!({ "type": "file", "open": "index.html" })).unwrap();
    assert!(as_file["projects"][0].get("restartOnCrash").is_none());
}

// ---- duplicate.test.js ----

fn dup_base() -> Value {
    json!({
        "instance": { "port": 9000 },
        "projects": [
            { "id": "api", "name": "API", "cwd": "C:/api", "cmd": "node", "args": ["server.js"], "port": 3001, "hostControl": "env", "env": { "TOKEN": "x" }, "autostart": true, "expose": true, "fixedPort": "codé en dur", "tags": ["Node"] },
            { "id": "doc", "name": "Doc", "type": "file", "cwd": "C:/doc", "open": "index.html" },
            { "id": "web", "name": "Web", "cwd": "C:/web", "cmd": "node", "args": [], "port": 3002, "hostControl": "env", "requires": ["api"] },
        ],
    })
}

fn ids(config: &Value) -> Vec<String> {
    config["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

/// JS : duplicate.test.js › dupliquer un projet › copie juste après l’original : nouvel identifiant, nom et port
#[test]
fn copie_apres_l_original() {
    let base = dup_base();
    let (config, id) = duplicate_project(&base, "api", "API (copie)", Some(3100)).unwrap();
    assert_eq!(id, "api-2");
    assert_eq!(ids(&config), ["api", "api-2", "doc", "web"]);
    let copy = &config["projects"][1];
    assert_eq!(copy["name"], "API (copie)");
    assert_eq!(copy["port"], 3100);
    assert_eq!(
        json!([copy["cmd"], copy["args"], copy["cwd"], copy["env"], copy["tags"]]),
        json!(["node", ["server.js"], "C:/api", { "TOKEN": "x" }, ["Node"]])
    );
    assert!(
        copy.get("autostart").is_none(),
        "pas lancé au démarrage du PC sans le demander"
    );
    assert!(
        copy.get("expose").is_none(),
        "pas exposé au réseau sans le demander"
    );
    assert!(
        copy.get("fixedPort").is_none(),
        "le port fixe est celui de l’original"
    );
    assert_eq!(base["projects"].as_array().unwrap().len(), 3, "original intact");
    assert_eq!(
        duplicate_project(&config, "api", "API (copie)", Some(3101))
            .unwrap()
            .1,
        "api-3"
    );
}

/// JS : duplicate.test.js › dupliquer un projet › dépendances gardées ; page locale sans port
#[test]
fn dependances_gardees_page_sans_port() {
    let (config, _) = duplicate_project(&dup_base(), "web", "Web 2", Some(3200)).unwrap();
    assert_eq!(config["projects"][3]["requires"], json!(["api"]));
    let (config, _) = duplicate_project(&dup_base(), "doc", "Doc (copie)", None).unwrap();
    assert!(config["projects"][2].get("port").is_none());
    assert_eq!(config["projects"][2]["open"], "index.html");
}

/// JS : duplicate.test.js › dupliquer un projet › projet inconnu
#[test]
fn duplication_projet_inconnu() {
    assert_match!(
        fr(duplicate_project(&dup_base(), "zz", "x", Some(1))),
        "(?i)inconnu"
    );
}

// ---- access.test.js ----

/// JS : access.test.js › accès depuis le réseau privé › instance.access : « local » ou « lan »
#[test]
fn instance_access() {
    let conf = |access: &str| json!({ "instance": { "port": 9000, "access": access }, "projects": [] });
    validate(&conf("local")).unwrap();
    validate(&conf("lan")).unwrap();
    assert_match!(fr(validate(&conf("internet"))), r"instance\.access");
}

// ---- notify.test.js ----

/// JS : notify.test.js › notifications système › instance.notify : oui ou non
#[test]
fn instance_notify() {
    let conf = |notify: Value| json!({ "instance": { "port": 9000, "notify": notify }, "projects": [] });
    validate(&conf(json!(true))).unwrap();
    validate(&conf(json!(false))).unwrap();
    assert_match!(fr(validate(&conf(json!("oui")))), r"instance\.notify");
}

// ---- auth.test.js ----

/// JS : auth.test.js › fichier des comptes changé sur le disque : relu même à la même date de modification
#[test]
fn comptes_relus_meme_date() {
    let dir = temp_dir("servynex-store-");
    let file = dir.path().join("auth.json");
    let mut store = AuthStore::new(&file);
    store.load()["enabled"] = json!(true);
    store.save().unwrap();
    write(&file, "{ cassé");
    // Modifié dans le même instant que l'enregistrement.
    store.set_seen_mtime(std::fs::metadata(&file).unwrap().modified().unwrap());
    store.load();
    assert!(store.error().is_some(), "changement vu");
}
