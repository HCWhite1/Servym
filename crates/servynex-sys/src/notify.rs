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

impl Os {
    /// Le système de cette machine.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        }
    }
}

struct SystemNotifier {
    os: Os,
    run: Runner,
    /// Windows : application au nom de laquelle la notification s'affiche.
    #[cfg_attr(not(windows), allow(dead_code))]
    app_id: String,
}

impl Notifier for SystemNotifier {
    fn notify(&self, n: Notification) -> BoxFuture<'_, Result<bool, String>> {
        Box::pin(async move {
            match self.os {
                Os::Linux => {
                    (self.run)(
                        "notify-send".into(),
                        vec!["--app-name=Servynex".into(), n.title, n.body],
                    )
                    .await?;
                    Ok(true)
                }
                #[cfg(windows)]
                Os::Windows => {
                    let app_id = self.app_id.clone();
                    tokio::task::spawn_blocking(move || crate::windows::toast::show(&n, &app_id))
                        .await
                        .map_err(|e| e.to_string())?
                        .map(|()| true)
                }
                _ => Ok(false),
            }
        })
    }
}

/// Notificateur d'un système. Linux : `notify-send` par `run`, jamais par un shell.
/// Sous Windows, au nom de Windows PowerShell (connu de Windows sans installation).
pub fn notifier_for(os: Os, run: Runner) -> Box<dyn Notifier> {
    #[cfg(windows)]
    let app_id = crate::windows::toast::POWERSHELL_APP_ID.to_string();
    #[cfg(not(windows))]
    let app_id = String::new();
    Box::new(SystemNotifier { os, run, app_id })
}

/// Notificateur Windows au nom d'une application installée (identifiant déclaré par son raccourci
/// du menu Démarrer).
pub fn app_notifier(app_id: &str, run: Runner) -> Box<dyn Notifier> {
    Box::new(SystemNotifier {
        os: Os::current(),
        run,
        app_id: app_id.to_string(),
    })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Caractères interdits en XML 1.0 : retirés.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Contenu XML d'une notification Windows : les textes y sont échappés, jamais interprétés.
pub fn toast_xml(n: &Notification) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text><text placement=\"attribution\">{}</text></binding></visual></toast>",
        escape(&n.title),
        escape(&n.body),
        escape(n.source.as_deref().unwrap_or_default())
    )
}
