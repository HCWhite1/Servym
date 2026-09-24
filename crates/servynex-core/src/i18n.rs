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
    let _ = (key, params, lang);
    unimplemented!("phase 2")
}

/// Tous les textes d'une langue (clé → texte), pour les tests de parité.
pub fn catalog(lang: Lang) -> &'static serde_json::Map<String, serde_json::Value> {
    let _ = lang;
    unimplemented!("phase 2")
}

/// Langue du serveur : `SERVYNEX_LANG`, sinon celle du système, sinon l'anglais.
pub fn detect_lang(system_locale: Option<&str>, env: Option<&str>) -> Lang {
    let _ = (system_locale, env);
    unimplemented!("phase 2")
}

/// Langue d'une requête : la première prise en charge de `Accept-Language`, sinon celle du serveur.
pub fn lang_of(accept_language: Option<&str>, server: Lang) -> Lang {
    let _ = (accept_language, server);
    unimplemented!("phase 2")
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
