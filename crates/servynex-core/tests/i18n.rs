//! tests/i18n.test.js : textes du serveur et de l'interface.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{Map, Value};
use servynex_core::i18n::{Lang, LocalError, catalog, detect_lang, lang_of, msg, tr};
use servynex_core::params;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Textes de l'interface : `public/locales/<langue>.js`, un objet JSON après `export default`.
fn ui_catalog(lang: &str) -> Map<String, Value> {
    let text = std::fs::read_to_string(root().join("public/locales").join(format!("{lang}.js"))).unwrap();
    let start = text.find("export default {").unwrap() + "export default ".len();
    let end = text.rfind('}').unwrap();
    // Objet JavaScript : virgule permise avant l'accolade finale.
    let body = Regex::new(r",\s*$")
        .unwrap()
        .replace(text[start..end].trim_end(), "")
        .into_owned();
    serde_json::from_str(&format!("{body}}}")).unwrap()
}

fn params_of(text: &str) -> Vec<String> {
    let mut list: Vec<String> = Regex::new(r"\{(\w+)\}")
        .unwrap()
        .captures_iter(text)
        .map(|c| c[1].to_string())
        .collect();
    list.sort();
    list
}

/// Chaque texte existe dans les deux langues, avec les mêmes paramètres {…}.
fn same_shape(en: &Map<String, Value>, fr: &Map<String, Value>, place: &str) {
    let keys = |m: &Map<String, Value>| m.keys().cloned().collect::<BTreeSet<_>>();
    assert_eq!(keys(fr), keys(en), "{place} : mêmes clés");
    for (key, value) in en {
        let en_text = value.as_str().unwrap_or_else(|| panic!("{place} {key}"));
        let fr_text = fr[key].as_str().unwrap_or_else(|| panic!("{place} {key}"));
        assert!(
            !en_text.trim().is_empty() && !fr_text.trim().is_empty(),
            "{place} {key} : texte vide"
        );
        assert_eq!(
            params_of(fr_text),
            params_of(en_text),
            "{place} {key} : mêmes paramètres"
        );
    }
}

/// JS : i18n.test.js › langues › serveur et interface : mêmes clés et mêmes paramètres en anglais et en français
#[test]
fn memes_cles_et_parametres() {
    same_shape(catalog(Lang::En), catalog(Lang::Fr), "serveur");
    same_shape(&ui_catalog("en"), &ui_catalog("fr"), "interface");
}

/// JS : i18n.test.js › langues › traduction : paramètres, message imbriqué, clé inconnue
#[test]
fn traduction() {
    let en = catalog(Lang::En);
    let (key, text) = en
        .iter()
        .find(|(_, v)| !params_of(v.as_str().unwrap()).is_empty())
        .expect("aucun message à paramètre");
    let values = params_of(text.as_str().unwrap())
        .into_iter()
        .map(|p| (p.clone(), format!("<{p}>").into()))
        .collect();
    assert!(
        !Regex::new(r"\{\w+\}")
            .unwrap()
            .is_match(&tr(key, &values, Lang::En)),
        "paramètres remplacés"
    );
    assert_eq!(tr("clé.inconnue", &params![], Lang::Fr), "clé.inconnue");
    let p = params![
        port = 3001u16,
        first = msg("config.instance", params![]),
        second = "B"
    ];
    assert_eq!(
        tr("config.duplicatePort", &p, Lang::En),
        "Port 3001 used by both Servynex and B"
    );
    assert_eq!(
        tr("config.duplicatePort", &p, Lang::Fr),
        "Port 3001 utilisé à la fois par Servynex et B"
    );
}

/// JS : i18n.test.js › langues › langue du serveur : variable SERVYNEX_LANG, sinon celle du système, sinon l’anglais
#[test]
fn langue_du_serveur() {
    assert_eq!(detect_lang(Some("fr-FR"), None), Lang::Fr);
    assert_eq!(detect_lang(Some("de-DE"), None), Lang::En);
    assert_eq!(detect_lang(Some("fr-FR"), Some("en")), Lang::En);
    assert_eq!(detect_lang(Some("en-US"), Some("fr-CA")), Lang::Fr);
}

/// JS : i18n.test.js › langues › langue d’une requête : Accept-Language, sinon celle du serveur
#[test]
fn langue_d_une_requete() {
    assert_eq!(lang_of(Some("en-US,en;q=0.9"), Lang::Fr), Lang::En);
    assert_eq!(lang_of(Some("de-DE, fr;q=0.8"), Lang::Fr), Lang::Fr);
    assert_eq!(lang_of(None, Lang::Fr), Lang::Fr);
}

/// JS : i18n.test.js › langues › erreur traduisible : message dans la langue du serveur, retraduit à la demande
#[test]
fn erreur_traduisible() {
    let err = LocalError::new(404, "project.unknown", params![]);
    assert_eq!(
        err.message_in(Lang::Fr),
        catalog(Lang::Fr)["project.unknown"].as_str().unwrap()
    );
    assert_eq!(
        err.message_in(Lang::En),
        catalog(Lang::En)["project.unknown"].as_str().unwrap()
    );
    assert_eq!(err.status, 404);
}

fn read_all(dir: &Path, ext: &str, recursive: bool) -> String {
    let mut out = String::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                out.push_str(&read_all(&path, ext, true));
            }
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push_str(&std::fs::read_to_string(&path).unwrap());
            out.push('\n');
        }
    }
    out
}

/// JS : i18n.test.js › langues › chaque clé utilisée par l’interface et le serveur existe
#[test]
fn chaque_cle_utilisee_existe() {
    let ui = read_all(&root().join("public/js"), "js", false);
    let pages = read_all(&root().join("public"), "html", false);
    let mut used = BTreeSet::new();
    for c in Regex::new(r"\bt\('([\w.]+)'").unwrap().captures_iter(&ui) {
        used.insert(c[1].to_string());
    }
    for c in Regex::new(r"\btn\('([\w.]+)'").unwrap().captures_iter(&ui) {
        used.insert(format!("{}.one", &c[1]));
        used.insert(format!("{}.other", &c[1]));
    }
    for c in Regex::new(r#"data-i18n(?:-rich)?="([\w.]+)""#)
        .unwrap()
        .captures_iter(&pages)
    {
        used.insert(c[1].to_string());
    }
    for c in Regex::new(r#"data-i18n-attr="([^"]+)""#)
        .unwrap()
        .captures_iter(&pages)
    {
        for pair in c[1].split(';') {
            used.insert(pair.split(':').nth(1).unwrap_or_default().to_string());
        }
    }
    let ui_en = ui_catalog("en");
    assert_eq!(
        used.iter()
            .filter(|k| !ui_en.contains_key(*k))
            .collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "interface"
    );

    // Serveur : clés écrites dans le code Rust.
    let server = read_all(&root().join("crates"), "rs", true);
    let re = Regex::new(
        r#""((?:config|project|process|http|auth|audit|history|log|system|cli|time|role)\.[A-Za-z.]+)""#,
    )
    .unwrap();
    let en = catalog(Lang::En);
    let missing: Vec<String> = re
        .captures_iter(&server)
        .map(|c| c[1].to_string())
        .filter(|k| !k.ends_with(".json") && !k.ends_with(".rs") && !en.contains_key(k))
        .collect();
    assert_eq!(missing, Vec::<String>::new(), "serveur");
}
