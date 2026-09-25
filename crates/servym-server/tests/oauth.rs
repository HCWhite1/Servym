//! tests/oauth.test.js : connexion avec GitHub, contre un faux GitHub local.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use common::Servym;
use serde_json::{Value, json};
use servym_fixtures::{Req, Res, assert_match};
use servym_server::ProviderUrls;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Profils du faux GitHub, par code d'autorisation.
fn profile(code: &str) -> Option<Value> {
    match code {
        "alice-code" => Some(json!({ "id": 101, "login": "alice-gh", "name": "Alice" })),
        "stranger" => Some(json!({ "id": 999, "login": "stranger" })),
        _ => None,
    }
}

/// Faux GitHub : échange du code contre un jeton, puis profil du compte. Rend son adresse.
async fn fake_github() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                // En-têtes, puis le corps annoncé par Content-Length.
                let (head, body) = loop {
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..end]).to_string();
                        let length = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        while raw.len() < end + 4 + length {
                            let n = stream.read(&mut buf).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            raw.extend_from_slice(&buf[..n]);
                        }
                        break (head, String::from_utf8_lossy(&raw[end + 4..]).to_string());
                    }
                };
                let first = head.lines().next().unwrap_or_default().to_string();
                let header = |name: &str| {
                    head.lines().find_map(|l| {
                        let (n, v) = l.split_once(':')?;
                        n.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
                    })
                };
                let (status, reply) = if first.starts_with("POST /login/oauth/access_token ") {
                    // Format standard OAuth : formulaire encodé.
                    let form: std::collections::HashMap<String, String> = body
                        .split('&')
                        .filter_map(|pair| pair.split_once('='))
                        .map(|(k, v)| (k.to_string(), v.replace('+', " ")))
                        .collect();
                    let code = form.get("code").cloned().unwrap_or_default();
                    if form.get("client_id").map(String::as_str) != Some("cid")
                        || form.get("client_secret").map(String::as_str) != Some("secret-123")
                        || profile(&code).is_none()
                    {
                        (200, json!({ "error": "bad_verification_code" }))
                    } else {
                        (
                            200,
                            json!({ "access_token": format!("tok-{code}"), "token_type": "bearer" }),
                        )
                    }
                } else if first.starts_with("GET /user ") {
                    let code = header("authorization")
                        .unwrap_or_default()
                        .trim_start_matches("Bearer tok-")
                        .to_string();
                    match profile(&code) {
                        Some(p) if header("user-agent").is_some() => (200, p),
                        _ => (401, json!({})),
                    }
                } else {
                    (404, json!({}))
                };
                let body = reply.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    base
}

struct OAuth {
    s: Servym,
    github: String,
    now: Arc<AtomicI64>,
    admin: String,
}

/// Démarrage d'une connexion (ou d'une liaison).
struct Started {
    url: url_lite::Url,
    state: String,
    cookie: Option<String>,
    set_cookie: String,
}

