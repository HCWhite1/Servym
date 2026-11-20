//! Comptes, sessions et secrets (`runtime/auth.json`) : stockage, hachages et codes.
//!
//! Formats identiques à la version Node : les hachages scrypt existants restent valides.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use unicode_normalization::UnicodeNormalization;

use crate::i18n::LocalError;
use crate::params;

/// Permissions, de la plus faible à la plus forte.
pub const PERMISSIONS: [&str; 6] = ["view", "operate", "edit", "commands", "instance", "admin"];
/// Verrouillage après inactivité (secondes, 0 : jamais).
pub const LOCK_CHOICES: [i64; 6] = [0, 120, 300, 900, 1800, 3600];
/// Durée d'un code de réinitialisation (ms).
pub const RESET_TTL: i64 = 15 * 60_000;

const ROLES: [(&str, &[&str]); 3] = [
    ("readonly", &["view"]),
    ("operator", &["view", "operate"]),
    ("editor", &["view", "operate", "edit"]),
];

/// Nom du rôle d'un ensemble de permissions (`custom` s'il n'en a pas).
pub fn role_of(permissions: &[String]) -> &'static str {
    if permissions.iter().any(|p| p == "admin") {
        return "admin";
    }
    let mut key: Vec<&str> = permissions.iter().map(String::as_str).collect();
    key.sort();
    for (name, list) in ROLES {
        let mut expected = list.to_vec();
        expected.sort();
        if expected == key {
            return name;
        }
    }
    "custom"
}

/// Permissions complétées : `view` toujours, `commands` implique `edit`, qui implique `operate` ;
/// `admin` les donne toutes. Erreur 400 pour une permission inconnue.
pub fn normalize_permissions(list: &Value) -> Result<Vec<String>, LocalError> {
    let names: Option<Vec<&str>> = list
        .as_array()
        .and_then(|l| l.iter().map(Value::as_str).collect());
    let names = names
        .filter(|n| n.iter().all(|p| PERMISSIONS.contains(p)))
        .ok_or_else(|| LocalError::new(400, "auth.badPermissions", params![]))?;
    if names.contains(&"admin") {
        return Ok(PERMISSIONS.iter().map(|p| p.to_string()).collect());
    }
    let mut set: Vec<&str> = vec!["view"];
    set.extend(names);
    if set.contains(&"commands") {
        set.push("edit");
    }
    if set.contains(&"edit") {
        set.push("operate");
    }
    Ok(PERMISSIONS
        .iter()
        .filter(|p| set.contains(p))
        .map(|p| p.to_string())
        .collect())
}

// ---- Secrets ----

const SCRYPT_N: u64 = 32768;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;

fn derive(secret: &str, salt: &[u8], n: u64, r: u32, p: u32, len: usize) -> Option<Vec<u8>> {
    if !n.is_power_of_two() || n < 2 || len == 0 || len > 1024 {
        return None;
    }
    let params = scrypt::Params::new(n.trailing_zeros() as u8, r, p, len).ok()?;
    let normalized: String = secret.nfkc().collect();
    let mut out = vec![0u8; len];
    scrypt::scrypt(normalized.as_bytes(), salt, &params, &mut out).ok()?;
    Some(out)
}

/// Hachage scrypt au format de la version Node (`scrypt$N$r$p$sel$hachage`, secret normalisé NFKC).
pub fn hash_secret(secret: &str) -> String {
    let salt: [u8; 16] = rand::random();
    let hash = derive(secret, &salt, SCRYPT_N, SCRYPT_R, SCRYPT_P, 64).expect("paramètres scrypt");
    format!(
        "scrypt${SCRYPT_N}${SCRYPT_R}${SCRYPT_P}${}${}",
        STANDARD.encode(salt),
        STANDARD.encode(hash)
    )
}

/// Vérifie un secret contre un hachage (durée constante, même pour un hachage illisible).
pub fn verify_secret(secret: &str, stored: &str) -> bool {
    let parts: Vec<&str> = stored.split('$').collect();
    let parsed = match parts[..] {
        ["scrypt", n, r, p, salt, hash] => (|| {
            Some((
                n.parse().ok()?,
                r.parse().ok()?,
                p.parse().ok()?,
                STANDARD.decode(salt).ok()?,
                STANDARD.decode(hash).ok()?,
            ))
        })(),
        _ => None,
    };
    let Some((n, r, p, salt, expected)) = parsed else {
        let _ = hash_secret("x"); // même durée qu'une vraie vérification
        return false;
    };
    derive(secret, &salt, n, r, p, expected.len()).is_some_and(|actual| actual.ct_eq(&expected).into())
}

/// SHA-256 en hexadécimal (codes de réinitialisation, jetons de session).
pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Jeton aléatoire de 32 octets, en base64url.
pub fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

