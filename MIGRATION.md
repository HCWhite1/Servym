# Migration vers Rust : journal de bord

Plan complet : rapport « Servynex-migration-rust » (C:\audits). Ce fichier garde, phase par phase,
ce qui a été fait, les décisions prises en cours de route et les mesures. La version Node a servi de
référence jusqu'à la bascule ; elle est retirée à la phase 7 (l'historique Git la garde).

**Servynex s'appelle désormais Servym** (renommé le 25/09/2026, avant d'être public : aucun
crate, paquet npm, dépôt ni domaine de ce nom). Les sections ci-dessous gardent l'ancien nom,
celui en vigueur au moment où elles ont été écrites.

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

## Phase 4 · Gestionnaire de processus

`crates/servynex-core/src/manager.rs` porte `processes.js` : mêmes états, mêmes règles (jetons
d'action, relance d'un projet qui se relance lui-même, lanceur sorti pendant le lancement,
dépendances en cascade, relance après plantage, « autostart » une fois par démarrage du PC).
L'état est derrière un verrou jamais tenu pendant une attente : le compilateur a refusé deux
endroits qui le faisaient, corrigés.

Nouveautés (tests dans `crates/servynex-core/tests/manager.rs`) :

- `pids.json` garde l'heure de création du processus (`createdAt`) : un PID réutilisé n'est jamais
  repris. Fichier de la version Node (sans `createdAt`) : repris seulement si le processus a été
  créé au plus 10 s après le lancement enregistré.
- Un projet repris qui plante passe en échec avec son code de sortie (Windows) ; la version Node y
  voyait une sortie propre.
- Chaque PID managed est surveillé : sa fin arrive avec son code, même pour la nouvelle instance
  d'un projet qui se relance lui-même.

Écarts et corrections trouvés en route :

- Un plantage immédiat (commande introuvable) peut être signalé avant que le PID soit enregistré :
  la sortie est gardée de côté et traitée dès l'enregistrement.
- `start()` pose l'état « starting » avant de rendre la main, comme en JavaScript (où c'était la
  partie synchrone de `#start`).
- Pendant un arrêt demandé, la sortie du processus est laissée à `stop` (en JavaScript, elle
  arrivait toujours après) : l'historique ne montre pas de code de sortie pour un arrêt.
- cmd.exe écrit « n’est pas reconnu » avec une apostrophe typographique en UTF-8 : le motif des
  lignes d'erreur accepte les deux apostrophes (`public/js/log-patterns.js` aussi, même motif pour
  l'interface).
- Sites statiques : servis par axum dans `servynex-core` (au lieu d'un module du serveur), puisque
  c'est le gestionnaire qui les ouvre et les ferme.
- Pile des fils qui attendent la fin des projets : 1 Mo réservé (64 Ko débordaient en debug).

Reste à faire au jalon de la phase 5 : le scénario réel (MySQL puis Bogue, MangaFlow exposé, un site
statique) comparé à l'historique de la version Node, quand Servynex Rust remplacera Node.

## Phase 5 · Serveur axum

`crates/servynex-server` porte `server.js`, `src/auth.js` et `src/oauth.js` : mêmes routes, mêmes
réponses JSON, mêmes protections (Host, Sec-Fetch-Site, `X-Servynex-Request`, Origin), même table
des permissions, mêmes codes (415, 413, 400…) et messages traduits. axum ne sert que de plomberie :
une seule fonction reçoit chaque requête et l'aiguille comme `server.js`, pour garder l'ordre exact
des contrôles. Interface (`public/`) embarquée dans le binaire (rust-embed).

Les **142 tests JavaScript portés passent tous** (plus 31 tests propres à la version Rust : 173).

Binaire `servynex` (6,3 Mo, interface comprise) :

- `servynex [--data <dossier>]` : données dans `--data`, sinon `SERVYNEX_HOME`, sinon le dossier
  courant (même disposition que la version Node : `projects.json`, `logs/`, `runtime/`, `config/`) ;
- `servynex --supervised` : superviseur (mêmes règles que `start-hidden.vbs`) ;
- `servynex auth list` et `servynex auth reset-password <nom>` (remplacent `npm run auth`).

Mesure du 24/09/2026 sur une copie des données réelles (7 projets, dont Bogue, MangaFlow et MySQL
repris) : **4,5 Mo de mémoire privée** (Node : 85 Mo, plus 92 Mo pour la sonde PowerShell) ;
`/api/ports` 0,26 s, `/api/stats` 0,23 s, `/api/network` 0,21 s.

Trouvé en route :

- **Connexion refusée lente sous Windows** : vers un port fermé, Windows réessaie pendant près
  d'une seconde (806 ms mesurées), là où Node répond en 4 ms. libuv désactive ces réessais pour la
  boucle locale (`SIO_TCP_INITIAL_RTO`) ; `servynex_sys::net` fait de même (0,9 ms). Sans cela,
  chaque sonde d'un projet arrêté et chaque recherche de port libre auraient été très lentes.
- Vue d'un projet : pas de champ `port` pour une page locale (comme `JSON.stringify` d'un champ
  indéfini).
