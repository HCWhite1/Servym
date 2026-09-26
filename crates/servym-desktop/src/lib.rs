//! Application de bureau (Tauri 2, Windows et Linux) autour du serveur : une coquille, pas un
//! remplacement.
//!
//! - Le serveur HTTP embarqué reste (navigateur, accès réseau, page d'attente, GitHub, sites
//!   statiques) ; la fenêtre charge `http://localhost:{port}` comme un onglet, sans aucune IPC Tauri
//!   ouverte à la page.
//! - Zone de notification : projets et actions (démarrer, arrêter, redémarrer, ouvrir), sans mot de
//!   passe, sur la machine hôte seulement ; inscrites au journal.
//! - Fermer la fenêtre la détruit (le moteur web rend sa mémoire) ; Servym reste dans la zone de
//!   notification. Fenêtre ou navigateur : réglage de la machine (`runtime/ui.json`).
//! - Superviseur : lancé sans `SERVYM_SUPERVISOR`, l'application se relance elle-même comme
//!   enfant et la relance après un plantage ou un redémarrage demandé.
//! - Instance unique : un 2ᵉ lancement ouvre l'interface de la première.
//! - Démarrage avec la session : clé Run (Windows), service systemd utilisateur (Linux).

pub mod data;

#[cfg(any(windows, target_os = "linux"))]
mod app;
#[cfg(any(windows, target_os = "linux"))]
mod system;
#[cfg(any(windows, target_os = "linux"))]
mod tray;
#[cfg(any(windows, target_os = "linux"))]
mod window;

#[cfg(any(windows, target_os = "linux"))]
pub use app::run;
