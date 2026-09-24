//! Notifications du système : Windows (notification native), Linux (`notify-send`, sans shell).

use serde::Serialize;

use crate::BoxFuture;
use crate::linux::Runner;

/// Une notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// Nom affiché comme émetteur (nom de l'instance).
    pub source: Option<String>,
}

/// Envoi d'une notification : `Ok(true)` envoyée, `Ok(false)` indisponible sur ce système.
pub trait Notifier: Send + Sync {
    fn notify(&self, notification: Notification) -> BoxFuture<'_, Result<bool, String>>;
}

/// Système visé par un notificateur.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Other,
}

/// Notificateur d'un système. Linux : `notify-send` par `run`, jamais par un shell.
pub fn notifier_for(os: Os, run: Runner) -> Box<dyn Notifier> {
    let _ = (os, run);
    unimplemented!("phase 3")
}

/// Contenu XML d'une notification Windows : les textes y sont des nœuds texte, jamais
/// concaténés dans le XML.
pub fn toast_xml(notification: &Notification) -> String {
    let _ = notification;
    unimplemented!("phase 3")
}
