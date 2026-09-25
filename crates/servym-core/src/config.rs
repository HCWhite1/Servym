//! Configuration (`projects.json`) : validation, mise en forme, modifications.
//!
//! La configuration reste un document JSON (`serde_json::Value`, ordre des clés gardé) : les champs
//! inconnus et l'ordre écrit à la main survivent à chaque modification faite par l'interface.
//! Toutes les fonctions de modification rendent une copie ; l'original n'est jamais modifié.
//!
//! Les règles reprennent celles de la version Node à l'identique (mêmes messages, mêmes limites,
//! longueurs comptées en unités UTF-16 comme en JavaScript).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::i18n::{Arg, LocalError, Params, msg};
use crate::params;

static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9-]{1,32}$").unwrap());
static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^#[0-9a-f]{6}$").unwrap());
static ICON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{0,31}$").unwrap());
static ENV_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]{0,63}$").unwrap());
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?://").unwrap());

const HOST_CONTROLS: [&str; 3] = ["env", "args", "local"];
/// Accès à la page : ce PC seulement, ou aussi le réseau local.
pub const ACCESS: [&str; 2] = ["local", "lan"];
const LANGS: [&str; 2] = ["en", "fr"];

/// Configuration utilisée quand `projects.json` n'existe pas.
pub fn default_config() -> Value {
    json!({ "instance": { "port": 9000 }, "projects": [] })
}

// ---- Outils de valeur, à la manière de JavaScript ----

/// Longueur d'un texte en unités UTF-16 (`String.length`).
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Nombre entier (`Number.isInteger`) : `3000` comme `3000.0`.
fn as_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| {
            let f = n.as_f64()?;
            (f.fract() == 0.0 && f.abs() < 9.0e15).then_some(f as i64)
        }),
        _ => None,
    }
}

fn is_port(v: Option<&Value>) -> bool {
    v.and_then(as_int).is_some_and(|p| (1..=65535).contains(&p))
}

/// Port d'un projet, s'il en a un valide.
pub fn port_of(project: &Value) -> Option<u16> {
    project
        .get("port")
        .and_then(as_int)
        .and_then(|p| u16::try_from(p).ok())
}

fn is_text(v: Option<&Value>, max: usize) -> bool {
    matches!(v, Some(Value::String(s)) if !s.trim().is_empty() && js_len(s) <= max)
}

/// Les arguments passent par cmd.exe : guillemets et % y garderaient un sens même entre
/// guillemets. & | < > ^ ( ) sont refusés aussi, pour que la sécurité ne repose pas sur le seul
/// placement des guillemets.
fn safe_arg(s: &str) -> bool {
    !s.chars().any(|c| {
        matches!(
            c,
            '"' | '%' | '\r' | '\n' | '&' | '|' | '<' | '>' | '^' | '(' | ')'
        )
    })
}

fn is_arg_list(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Array(list)) => {
            list.len() <= 64
                && list
                    .iter()
                    .all(|a| matches!(a, Value::String(s) if js_len(s) <= 1000 && safe_arg(s)))
        }
        _ => false,
    }
}

/// Chemin absolu, formes de Windows comprises (`C:/…`, `\\serveur`, `/…`), sur tous les systèmes.
pub fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    matches!(b.first(), Some(b'/' | b'\\'))
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\'))
}

