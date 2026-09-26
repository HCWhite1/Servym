//! `auth` en ligne de commande, commun à `servym` et à l'application de bureau.

use std::path::Path;
use std::process::ExitCode;

use servym_core::auth::{AuthStore, create_reset_code, role_of};
use servym_core::config::load_config;
use servym_core::i18n::{Lang, detect_lang, tr};
use servym_core::params;

/// Même langue que le serveur : `instance.lang`, sinon celle du système.
fn cli_lang(data: &Path) -> Lang {
    let system = detect_lang(
        servym_sys::system_locale().as_deref(),
        std::env::var("SERVYM_LANG").ok().as_deref(),
    );
    load_config(&data.join("projects.json"))
        .ok()
        .and_then(|c| c["instance"]["lang"].as_str().and_then(Lang::pick))
        .unwrap_or(system)
}

/// Secours en ligne de commande pour les comptes, à lancer sur la machine. Avoir accès à cette
/// machine donne de toute façon la main sur Servym : ces commandes n'ouvrent aucune porte
/// nouvelle. Dernier recours : supprimer runtime/auth.json retire la protection (et les comptes).
pub fn auth_command(data: &Path, args: &[String]) -> ExitCode {
    let lang = cli_lang(data);
    let t = |key: &str, p| tr(key, &p, lang);
    let mut store = AuthStore::new(&data.join("runtime").join("auth.json"));
    if let Some(error) = store.error() {
        eprintln!("{}", t("auth.brokenFile", params![error = error.to_string()]));
        return ExitCode::FAILURE;
    }
    let command = args.first().map(String::as_str);
    match (command, args.get(1)) {
        (Some("list"), _) => {
            let data = store.load().clone();
            let users = data["users"].as_array().cloned().unwrap_or_default();
            if users.is_empty() {
                println!("{}", t("cli.noAccounts", params![]));
            } else {
                let key = if data["enabled"] == true {
                    "cli.summaryOn"
                } else {
                    "cli.summaryOff"
                };
                println!("{}", t(key, params![count = users.len()]));
                for u in users {
                    let username = u["username"].as_str().unwrap_or_default();
                    let perms: Vec<String> = u["permissions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.as_str().map(str::to_string))
                        .collect();
                    let role = t(&format!("role.{}", role_of(&perms)), params![]);
                    let name = u["name"].as_str().filter(|n| !n.is_empty() && *n != username);
                    let name = name.map(|n| format!(" ({n})")).unwrap_or_default();
                    println!("  {username:<20} {role}{name}");
                }
            }
            ExitCode::SUCCESS
        }
        (Some("reset-password"), Some(name)) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64);
            match create_reset_code(&mut store, &name.to_lowercase(), now) {
                Ok(code) => {
                    println!(
                        "{}",
                        t("cli.resetCode", params![name = name.as_str(), code = code])
                    );
                    println!("{}", t("cli.resetHelp", params![]));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{}", error.message_in(lang));
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            println!("{}", t("cli.usage", params![]));
            if command.is_some() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
