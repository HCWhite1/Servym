//! Sondes : un port répond-il, et est-ce le bon site ?

use std::time::Duration;

/// Résultat d'une sonde HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Ok,
    Down,
    /// Le port répond, mais pas le bon site (texte attendu absent).
    Mismatch,
}

async fn tcp_open(host: &str, port: u16) -> bool {
    let Ok(ip) = host.parse::<std::net::IpAddr>() else {
        return false;
    };
    servym_sys::net::tcp_open((ip, port).into(), Duration::from_millis(800)).await
}

/// Quelqu'un écoute-t-il sur ce port ? Un serveur sur `::` accepte aussi 127.0.0.1 ; `::1` couvre
/// ceux qui n'écoutent qu'en IPv6.
pub async fn port_in_use(port: u16) -> bool {
    let (v4, v6) = tokio::join!(tcp_open("127.0.0.1", port), tcp_open("::1", port));
    v4 || v6
}

/// Client des sondes : pas de proxy du système, certificat local (symfony serve) non vérifié : on
/// ne teste que la présence du site.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(1500))
        .build()
        .expect("client HTTP des sondes")
}

/// Sonde HTTP : `Ok`, `Down` ou `Mismatch`. Une erreur 5xx compte comme `Ok` : le serveur tourne,
/// c'est l'application qui échoue. Le corps est lu jusqu'au texte attendu (64 Ko au plus).
pub async fn http_health(client: &reqwest::Client, port: u16, path: &str, expect: &str, tls: bool) -> Health {
    let scheme = if tls { "https" } else { "http" };
    // Port fermé : réponse immédiate, sans attendre le client HTTP (voir servym_sys::net).
    let (v4, v6) = tokio::join!(tcp_open("127.0.0.1", port), tcp_open("::1", port));
    let host = if v4 {
        "127.0.0.1"
    } else if v6 {
        "[::1]"
    } else {
        return Health::Down;
    };
    let url = format!("{scheme}://{host}:{port}{path}");
    let Ok(mut res) = client
        .get(&url)
        .header("host", format!("localhost:{port}"))
        .send()
        .await
    else {
        return Health::Down;
    };
    if expect.is_empty() || res.status().is_server_error() {
        return Health::Ok;
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        body.extend_from_slice(&chunk);
        if String::from_utf8_lossy(&body).contains(expect) {
            return Health::Ok;
        }
        if body.len() > 65_536 {
            return Health::Mismatch;
        }
    }
    if String::from_utf8_lossy(&body).contains(expect) {
        Health::Ok
    } else {
        Health::Mismatch
    }
}