fn is_bool(v: Option<&Value>) -> bool {
    matches!(v, None | Some(Value::Bool(_)))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `String(value)` de JavaScript, pour les paramètres des messages.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Array(list) => list
            .iter()
            .map(|x| if x.is_null() { String::new() } else { js_string(x) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
        other => js_json(other),
    }
}

/// Texte d'un champ (absent : `undefined`, comme en JavaScript).
fn field_text(p: &Value, key: &str) -> String {
    p.get(key).map_or_else(|| "undefined".into(), js_string)
}

/// `JSON.stringify` compact : les nombres entiers écrits sans décimale (`3000.0` → `3000`).
pub fn js_json(v: &Value) -> String {
    match v {
        Value::Number(n) if n.is_f64() => {
            let f = n.as_f64().unwrap();
            if f.fract() == 0.0 && f.abs() < 1e21 {
                format!("{}", f as i128)
            } else {
                n.to_string()
            }
        }
        Value::Array(list) => format!("[{}]", list.iter().map(js_json).collect::<Vec<_>>().join(",")),
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, x)| format!("{}:{}", Value::String(k.clone()), js_json(x)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => other.to_string(),
    }
}

fn check(ok: bool, key: &str, params: Params) -> Result<(), LocalError> {
    if ok {
        Ok(())
    } else {
        Err(LocalError::new(400, key, params))
    }
}

fn unknown_project() -> LocalError {
    LocalError::new(404, "project.unknown", params![])
}

fn where_of(p: &Value) -> Params {
    let name = p
        .get("name")
        .or_else(|| p.get("id"))
        .map_or_else(|| "undefined".into(), js_string);
    params![where = msg("config.where", params![name = name])]
}

fn with_where(p: &Value, mut extra: Params) -> Params {
    let mut params = where_of(p);
    params.append(&mut extra);
    params
}

// ---- Validation ----

fn check_project(p: &Value, ids: &mut HashSet<String>) -> Result<(), LocalError> {
    check(p.is_object(), "config.invalidProject", params![])?;
    let id = p.get("id");
    check(
        matches!(id, Some(Value::String(s)) if ID.is_match(s)),
        "config.badId",
        params![id = field_text(p, "id")],
    )?;
    let id = id.and_then(Value::as_str).unwrap_or_default().to_string();
    let w = || where_of(p);
    check(!ids.contains(&id), "config.duplicateId", w())?;
    ids.insert(id);
    check(is_text(p.get("name"), 40), "config.badName", w())?;
    check(
        p.get("desc").is_none() || matches!(p.get("desc"), Some(Value::String(s)) if js_len(s) <= 160),
        "config.badDesc",
        w(),
    )?;
    let icon = p.get("icon");
    check(
        icon.is_none() || matches!(icon, Some(Value::String(s)) if ICON.is_match(s)) || is_text(icon, 8),
        "config.badIcon",
        w(),
    )?;
    check(
        p.get("color").is_none() || matches!(p.get("color"), Some(Value::String(s)) if COLOR.is_match(s)),
        "config.badColor",
        w(),
    )?;
    check(
        match p.get("tags") {
            None => true,
            Some(Value::Array(tags)) => tags.len() <= 6 && tags.iter().all(|t| is_text(Some(t), 20)),
            _ => false,
        },
        "config.badTags",
        w(),
    )?;
    let kind = p.get("type");
    check(
        kind.is_none() || matches!(kind, Some(Value::String(s)) if s == "static" || s == "file"),
        "config.badType",
        w(),
    )?;
    let kind = kind.and_then(Value::as_str);
    if kind == Some("file") {
        // Fichier ouvert directement dans le navigateur : pas de port, pas de processus.
        check(p.get("port").is_none(), "config.filePort", w())?;
        let open = p.get("open");
        check(
            is_text(open, 260)
                && open
                    .and_then(Value::as_str)
                    .is_some_and(|o| !is_absolute(o) && !o.split(['\\', '/']).any(|part| part == "..")),
            "config.fileOpen",
            w(),
        )?;
    } else {
        check(is_port(p.get("port")), "config.badPort", w())?;
    }
    check(
        matches!(p.get("cwd"), Some(Value::String(s)) if js_len(s) <= 260 && is_absolute(s)),
        "config.badCwd",
        w(),
    )?;
    if kind.is_none() {
        check(
            is_text(p.get("cmd"), 260) && p["cmd"].as_str().is_some_and(safe_arg),
            "config.badCmd",
            w(),
        )?;
        check(is_arg_list(p.get("args")), "config.badArgs", w())?;
        check(
            matches!(p.get("hostControl"), Some(Value::String(s)) if HOST_CONTROLS.contains(&s.as_str())),
            "config.badHostControl",
            with_where(p, params![choices = HOST_CONTROLS.join(", ")]),
        )?;
    }
    if let Some(stop) = p.get("stop") {
        // Exécutée sans shell, mais validée comme cmd/args : une erreur remonte au chargement
        // plutôt qu'au moment d'arrêter le projet.
        check(kind.is_none(), "config.stopNotProgram", w())?;
        let args_ok = match stop.get("args") {
            None => true,
            some => is_arg_list(some),
        };
        check(
            truthy(Some(stop))
                && is_text(stop.get("cmd"), 260)
                && stop["cmd"].as_str().is_some_and(safe_arg)
                && args_ok,
            "config.badStop",
            w(),
        )?;
    }
    if let Some(env) = p.get("env") {
        check(
            match env {
                Value::Object(map) => {
                    map.len() <= 64
                        && map.iter().all(|(k, v)| {
                            ENV_NAME.is_match(k)
                                && matches!(v, Value::String(s) if js_len(s) <= 1000 && safe_arg(s))
                        })
                }
                _ => false,
            },
            "config.badEnv",
            w(),
        )?;
    }
    check(
        p.get("url").is_none()
            || matches!(p.get("url"), Some(Value::String(s)) if js_len(s) <= 300 && URL.is_match(s)),
        "config.badUrl",
        w(),
    )?;
    if let Some(hc) = p.get("health") {
        let kind_ok = match hc.get("type") {
            None => true,
            Some(Value::String(t)) => t == "http" || t == "tcp",
            _ => false,
        };
        check(
            truthy(Some(hc)) && (hc.is_object() || hc.is_array()) && kind_ok,
            "config.badHealth",
            w(),
        )?;
        check(
            hc.get("path").is_none()
                || matches!(hc.get("path"), Some(Value::String(s)) if s.starts_with('/') && js_len(s) <= 200),
            "config.badHealthPath",
            w(),
        )?;
        check(
            hc.get("expect").is_none()
                || matches!(hc.get("expect"), Some(Value::String(s)) if js_len(s) <= 200),
            "config.badHealthExpect",
            w(),
        )?;
    }
    check(
        p.get("startTimeout").is_none()
            || p.get("startTimeout")
                .and_then(as_int)
                .is_some_and(|t| (1..=600).contains(&t)),
        "config.badTimeout",
        w(),
    )?;
    check(
        match p.get("requires") {
            None => true,
            Some(Value::Array(list)) => list.iter().all(Value::is_string),
            _ => false,
        },
        "config.badRequires",
        w(),
    )?;
    check(is_bool(p.get("service")), "config.badService", w())?;
    check(is_bool(p.get("expose")), "config.badExpose", w())?;
    check(
        p.get("fixedPort").is_none() || is_text(p.get("fixedPort"), 300),
        "config.badFixedPort",
        w(),
    )?;
    check(is_bool(p.get("autostart")), "config.badAutostart", w())?;
    check(
        !(truthy(p.get("autostart")) && kind == Some("file")),
        "config.fileAutostart",
        w(),
    )?;
    // Relance après un plantage : seul un programme lancé par Servym peut planter.
    check(is_bool(p.get("restartOnCrash")), "config.badRestartOnCrash", w())?;
    check(
        !(truthy(p.get("restartOnCrash")) && kind.is_some()),
        "config.restartNotProgram",
        w(),
    )?;
    Ok(())
}

fn name_of(p: &Value) -> String {
    field_text(p, "name")
}

fn requires_of(p: &Value) -> Vec<&str> {
    p.get("requires")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |l| l.iter().filter_map(Value::as_str).collect())
}

