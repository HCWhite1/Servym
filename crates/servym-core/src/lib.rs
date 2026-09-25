//! Domaine de Servym, sans HTTP : configuration, gestionnaire de processus, comptes,
//! journaux, processeur et mémoire, textes traduits.

pub mod auth;
pub mod config;
pub mod health;
pub mod i18n;
pub mod logs;
pub mod manager;
pub mod static_files;
pub mod static_site;
pub mod stats;

pub use servym_sys::BoxFuture;
