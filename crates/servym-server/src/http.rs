//! Requêtes et réponses : lecture du corps JSON (mêmes codes 415, 413, 400 que la version Node),
//! réponses JSON, en-têtes de sécurité, protections (Host, Sec-Fetch-Site, en-tête de l'interface,
//! Origin) et permission exigée par chaque route.

use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;
use serde_json::Value;
use servym_core::i18n::{Lang, LocalError, lang_of};
use servym_core::params;

pub const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

/// En-têtes de sécurité posés sur toutes les réponses.
pub fn security_headers(headers: &mut HeaderMap) {
    headers.insert("content-security-policy", HeaderValue::from_static(CSP));
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
}

/// Erreur au message traduisible (statut HTTP compris).
pub fn err(status: u16, key: &str) -> LocalError {
    LocalError::new(status, key, params![])
}

/// Requête reçue, avec son corps lu à la demande.
pub struct Ctx {
    pub method: Method,
    /// Chemin tel que reçu (non décodé), comme `url.pathname`.
    pub path: String,
    /// Chemin et requête, comme `req.url`.
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: HeaderMap,
    pub peer: SocketAddr,
    // Dans un Mutex : le corps n'est pas partageable entre threads, `&Ctx` doit l'être.
    body: std::sync::Mutex<Option<Body>>,
}

impl Ctx {
    pub fn new(req: axum::extract::Request, peer: SocketAddr) -> Ctx {
        let (parts, body) = req.into_parts();
        let url = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path().to_string(), |p| p.as_str().to_string());
        let query = parts
            .uri
            .query()
            .map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect())
            .unwrap_or_default();
        Ctx {
            method: parts.method,
            path: parts.uri.path().to_string(),
            url,
            query,
            headers: parts.headers,
            peer,
            body: std::sync::Mutex::new(Some(body)),
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn has_query(&self, name: &str) -> bool {
        self.query.iter().any(|(k, _)| k == name)
    }

    pub fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Langue de la page (Accept-Language), sinon celle du serveur.
    pub fn lang(&self, server: Lang) -> Lang {
        lang_of(self.header("accept-language"), server)
    }

    pub fn is_get(&self) -> bool {
        self.method == Method::GET || self.method == Method::HEAD
    }

    /// Corps JSON : `Content-Type: application/json` exigé (415), `limit` octets au plus (413),
    /// JSON valide (400) ; corps vide : `{}`.
    pub async fn read_json(&mut self, limit: usize) -> Result<Value, LocalError> {
        if !self
            .header("content-type")
            .unwrap_or("undefined")
            .starts_with("application/json")
        {
            return Err(err(415, "http.jsonExpected"));
        }
        let body = self
            .body
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .unwrap_or_default();
        let bytes = axum::body::to_bytes(body, limit)
            .await
            .map_err(|_| err(413, "http.tooLarge"))?;
        if bytes.is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_slice(&bytes).map_err(|_| err(400, "http.badJson"))
    }

    /// Corps JSON s'il y en a un (`Content-Type` présent), sinon `{}`.
    pub async fn read_json_if_any(&mut self, limit: usize) -> Result<Value, LocalError> {
        if self.header("content-type").is_none() {
            return Ok(Value::Object(Default::default()));
        }
        self.read_json(limit).await
    }
}

/// Réponse JSON (jamais mise en cache).
pub fn send(status: u16, body: &Value) -> Response {
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body.to_string()))
        .unwrap()
}

/// Réponse d'erreur dans la langue de la page, avec ses champs en plus (`{ auth: "login" }`).
pub fn error_response(error: &LocalError, lang: Lang) -> Response {
    let mut body = serde_json::json!({ "error": error.message_in(lang) });
    if let Some(Value::Object(extra)) = error.extra.as_deref() {
        body.as_object_mut().unwrap().extend(extra.clone());
    }
    send(error.status, &body)
}

/// Ajoute des champs à la réponse JSON d'une erreur.
pub fn with_extra(mut error: LocalError, extra: Value) -> LocalError {
    error.extra = Some(Box::new(extra));
    error
}

// ---- Protections ----

const LOOPBACK: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// GET qui font travailler la machine (ports, git, réseau…) : même en-tête exigé que pour les
/// écritures, pour qu'un `<img>` tiers ne puisse pas les déclencher. Les flux SSE (EventSource ne
/// pose pas d'en-tête) restent libres : ils ne font que lire.
fn costly_get(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("/api/") else {
        return false;
    };
    [
        "free-port",
        "git",
        "ports",
        "network",
        "autostart",
        "stats",
        "config/export",
    ]
    .iter()
    .any(|route| {
        rest.strip_prefix(route)
            .is_some_and(|after| after.is_empty() || after.starts_with('?'))
    })
}

