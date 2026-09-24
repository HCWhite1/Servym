//! Comptes : sessions, limite des essais, routes `/api/auth/*`, connexion par GitHub (OAuth 2,
//! « web application flow »). Portage de `src/auth.js` et `src/oauth.js` ; les données restent au
//! format de `runtime/auth.json`.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde_json::{Value, json};
use servynex_core::auth::{
    AuthStore, LOCK_CHOICES, PERMISSIONS, RESET_TTL, create_reset_code, hash_secret, human_code,
    normalize_code, normalize_permissions, random_token, role_of, sha256_hex, verify_secret,
};
use servynex_core::i18n::{LocalError, Params, msg, tr};
use servynex_core::params;
use subtle::ConstantTimeEq;

use crate::ProviderUrls;
use crate::app::AppInner;
use crate::http::{Ctx, err, send, with_extra};

const SESSION_SHORT: i64 = 12 * 3_600_000; // sans « rester connecté » : 12 h sans activité
const SESSION_LONG: i64 = 30 * 86_400_000;
const INVITE_TTL: i64 = 48 * 3_600_000;
const MAX_SESSIONS: usize = 20;
const COOKIE: &str = "servynex_session";
const OAUTH_COOKIE: &str = "servynex_oauth";
const OAUTH_TTL: i64 = 10 * 60_000; // pour revenir du fournisseur

/// Fournisseur de connexion. Un autre s'ajoute dans [`providers`].
#[derive(Clone, Debug)]
pub struct Provider {
    pub key: &'static str,
    pub name: &'static str,
    pub authorize_url: String,
    pub token_url: String,
    pub user_url: String,
    /// Portée demandée ; vide : profil public (identifiant et nom d'utilisateur, rien d'autre).
    pub scope: &'static str,
    /// Page où l'administrateur crée l'application (Paramètres › Sécurité).
    pub register_url: &'static str,
}

/// Fournisseurs, avec leurs adresses éventuellement remplacées (tests : faux GitHub local).
pub fn providers(github: Option<&ProviderUrls>) -> Vec<Provider> {
    let mut gh = Provider {
        key: "github",
        name: "GitHub",
        authorize_url: "https://github.com/login/oauth/authorize".into(),
        token_url: "https://github.com/login/oauth/access_token".into(),
        user_url: "https://api.github.com/user".into(),
        scope: "",
        register_url: "https://github.com/settings/applications/new",
    };
    if let Some(urls) = github {
        gh.authorize_url = urls.authorize.clone();
        gh.token_url = urls.token.clone();
        gh.user_url = urls.user.clone();
    }
    vec![gh]
}

struct OAuthState {
    provider: &'static str,
    user_id: Option<String>,
    redirect_uri: String,
    expires_at: i64,
}

/// État de l'authentification (fichier des comptes et mémoire du processus).
pub struct AuthCore {
    pub store: AuthStore,
    /// Essais ratés : clé → (nombre, bloqué jusqu'à).
    failures: HashMap<String, (u32, i64)>,
    /// Session → dernière écriture de `lastSeen`.
    seen_saved: HashMap<String, i64>,
    /// États OAuth en attente, dans l'ordre de création.
    states: Vec<(String, OAuthState)>,
    http: reqwest::Client,
}

impl AuthCore {
    pub fn new(store: AuthStore) -> AuthCore {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("client HTTP");
        AuthCore {
            store,
            failures: HashMap::new(),
            seen_saved: HashMap::new(),
            states: Vec::new(),
            http,
        }
    }

    fn data(&mut self) -> &mut Value {
        self.store.load()
    }

    /// Protection active : au moins un compte et activée (ou fichier illisible : échec fermé).
    pub fn enabled(&mut self) -> bool {
        let d = self.data();
        let on = d["enabled"] == true && d["users"].as_array().is_some_and(|u| !u.is_empty());
        self.store.error().is_some() || on
    }

    fn expires_at(s: &Value) -> i64 {
        s["lastSeen"].as_i64().unwrap_or(0)
            + if s["remember"] == true {
                SESSION_LONG
            } else {
                SESSION_SHORT
            }
    }

    /// Session de la requête (identifiant, utilisateur), ou `None`. Met à jour la dernière
    /// activité (écrite au plus une fois par minute).
    pub fn session(&mut self, ctx: &Ctx, now: i64) -> Option<(String, Value)> {
        let token = cookie(ctx, COOKIE)?;
        let hash = sha256_hex(&token);
        let d = self.store.load();
        let index = d["sessions"]
            .as_array()?
            .iter()
            .position(|s| s["id"] == hash.as_str())?;
        let user_id = d["sessions"][index]["userId"].clone();
        let user = d["users"]
            .as_array()?
            .iter()
            .find(|u| u["id"] == user_id)?
            .clone();
        if Self::expires_at(&d["sessions"][index]) < now {
            return None;
        }
        d["sessions"][index]["lastSeen"] = json!(now);
        if now - self.seen_saved.get(&hash).copied().unwrap_or(0) > 60_000 {
            self.seen_saved.insert(hash.clone(), now);
            let _ = self.store.save();
        }
        Some((hash, user))
    }

    fn session_value(&mut self, id: &str) -> Option<&mut Value> {
        self.store.load()["sessions"]
            .as_array_mut()?
            .iter_mut()
            .find(|s| s["id"] == id)
    }

    pub fn is_locked(&mut self, session_id: &str, now: i64) -> bool {
        let lock_after = self.data()["lockAfter"].as_i64().unwrap_or(0);
        let Some(s) = self.session_value(session_id) else {
            return false;
        };
        let active = s
            .get("activeAt")
            .and_then(Value::as_i64)
            .or_else(|| s["createdAt"].as_i64())
            .unwrap_or(0);
        s["locked"] == true || (lock_after > 0 && now - active > lock_after * 1000 + 15_000)
    }

    fn user_mut(&mut self, id: &Value) -> Option<&mut Value> {
        self.store.load()["users"]
            .as_array_mut()?
            .iter_mut()
            .find(|u| u["id"] == *id)
    }