/// Petite analyse d'adresse, suffisante pour les paramètres de la redirection.
mod url_lite {
    pub struct Url {
        pub without_query: String,
        pub query: Vec<(String, String)>,
    }
    impl Url {
        pub fn parse(text: &str) -> Url {
            let (base, query) = text.split_once('?').unwrap_or((text, ""));
            let query = query
                .split('&')
                .filter_map(|p| p.split_once('='))
                .map(|(k, v)| (k.to_string(), decode(v)))
                .collect();
            Url {
                without_query: base.to_string(),
                query,
            }
        }
        pub fn param(&self, name: &str) -> Option<&str> {
            self.query
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        }
    }
    fn decode(v: &str) -> String {
        let bytes = v.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && i + 2 < bytes.len()
                && let Ok(b) = u8::from_str_radix(&v[i + 1..i + 3], 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

impl OAuth {
    async fn new() -> OAuth {
        let github = fake_github().await;
        let now = Arc::new(AtomicI64::new(1_800_000_000_000));
        let clock = now.clone();
        let urls = ProviderUrls {
            authorize: format!("{github}/login/oauth/authorize"),
            token: format!("{github}/login/oauth/access_token"),
            user: format!("{github}/user"),
        };
        let s = Servym::start(
            "servym-oauth-",
            json!({ "instance": { "port": 1 }, "projects": [] }),
            |b| {
                b.clock(Arc::new(move || clock.load(Ordering::SeqCst)))
                    .github(urls)
            },
        )
        .await;
        let admin = s.setup_admin().await;
        OAuth {
            s,
            github,
            now,
            admin,
        }
    }

    /// Administrateur, avec GitHub configuré.
    async fn configured() -> OAuth {
        let o = Self::new().await;
        let res =
            o.s.call_json(
                "PUT",
                "/api/auth/providers/github",
                json!({ "clientId": "cid", "clientSecret": "secret-123" }),
                Some(&o.admin),
            )
            .await;
        assert_eq!(res.status, 200);
        o
    }

    async fn start(&self, cookie: Option<&str>, link: bool) -> Started {
        let path = if link {
            "/api/auth/github/start?link=1"
        } else {
            "/api/auth/github/start"
        };
        let res = self.s.send(Req::get(path).cookie(cookie)).await;
        assert_eq!(res.status, 302, "démarrage : {}", res.body);
        let url = url_lite::Url::parse(res.header("location").unwrap());
        Started {
            state: url.param("state").unwrap_or_default().to_string(),
            url,
            cookie: res.cookie("servym_oauth"),
            set_cookie: res.set_cookie("servym_oauth").unwrap_or_default().to_string(),
        }
    }

    async fn back(&self, query: &str, cookie: Option<&str>) -> Res {
        self.s
            .send(Req::get(&format!("/auth/github/callback?{query}")).cookie(cookie))
            .await
    }
}

/// Page de retour : Servym redirige par une balise meta.
fn target(res: &Res) -> String {
    regex::Regex::new(r#"url=([^"]+)""#)
        .unwrap()
        .captures(&res.body)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// JS : oauth.test.js › connexion avec GitHub › absente tant qu’un administrateur ne l’a pas configurée
#[tokio::test]
async fn absente_tant_que_non_configuree() {
    let o = OAuth::new().await;
    assert_eq!(
        o.s.send(Req::get("/api/auth/status")).await.json()["methods"]["github"],
        false
    );
    assert_eq!(o.s.send(Req::get("/api/auth/github/start")).await.status, 409);
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › configuration par un administrateur ; le secret n’est jamais renvoyé
#[tokio::test]
async fn configuration_par_un_administrateur() {
    let o = OAuth::new().await;
    let body = json!({ "clientId": "cid", "clientSecret": "secret-123" });
    let put =
        |body: Value, cookie: Option<&str>| o.s.call_json("PUT", "/api/auth/providers/github", body, cookie);
    assert_eq!(put(body.clone(), None).await.status, 401, "sans session");
    assert_eq!(
        put(json!({ "clientId": "cid" }), Some(&o.admin)).await.status,
        400,
        "secret obligatoire la première fois"
    );
    assert_eq!(put(body, Some(&o.admin)).await.status, 200);
    let res = o.s.call_as("GET", "/api/auth/providers", Some(&o.admin)).await;
    assert_eq!(res.json()["github"]["configured"], true);
    assert_eq!(res.json()["github"]["clientId"], "cid");
    assert!(!res.body.contains("secret-123"), "secret absent de la réponse");
    let status = o.s.send(Req::get("/api/auth/status")).await;
    assert!(!status.body.contains("secret-123"));
    assert_eq!(status.json()["methods"]["github"], true);
    assert_eq!(
        put(json!({ "clientId": "cid" }), Some(&o.admin)).await.status,
        200,
        "secret gardé si on ne change que l’identifiant"
    );
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › lier son compte : état à usage unique, lié au navigateur
#[tokio::test]
async fn lier_son_compte() {
    let o = OAuth::configured().await;
    let s = o.start(Some(&o.admin), true).await;
    assert_eq!(s.url.without_query, format!("{}/login/oauth/authorize", o.github));
    assert_eq!(s.url.param("client_id"), Some("cid"));
    assert_eq!(
        s.url.param("redirect_uri").map(str::to_string),
        Some(format!("http://localhost:{}/auth/github/callback", o.s.port))
    );
    assert_match!(s.state, r"^[\w-]{32,}$");
    assert_match!(s.set_cookie, "HttpOnly; SameSite=Lax");
    let q = |state: &str| format!("code=alice-code&state={state}");
    assert_eq!(
        target(&o.back(&q(&s.state), None).await),
        "/?auth_error=state",
        "sans le cookie du navigateur"
    );
    assert_eq!(
        target(&o.back(&q("autre"), s.cookie.as_deref()).await),
        "/?auth_error=state",
        "état différent"
    );
    let ok = o.back(&q(&s.state), s.cookie.as_deref()).await;
    assert_eq!(ok.status, 200);
    assert_eq!(target(&ok), "/?auth_done=linked");
    assert_eq!(
        target(&o.back(&q(&s.state), s.cookie.as_deref()).await),
        "/?auth_error=state",
        "usage unique"
    );
    let status =
        o.s.call_as("GET", "/api/auth/status", Some(&o.admin))
            .await
            .json();
    assert_eq!(
        status["user"]["identities"],
        json!([{ "provider": "github", "login": "alice-gh" }])
    );
    o.s.close(&[]).await;
}

async fn link_alice(o: &OAuth) {
    let s = o.start(Some(&o.admin), true).await;
    let res = o
        .back(&format!("code=alice-code&state={}", s.state), s.cookie.as_deref())
        .await;
    assert_eq!(target(&res), "/?auth_done=linked");
}

/// JS : oauth.test.js › connexion avec GitHub › se connecter avec GitHub : seulement un compte lié
#[tokio::test]
async fn se_connecter_compte_lie_seulement() {
    let o = OAuth::configured().await;
    link_alice(&o).await;
    let s = o.start(None, false).await;
    let res = o
        .back(&format!("code=stranger&state={}", s.state), s.cookie.as_deref())
        .await;
    assert_eq!(target(&res), "/?auth_error=unlinked");
    assert_eq!(res.cookie("servym_session"), None, "aucune session");
    let s = o.start(None, false).await;
    let res = o
        .back(&format!("code=alice-code&state={}", s.state), s.cookie.as_deref())
        .await;
    assert_eq!(target(&res), "/");
    let session = res.cookie("servym_session").expect("session ouverte");
    assert_eq!(
        o.s.call_as("GET", "/api/projects", Some(&session)).await.status,
        200
    );
    let audit = Servym::audit_texts(&o.s.audit(Some(&o.admin)).await);
    assert!(
        audit
            .iter()
            .any(|t| t.contains("GitHub") && t.contains("Connexion")),
        "connexion inscrite au journal"
    );
    assert!(
        audit
            .iter()
            .any(|t| t.contains("GitHub") && t.contains("stranger")),
        "échec inscrit au journal"
    );
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › état expiré après 10 minutes ; échec chez GitHub
#[tokio::test]
async fn etat_expire_et_echec_chez_github() {
    let o = OAuth::configured().await;
    link_alice(&o).await;
    let s = o.start(None, false).await;
    o.now.fetch_add(11 * 60_000, Ordering::SeqCst);
    assert_eq!(
        target(
            &o.back(&format!("code=alice-code&state={}", s.state), s.cookie.as_deref())
                .await
        ),
        "/?auth_error=state"
    );
    let s = o.start(None, false).await;
    assert_eq!(
        target(
            &o.back(&format!("code=inconnu&state={}", s.state), s.cookie.as_deref())
                .await
        ),
        "/?auth_error=failed"
    );
    let s = o.start(None, false).await;
    assert_eq!(
        target(
            &o.back(
                &format!("error=access_denied&state={}", s.state),
                s.cookie.as_deref()
            )
            .await
        ),
        "/?auth_error=denied",
        "refusé sur GitHub"
    );
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › page de retour : rien de la requête n’y est recopié
#[tokio::test]
async fn page_de_retour_sans_recopie() {
    let o = OAuth::configured().await;
    let s = o.start(None, false).await;
    let res = o
        .back(
            "code=%3Cscript%3E&state=%22%3E%3Cscript%3Ealert(1)%3C%2Fscript%3E",
            s.cookie.as_deref(),
        )
        .await;
    assert!(!res.body.contains("<script"), "aucun script");
    assert!(!res.body.contains("alert(1)"));
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › un compte GitHub ne se lie qu’à un seul compte Servym ; délier
#[tokio::test]
async fn un_seul_compte_servym_et_delier() {
    let o = OAuth::configured().await;
    link_alice(&o).await;
    let bob = o.s.join(&o.admin, &["operate"], "bob", "mot de passe 2").await;
    let s = o.start(Some(&bob), true).await;
    assert_eq!(
        target(
            &o.back(&format!("code=alice-code&state={}", s.state), s.cookie.as_deref())
                .await
        ),
        "/?auth_error=taken"
    );
    assert_eq!(
        o.s.call_as("DELETE", "/api/auth/me/identities/github", Some(&o.admin))
            .await
            .status,
        200
    );
    assert_eq!(
        o.s.call_as("GET", "/api/auth/status", Some(&o.admin))
            .await
            .json()["user"]["identities"],
        json!([])
    );
    let again = o.start(None, false).await;
    assert_eq!(
        target(
            &o.back(
                &format!("code=alice-code&state={}", again.state),
                again.cookie.as_deref()
            )
            .await
        ),
        "/?auth_error=unlinked"
    );
    o.s.close(&[]).await;
}

/// JS : oauth.test.js › connexion avec GitHub › retirer la configuration : bouton et démarrage disparaissent
#[tokio::test]
async fn retirer_la_configuration() {
    let o = OAuth::configured().await;
    assert_eq!(
        o.s.call_as("DELETE", "/api/auth/providers/github", Some(&o.admin))
            .await
            .status,
        200
    );
    assert_eq!(
        o.s.send(Req::get("/api/auth/status")).await.json()["methods"]["github"],
        false
    );
    assert_eq!(o.s.send(Req::get("/api/auth/github/start")).await.status, 409);
    o.s.close(&[]).await;
}
