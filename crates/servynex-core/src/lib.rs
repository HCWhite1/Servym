//! Domaine de Servynex, sans HTTP : configuration, gestionnaire de processus, comptes,
//! journaux, processeur et mémoire, textes traduits.

pub mod auth;
pub mod config;
pub mod i18n;
pub mod logs;
pub mod manager;
pub mod static_files;
pub mod stats;

pub use servynex_sys::BoxFuture;