- Un projet en cours de lancement ne peut pas être retiré de la configuration (import, relecture) :
  il serait perdu de vue.
- `instance.node` vaut `null` (la ligne « Node.js » des Paramètres sera renommée à la phase 7).
- Superviseur `vbs` accepté le temps de la transition (tâche planifiée actuelle).

Jalon (à faire avec toi) : `servynex.exe` remplace `node server.js` sur ta machine.

### Jalon (24/09/2026, 21 h 10)

`servynex.exe` a remplacé `node server.js` sur la machine de développement :

- installé dans `%LOCALAPPDATA%\Programs\Servynex\servynex.exe`, données inchangées dans
  `C:\Servynex` (`--data C:\Servynex`) ;
- tâche planifiée « Servynex » (VBS + Node) **désactivée**, pas supprimée ;
- démarrage avec Windows : clé Run de HKCU,
  `"…\servynex.exe" --supervised --data "C:\Servynex"` ;
- Bogue, MangaFlow et MySQL repris sans redémarrage (MySQL toujours dépendance de Bogue) ;
  `pids.json` réécrit avec l'heure de création des processus ;
- mémoire : superviseur 1,3 Mo et serveur 3,4 Mo de mémoire privée (Node : wscript 13 Mo, node
  85 Mo, sonde PowerShell 92 Mo, plus environ 75 Mo par PowerShell d'attente de projet).

Trouvé pendant la bascule : le superviseur détaché de sa console transmettait des sorties
invalides au serveur, dont le lancement échouait sans rien dire. Corrigé (sorties vers « nul »,
échec inscrit dans `servynex.log`).

Retour arrière : arrêter les deux `servynex.exe`, retirer la valeur « Servynex » de la clé Run
(`reg delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v Servynex /f`), réactiver la
tâche (`Enable-ScheduledTask -TaskName Servynex`) et la lancer (`Start-ScheduledTask -TaskName
Servynex`). Les fichiers de données restent lisibles par la version Node.

CI GitHub Actions entièrement verte (Rust sous Windows et Linux, suite Node sous Windows).

## Phase 6 · Application de bureau (Tauri)

`servynex-desktop` produit `Servynex.exe` (11,9 Mo) et un installateur NSIS par utilisateur
(3,4 Mo, sans droits administrateur). Tauri n'est construit que sous Windows ; ailleurs, le
binaire sans écran `servynex` reste la version à utiliser.

- **Coquille autour du serveur** : même serveur HTTP, même interface. La fenêtre charge
  `http://localhost:{port}` sans aucune capability : l'IPC de Tauri est refusée à la page (vérifié).
  Liens vers une autre origine : navigateur par défaut ; page d'attente : petite fenêtre qui garde
  la session ; GitHub : dans la fenêtre, pour que le cookie y arrive.
- **Superviseur intégré** : lancé sans `SERVYNEX_SUPERVISOR`, `Servynex.exe` se relance lui-même
  comme enfant (code partagé avec `servynex --supervised`, `servynex_server::supervisor`).
  Redémarrage demandé : code 75 (sans fenêtre) ou 76 (la fenêtre était ouverte, elle revient).
  Tauri rend toujours 0 à la sortie : le code demandé est gardé à part.
- **Instance unique** : un 2ᵉ lancement ouvre l'interface de la première, puis s'arrête.
- **Fenêtre détruite à la fermeture** : WebView2 rend toute sa mémoire (6 processus, environ
  200 Mo, puis aucun). Sans fenêtre, les deux processus de Servynex tiennent en 5,4 Mo privés
  (version de développement).
- **Zone de notification** : projets et actions (démarrer, arrêter, redémarrer, ouvrir),
  « Ouvrir Servynex », « Ouvrir dans le navigateur », « Démarrer avec Windows », « Quitter » ; icône
  rouge quand un projet est en échec. Les actions passent par le même chemin que l'interface et
  sont inscrites « via la zone de notification ».
- **Fenêtre ou navigateur** : réglage de la machine (`runtime/ui.json`, `opener`), proposé à la
  fin de la présentation et dans Paramètres › Général, devant la machine seulement. La fenêtre
  se signale à la page par `window.servynexWindow` (une valeur, aucune fonction native).
- **Mot de passe oublié devant la machine** : « Générer un code sur cette machine » (Q2).
- **Notifications** au nom de l'application une fois installée (le raccourci du menu Démarrer
  déclare son identifiant), sinon au nom de Windows PowerShell comme avant.