    fn configured(&mut self, key: &str) -> bool {
        let enabled = self.enabled();
        let c = &self.data()["providers"][key];
        enabled
            && c["clientId"].as_str().is_some_and(|s| !s.is_empty())
            && c["clientSecret"].as_str().is_some_and(|s| !s.is_empty())
    }

    // ---- Limite des essais ----

    fn check_rate(&self, keys: &[String], now: i64) -> Result<(), LocalError> {
        for key in keys {
            if let Some((_, until)) = self.failures.get(key)
                && *until > now
            {
                let s = (until - now + 999) / 1000;
                let wait = if s < 60 {
                    msg("time.seconds", params![n = s])
                } else {
                    msg("time.minutes", params![n = (s + 59) / 60])
                };
                return Err(LocalError::new(429, "auth.tooMany", params![wait = wait]));
            }
        }
        Ok(())
    }

    fn fail(&mut self, keys: &[(String, u32)], now: i64) {
        for (key, threshold) in keys {
            let f = self.failures.entry(key.clone()).or_insert((0, 0));
            f.0 += 1;
            if f.0 >= *threshold {
                let delay = 30_000i64.saturating_mul(1i64 << (f.0 - threshold).min(20));
                f.1 = now + delay.min(15 * 60_000);
            }
        }
    }
}

// ---- Petits outils ----