/// Host : bloque le DNS rebinding (seuls les noms de cette machine sont acceptés). En-tête
/// X-Servym-Request et Origin : bloquent le CSRF (un site tiers ne peut pas poser l'en-tête sans
/// pré-vol CORS, refusé).
pub fn guard(ctx: &Ctx, lan_names: Option<&[String]>) -> Result<(), LocalError> {
    let raw = ctx.header("host").unwrap_or_default().to_ascii_lowercase();
    let host = match raw.rfind(':') {
        Some(i) if raw[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < raw.len() => &raw[..i],
        _ => raw.as_str(),
    };
    let allowed = LOOPBACK.contains(&host)
        || lan_names.is_some_and(|names| names.iter().any(|n| n.to_lowercase() == host));
    if !allowed {
        return Err(err(421, "http.badHost"));
    }
    let site = ctx.header("sec-fetch-site");
    if ctx.url.starts_with("/api/") && site.is_some_and(|s| s != "same-origin" && s != "none") {
        return Err(err(403, "http.crossSite"));
    }
    if ctx.is_get() && !costly_get(&ctx.url) {
        return Ok(());
    }
    if ctx.header("x-servym-request") != Some("1") {
        return Err(err(403, "http.forbidden"));
    }
    let Some(origin) = ctx.header("origin") else {
        return Ok(());
    };
    let same = url::Url::parse(origin).ok().is_some_and(|u| {
        let host = u.host_str().map(|h| match u.port() {
            Some(p) => format!("{h}:{p}"),
            None => h.to_string(),
        });
        host.as_deref() == ctx.header("host")
    });
    if same {
        Ok(())
    } else {
        Err(err(403, "http.badOrigin"))
    }
}

/// Permission exigée par une route de l'API.
pub fn permission_for(
    section: &str,
    id: Option<&str>,
    action: Option<&str>,
    method: &Method,
) -> &'static str {
    const OPERATE: [&str; 7] = ["start", "stop", "restart", "open", "folder", "code", "terminal"];
    let get = method == Method::GET || method == Method::HEAD;
    if section == "config" {
        return "commands"; // export et import : tout projects.json, commandes comprises
    }
    if get {
        return "view";
    }
    let projects = section == "projects";
    match (id, action) {
        (None, _) if projects && method == Method::POST => return "commands",
        (None, _) if projects && method == Method::PUT => return "edit", // ordre des projets
        (Some(_), None) if projects && method == Method::PATCH => return "edit",
        (Some(_), None) if projects && method == Method::DELETE => return "commands",
        (_, Some("duplicate")) if projects && method == Method::POST => return "commands", // copie sa commande
        (_, Some(a)) if projects && OPERATE.contains(&a) && method == Method::POST => return "operate",
        _ => {}
    }
    if section == "reload" {
        return "edit";
    }
    if section == "instance"
        && let Some(id) = id
    {
        return match id {
            "config" => "commands",
            "log" | "folder" | "lang" | "notify" | "notify-test" => "instance",
            // Devant la machine seulement (application de bureau) : qui est devant elle a la main.
            "onboarded" | "opener" | "browser" => "view",
            _ => "admin",
        };
    }
    if matches!(section, "instance" | "restart" | "autostart") {
        return "instance";
    }
    "admin"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_couteuses() {
        assert!(costly_get("/api/ports"));
        assert!(costly_get("/api/free-port?from=1024"));
        assert!(costly_get("/api/config/export"));
        assert!(!costly_get("/api/portsx"));
        assert!(!costly_get("/api/projects"));
    }

    #[test]
    fn permissions() {
        assert_eq!(
            permission_for("projects", Some("a"), Some("start"), &Method::POST),
            "operate"
        );
        assert_eq!(permission_for("projects", None, None, &Method::PUT), "edit");
        assert_eq!(
            permission_for("instance", Some("onboarded"), None, &Method::POST),
            "view"
        );
        assert_eq!(
            permission_for("instance", Some("access"), None, &Method::PUT),
            "admin"
        );
        assert_eq!(permission_for("restart", None, None, &Method::POST), "instance");
        assert_eq!(
            permission_for("config", Some("export"), None, &Method::GET),
            "commands"
        );
    }
}
