# Migration vers Rust : journal de bord

Plan complet : rapport « Servynex-migration-rust » (C:\audits). Ce fichier garde, phase par phase,
ce qui a été fait, les décisions prises en cours de route et les mesures. La version Node reste la
référence jusqu'à la bascule (phase 7).

## Phase 0 · Prototype des risques

Voir [spikes/RESULTATS.md](spikes/RESULTATS.md). Quatre essais concluants. Décisions :

- lancement par `CreateProcessW` avec la liste explicite des handles transmis (fuite de handles
  constatée avec `std::process::Command`) ;
- en mode fenêtre, une page de Servynex ouverte par `window.open` (page d'attente) s'ouvre dans une
  petite fenêtre Tauri qui garde la session ; le reste part dans le navigateur ;
- fenêtre par défaut malgré ≈ 240 Mo pendant qu'elle est ouverte (5 Mo fermée).

## Phase 1 · Harnais de tests

- API des quatre crates posée ; `App::builder()` reprend les points d'injection de `createApp()`.
- Applications factices en Rust (`servynex-fixture`) : plus besoin de Node pour les tests.
- 142 tests JS écrits en Rust, chacun nommé `/// JS : <fichier> › … › <test>` ;
  `tools/test-parity.mjs` vérifie la correspondance (4 tests de `urls.test.js` restent en JS).
- Écart assumé : les tests JS de « serveur et processus » s'enchaînent sur une même application ;
  en Rust, chacun prépare la sienne (indépendants, parallèles).
- Les tests du gestionnaire passent par l'API HTTP : ils ne peuvent passer qu'à la phase 5. La
  phase 4 ajoute ses propres tests au niveau du gestionnaire.

## Phase 2 · Cœur sans système

- Configuration portée à l'identique (règles, messages, longueurs en UTF-16, `3000.0` accepté).
- Formateur vérifié octet par octet contre `formatConfig` de Node : `tools/gen-format-golden.mjs`
  écrit les fichiers de référence de `crates/servynex-core/tests/golden/`.
- Textes du serveur en JSON (`locales/`) ; la langue du serveur appartient à l'application (plus
  de variable globale).
- scrypt compilé optimisé même en debug (sinon les tests prennent des minutes).
- `resolve_inside` découpe sur `/` et `\` partout (en Node, le résultat dépendait du système).

## Phase 3 · `servynex-sys`

Windows sans PowerShell ni netstat, tasklist ou taskkill :

| Opération | Avant (Node) | Rust natif (mesuré le 24/09/2026) |
| --- | --- | --- |
| Relevé des processus (339) | sonde PowerShell gardée ouverte (92 Mo) | 10,5 ms |
| Ports en écoute | 79 ms (netstat ×2) | 0,95 ms |
| Profils réseau | 692 ms (PowerShell) | 25 ms (COM `INetworkListManager`) |
| Lancement d'un projet | PowerShell par projet (≈ 75 Mo pendant toute la vie du projet) | `CreateProcessW`, un thread de 64 Ko qui attend la fin |
| Notification | PowerShell | WinRT natif |
| Démarrage avec Windows | tâche planifiée + VBS | clé Run de HKCU (`--supervised`), état « désactivé » du Gestionnaire des tâches respecté |

Décisions :

- **Deux cmd.exe par projet.** cmd.exe fixe l'encodage de ses propres messages à son démarrage
  quand sa sortie est un fichier : « commande introuvable » arrivait en CP850. Le premier cmd.exe
  fait `chcp 65001`, le second (démarré après) lance le projet ; ses messages sont en UTF-8. Le
  chemin du journal n'apparaît jamais dans la ligne de commande.
- Arrêt : même règle que le relevé (un PID réutilisé n'est pas dans l'arborescence), processus les
  plus récents d'abord, `TerminateProcess` (code 1, comme `taskkill /F`).
- Un projet repris après un redémarrage de Servynex : son code de sortie est lu par son PID
  (Windows). Sous Linux, un processus qui n'est pas notre enfant ne donne pas son code : inconnu.
- Notification sous Windows : identifiant d'application de PowerShell, comme avant (aucun raccourci
  à créer) ; l'application de bureau utilisera le sien (phase 6).
- Linux : fonctions d'analyse testées partout ; la plateforme réelle compile sous Unix (vérifié par
  `cargo clippy --target x86_64-unknown-linux-gnu`), exécutée par la CI.

Vérification à la main : `cargo run --release --example check -p servynex-sys [notify]`.
