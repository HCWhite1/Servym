//! Comptes, sessions et secrets (`runtime/auth.json`).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::i18n::LocalError;

/// Fichier des comptes, relu dès qu'il change sur le disque (commande `servynex auth`,
/// modification à la main).
pub struct AuthStore {
    file: PathBuf,
    data: Value,
    error: Option<String>,
    seen: Option<(SystemTime, u64)>,
}

impl AuthStore {
    pub fn new(file: &Path) -> Self {
        let _ = (&file, &Value::Null);
        unimplemented!("phase 2")
    }

    /// Relit le fichier s'il a changé (date ou taille) et rend les données.
    pub fn load(&mut self) -> &mut Value {
        unimplemented!("phase 2")
    }

    /// Enregistre (fichier temporaire puis renommage).
    pub fn save(&mut self) -> std::io::Result<()> {
        unimplemented!("phase 2")
    }

    /// Erreur de lecture du fichier (JSON invalide), s'il y en a une.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Date de modification déjà vue (tests : un changement fait dans le même instant qu'un
    /// enregistrement doit être vu grâce à la taille).
    pub fn set_seen_mtime(&mut self, mtime: SystemTime) {
        let _ = (&self.file, &self.data, &self.seen, mtime);
        unimplemented!("phase 2")
    }
}

/// Code de réinitialisation valable 15 minutes (commande `servynex auth`, ou un administrateur).
pub fn create_reset_code(store: &mut AuthStore, username: &str, now: i64) -> Result<String, LocalError> {
    let _ = (store, username, now);
    unimplemented!("phase 2")
}

/// Hachage scrypt au format de la version Node (`scrypt$N$r$p$sel$hachage`).
pub fn hash_secret(secret: &str) -> String {
    let _ = secret;
    unimplemented!("phase 2")
}

/// Vérifie un secret contre un hachage (durée constante, même pour un hachage illisible).
pub fn verify_secret(secret: &str, stored: &str) -> bool {
    let _ = (secret, stored);
    unimplemented!("phase 2")
}
