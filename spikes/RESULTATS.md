# Phase 0 : résultats des essais

Mesures du 24/09/2026 sur la machine de développement (Windows 11 Pro 26200, i7-12650H),
Rust 1.98.1, Tauri 2.11.6, WebView2 153. La version Node tournait en même temps sur le port 9000.

## Essai 4 : scrypt (`scrypt-node`) : concluant

- Les 7 hachages produits par `hashSecret()` de Node sont acceptés par Rust (crate `scrypt` 0.11,
  N=32768, r=8, p=1, 64 octets, NFKC via `unicode-normalization`). Un secret faux est refusé.
- La forme Unicode décomposée (`e` + accent combinant) valide le hachage de la forme précomposée :
  la normalisation NFKC est identique.
- Sens inverse : les 7 hachages produits par Rust sont acceptés par `verifySecret()` de Node.
- Refaire : `node gen-scrypt.mjs`, `cargo run --release -p scrypt-node`, `node check-scrypt.mjs`.

## Essai 2 : ports (`tcp-table`) : concluant

- `GetExtendedTcpTable` (TCP_TABLE_OWNER_PID_LISTENER, IPv4 puis IPv6) donne exactement le même
  ensemble que `netstat -ano` analysé comme dans `windows.js` : 61 ports, adresses au format de
  netstat (`0.0.0.0`, `127.0.0.1`, `[::]`, `[::1]`, adresses du réseau local).
- **0,9 ms au lieu de 79 ms** pour les deux appels à netstat.
- Seule différence : l'ordre des lignes. Le code actuel ne dépend que de « IPv4 avant IPv6 »
  (`pidOnPort` prend la première ligne) ; l'implémentation triera par famille, adresse, port.
- À faire en phase 3 : l'identifiant de portée des adresses IPv6 de lien local (`%12`) n'était
  pas présent sur la machine : test avec une sortie construite.

## Essai 1 : lancement caché (`hidden-launch`) : concluant sur une arborescence de test

`cmd.exe /d /s /c "chcp 65001 >nul & "node" "tree.mjs" …"` par `CommandExt::raw_arg`,
`CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`, sorties vers le journal ouvert en ajout,
entrée vide. Arborescence : node → cmd → ping, node → php -S (WAMP), conhost.

- **Aucune fenêtre visible** (surveillance de toutes les fenêtres de premier niveau, pour voir aussi
  une délégation à Windows Terminal).
- Journal : sorties standard et d'erreur des petits-enfants, accents en UTF-8.
- Lancement en 18 ms (contre environ 160 ms pour démarrer PowerShell).
- **Survie** : l'essai placé dans un job « tuer à la fermeture » qui permet le breakaway, puis
  arrêté : le projet continue de répondre. Job strict (breakaway interdit) : `CreateProcess`
  refuse (erreur 5), le repli sans le drapeau fonctionne, et le projet meurt avec le job, comme prévu.
- **Code de sortie d'un projet repris** : un autre processus ouvre le PID
  (`PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE`), attend et lit le code (7 attendu, 7 lu).
  C'est ce que la version Node ne sait pas faire.
- **Arrêt** sans taskkill (arborescence puis `TerminateProcess`) : 6 processus en 0,3 ms.

### Trouvé : héritage des handles

Le projet lancé hérite de **tous les handles héritables** de Servym (Rust appelle
`CreateProcessW` avec `bInheritHandles = TRUE`). Pendant l'essai, le tube de sortie de l'essai
(vers `grep`) est resté ouvert tant que le projet tournait. Rust crée ses propres fichiers et sockets
non héritables, mais un handle reçu du parent (superviseur, console, tube) serait transmis au projet
et le garderait ouvert.

