//! Faux notificateur et faux démarrage avec la session, pour les tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use servym_sys::notify::{Notification, Notifier};
use servym_sys::{Autostart, BoxFuture};

/// Démarrage avec la session en mémoire.
#[derive(Default)]
pub struct FakeAutostart {
    pub on: AtomicBool,
}

impl Autostart for FakeAutostart {
    fn enabled(&self) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.on.load(Ordering::SeqCst) })
    }
    fn set(&self, on: bool) -> BoxFuture<'_, Result<(), String>> {
        self.on.store(on, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    }
}

/// Réponse du faux notificateur.
#[derive(Clone, Debug)]
pub enum Reply {
    Sent,
    Unsupported,
    Fail(String),
}

/// Notificateur qui garde ce qu'on lui envoie ; sa réponse est modifiable en cours de test.
pub struct FakeNotifier {
    pub sent: Mutex<Vec<Notification>>,
    pub reply: Mutex<Reply>,
}

impl FakeNotifier {
    pub fn new(reply: Reply) -> Arc<Self> {
        Arc::new(FakeNotifier {
            sent: Mutex::new(Vec::new()),
            reply: Mutex::new(reply),
        })
    }
    pub fn sent(&self) -> Vec<Notification> {
        self.sent.lock().unwrap().clone()
    }
}

impl Notifier for FakeNotifier {
    fn notify(&self, notification: Notification) -> BoxFuture<'_, Result<bool, String>> {
        self.sent.lock().unwrap().push(notification);
        let reply = self.reply.lock().unwrap().clone();
        Box::pin(async move {
            match reply {
                Reply::Sent => Ok(true),
                Reply::Unsupported => Ok(false),
                Reply::Fail(e) => Err(e),
            }
        })
    }
}
