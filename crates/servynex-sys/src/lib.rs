//! Accès au système : ports, processus, lancement, arrêt, réseau, ouverture de dossiers.
//!
//! Tout le code `unsafe` (Win32, COM, appels Linux) vit dans ce crate, derrière le trait `Platform`.
