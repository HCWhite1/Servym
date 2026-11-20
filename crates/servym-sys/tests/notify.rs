//! Partie « notificateur » de tests/notify.test.js. Sous Windows, la version Rust affiche la
//! notification elle-même (plus de PowerShell) : les deux tests propres à notify.ps1 deviennent
//! des tests du contenu XML de la notification.

use std::sync::{Arc, Mutex};

use servym_sys::BoxFuture;
use servym_sys::notify::{Notification, Os, notifier_for, toast_xml};

const TITLE: &str = "Servym · « Mon API »";
const BODY: &str = "Error: $(Remove-Item x) ' \" <b>&amp;";

fn notification(source: Option<&str>) -> Notification {
    Notification {
        title: TITLE.into(),
        body: BODY.into(),
        source: source.map(str::to_string),
    }
}

type Calls = Arc<Mutex<Vec<(String, Vec<String>)>>>;

fn recorder() -> (Calls, servym_sys::linux::Runner) {
    let calls: Calls = Arc::default();
    let c = calls.clone();
    let run = Arc::new(
        move |cmd: String, args: Vec<String>| -> BoxFuture<'static, Result<String, String>> {
            c.lock().unwrap().push((cmd, args));
            Box::pin(async { Ok(String::new()) })
        },
    );
    (calls, run)
}

/// JS : notify.test.js › notificateur du système › Windows : PowerShell caché, les textes par variables d’environnement
/// (remplacé : notification native ; les textes ne passent par aucune ligne de commande ni aucun script.)
#[test]
fn windows_textes_echappes() {
    let xml = toast_xml(&notification(Some("Serveur maison")));
    assert!(xml.contains("Servym · « Mon API »"));
    assert!(
        xml.contains("&lt;b&gt;&amp;amp;"),
        "balises et entités du texte échappées : {xml}"
    );
    assert!(!xml.contains("<b>"), "{xml}");
    assert!(xml.contains("Serveur maison"), "émetteur affiché : {xml}");
}

/// JS : notify.test.js › notificateur du système › Linux : notify-send, sans shell
#[tokio::test]
async fn linux_notify_send_sans_shell() {
    let (calls, run) = recorder();
    let send = notifier_for(Os::Linux, run);
    assert_eq!(send.notify(notification(None)).await, Ok(true));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, "notify-send");
    assert_eq!(calls[0].1, ["--app-name=Servym", "--", TITLE, BODY]);
}

/// JS : notify.test.js › notificateur du système › ailleurs : rien n’est envoyé
#[tokio::test]
async fn ailleurs_rien() {
    let (calls, run) = recorder();
    let send = notifier_for(Os::Other, run);
    assert_eq!(send.notify(notification(None)).await, Ok(false));
    assert!(calls.lock().unwrap().is_empty());
}

/// JS : notify.test.js › notificateur du système › notify.ps1 : textes insérés comme nœuds XML, script valide
/// (remplacé : le XML produit est bien formé, quels que soient les textes.)
#[test]
fn xml_bien_forme() {
    let tricky = Notification {
        title: "]]><x a=\"".into(),
        body: "</text></binding>".into(),
        source: Some("<s>".into()),
    };
    let xml = toast_xml(&tricky);
    assert!(xml.starts_with("<toast"), "{xml}");
    assert!(xml.trim_end().ends_with("</toast>"), "{xml}");
    assert_eq!(
        xml.matches("<text").count(),
        xml.matches("</text>").count(),
        "{xml}"
    );
    assert!(!xml.contains("<x "), "{xml}");
    assert!(!xml.contains("<s>"), "{xml}");
}