/// Dépendances : projet existant, pas un fichier, pas de boucle (A attend B qui attend A).
fn check_requires(projects: &[Value]) -> Result<(), LocalError> {
    let by_id: HashMap<&str, &Value> = projects
        .iter()
        .map(|p| (p["id"].as_str().unwrap_or_default(), p))
        .collect();
    for p in projects {
        let own = p["id"].as_str().unwrap_or_default();
        for dep in requires_of(p) {
            check(dep != own, "config.requiresSelf", params![name = name_of(p)])?;
            check(
                by_id.contains_key(dep),
                "config.requiresUnknown",
                params![name = name_of(p), dep = dep],
            )?;
            let target = by_id[dep];
            check(
                target.get("type").and_then(Value::as_str) != Some("file"),
                "config.requiresFile",
                params![name = name_of(p), dep = name_of(target)],
            )?;
        }
    }
    fn visit<'a>(
        id: &'a str,
        trail: &mut Vec<&'a str>,
        done: &mut HashSet<&'a str>,
        by_id: &HashMap<&'a str, &'a Value>,
    ) -> Result<(), LocalError> {
        if let Some(start) = trail.iter().position(|x| *x == id) {
            let mut names: Vec<String> = trail[start..].iter().map(|x| name_of(by_id[x])).collect();
            names.push(name_of(by_id[id]));
            return Err(LocalError::new(
                400,
                "config.requiresLoop",
                params![loop = names.join(" → ")],
            ));
        }
        if done.contains(id) {
            return Ok(());
        }
        trail.push(id);
        for dep in requires_of(by_id[id]) {
            visit(dep, trail, done, by_id)?;
        }
        trail.pop();
        done.insert(id);
        Ok(())
    }
    let mut done = HashSet::new();
    for p in projects {
        visit(
            p["id"].as_str().unwrap_or_default(),
            &mut Vec::new(),
            &mut done,
            &by_id,
        )?;
    }
    Ok(())
}

