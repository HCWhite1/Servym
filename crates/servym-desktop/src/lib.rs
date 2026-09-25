//! Application de bureau (Tauri 2) autour du serveur : une coquille, pas un remplacement.
//!
//! - Le serveur HTTP embarqué reste (navigateur, accès réseau, page d'attente, GitHub, sites
//!   statiques) ; la fenêtre charge `http://localhost:{port}` comme un onglet, sans aucune IPC Tauri
//!   ouverte à la page.
//! - Zone de notification : projets et actions (démarrer, arrêter, redémarrer, ouvrir), sans mot de
//!   passe, sur la machine hôte seulement ; inscrites au journal.
//! - Fermer la fenêtre la détruit (WebView2 rend sa mémoire) ; Servym reste dans la zone de
//!   notification. Fenêtre ou navigateur : réglage de la machine (`runtime/ui.json`).
//! - Superviseur : lancé sans `SERVYM_SUPERVISOR`, `Servym.exe` se relance lui-même comme
//!   enfant et le relance après un plantage ou un redémarrage demandé.
//! - Instance unique : un 2ᵉ lancement ouvre l'interface de la première.

pub mod data;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use app::run;
