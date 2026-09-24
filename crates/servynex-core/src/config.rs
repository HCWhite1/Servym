//! Configuration (`projects.json`) : validation, mise en forme, modifications.
//!
//! La configuration reste un document JSON (`serde_json::Value`, ordre des clés gardé) : les champs
//! inconnus et l'ordre écrit à la main survivent à chaque modification faite par l'interface.
//! Toutes les fonctions de modification rendent une copie ; l'original n'est jamais modifié.

use std::path::Path;

use serde_json::Value;

use crate::i18n::LocalError;

/// Configuration utilisée quand `projects.json` n'existe pas.
pub fn default_config() -> Value {
    serde_json::json!({ "instance": { "port": 9000 }, "projects": [] })
}

/// Vérifie toute la configuration (erreur 400 au message traduisible).
pub fn validate(config: &Value) -> Result<(), LocalError> {
    let _ = config;
    unimplemented!("phase 2")
}

/// Lit et valide un fichier ; fichier absent : configuration vide.
pub fn load_config(file: &Path) -> Result<Value, LocalError> {
    let _ = file;
    unimplemented!("phase 2")
}

/// Mise en forme compacte de `projects.json` (identique à `formatConfig` de la version Node).
pub fn format_config(config: &Value) -> String {
    let _ = config;
    unimplemented!("phase 2")
}

/// Écrit le fichier (fichier temporaire puis renommage).
pub fn save_config(file: &Path, config: &Value) -> std::io::Result<()> {
    let _ = (file, config);
    unimplemented!("phase 2")
}

/// Modifie un projet depuis l'interface : champs protégés, validation complète.
pub fn patch_project(config: &Value, id: &str, patch: &Value) -> Result<Value, LocalError> {
    let _ = (config, id, patch);
    unimplemented!("phase 2")
}

/// Ajoute un projet ; identifiant tiré du nom. Rend la configuration et l'identifiant.
pub fn create_project(config: &Value, input: &Value) -> Result<(Value, String), LocalError> {
    let _ = (config, input);
    unimplemented!("phase 2")
}

/// Copie d'un projet juste après l'original, avec un nouveau nom et un nouveau port.
pub fn duplicate_project(
    config: &Value,
    id: &str,
    name: &str,
    port: Option<u16>,
) -> Result<(Value, String), LocalError> {
    let _ = (config, id, name, port);
    unimplemented!("phase 2")
}

/// Retire un projet, et des dépendances des autres (erreur 404 s'il n'existe pas).
pub fn remove_project(config: &Value, id: &str) -> Result<Value, LocalError> {
    let _ = (config, id);
    unimplemented!("phase 2")
}

/// Nouvel ordre : tous les identifiants, chacun une fois (erreur 400 sinon).
pub fn reorder_projects(config: &Value, order: &Value) -> Result<Value, LocalError> {
    let _ = (config, order);
    unimplemented!("phase 2")
}

/// Différences lisibles entre deux versions d'un projet ; les valeurs de `env` sont masquées.
pub fn describe_changes(before: &Value, after: &Value) -> String {
    let _ = (before, after);
    unimplemented!("phase 2")
}
