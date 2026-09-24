//! Langues des messages du serveur : erreurs, états, journaux.
//!
//! Les textes sont dans `locales/<langue>.json`, sous des clés (« config.duplicatePort ») ;
//! `{nom}` y est remplacé par le paramètre du même nom. Contrairement à la version Node, la langue
//! du serveur n'est pas globale : chaque application a la sienne (tests en parallèle).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Langue d'un texte.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    En,
    Fr,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Fr];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
        }
    }

    /// Langue d'un code (« fr-FR », « en ») ; `None` si elle n'est pas prise en charge.
    pub fn pick(value: &str) -> Option<Lang> {
        let value = value.trim().to_ascii_lowercase();
        Lang::ALL.into_iter().find(|l| value.starts_with(l.code()))
    }
}

/// Valeur d'un paramètre : un texte, ou un autre message traduit dans la même langue.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Text(String),
    Msg(Msg),
}

/// Message à traduire plus tard (paramètre d'un autre message, état d'un projet).
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub key: String,
    pub params: Params,
}

/// Paramètres d'un message, dans l'ordre.
pub type Params = Vec<(String, Arg)>;

pub fn msg(key: &str, params: Params) -> Msg {
    Msg {
        key: key.into(),
        params,
    }
}

macro_rules! arg_from_display {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(value: $t) -> Self {
                Arg::Text(value.to_string())
            }
        }
    )*};
}
arg_from_display!(&str, String, &String, u16, u32, u64, i32, i64, usize, f64);

impl From<Msg> for Arg {
    fn from(value: Msg) -> Self {
        Arg::Msg(value)
    }
}

/// `params![port = 3001, first = msg("config.instance", params![])]`
#[macro_export]
macro_rules! params {
    () => { ::std::vec::Vec::<(::std::string::String, $crate::i18n::Arg)>::new() };
    ($($name:ident = $value:expr),+ $(,)?) => {
        vec![$((stringify!($name).to_string(), $crate::i18n::Arg::from($value))),+]
    };
}

/// Texte d'une clé dans une langue, paramètres remplacés. Clé inconnue : la clé elle-même.
pub fn tr(key: &str, params: &Params, lang: Lang) -> String {
    let text = catalog(lang)
        .get(key)
        .or_else(|| catalog(Lang::En).get(key))
        .and_then(|v| v.as_str())
        .unwrap_or(key);
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        let closed = !name.is_empty() && after[name_len..].starts_with('}');
        match params.iter().find(|(n, _)| n == name) {
            Some((_, value)) if closed => {
                match value {
                    Arg::Text(t) => out.push_str(t),
                    Arg::Msg(m) => out.push_str(&tr(&m.key, &m.params, lang)),
                }
                rest = &after[name_len + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tous les textes d'une langue (clé → texte), pour les tests de parité.
pub fn catalog(lang: Lang) -> &'static serde_json::Map<String, serde_json::Value> {
    use std::sync::OnceLock;
    static EN: OnceLock<serde_json::Map<String, serde_json::Value>> = OnceLock::new();
    static FR: OnceLock<serde_json::Map<String, serde_json::Value>> = OnceLock::new();
    let (cell, text) = match lang {
        Lang::En => (&EN, include_str!("../../../locales/en.json")),
        Lang::Fr => (&FR, include_str!("../../../locales/fr.json")),
    };
    cell.get_or_init(|| serde_json::from_str(text).expect("textes du serveur"))
}

/// Langue du serveur : `SERVYNEX_LANG`, sinon celle du système, sinon l'anglais.
pub fn detect_lang(system_locale: Option<&str>, env: Option<&str>) -> Lang {
    env.and_then(Lang::pick)
        .or_else(|| system_locale.and_then(Lang::pick))
        .unwrap_or(Lang::En)
}

/// Langue d'une requête : la première prise en charge de `Accept-Language`, sinon celle du serveur.
pub fn lang_of(accept_language: Option<&str>, server: Lang) -> Lang {
    accept_language
        .unwrap_or_default()
        .split(',')
        .find_map(Lang::pick)
        .unwrap_or(server)
}

/// Erreur au message traduisible, avec le statut HTTP à renvoyer.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalError {
    pub status: u16,
    pub key: String,
    pub params: Params,
    /// Champs ajoutés à la réponse JSON (`{ "auth": "login" }`…).
    pub extra: Option<Box<serde_json::Value>>,
}

impl LocalError {
    pub fn new(status: u16, key: &str, params: Params) -> Self {
        LocalError {
            status,
            key: key.into(),
            params,
            extra: None,
        }
    }

    /// Message dans une langue.
    pub fn message_in(&self, lang: Lang) -> String {
        tr(&self.key, &self.params, lang)
    }
}

impl fmt::Display for LocalError {
    /// Clé et paramètres : le texte dépend de la langue du lecteur (voir `message_in`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.key)
    }
}

impl std::error::Error for LocalError {}