/// Sans 0/O ni 1/I.
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// « 7KQF-M2XA-… » : groupes de 4 caractères faciles à recopier.
pub fn human_code(groups: usize) -> String {
    (0..groups)
        .map(|_| {
            (0..4)
                .map(|_| ALPHABET[usize::from(rand::random::<u8>()) % ALPHABET.len()] as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Code tel que tapé → forme comparable (majuscules, lettres et chiffres seulement).
pub fn normalize_code(text: &str) -> String {
    text.to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .collect()
}

// ---- Stockage ----

fn empty() -> Value {
    json!({ "version": 1, "enabled": false, "lockAfter": 0, "users": [], "sessions": [], "invites": [], "resets": [], "providers": {} })
}

/// Fichier des comptes, relu dès qu'il change sur le disque (commande `servym auth`,
/// modification à la main).
pub struct AuthStore {
    file: PathBuf,
    data: Value,
    error: Option<String>,
    /// Date et taille à la dernière lecture ou écriture : un changement fait dans le même instant
    /// qu'un enregistrement est vu aussi, grâce à la taille.
    seen: Option<(SystemTime, u64)>,
}

impl AuthStore {
    pub fn new(file: &Path) -> Self {
        let mut store = AuthStore {
            file: file.to_path_buf(),
            data: empty(),
            error: None,
            seen: None,
        };
        store.load();
        store
    }

    /// Relit le fichier s'il a changé (date ou taille) et rend les données.
    pub fn load(&mut self) -> &mut Value {
        let Ok(info) = std::fs::metadata(&self.file) else {
            if self.seen.is_some() || self.error.is_some() {
                self.data = empty();
                self.seen = None;
                self.error = None;
            }
            return &mut self.data;
        };
        let now = (info.modified().unwrap_or(SystemTime::UNIX_EPOCH), info.len());
        if self.seen == Some(now) {
            return &mut self.data;
        }
        match std::fs::read_to_string(&self.file)
            .map_err(|e| e.to_string())
            .and_then(|t| {
                serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
            }) {
            Ok(Value::Object(read)) => {
                let mut data = empty();
                for (k, v) in read {
                    data[k] = v;
                }
                self.data = data;
                self.error = None;
            }
            Ok(_) => self.error = Some("auth.json : objet attendu".into()),
            Err(e) => self.error = Some(e),
        }
        self.seen = Some(now);
        &mut self.data
    }

    /// Données sans relecture.
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// Enregistre (fichier temporaire puis renommage).
    pub fn save(&mut self) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = self.file.as_os_str().to_owned();
        tmp.push(".tmp");
        let text = format!("{}\n", serde_json::to_string_pretty(&self.data).unwrap());
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            f.write_all(text.as_bytes())?;
        }
        #[cfg(not(unix))]
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.file)?;
        let info = std::fs::metadata(&self.file)?;
        self.seen = Some((info.modified()?, info.len()));
        Ok(())
    }

    /// Erreur de lecture du fichier (JSON invalide), s'il y en a une.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Date de modification déjà vue (tests : un changement fait dans le même instant qu'un
    /// enregistrement doit être vu grâce à la taille).
    pub fn set_seen_mtime(&mut self, mtime: SystemTime) {
        let size = self.seen.map_or(0, |(_, size)| size);
        self.seen = Some((mtime, size));
    }
}

/// Code de réinitialisation valable 15 minutes (commande `servym auth`, ou un administrateur).
pub fn create_reset_code(store: &mut AuthStore, username: &str, now: i64) -> Result<String, LocalError> {
    let data = store.load();
    let user_id = data["users"]
        .as_array()
        .and_then(|users| users.iter().find(|u| u["username"] == username))
        .and_then(|u| u["id"].as_str())
        .map(str::to_string)
        .ok_or_else(|| LocalError::new(404, "auth.unknownUser", params![username = username]))?;
    let code = human_code(2);
    if !data["resets"].is_array() {
        data["resets"] = json!([]);
    }
    let resets = data["resets"].as_array_mut().unwrap();
    resets.retain(|r| r["userId"] != user_id.as_str() && r["expiresAt"].as_i64().unwrap_or(0) > now);
    resets.push(json!({ "userId": user_id, "codeHash": sha256_hex(&normalize_code(&code)), "expiresAt": now + RESET_TTL }));
    store
        .save()
        .map_err(|e| LocalError::new(500, "raw", params![text = e.to_string()]))?;
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hachages produits par `hashSecret()` de la version Node (essai 4 de la phase 0).
    #[test]
    fn hachages_de_la_version_node() {
        let node = "scrypt$32768$8$1$WHHrLHKqAu3y9WMsCyGq1A==$AerrISzZrRwDjZHDUXzligiH+kZxID2z41yZ8Jrnz4Xco0I6U4x78ujwQXXovsF2RrP817QmWvY3DeJv1tQQtw==";
        assert!(verify_secret("Café crème", node));
        assert!(
            verify_secret("Cafe\u{301} cre\u{300}me", node),
            "NFKC : forme décomposée"
        );
        assert!(!verify_secret("Café crème!", node));
        assert!(!verify_secret("x", "illisible"));
        let own = hash_secret("mot de passe");
        assert!(verify_secret("mot de passe", &own));
    }

    #[test]
    fn codes_et_permissions() {
        let code = human_code(5);
        assert!(
            regex::Regex::new("^([A-Z2-9]{4}-){4}[A-Z2-9]{4}$")
                .unwrap()
                .is_match(&code),
            "{code}"
        );
        assert_eq!(normalize_code("ab12-cd34"), "AB12CD34");
        let perms = normalize_permissions(&json!(["commands", "instance"])).unwrap();
        assert_eq!(perms, ["view", "operate", "edit", "commands", "instance"]);
        assert_eq!(role_of(&perms), "custom");
        assert_eq!(
            role_of(&normalize_permissions(&json!(["operate"])).unwrap()),
            "operator"
        );
        assert!(normalize_permissions(&json!(["root"])).is_err());
    }
}