/// Vérifie toute la configuration (erreur 400 au message traduisible).
pub fn validate(config: &Value) -> Result<(), LocalError> {
    let projects = config.get("projects").and_then(Value::as_array);
    check(projects.is_some(), "config.noProjects", params![])?;
    let instance = config.get("instance");
    check(
        is_port(instance.and_then(|i| i.get("port"))),
        "config.badInstancePort",
        params![],
    )?;
    let instance = instance.unwrap();
    check(
        instance.get("name").is_none() || is_text(instance.get("name"), 40),
        "config.badInstanceName",
        params![],
    )?;
    check(
        instance.get("lang").is_none()
            || matches!(instance.get("lang"), Some(Value::String(s)) if LANGS.contains(&s.as_str())),
        "config.badInstanceLang",
        params![],
    )?;
    check(
        instance.get("access").is_none()
            || matches!(instance.get("access"), Some(Value::String(s)) if ACCESS.contains(&s.as_str())),
        "config.badInstanceAccess",
        params![],
    )?;
    check(
        is_bool(instance.get("notify")),
        "config.badInstanceNotify",
        params![],
    )?;
    let mut ids = HashSet::new();
    let mut ports: HashMap<i64, Arg> = HashMap::new();
    ports.insert(
        as_int(&instance["port"]).unwrap(),
        msg("config.instance", params![]).into(),
    );
    let projects = projects.unwrap();
    for p in projects {
        check_project(p, &mut ids)?;
        let Some(port) = p.get("port").and_then(as_int) else {
            continue;
        };
        if let Some(first) = ports.get(&port) {
            return Err(LocalError::new(
                400,
                "config.duplicatePort",
                vec![
                    ("port".into(), port.into()),
                    ("first".into(), first.clone()),
                    ("second".into(), name_of(p).into()),
                ],
            ));
        }
        ports.insert(port, name_of(p).into());
    }
    check_requires(projects)
}

/// Lit et valide un fichier ; fichier absent : configuration vide. Illisible ou invalide :
/// erreur 400, avec la position de l'erreur.
pub fn load_config(file: &Path) -> Result<Value, LocalError> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default_config()),
        Err(e) => return Err(unreadable(file, &e.to_string())),
    };
    let raw: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| unreadable(file, &e.to_string()))?;
    validate(&raw)?;
    Ok(raw)
}

fn unreadable(file: &Path, error: &str) -> LocalError {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    LocalError::new(400, "config.unreadable", params![file = name, error = error])
}

// ---- Mise en forme ----