fn cookie(ctx: &Ctx, name: &str) -> Option<String> {
    for value in ctx.headers.get_all(header::COOKIE) {
        for part in value.to_str().unwrap_or_default().split(';') {
            if let Some((k, v)) = part.split_once('=')
                && k.trim() == name
            {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn describe_agent(ua: &str) -> String {
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("OPR/") {
        "Opera"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Navigateur"
    };
    let os = if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPhone") || ua.contains("iPad") {
        "iOS"
    } else if ua.contains("Mac OS") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    };
    if os.is_empty() {
        browser.to_string()
    } else {
        format!("{browser} · {os}")
    }
}

fn client_ip(ctx: &Ctx) -> String {
    ctx.peer.ip().to_string()
}

fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn check_password(password: &Value) -> Result<(), LocalError> {
    let Some(p) = password.as_str().filter(|p| p.encode_utf16().count() >= 8) else {
        return Err(err(400, "auth.passwordShort"));
    };
    if p.encode_utf16().count() > 200 {
        return Err(err(400, "auth.passwordLong"));
    }
    Ok(())
}

fn check_username(username: &str) -> Result<(), LocalError> {
    let ok = username.len() >= 2
        && username.len() <= 32
        && username
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && username
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(err(400, "auth.badUsername"))
    }
}

fn check_name(name: &Value) -> Result<(), LocalError> {
    match name {
        Value::Null => Ok(()),
        Value::String(s) if s.trim().encode_utf16().count() <= 60 => Ok(()),
        _ => Err(err(400, "auth.nameLong")),
    }
}

fn perms_of(user: &Value) -> Vec<String> {
    user["permissions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Nom du rôle dans un message (journal d'audit), traduit avec lui.
fn role_name(permissions: &[String]) -> servynex_core::i18n::Msg {
    msg(&format!("role.{}", role_of(permissions)), params![])
}

pub fn public_user(u: &Value) -> Value {
    let perms = perms_of(u);
    let identities: Vec<Value> = u["identities"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|i| json!({ "provider": i["provider"], "login": i["login"] }))
        .collect();
    let name = u["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .unwrap_or(u["username"].as_str().unwrap_or_default());
    json!({
        "id": u["id"], "username": u["username"], "name": name, "permissions": perms, "role": role_of(&perms),
        "prefs": u.get("prefs").cloned().unwrap_or(json!({})), "createdAt": u["createdAt"], "identities": identities,
    })
}

/// Vérification d'un secret hors du fil asynchrone (scrypt : quelques dizaines de ms).
async fn verify(secret: String, stored: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match stored {
        Some(stored) => verify_secret(&secret, &stored),
        None => {
            let _ = hash_secret("x"); // même durée qu'une vraie vérification
            false
        }
    })
    .await
    .unwrap_or(false)
}

async fn hash(secret: String) -> String {
    tokio::task::spawn_blocking(move || hash_secret(&secret))
        .await
        .expect("hachage")
}

/// Réponse avec des cookies en plus.
fn with_cookies(mut res: Response, cookies: &[String]) -> Response {
    for c in cookies {
        res.headers_mut()
            .append(header::SET_COOKIE, HeaderValue::from_str(c).unwrap());
    }
    res
}

impl AppInner {
    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn instance_name(&self) -> Value {
        self.config()["instance"]
            .get("name")
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn auth_audit(&self, who: Option<&str>, key: &str, params: Params) {
        self.audit("-", who.unwrap_or("-"), key, params);
    }

    /// Contrôle d'accès d'une route : l'utilisateur (`None` : protection désactivée).
    pub async fn require(&self, ctx: &Ctx, permission: &str) -> Result<Option<Value>, LocalError> {
        let now = self.now();
        let mut core = self.auth.lock().await;
        if !core.enabled() {
            return Ok(None);
        }
        if let Some(error) = core.store.error() {
            let e = LocalError::new(503, "auth.brokenFile", params![error = error.to_string()]);
            return Err(with_extra(e, json!({ "auth": "broken" })));
        }
        let Some((sid, user)) = core.session(ctx, now) else {
            return Err(with_extra(
                err(401, "auth.loginRequired"),
                json!({ "auth": "login" }),
            ));
        };
        if core.is_locked(&sid, now) {
            return Err(with_extra(err(401, "auth.locked"), json!({ "auth": "locked" })));
        }
        let perms = perms_of(&user);
        if !permission.is_empty() && !perms.iter().any(|p| p == permission || p == "admin") {
            return Err(err(403, "auth.forbidden"));
        }
        Ok(Some(user))
    }

    /// Session de la requête (pour les flux SSE à fermer à la révocation).
    pub async fn session_id(&self, ctx: &Ctx) -> Option<String> {
        let now = self.now();
        let mut core = self.auth.lock().await;
        if !core.enabled() {
            return None;
        }
        core.session(ctx, now).map(|(id, _)| id)
    }

    pub async fn auth_enabled(&self) -> bool {
        self.auth.lock().await.enabled()
    }

    pub fn can(user: Option<&Value>, permission: &str) -> bool {
        user.is_none_or(|u| perms_of(u).iter().any(|p| p == "admin" || p == permission))
    }

    pub async fn set_prefs(&self, user_id: &Value, prefs: Value) {
        let mut core = self.auth.lock().await;
        if let Some(user) = core.user_mut(user_id) {
            if !user["prefs"].is_object() {
                user["prefs"] = json!({});
            }
            for (k, v) in prefs.as_object().into_iter().flatten() {
                user["prefs"][k] = v.clone();
            }
            let _ = core.store.save();
        }
    }

    /// Vérifie un secret sous limite d'essais (5 par compte et par adresse, 20 par adresse).
    async fn attempt(
        &self,
        core: &mut AuthCore,
        ctx: &Ctx,
        kind: &str,
        username: &str,
        check: impl std::future::Future<Output = bool>,
    ) -> Result<bool, LocalError> {
        let ip = client_ip(ctx);
        let keys = [format!("{kind}:{ip}:{username}"), format!("ip:{ip}")];
        core.check_rate(&keys, self.now())?;
        if !check.await {
            let now = self.now();
            core.fail(&[(keys[0].clone(), 5), (keys[1].clone(), 20)], now);
            return Ok(false);
        }
        core.failures.remove(&keys[0]);
        Ok(true)
    }

    /// Mot de passe redemandé avant une action grave (supprimer un projet, ouvrir au réseau), sous la
    /// même limite d'essais.
    pub async fn confirm_password(
        &self,
        ctx: &Ctx,
        user: &Value,
        password: &Value,
    ) -> Result<(), LocalError> {
        let mut core = self.auth.lock().await;
        let stored = core
            .user_mut(&user["id"])
            .and_then(|u| u["password"].as_str().map(str::to_string));
        let username = text(&user["username"]);
        let ok = self
            .attempt(&mut core, ctx, "login", &username, verify(text(password), stored))
            .await?;
        if ok {
            Ok(())
        } else {
            Err(err(403, "auth.wrongPassword"))
        }
    }

    /// Ouvre une session : rend le cookie à poser.
    fn open_session(&self, core: &mut AuthCore, ctx: &Ctx, user: &Value, remember: bool) -> String {
        let token = random_token();
        let now = self.now();
        let session = json!({
            "id": sha256_hex(&token), "userId": user["id"], "createdAt": now, "lastSeen": now, "activeAt": now,
            "remember": remember, "agent": describe_agent(ctx.header("user-agent").unwrap_or_default()), "ip": client_ip(ctx),
        });
        let d = core.data();
        let sessions = d["sessions"].as_array_mut().unwrap();
        sessions.push(session);
        // Au plus MAX_SESSIONS par compte : les plus anciennes partent.
        let mut mine: Vec<(i64, String)> = sessions
            .iter()
            .filter(|s| s["userId"] == user["id"])
            .map(|s| (s["lastSeen"].as_i64().unwrap_or(0), text(&s["id"])))
            .collect();
        mine.sort();
        let drop_count = mine.len().saturating_sub(MAX_SESSIONS);
        let dropped: Vec<String> = mine.into_iter().take(drop_count).map(|(_, id)| id).collect();
        sessions.retain(|s| !dropped.contains(&text(&s["id"])) && AuthCore::expires_at(s) >= now);
        let _ = core.store.save();
        let age = if remember {
            format!("; Max-Age={}", SESSION_LONG / 1000)
        } else {
            String::new()
        };
        format!("{COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/{age}")
    }

    fn clear_cookie() -> String {
        format!("{COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0")
    }

    /// Ferme des sessions (et leurs flux) ; rend leur nombre.
    fn revoke(&self, core: &mut AuthCore, filter: impl Fn(&Value) -> bool) -> usize {
        let d = core.data();
        let sessions = d["sessions"].as_array_mut().unwrap();
        let gone: Vec<String> = sessions
            .iter()
            .filter(|s| filter(s))
            .map(|s| text(&s["id"]))
            .collect();
        sessions.retain(|s| !gone.contains(&text(&s["id"])));
        let _ = core.store.save();
        if !gone.is_empty() {
            self.revoke_streams(&gone);
        }
        gone.len()
    }

    async fn create_user(
        &self,
        core: &mut AuthCore,
        body: &Value,
        permissions: &Value,
    ) -> Result<(Value, String), LocalError> {
        let username = text(&body["username"]).trim().to_lowercase();
        check_username(&username)?;
        check_name(body.get("name").unwrap_or(&Value::Null))?;
        check_password(&body["password"])?;
        if core.data()["users"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["username"] == username.as_str())
        {
            return Err(err(409, "auth.usernameTaken"));
        }
        let perms = normalize_permissions(permissions)?;
        let recovery = human_code(5);
        let name = body["name"]
            .as_str()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&username)
            .to_string();
        let id: String = rand::random::<[u8; 6]>()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let user = json!({
            "id": format!("u_{id}"), "username": username, "name": name,
            "password": hash(text(&body["password"])).await,
            "recovery": hash(normalize_code(&recovery)).await,
            "permissions": perms, "prefs": {}, "createdAt": self.now(),
        });
        core.data()["users"].as_array_mut().unwrap().push(user.clone());
        Ok((user, recovery))
    }

    fn admins(core: &mut AuthCore) -> Vec<Value> {
        core.data()["users"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|u| perms_of(u).iter().any(|p| p == "admin"))
            .cloned()
            .collect()
    }

    // ---- Connexion par un fournisseur ----

    fn provider(&self, key: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.key == key)
    }

    /// Départ vers le fournisseur : état aléatoire, à usage unique, valable 10 minutes, lié au
    /// navigateur par un cookie SameSite=Lax (il revient avec la redirection du fournisseur,
    /// contrairement au cookie de session, Strict). `link` : lier le compte connecté au lieu de se
    /// connecter.
    async fn start_oauth(&self, ctx: &Ctx, key: &str, link: bool) -> Result<Response, LocalError> {
        let provider = self.provider(key).unwrap().clone();
        let now = self.now();
        let mut core = self.auth.lock().await;
        if !core.configured(key) {
            return Err(LocalError::new(
                409,
                "auth.providerOff",
                params![provider = provider.name],
            ));
        }
        let mut user_id = None;
        if link {
            let session = core.session(ctx, now);
            let Some((_, user)) = session.filter(|(sid, _)| !core.is_locked(sid, now)) else {
                return Err(with_extra(
                    err(401, "auth.loginRequired"),
                    json!({ "auth": "login" }),
                ));
            };
            user_id = Some(text(&user["id"]));
        }
        core.states.retain(|(_, e)| e.expires_at > now);
        if core.states.len() >= 200 {
            core.states.remove(0);
        }
        let state = random_token();
        let redirect_uri = format!(
            "http://{}/auth/{key}/callback",
            ctx.header("host").unwrap_or_default()
        );
        let client_id = text(&core.data()["providers"][key]["clientId"]);
        core.states.push((
            state.clone(),
            OAuthState {
                provider: provider.key,
                user_id,
                redirect_uri: redirect_uri.clone(),
                expires_at: now + OAUTH_TTL,
            },
        ));
        let mut query = vec![
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("state", state.as_str()),
        ];
        if !provider.scope.is_empty() {
            query.push(("scope", provider.scope));
        }
        query.push(("allow_signup", "false"));
        let location = url::Url::parse_with_params(&provider.authorize_url, &query)
            .map(|u| u.to_string())
            .unwrap_or_default();
        let res = Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, location)
            .header(
                header::SET_COOKIE,
                format!(
                    "{OAUTH_COOKIE}={state}; HttpOnly; SameSite=Lax; Path=/auth/; Max-Age={}",
                    OAUTH_TTL / 1000
                ),
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::empty())
            .unwrap();
        Ok(res)
    }

    /// Retour du fournisseur (GET /auth/<fournisseur>/callback, navigation venue de son site) :
    /// jamais d'erreur HTTP, toujours une page qui renvoie vers Servynex avec un code fixe (rien de
    /// la requête n'y est recopié).
    pub async fn oauth_callback(&self, ctx: &Ctx, key: &str) -> Response {
        let back = |target: &str, session: Option<String>| {
            let mut cookies: Vec<String> = session.into_iter().collect();
            cookies.push(format!(
                "{OAUTH_COOKIE}=; HttpOnly; SameSite=Lax; Path=/auth/; Max-Age=0"
            ));
            // Redirection par la page elle-même : la navigation suivante part de Servynex, donc le
            // cookie de session (SameSite=Strict) l'accompagne.
            let html = format!(
                "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"refresh\" content=\"0;url={target}\"><title>Servynex</title></head><body></body></html>"
            );
            let res = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .header(header::CACHE_CONTROL, "no-store")
                .body(Body::from(html))
                .unwrap();
            with_cookies(res, &cookies)
        };
        let fail = |code: &str| back(&format!("/?auth_error={code}"), None);
        let Some(provider) = self.provider(key).cloned() else {
            return fail("state");
        };
        let state = ctx.query("state").unwrap_or_default().to_string();
        let now = self.now();
        let mut core = self.auth.lock().await;
        let Some(index) = core.states.iter().position(|(s, _)| *s == state) else {
            return fail("state");
        };
        if core.states[index].1.provider != key
            || cookie(ctx, OAUTH_COOKIE).as_deref() != Some(state.as_str())
        {
            return fail("state");
        }
        let (_, entry) = core.states.remove(index);
        if entry.expires_at <= now {
            return fail("state");
        }
        if ctx.query("error").is_some_and(|e| !e.is_empty()) {
            return fail("denied");
        }
        if !core.configured(key) {
            return fail("failed");
        }
        let creds = core.data()["providers"][key].clone();
        let http = core.http.clone();
        let code = ctx.query("code").unwrap_or_default().to_string();
        let profile = fetch_profile(
            &http,
            &provider,
            &text(&creds["clientId"]),
            &text(&creds["clientSecret"]),
            &code,
            &entry.redirect_uri,
        )
        .await;
        let Ok((pid, login)) = profile else {
            return fail("failed");
        };
        let params = || params![provider = provider.name, login = login.clone()];
        let owner = core.data()["users"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| {
                u["identities"]
                    .as_array()
                    .is_some_and(|l| l.iter().any(|i| i["provider"] == key && i["id"] == pid.as_str()))
            })
            .cloned();
        if let Some(user_id) = entry.user_id {
            let Some(user) = core.user_mut(&json!(user_id)) else {
                return fail("state");
            };
            if owner.as_ref().is_some_and(|o| o["id"] != user["id"]) {
                return fail("taken");
            }
            let mut identities: Vec<Value> = user["identities"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|i| i["provider"] != key)
                .collect();
            identities.push(json!({ "provider": key, "id": pid, "login": login }));
            user["identities"] = Value::Array(identities);
            let username = text(&user["username"]);
            let _ = core.store.save();
            self.auth_audit(Some(&username), "audit.providerLinked", params());
            return back("/?auth_done=linked", None);
        }
        let Some(owner) = owner else {
            self.auth_audit(None, "audit.providerLoginFailed", params());
            return fail("unlinked");
        };
        let session = self.open_session(&mut core, ctx, &owner, false);
        self.auth_audit(owner["username"].as_str(), "audit.providerLogin", params());
        back("/", Some(session))
    }

    // ---- Routes /api/auth/... ----

    pub async fn auth_handle(&self, ctx: &mut Ctx, parts: &[&str]) -> Result<Response, LocalError> {
        let section = parts.first().copied().unwrap_or_default();
        let id = parts.get(1).copied();
        let sub = parts.get(2).copied();
        let method = ctx.method.clone();
        let (get, post, patch, put, delete) = (
            method == "GET",
            method == "POST",
            method == "PATCH",
            method == "PUT",
            method == "DELETE",
        );
        let body = if get || delete || ctx.header("content-type").is_none() {
            json!({})
        } else {
            ctx.read_json(16_000).await?
        };
        let now = self.now();
        let lang = ctx.lang(self.lang());

        // -- Publiques --
        if section == "status" && get {
            let mut core = self.auth.lock().await;
            let enabled = core.enabled();
            let broken = core.store.error().map(str::to_string);
            let session = if enabled && broken.is_none() {
                core.session(ctx, now)
            } else {
                None
            };
            let locked = session.as_ref().is_some_and(|(sid, _)| core.is_locked(sid, now));
            let mut methods = json!({ "password": true });
            for p in &self.providers {
                methods[p.key] = json!(core.configured(p.key));
            }
            let d = core.data();
            return Ok(send(
                200,
                &json!({
                    "enabled": enabled,
                    "name": self.instance_name(),
                    "error": broken.map(|e| tr("auth.brokenFile", &params![error = e], lang)),
                    "hasAccounts": d["users"].as_array().is_some_and(|u| !u.is_empty()),
                    "lockAfter": d["lockAfter"],
                    "methods": methods,
                    "user": session.as_ref().map(|(_, u)| public_user(u)),
                    "locked": locked,
                }),
            ));
        }
        {
            let mut core = self.auth.lock().await;
            core.store.load();
            if let Some(error) = core.store.error() {
                return Err(LocalError::new(
                    503,
                    "auth.brokenFile",
                    params![error = error.to_string()],
                ));
            }
        }
        if self.provider(section).is_some() && id == Some("start") && get {
            return self.start_oauth(ctx, section, ctx.has_query("link")).await;
        }
        let mut core = self.auth.lock().await;
        let remember = body["remember"] == true;
        if section == "setup" && post {
            // Première activation : crée le compte administrateur.
            if core.data()["users"].as_array().is_some_and(|u| !u.is_empty()) {
                return Err(err(409, "auth.accountsExist"));
            }
            let (user, recovery) = self.create_user(&mut core, &body, &json!(["admin"])).await?;
            core.data()["enabled"] = json!(true);
            let cookie = self.open_session(&mut core, ctx, &user, remember);
            self.auth_audit(user["username"].as_str(), "audit.protectionOn", params![]);
            return Ok(with_cookies(
                send(201, &json!({ "user": public_user(&user), "recovery": recovery })),
                &[cookie],
            ));
        }
        if section == "enable" && post {
            // Réactivation : il faut les identifiants d'un administrateur existant.
            if core.enabled() {
                return Err(err(409, "auth.alreadyOn"));
            }
            let wanted = text(&body["username"]).to_lowercase();
            let user = Self::admins(&mut core)
                .into_iter()
                .find(|u| u["username"] == wanted.as_str());
            let stored = user
                .as_ref()
                .and_then(|u| u["password"].as_str().map(str::to_string));
            let ok = self
                .attempt(
                    &mut core,
                    ctx,
                    "login",
                    &text(&body["username"]),
                    verify(text(&body["password"]), stored),
                )
                .await?;
            let (true, Some(user)) = (ok, user) else {
                return Err(err(401, "auth.badAdmin"));
            };
            core.data()["enabled"] = json!(true);
            let cookie = self.open_session(&mut core, ctx, &user, remember);
            self.auth_audit(user["username"].as_str(), "audit.protectionBack", params![]);
            return Ok(with_cookies(
                send(200, &json!({ "user": public_user(&user) })),
                &[cookie],
            ));
        }
        if section == "login" && post {
            if !core.enabled() {
                return Err(err(409, "auth.notOn"));
            }
            let username = text(&body["username"]).trim().to_lowercase();
            let user = core.data()["users"]
                .as_array()
                .unwrap()
                .iter()
                .find(|u| u["username"] == username.as_str())
                .cloned();
            let stored = user
                .as_ref()
                .and_then(|u| u["password"].as_str().map(str::to_string));
            let ok = self
                .attempt(
                    &mut core,
                    ctx,
                    "login",
                    &username,
                    verify(text(&body["password"]), stored),
                )
                .await?;
            let (true, Some(user)) = (ok, user) else {
                let who = if username.is_empty() {
                    "?".to_string()
                } else {
                    username
                };
                self.auth_audit(Some(&who), "audit.loginFailed", params![ip = client_ip(ctx)]);
                return Err(err(401, "auth.badLogin"));
            };
            let cookie = self.open_session(&mut core, ctx, &user, remember);
            self.auth_audit(user["username"].as_str(), "audit.login", params![]);
            return Ok(with_cookies(
                send(200, &json!({ "user": public_user(&user) })),
                &[cookie],
            ));
        }
        if section == "logout" && post {
            if let Some((sid, user)) = core.session(ctx, now) {
                self.revoke(&mut core, |s| s["id"] == sid.as_str());
                self.auth_audit(user["username"].as_str(), "audit.logout", params![]);
            }
            return Ok(with_cookies(
                send(200, &json!({ "ok": true })),
                &[Self::clear_cookie()],
            ));
        }
        if (section == "recover" || section == "reset") && post {
            // recover : clé de secours · reset : code de 15 minutes (servynex auth ou un administrateur).
            let username = text(&body["username"]).trim().to_lowercase();
            let user = core.data()["users"]
                .as_array()
                .unwrap()
                .iter()
                .find(|u| u["username"] == username.as_str())
                .cloned();
            let recover = section == "recover";
            let secret = normalize_code(&text(if recover { &body["key"] } else { &body["code"] }));
            // Vérifié seulement si la limite des essais le permet (scrypt coûte).
            let check: std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> = if recover {
                Box::pin(verify(
                    secret.clone(),
                    user.as_ref()
                        .and_then(|u| u["recovery"].as_str().map(str::to_string)),
                ))
            } else {
                let uid = user.as_ref().map(|u| u["id"].clone());
                let reset = core.data()["resets"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|r| {
                        uid.as_ref().is_some_and(|id| r["userId"] == *id)
                            && r["expiresAt"].as_i64().unwrap_or(0) > now
                    })
                    .cloned();
                let ok = reset.is_some_and(|r| {
                    bool::from(
                        sha256_hex(&secret)
                            .as_bytes()
                            .ct_eq(text(&r["codeHash"]).as_bytes()),
                    )
                });
                Box::pin(async move { ok })
            };
            let ok = self.attempt(&mut core, ctx, section, &username, check).await?;
            let (true, Some(user)) = (ok, user) else {
                return Err(err(401, if recover { "auth.badKey" } else { "auth.badCode" }));
            };
            check_password(&body["password"])?;
            let password = hash(text(&body["password"])).await;
            let recovery = if recover { Some(human_code(5)) } else { None };
            let recovery_hash = match &recovery {
                Some(r) => Some(hash(normalize_code(r)).await),
                None => None,
            };
            let uid = user["id"].clone();
            {
                let u = core.user_mut(&uid).unwrap();
                u["password"] = json!(password);
                if let Some(h) = recovery_hash {
                    u["recovery"] = json!(h);
                }
            }
            core.data()["resets"]
                .as_array_mut()
                .unwrap()
                .retain(|r| r["userId"] != uid);
            self.revoke(&mut core, |s| s["userId"] == uid);
            let fresh = core.user_mut(&uid).unwrap().clone();
            let cookie = self.open_session(&mut core, ctx, &fresh, false);
            self.auth_audit(
                fresh["username"].as_str(),
                if recover {
                    "audit.resetWithKey"
                } else {
                    "audit.resetWithCode"
                },
                params![],
            );
            return Ok(with_cookies(
                send(200, &json!({ "user": public_user(&fresh), "recovery": recovery })),
                &[cookie],
            ));
        }
        if section == "invite" {
            let token = if get {
                id.unwrap_or_default().to_string()
            } else {
                text(&body["token"])
            };
            let hash_token = sha256_hex(&token);
            let invite = core.data()["invites"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["tokenHash"] == hash_token.as_str() && i["expiresAt"].as_i64().unwrap_or(0) > now)
                .cloned();
            if get {
                return Ok(send(
                    200,
                    &match invite {
                        Some(i) => {
                            json!({ "valid": true, "role": role_of(&perms_of(&i)), "expiresAt": i["expiresAt"] })
                        }
                        None => json!({ "valid": false }),
                    },
                ));
            }
            if post {
                let Some(invite) = invite else {
                    return Err(err(410, "auth.badInvite"));
                };
                let (user, recovery) = self.create_user(&mut core, &body, &invite["permissions"]).await?;
                core.data()["invites"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|i| i["tokenHash"] != invite["tokenHash"]);
                let cookie = self.open_session(&mut core, ctx, &user, remember);
                self.auth_audit(
                    user["username"].as_str(),
                    "audit.joined",
                    params![role = role_name(&perms_of(&user))],
                );
                return Ok(with_cookies(
                    send(201, &json!({ "user": public_user(&user), "recovery": recovery })),
                    &[cookie],
                ));
            }
        }

        // -- Session ouverte (même verrouillée pour déverrouiller) --
        if !core.enabled() {
            return Err(err(409, "auth.notOn"));
        }
        let Some((sid, me)) = core.session(ctx, now) else {
            return Err(with_extra(
                err(401, "auth.loginRequired"),
                json!({ "auth": "login" }),
            ));
        };
        let me_name = text(&me["username"]);
        let me_id = me["id"].clone();
        if section == "unlock" && post {
            let stored = me["password"].as_str().map(str::to_string);
            let ok = self
                .attempt(
                    &mut core,
                    ctx,
                    "login",
                    &me_name,
                    verify(text(&body["password"]), stored),
                )
                .await?;
            if !ok {
                return Err(err(401, "auth.wrongPassword"));
            }
            if let Some(s) = core.session_value(&sid) {
                s["locked"] = json!(false);
                s["activeAt"] = json!(now);
            }
            let _ = core.store.save();
            return Ok(send(200, &json!({ "ok": true })));
        }
        if core.is_locked(&sid, now) {
            return Err(with_extra(err(401, "auth.locked"), json!({ "auth": "locked" })));
        }
        if section == "activity" && post {
            if let Some(s) = core.session_value(&sid) {
                s["activeAt"] = json!(now);
            }
            let _ = core.store.save();
            return Ok(send(200, &json!({ "ok": true })));
        }
        if section == "lock" && post {
            if let Some(s) = core.session_value(&sid) {
                s["locked"] = json!(true);
            }
            let _ = core.store.save();
            self.revoke_streams(std::slice::from_ref(&sid)); // plus d'évènements vers une page verrouillée
            return Ok(send(200, &json!({ "ok": true })));
        }
        if section == "me" {
            if id.is_none() && patch {
                check_name(body.get("name").unwrap_or(&Value::Null))?;
                let prefs = body.get("prefs").cloned();
                if let Some(p) = &prefs
                    && !(p.is_object() && p.to_string().len() < 500)
                {
                    return Err(err(400, "auth.badPrefs"));
                }
                let user = core.user_mut(&me_id).unwrap();
                if let Some(name) = body.get("name").and_then(Value::as_str) {
                    user["name"] = json!(if name.trim().is_empty() {
                        me_name.clone()
                    } else {
                        name.trim().to_string()
                    });
                }
                if let Some(Value::Object(p)) = prefs {
                    if !user["prefs"].is_object() {
                        user["prefs"] = json!({});
                    }
                    for (k, v) in p {
                        user["prefs"][k] = v;
                    }
                }
                let fresh = user.clone();
                let _ = core.store.save();
                return Ok(send(200, &json!({ "user": public_user(&fresh) })));
            }
            if id == Some("password") && post {
                let stored = me["password"].as_str().map(str::to_string);
                let ok = self
                    .attempt(
                        &mut core,
                        ctx,
                        "login",
                        &me_name,
                        verify(text(&body["current"]), stored),
                    )
                    .await?;
                if !ok {
                    return Err(err(401, "auth.wrongCurrent"));
                }
                check_password(&body["password"])?;
                let password = hash(text(&body["password"])).await;
                core.user_mut(&me_id).unwrap()["password"] = json!(password);
                let closed = self.revoke(&mut core, |s| s["userId"] == me_id && s["id"] != sid.as_str());
                self.auth_audit(Some(&me_name), "audit.passwordChanged", params![]);
                return Ok(send(200, &json!({ "closed": closed })));
            }
            if id == Some("recovery") && post {
                let stored = me["password"].as_str().map(str::to_string);
                let ok = self
                    .attempt(
                        &mut core,
                        ctx,
                        "login",
                        &me_name,
                        verify(text(&body["password"]), stored),
                    )
                    .await?;
                if !ok {
                    return Err(err(401, "auth.wrongPassword"));
                }
                let recovery = human_code(5);
                let h = hash(normalize_code(&recovery)).await;
                core.user_mut(&me_id).unwrap()["recovery"] = json!(h);
                let _ = core.store.save();
                self.auth_audit(Some(&me_name), "audit.newKey", params![]);
                return Ok(send(200, &json!({ "recovery": recovery })));
            }
            if id == Some("identities")
                && let Some(p) = sub.and_then(|s| self.provider(s))
                && delete
            {
                let user = core.user_mut(&me_id).unwrap();
                let kept: Vec<Value> = user["identities"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|i| i["provider"] != p.key)
                    .collect();
                user["identities"] = Value::Array(kept);
                let fresh = user.clone();
                let _ = core.store.save();
                self.auth_audit(
                    Some(&me_name),
                    "audit.providerUnlinked",
                    params![provider = p.name],
                );
                return Ok(send(200, &json!({ "user": public_user(&fresh) })));
            }
            if id == Some("sessions") {
                if get {
                    let mut list: Vec<Value> = core.data()["sessions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|s| s["userId"] == me_id && AuthCore::expires_at(s) >= now)
                        .cloned()
                        .collect();
                    list.sort_by_key(|s| std::cmp::Reverse(s["lastSeen"].as_i64().unwrap_or(0)));
                    let list: Vec<Value> = list
                        .iter()
                        .map(|s| {
                            let full = text(&s["id"]);
                            json!({
                                "id": &full[..full.len().min(16)], "agent": s["agent"], "ip": s["ip"], "createdAt": s["createdAt"],
                                "lastSeen": s["lastSeen"], "remember": s["remember"], "current": full == sid,
                            })
                        })
                        .collect();
                    return Ok(send(200, &json!({ "sessions": list })));
                }
                if delete && let Some(prefix) = sub {
                    let n = self.revoke(&mut core, |s| {
                        s["userId"] == me_id && text(&s["id"]).starts_with(prefix) && s["id"] != sid.as_str()
                    });
                    return Ok(send(200, &json!({ "closed": n })));
                }
                if post && sub == Some("others") {
                    let n = self.revoke(&mut core, |s| s["userId"] == me_id && s["id"] != sid.as_str());
                    self.auth_audit(Some(&me_name), "audit.othersClosed", params![]);
                    return Ok(send(200, &json!({ "closed": n })));
                }
            }
        }

        // -- Administration --
        if !perms_of(&me).iter().any(|p| p == "admin") {
            return Err(err(403, "auth.adminOnly"));
        }
        if section == "providers" {
            if id.is_none() && get {
                let mut out = json!({});
                for p in &self.providers {
                    let c = core.data()["providers"][p.key].clone();
                    let configured = c["clientId"].as_str().is_some_and(|s| !s.is_empty())
                        && c["clientSecret"].as_str().is_some_and(|s| !s.is_empty());
                    // Le secret ne quitte jamais le serveur : seulement « configuré » et l'identifiant.
                    out[p.key] = json!({ "name": p.name, "configured": configured, "clientId": c.get("clientId").cloned().unwrap_or(Value::Null), "registerUrl": p.register_url });
                }
                return Ok(send(200, &out));
            }
            let Some(provider) = id.and_then(|i| self.provider(i)).cloned() else {
                return Err(err(404, "http.unknownRoute"));
            };
            if put {
                let client_id = body["clientId"]
                    .as_str()
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let secret = body["clientSecret"]
                    .as_str()
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string();
                let current = core.data()["providers"][provider.key]["clientSecret"]
                    .as_str()
                    .map(str::to_string);
                if client_id.is_empty() || client_id.chars().count() > 200 || secret.chars().count() > 200 {
                    return Err(err(400, "auth.providerBad"));
                }
                if secret.is_empty() && current.is_none() {
                    return Err(err(400, "auth.providerSecret"));
                }
                let d = core.data();
                if !d["providers"].is_object() {
                    d["providers"] = json!({});
                }
                d["providers"][provider.key] = json!({ "clientId": client_id, "clientSecret": if secret.is_empty() { current.unwrap() } else { secret } });
                let _ = core.store.save();
                self.auth_audit(
                    Some(&me_name),
                    "audit.providerSet",
                    params![provider = provider.name],
                );
                return Ok(send(200, &json!({ "configured": true, "clientId": client_id })));
            }
            if delete {
                if let Some(map) = core.data()["providers"].as_object_mut() {
                    map.shift_remove(provider.key);
                }
                let _ = core.store.save();
                self.auth_audit(
                    Some(&me_name),
                    "audit.providerRemoved",
                    params![provider = provider.name],
                );
                return Ok(send(200, &json!({ "configured": false })));
            }
        }
        if section == "users" {
            if id.is_none() && get {
                let users: Vec<Value> = core.data()["users"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(public_user)
                    .collect();
                return Ok(send(200, &json!({ "users": users })));
            }
            let uid = json!(id.unwrap_or_default());
            let Some(user) = core.user_mut(&uid).cloned() else {
                return Err(err(404, "auth.unknownAccount"));
            };
            let username = text(&user["username"]);
            let last_admin =
                perms_of(&user).iter().any(|p| p == "admin") && Self::admins(&mut core).len() == 1;
            if sub.is_none() && patch {
                check_name(body.get("name").unwrap_or(&Value::Null))?;
                let perms = match body.get("permissions") {
                    Some(p) => {
                        let perms = normalize_permissions(p)?;
                        if last_admin && !perms.iter().any(|p| p == "admin") {
                            return Err(err(409, "auth.lastAdminRights"));
                        }
                        Some(perms)
                    }
                    None => None,
                };
                let u = core.user_mut(&uid).unwrap();
                if let Some(perms) = perms {
                    u["permissions"] = json!(perms);
                }
                if let Some(name) = body.get("name").and_then(Value::as_str) {
                    u["name"] = json!(if name.trim().is_empty() {
                        username.clone()
                    } else {
                        name.trim().to_string()
                    });
                }
                let fresh = u.clone();
                let _ = core.store.save();
                self.auth_audit(
                    Some(&me_name),
                    "audit.userChanged",
                    params![username = username, role = role_name(&perms_of(&fresh))],
                );
                return Ok(send(200, &json!({ "user": public_user(&fresh) })));
            }
            if sub.is_none() && delete {
                if last_admin {
                    return Err(err(409, "auth.lastAdminDelete"));
                }
                let d = core.data();
                d["users"].as_array_mut().unwrap().retain(|u| u["id"] != uid);
                d["resets"].as_array_mut().unwrap().retain(|r| r["userId"] != uid);
                self.revoke(&mut core, |s| s["userId"] == uid);
                self.auth_audit(Some(&me_name), "audit.userDeleted", params![username = username]);
                return Ok(send(200, &json!({ "ok": true })));
            }
            if sub == Some("reset") && post {
                let code = create_reset_code(&mut core.store, &username, now)?;
                self.auth_audit(Some(&me_name), "audit.resetCode", params![username = username]);
                return Ok(send(200, &json!({ "code": code, "expiresAt": now + RESET_TTL })));
            }
        }
        if section == "invites" {
            core.data()["invites"]
                .as_array_mut()
                .unwrap()
                .retain(|i| i["expiresAt"].as_i64().unwrap_or(0) > now);
            if get {
                let invites: Vec<Value> = core.data()["invites"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| {
                        let h = text(&i["tokenHash"]);
                        json!({ "id": &h[..h.len().min(16)], "role": role_of(&perms_of(i)), "permissions": i["permissions"], "expiresAt": i["expiresAt"], "createdBy": i["createdBy"] })
                    })
                    .collect();
                return Ok(send(200, &json!({ "invites": invites })));
            }
            if post {
                let token = random_token();
                let wanted = body
                    .get("permissions")
                    .cloned()
                    .unwrap_or(json!(["view", "operate"]));
                let permissions = normalize_permissions(&wanted)?;
                core.data()["invites"].as_array_mut().unwrap().push(json!({
                    "tokenHash": sha256_hex(&token), "permissions": permissions, "expiresAt": now + INVITE_TTL, "createdBy": me_name,
                }));
                let _ = core.store.save();
                self.auth_audit(
                    Some(&me_name),
                    "audit.invited",
                    params![role = role_name(&permissions)],
                );
                return Ok(send(
                    201,
                    &json!({ "token": token, "expiresAt": now + INVITE_TTL }),
                ));
            }
            if delete && let Some(prefix) = id {
                core.data()["invites"]
                    .as_array_mut()
                    .unwrap()
                    .retain(|i| !text(&i["tokenHash"]).starts_with(prefix));
                let _ = core.store.save();
                return Ok(send(200, &json!({ "ok": true })));
            }
        }
        if section == "settings" && patch {
            if let Some(lock) = body.get("lockAfter") {
                let Some(value) = lock.as_i64().filter(|v| LOCK_CHOICES.contains(v)) else {
                    return Err(err(400, "auth.badLock"));
                };
                let d = core.data();
                d["lockAfter"] = json!(value);
                for s in d["sessions"].as_array_mut().unwrap() {
                    s["activeAt"] = json!(now);
                }
            }
            let _ = core.store.save();
            let lock = core.data()["lockAfter"].as_i64().unwrap_or(0);
            let value = if lock > 0 {
                msg("time.minutes", params![n = lock / 60])
            } else {
                msg("auth.never", params![])
            };
            self.auth_audit(Some(&me_name), "audit.lock", params![value = value]);
            return Ok(send(200, &json!({ "lockAfter": lock })));
        }
        if section == "disable" && post {
            // Protection retirée alors que la page est ouverte au réseau : n'importe qui y aurait la main.
            if self.config()["instance"]["access"] == "lan" {
                return Err(err(409, "auth.disableLan"));
            }
            let stored = me["password"].as_str().map(str::to_string);
            let ok = self
                .attempt(
                    &mut core,
                    ctx,
                    "login",
                    &me_name,
                    verify(text(&body["password"]), stored),
                )
                .await?;
            if !ok {
                return Err(err(401, "auth.wrongPassword"));
            }
            core.data()["enabled"] = json!(false);
            self.revoke(&mut core, |_| true);
            self.auth_audit(Some(&me_name), "audit.protectionOff", params![]);
            return Ok(with_cookies(
                send(200, &json!({ "ok": true })),
                &[Self::clear_cookie()],
            ));
        }
        let _ = PERMISSIONS;
        Err(err(404, "http.unknownRoute"))
    }
}

/// Code reçu au retour → profil (identifiant, nom) du compte chez le fournisseur.
async fn fetch_profile(
    http: &reqwest::Client,
    provider: &Provider,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<(String, String), ()> {
    let form = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
        ("redirect_uri", redirect_uri),
    ];
    let res = http
        .post(&provider.token_url)
        .header("accept", "application/json")
        .header("user-agent", "Servynex")
        .form(&form)
        .send()
        .await
        .map_err(|_| ())?;
    let ok = res.status().is_success();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    let token = body["access_token"]
        .as_str()
        .filter(|t| ok && !t.is_empty())
        .ok_or(())?
        .to_string();
    let res = http
        .get(&provider.user_url)
        .header("accept", "application/json")
        .header("user-agent", "Servynex")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }
    let u: Value = res.json().await.map_err(|_| ())?;
    let id = match &u["id"] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let login = text(&u["login"]);
    if id.is_empty() || login.is_empty() {
        Err(())
    } else {
        Ok((id, login))
    }
}