**Décision pour la phase 3** : lancer par `CreateProcessW` avec
`PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (seuls le journal et l'entrée vide sont transmis), au lieu de
`std::process::Command`. C'est ce que font les lanceurs sérieux sous Windows.

### symfony

Bogue tournait sous la version Node : pour ne pas y toucher (ni à MySQL), `symfony server:start
--port=8123 --no-tls` a été lancé dans un dossier temporaire contenant un `public/index.php`. Même
mécanique que Bogue : symfony.exe → php.exe (détection des versions) → php-cgi.exe.

- 9 processus vus dans l'arborescence, **aucune fenêtre visible**, page servie (`symfony ok 8.2.18`).
- Journal : la sortie de symfony (couleurs ANSI comprises, comme aujourd'hui).
- Arrêt natif de l'arborescence : symfony, php-cgi, cmd, conhost ; Bogue (lancé par la version Node)
  toujours en ligne ensuite.

Le lancement de Bogue lui-même (avec MySQL) sera refait au jalon de la phase 4, dans le scénario
complet.

## Essai 3 : fenêtre Tauri (`tauri-window`) : concluant, deux décisions

Fenêtre sur `http://localhost:9000` (version Node en service), icône dans la zone de notification,
aucune capability. Piloté par le protocole de débogage de WebView2 (`drive.mjs`).

| Vérification | Résultat |
| --- | --- |
| Interface chargée | oui (20 tuiles, flux SSE actifs) |
| IPC depuis la page | refusée : « Command plugin:opener\|open_url not allowed by ACL » |
| `window.open` vers une autre origine | ouvert dans le navigateur par défaut, `null` rendu à la page |
| `window.open('/wait.html?id=bogue')` | petite fenêtre Tauri (même session), puis l'adresse du projet part dans le navigateur et la fenêtre se ferme |
| Export de la configuration (`blob:` + `download`) | enregistré dans Téléchargements, événements début et fin reçus |
| Navigation vers github.com puis retour | pages GitHub affichées dans la fenêtre, retour sur Servym, cookie gardé |
| Fermer la fenêtre | WebView détruite, l'application reste dans la zone de notification |

### Décision 1 : la page d'attente en mode fenêtre

`wait.html` lit `/api/events`, qui demande d'être connecté quand la protection est active. Ouverte
dans le navigateur, elle n'aurait pas la session de la fenêtre (cookies séparés). Solution validée :
une page de Servym ouverte par `window.open` s'affiche dans une petite fenêtre Tauri (mêmes
cookies) ; dès qu'elle quitte l'origine de Servym, l'adresse part dans le navigateur. L'interface
n'a pas à changer.

### Décision 2 : la mémoire de la fenêtre, plus élevée que prévu

| Fenêtre | Mémoire privée | Mémoire de travail (somme, pages partagées comptées plusieurs fois) |
| --- | --- | --- |
| Ouverte (7 processus WebView2) | **≈ 240 Mo** | ≈ 460 Mo |
| Fermée (application seule) | **5 Mo** | 39 Mo |

Le rapport annonçait 100 à 150 Mo (estimation) : c'est plutôt 240 Mo pendant que la fenêtre est
ouverte, dont 127 Mo pour le processus de rendu de la page. Un onglet de navigateur sur la même page
coûterait à peu près autant, mais il est partagé avec le reste du navigateur. D'où l'importance de
détruire la fenêtre à la fermeture (vérifié : tout est rendu) et de proposer le navigateur dans la
présentation.

### Non vérifié automatiquement

- Rouvrir la fenêtre depuis l'icône de la zone de notification (menu « Ouvrir Servym ») : code en
  place, à cliquer une fois à la main.
- Connexion GitHub complète : GitHub n'est pas configuré sur l'instance actuelle. L'aller-retour de
  navigation et la conservation des cookies, qui sont les deux conditions, sont vérifiés.

### Taille

`tauri-window.exe` : 8,8 Mo avec le profil par défaut, **5,7 Mo** avec le profil du projet
(`lto`, `opt-level = "s"`, `strip`). WebView2 est fourni par Windows. Pour comparer, `node.exe`
seul pèse 85,8 Mo.

## Autres constats

- La suite Node passe dans le worktree : **146 tests** à `bc7a6e8` (le rapport en comptait 144 ;
  les derniers commits en ont ajouté 2). La phase 1 portera les 146.
- Compilation de Tauri à froid : environ 2 minutes ; ensuite 40 s en release.