fn inline(v: &Value) -> String {
    match v {
        Value::Array(list) => format!("[{}]", list.iter().map(inline).collect::<Vec<_>>().join(", ")),
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => format!(
            "{{ {} }}",
            map.iter()
                .map(|(k, x)| format!("{}: {}", Value::String(k.clone()), inline(x)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => js_json(other),
    }
}

fn format_at(value: &Value, depth: usize) -> String {
    let pad = "  ".repeat(depth);
    if !value.is_array() && !value.is_object() {
        return js_json(value);
    }
    // Racine, liste des projets et chaque projet : toujours sur plusieurs lignes.
    let always_block = depth < 3 && (depth == 0 || value.is_array() || depth == 2);
    let flat = inline(value);
    if !always_block && pad.len() + js_len(&flat) <= 160 {
        return flat;
    }
    match value {
        Value::Array(list) if list.is_empty() => "[]".into(),
        Value::Array(list) => format!(
            "[\n{}\n{pad}]",
            list.iter()
                .map(|v| format!("{pad}  {}", format_at(v, depth + 1)))
                .collect::<Vec<_>>()
                .join(",\n")
        ),
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => format!(
            "{{\n{}\n{pad}}}",
            map.iter()
                .map(|(k, v)| format!("{pad}  {}: {}", Value::String(k.clone()), format_at(v, depth + 1)))
                .collect::<Vec<_>>()
                .join(",\n")
        ),
        _ => unreachable!(),
    }
}

/// Mise en forme compacte de `projects.json` (identique à `formatConfig` de la version Node) :
/// un projet par bloc, mais les listes et petits objets (tags, health, env…) sur une ligne.
pub fn format_config(config: &Value) -> String {
    format_at(config, 0)
}

/// Écrit le fichier (fichier temporaire puis renommage).
pub fn save_config(file: &Path, config: &Value) -> std::io::Result<()> {
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, format!("{}\n", format_config(config)))?;
    std::fs::rename(&tmp, file)
}

// ---- Modifications ----

/// Champs d'un projet modifiables depuis l'interface. L'identifiant ne l'est pas : il nomme le
/// journal et l'historique. La valeur `null` retire un champ facultatif.
const FIELDS: [&str; 23] = [
    "name",
    "desc",
    "icon",
    "color",
    "tags",
    "type",
    "cwd",
    "cmd",
    "args",
    "env",
    "stop",
    "port",
    "hostControl",
    "expose",
    "url",
    "health",
    "startTimeout",
    "open",
    "requires",
    "service",
    "fixedPort",
    "autostart",
    "restartOnCrash",
];
const REQUIRED: [&str; 2] = ["name", "cwd"];
const NOT_FOR_FILE: [&str; 13] = [
    "port",
    "cmd",
    "args",
    "env",
    "stop",
    "hostControl",
    "expose",
    "url",
    "health",
    "startTimeout",
    "autostart",
    "fixedPort",
    "restartOnCrash",
];
const NOT_FOR_STATIC: [&str; 7] = [
    "cmd",
    "args",
    "env",
    "stop",
    "hostControl",
    "open",
    "restartOnCrash",
];

fn apply_fields(project: &mut Map<String, Value>, patch: &Value) -> Result<(), LocalError> {
    let patch = patch.as_object();
    check(patch.is_some(), "config.invalidPatch", params![])?;
    for (key, value) in patch.unwrap() {
        check(
            FIELDS.contains(&key.as_str()),
            "config.readOnlyField",
            params![key = key.as_str()],
        )?;
        if value.is_null() {
            check(
                !REQUIRED.contains(&key.as_str()),
                "config.requiredField",
                params![key = key.as_str()],
            )?;
            project.shift_remove(key);
        } else if key == "tags"
            && let Value::Array(tags) = value
        {
            let trimmed = tags
                .iter()
                .map(|t| Value::String(js_string(t).trim().to_string()))
                .collect();
            project.insert(key.clone(), Value::Array(trimmed));
        } else {
            project.insert(key.clone(), value.clone());
        }
    }
    // Champs sans objet pour le type choisi : retirés plutôt que refusés.
    match project.get("type").and_then(Value::as_str) {
        Some("file") => NOT_FOR_FILE.iter().for_each(|k| {
            project.shift_remove(*k);
        }),
        Some("static") => NOT_FOR_STATIC.iter().for_each(|k| {
            project.shift_remove(*k);
        }),
        _ if !project.contains_key("type") => {
            project.shift_remove("open");
        }
        _ => {}
    }
    Ok(())
}

fn projects_mut(config: &mut Value) -> &mut Vec<Value> {
    config["projects"].as_array_mut().expect("configuration validée")
}

fn position(config: &Value, id: &str) -> Option<usize> {
    config["projects"].as_array()?.iter().position(|p| p["id"] == id)
}

/// Modifie un projet depuis l'interface : champs protégés, validation complète.
pub fn patch_project(config: &Value, id: &str, patch: &Value) -> Result<Value, LocalError> {
    let mut next = config.clone();
    let index = position(&next, id).ok_or_else(unknown_project)?;
    let project = projects_mut(&mut next)[index]
        .as_object_mut()
        .ok_or_else(unknown_project)?;
    let before_port = project.get("port").cloned();
    let fixed = project.get("fixedPort").cloned();
    apply_fields(project, patch)?;
    let in_patch = |k: &str| patch.get(k).is_some();
    if project.get("port") != before_port.as_ref()
        && truthy(fixed.as_ref())
        && in_patch("port")
        && !in_patch("fixedPort")
    {
        check(
            false,
            "config.fixedPort",
            params![reason = js_string(fixed.as_ref().unwrap())],
        )?;
    }
    if truthy(patch.get("expose")) {
        let local = project.get("hostControl").and_then(Value::as_str) == Some("local");
        check(
            !local,
            "config.localOnly",
            params![name = field_text(&Value::Object(project.clone()), "name")],
        )?;
    }
    validate(&next)?;
    Ok(next)
}

/// Identifiant tiré du nom : « Mon Application » → `mon-application` (unique).
pub fn slugify(name: &str, taken: &HashSet<String>) -> String {
    let stripped: String = name
        .nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect::<String>()
        .to_lowercase();
    let mut base = String::new();
    let mut dash = false;
    for c in stripped.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
            dash = false;
        } else if !dash {
            base.push('-');
            dash = true;
        }
    }
    let mut base: String = base.trim_matches('-').chars().take(28).collect();
    if base.is_empty() {
        base = "projet".into();
    }
    let mut id = base.clone();
    let mut i = 2;
    while taken.contains(&id) {
        id = format!("{base}-{i}");
        i += 1;
    }
    id
}