### Données

Dossier des données : `--data`, `SERVYNEX_HOME`, mode portable (`projects.json` à côté de
l'exécutable), sinon `%LOCALAPPDATA%\Servynex`. L'installateur pose aussi le programme dans ce
dossier : c'est sans risque, sa désinstallation ne supprime que `Servynex.exe` et
`uninstall.exe` (le dossier n'est retiré que s'il est vide).

### Bascule de la machine de développement (25/09/2026)

Un import a servi une seule fois : au premier lancement, `Servynex.exe` a repris `C:\Servynex`
(projets, comptes, réglages, journaux) dans `%LOCALAPPDATA%\Servynex`, arrêté l'ancien
`servynex --supervised` sans ses projets, désactivé la tâche planifiée, et suivi le journal des
projets repris resté dans l'ancien dossier. Bogue, MangaFlow et MySQL ont été repris sans
redémarrage. Servynex n'ayant pas d'autre utilisateur, ce code a ensuite été retiré (commit
« Import retiré ») : il ne se déclencherait plus jamais. L'historique Git le garde.

Trouvé en route : la clé Run était dite « activée » dès qu'une valeur « Servynex » existait ; elle
ne l'est plus que si elle lance bien cet exécutable (sinon l'application aurait cru démarrer avec
Windows alors que la clé lançait encore l'ancien `servynex.exe`).

Trouvé par la CI (Linux) : une sonde gardait l'état lu avant son contrôle de santé ; si une
relance automatique commençait **et aboutissait** pendant ce contrôle, la sonde écrasait « en
ligne » par un « arrêté » périmé, et le projet relancé tournait sans surveillance. La sonde
abandonne désormais son relevé si l'état a changé entre-temps. Côté Node, le nettoyage d'un test
de relance réessaie la suppression du dossier (verrouillé un instant sous Windows).

## Renommage en Servym (25/09/2026)

Sur la branche Rust seulement (la version Node sera retirée à la phase 7) : crates `servym-*`,
binaires `servym` et `Servym.exe`, variables `SERVYM_*`, en-tête `X-Servym-Request`, cookie
`servym_session`, identifiant `io.github.hcwhite1.servym`, données dans `%LOCALAPPDATA%Servym`,
valeur « Servym » de la clé Run, textes de l'interface. Nom vérifié libre sur crates.io, npm,
GitHub et les domaines usuels. Effets : cookie renommé et nouveau dossier WebView2, donc une
reconnexion dans le navigateur et dans la fenêtre.

## Phase 7 · Bascule et publication (26/09/2026)

### Textes de l'interface

- « Tâche planifiée » devient « Démarrage avec Windows » (« Démarrage avec la session » sous Linux) :
  description du réglage, état illisible, présentation. La ligne « Lancement » des Paramètres dit
  « Application de bureau », « Démarrage avec Windows » (superviseur) ou « À la main (servym) ».
- `npm run auth -- reset-password <nom>` devient `servym auth reset-password <nom>`. L'application
  de bureau accepte aussi `auth` (même code, déplacé dans `servym_server::cli`) : sans `servym`
  installé, la commande reste possible (`Servym.exe auth …`, qui emprunte la console du terminal).
- La ligne « Node.js » devient « Moteur » : l'API rend `instance.engine` au lieu de `node` (`Rust`,
  ou `Rust · Tauri 2.11.6 · WebView2 …` / `WebKitGTK …` pour l'application de bureau).
- Plus aucune mention d'`autostart.bat`. Le superviseur `vbs` (transition depuis la version Node)
  n'est plus reconnu.

### Backend Node retiré

`server.js`, `src/`, `scripts/`, `package.json`, `autostart.bat`, les tests JavaScript du serveur,
`tools/test-parity.mjs` et `tools/gen-format-golden.mjs` (les fichiers de référence qu'il a écrits
restent). Gardés : `public/` (l'interface, embarquée dans les exécutables), `tests/urls.test.mjs`
(le seul test JavaScript : code de l'interface, lancé par `node --test` en CI), `tools/gen-icons.mjs`
(icônes, indépendant du serveur) et `spikes/` (essais de la phase 0). La CI n'a plus de tâche
« Node » pour le serveur.

### Application de bureau sous Linux

`servym-desktop` se construit sous Windows et Linux (Tauri + WebKitGTK), avec le même choix
fenêtre ou navigateur. Ce qui diffère est rangé dans `crates/servym-desktop/src/system.rs` :

- **Démarrage avec la session** : service systemd utilisateur `servym`, variante « bureau »
  (`servym_sys::linux::Unit { graphical: true }`) : `After=`/`PartOf=`/`WantedBy=graphical-session.target`
  (il faut l'écran), `--background`, sans `SERVYM_SUPERVISOR` : l'application se supervise elle-même
  comme sous Windows (redémarrage demandé avec la fenêtre rouverte). Lancée depuis une AppImage,
  c'est le fichier `$APPIMAGE` qui est inscrit, pas le dossier monté le temps de sa vie.
- **Notifications** : `notify-send` (déjà dans `servym-sys`).
- **Message d'erreur** (port occupé…) : zenity, sinon kdialog, sinon une notification ; toujours
  sur la sortie d'erreur.
- **Zone de notification** : libayatana-appindicator. Sous Linux, pas d'évènement de clic : un clic
  ouvre le menu. Libellé « Démarrer avec la session ».
- **Données** : `$XDG_DATA_HOME/servym`, sinon `~/.local/share/servym` (même ordre de recherche
  qu'ailleurs : `--data`, `SERVYM_HOME`, mode portable).
- **Paquets** : `.rpm` (Fedora, ajouté après le premier essai), `.deb` (4,8 Mo ; dépendances WebKitGTK, GTK, appindicator ajoutées par Tauri,
  `libnotify-bin` recommandé) et AppImage (83 Mo, GTK et WebKitGTK compris). Exécutable
  `servym-desktop` (pour ne pas se confondre avec `servym` sans écran), entrée de menu « Servym ».

Le binaire sans écran `servym` reste pour les serveurs sans bureau.

Trouvé en route :

- Le service de `servym` sans écran avait pour dossier de travail celui de l'exécutable, et ignorait
  `--data` : il lance désormais `servym --data <dossier>`. Le démarrage avec la session n'est plus
  une méthode de `Platform` (il dépend du programme et des données, pas du système) ; il est
  construit par le serveur (`servym`) ou l'application de bureau.
- Service « activé » seulement s'il lance ce programme avec ces données (ligne `ExecStart`), comme
  la clé Run à la phase 6 : un service laissé par l'autre version (bureau ou sans écran) ne compte
  pas, et l'activer le remplace (`reenable`, les liens de l'ancienne cible sont refaits).
- `$` et `"` protégés dans `ExecStart` (variables de systemd).

### Vérifications

- Windows : `cargo clippy --workspace --all-targets -D warnings`, `cargo fmt --check`, 183 tests.
- Linux (conteneur Ubuntu 22.04, dépendances WebKitGTK de la CI) : clippy et tous les tests passent ;
  `.deb`, AppImage et `servym` construits par `cargo tauri build`.
- Pas encore essayé : l'application de bureau sur une vraie machine Linux avec bureau (liste des
  vérifications ci-dessous).

### CI et release

- CI : dépendances de Tauri installées sous Linux (Ubuntu 22.04), l'application de bureau est donc
  compilée, vérifiée par clippy et testée sous Windows et Linux ; tâche « Interface (Node) » pour
  `tests/urls.test.mjs`.
- `.github/workflows/release.yml` : une étiquette `v*` construit l'installateur NSIS, le `.deb`,
  l'AppImage et `servym_<version>_linux_x86_64.tar.gz`, puis prépare une release **en brouillon**
  avec `SHA256SUMS.txt`. Elle n'est publiée qu'à la main.
- Installateur non signé : le README explique l'avertissement SmartScreen et la vérification de
  l'empreinte.

### Mesures (26/09/2026)

Même machine, données vides, `node server.js` (Node 24) contre `servym` (release) :

| | Node | Rust |
| --- | --- | --- |
| Démarrage jusqu'à la 1re réponse de `/api/projects` | 0,7–0,8 s | 0,2 s |
| Mémoire privée, aucune page | 31 Mo | 4,5 Mo |
| Mémoire privée, liste affichée | 109 Mo (avec la sonde PowerShell) | 4,6 Mo |
| `/api/ports` | 790 ms | 33 ms |
| `/api/autostart` | 2 450 ms | 11 ms |
| `/api/network` | 18 ms (en cache) | 14 ms |

Installation réelle (application de bureau, fenêtre fermée, 7 projets) : superviseur 1,4 Mo et
application 6,9 Mo de mémoire privée. Tailles : `Servym.exe` 11,9 Mo, installateur 3,4 Mo (phase 6),
`servym.exe` 6,4 Mo, `servym` Linux 5,8 Mo.

Incident pendant les mesures : un arrêt par nom (`taskkill /IM servym.exe`, insensible à la casse)
a aussi arrêté le Servym installé, et un arrêt des `node … server.js` a arrêté MangaFlow
(`node server/server.js`). Servym a été relancé comme le fait la clé Run, puis MangaFlow par son
bouton « Démarrer ». Leçon : n'arrêter que les PID qu'on a lancés.

### Premier essai sous Fedora (26/09/2026)

Retour : le `.deb` ne sert à rien sous Fedora, et l'AppImage « ne fait rien ». Reproduit dans un
conteneur Fedora avec un écran virtuel (Xvfb) :

- **AppImage** : le serveur démarre, mais WebKitGTK s'arrête (`Could not create default EGL
  display: EGL_BAD_PARAMETER. Aborting...`) et la fenêtre reste vide. Cause : les `libwayland-*`
  d'Ubuntu 22.04 embarquées par linuxdeploy ne s'accordent pas avec le Mesa de Fedora (vérifié :
  sans elles, l'interface s'affiche ; avec elles, même paquets installés, l'erreur revient).
  `LINUXDEPLOY_EXCLUDED_LIBRARIES` est ignorée par la version utilisée par Tauri :
  `tools/fix-appimage.sh` les retire après coup et refait l'AppImage (appimagetool), dans le
  workflow de release. Vérifié sous Fedora et Ubuntu 24.04.
- **Paquet `.rpm`** ajouté (Tauri le construit depuis Ubuntu) : dépendances `webkit2gtk4.1`,
  `gtk3`, `libayatana-appindicator-gtk3`, `libnotify` recommandé. Installé par `dnf` dans le
  conteneur Fedora : l'interface s'affiche dans la fenêtre, la zone de notification se charge.
- **Rien ne se voyait** : le superviseur jetait la sortie d'erreur. Elle va désormais dans
  `logs/servym.stderr.log` (vidé à chaque démarrage du superviseur), chaque plantage est inscrit
  dans `servym.log`, une panique aussi, et après 5 plantages rapides un message l'annonce au lieu
  de rien. Une zone de notification indisponible n'arrête plus l'application (inscrit au journal).

### Reste

- Essai de l'application de bureau sur une vraie machine Linux.
- `rust-rewrite` devient `main` (26/09/2026), étiquette `v1.0.0` (première release : la version Node n'a jamais été publiée ; version du workspace passée de 2.0.0 à 1.0.0), publication de la release préparée par la CI.