fn taken_ids(config: &Value) -> HashSet<String> {
    config["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str().map(str::to_string))
        .collect()
}

/// Ajoute un projet ; identifiant tiré du nom. Rend la configuration et l'identifiant.
pub fn create_project(config: &Value, input: &Value) -> Result<(Value, String), LocalError> {
    check(input.is_object(), "config.invalidProject", params![])?;
    let mut next = config.clone();
    let taken = taken_ids(&next);
    let id = match input.get("id") {
        Some(v) if !v.is_null() => js_string(v),
        _ => slugify(
            &input
                .get("name")
                .filter(|v| !v.is_null())
                .map(js_string)
                .unwrap_or_default(),
            &taken,
        ),
    };
    check(!taken.contains(&id), "config.idTaken", params![id = id.as_str()])?;
    let mut project = Map::new();
    project.insert("id".into(), Value::String(id.clone()));
    let mut fields = input.clone();
    fields.as_object_mut().unwrap().shift_remove("id");
    apply_fields(&mut project, &fields)?;
    if !project.contains_key("type") {
        project.entry("args").or_insert_with(|| json!([]));
        project.entry("hostControl").or_insert_with(|| json!("env"));
    }
    if truthy(project.get("expose")) {
        let local = project.get("hostControl").and_then(Value::as_str) == Some("local");
        check(
            !local,
            "config.localOnly",
            params![name = field_text(&Value::Object(project.clone()), "name")],
        )?;
    }
    projects_mut(&mut next).push(Value::Object(project));
    validate(&next)?;
    Ok((next, id))
}

/// Copie d'un projet, placée juste après lui, sous un nouveau nom et un nouveau port (sauf page
/// locale). Ni lancement au démarrage, ni exposition au réseau, ni port fixe : c'étaient des
/// choix pour l'original.
pub fn duplicate_project(
    config: &Value,
    id: &str,
    name: &str,
    port: Option<u16>,
) -> Result<(Value, String), LocalError> {
    let index = position(config, id).ok_or_else(unknown_project)?;
    let mut next = config.clone();
    let mut copy = config["projects"][index]
        .as_object()
        .cloned()
        .ok_or_else(unknown_project)?;
    for k in ["autostart", "expose", "fixedPort"] {
        copy.shift_remove(k);
    }
    let new_id = slugify(id, &taken_ids(&next));
    copy.insert("id".into(), Value::String(new_id.clone()));
    copy.insert("name".into(), Value::String(name.into()));
    if copy.get("type").and_then(Value::as_str) != Some("file") {
        match port {
            Some(port) => {
                copy.insert("port".into(), json!(port));
            }
            None => {
                copy.shift_remove("port");
            }
        }
    }
    projects_mut(&mut next).insert(index + 1, Value::Object(copy));
    validate(&next)?;
    Ok((next, new_id))
}

/// Retire un projet, et des dépendances des autres (erreur 404 s'il n'existe pas).
pub fn remove_project(config: &Value, id: &str) -> Result<Value, LocalError> {
    let mut next = config.clone();
    let index = position(&next, id).ok_or_else(unknown_project)?;
    let projects = projects_mut(&mut next);
    projects.remove(index);
    for p in projects.iter_mut() {
        let Some(map) = p.as_object_mut() else { continue };
        let Some(Value::Array(requires)) = map.get_mut("requires") else {
            continue;
        };
        if !requires.iter().any(|r| r == id) {
            continue;
        }
        requires.retain(|r| r != id);
        if requires.is_empty() {
            map.shift_remove("requires");
        }
    }
    validate(&next)?;
    Ok(next)
}

/// Nouvel ordre : tous les identifiants, chacun une fois (erreur 400 sinon).
pub fn reorder_projects(config: &Value, order: &Value) -> Result<Value, LocalError> {
    let ids: Vec<&str> = config["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p["id"].as_str())
        .collect();
    let ok = match order.as_array() {
        Some(list) => {
            let unique: HashSet<String> = list.iter().map(js_json).collect();
            list.len() == ids.len()
                && unique.len() == ids.len()
                && list.iter().all(|x| x.as_str().is_some_and(|s| ids.contains(&s)))
        }
        None => false,
    };
    check(ok, "config.badOrder", params![])?;
    let mut next = config.clone();
    let reordered: Vec<Value> = order
        .as_array()
        .unwrap()
        .iter()
        .map(|id| {
            config["projects"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == *id)
                .cloned()
                .unwrap()
        })
        .collect();
    next["projects"] = Value::Array(reordered);
    Ok(next)
}

/// Différences lisibles entre deux versions d'un projet, pour le journal d'audit :
/// « port 3200 → 3300 · name "A" → "B" ». Les valeurs de `env` ne sont jamais écrites (elles
/// peuvent contenir des secrets) : seulement les noms des variables.
pub fn describe_changes(before: &Value, after: &Value) -> String {
    let show = |key: &str, value: Option<&Value>| -> String {
        let Some(value) = value else { return "—".into() };
        if key == "env" {
            let names: Vec<String> = value
                .as_object()
                .map_or_else(Vec::new, |m| m.keys().map(|k| format!("{k}=•••")).collect());
            return if names.is_empty() {
                "—".into()
            } else {
                names.join(", ")
            };
        }
        let text = js_json(value);
        if js_len(&text) <= 60 {
            return text;
        }
        // Coupé à 58 unités UTF-16, comme `slice(0, 58)`.
        let mut cut = String::new();
        let mut units = 0;
        for c in text.chars() {
            units += c.len_utf16();
            if units > 58 {
                break;
            }
            cut.push(c);
        }
        format!("{cut}…{}", if text.ends_with('"') { "\"" } else { "" })
    };
    let empty = Map::new();
    let b = before.as_object().unwrap_or(&empty);
    let a = after.as_object().unwrap_or(&empty);
    let mut keys: Vec<&String> = b.keys().collect();
    keys.extend(a.keys().filter(|k| !b.contains_key(*k)));
    keys.into_iter()
        .filter(|k| b.get(*k).map(js_json) != a.get(*k).map(js_json))
        .map(|k| format!("{k} {} → {}", show(k, b.get(k)), show(k, a.get(k))))
        .collect::<Vec<_>>()
        .join(" · ")
}

// ---- Valeurs dérivées ----

/// Adresse d'écoute demandée au projet.
pub fn host_of(p: &Value) -> &'static str {
    if truthy(p.get("expose")) && p.get("hostControl").and_then(Value::as_str) != Some("local") {
        "0.0.0.0"
    } else {
        "127.0.0.1"
    }
}

/// Remplace `{port}`, `{host}` et `{servym}` (dossier de Servym, barres obliques).
pub fn expand(text: &str, p: &Value, servym_dir: &str) -> String {
    text.replace("{port}", &field_text(p, "port"))
        .replace("{host}", host_of(p))
        .replace("{servym}", &servym_dir.replace('\\', "/"))
}

/// Adresse du projet, s'il en a une.
pub fn url_of(p: &Value, servym_dir: &str) -> Option<String> {
    p.get("url")
        .and_then(Value::as_str)
        .filter(|u| !u.is_empty())
        .map(|u| expand(u, p, servym_dir))
}
