# Servym

[English](README.md) · **Français**

Tableau de bord local pour lancer, surveiller et arrêter vos projets de développement : applications Node, PHP ou Python, sites statiques, bases de données. Une application de bureau légère pour Windows et Linux, écrite en Rust avec [Tauri](https://tauri.app), et la même interface dans le navigateur sur http://localhost:9000.

- Lancer un projet (et ses dépendances) en un clic, ouvrir son adresse dès qu'il répond.
- Journal en direct, historique des démarrages et des plantages, état Git, processeur et mémoire.
- Ports en écoute sur la machine, projets exposés sur le réseau, alerte sur les réseaux publics.
- Icône dans la zone de notification : démarrer, arrêter, ouvrir un projet sans ouvrir la fenêtre.
- Démarrage avec Windows ou avec la session Linux, projets lancés au démarrage du PC.
- Comptes facultatifs, interface en français et en anglais.

Servym a d'abord été écrit en Node.js (jamais publié) ; cette réécriture en Rust garde la même interface et les mêmes fichiers de configuration, avec un seul exécutable sans prérequis et une empreinte mémoire bien plus faible (voir [Mesures](#mesures)).

## Installation

Les fichiers sont joints à chaque [release](https://github.com/HCWhite1/Servym/releases), avec leurs empreintes SHA-256 (`SHA256SUMS.txt`).

### Windows 10 et 11

Télécharger et lancer `Servym_<version>_x64-setup.exe`. L'installation se fait pour l'utilisateur courant, sans droits administrateur : le programme et ses données vont dans `%LOCALAPPDATA%\Servym`. WebView2, le moteur de la fenêtre, est déjà présent dans Windows 10 et 11.

**Avertissement SmartScreen.** L'installateur n'est pas signé (un certificat de signature de code est payant) : au premier lancement, Windows affiche « Windows a protégé votre ordinateur ». Cliquer sur **Informations complémentaires**, puis **Exécuter quand même**. Pour vérifier le fichier avant de passer outre : `Get-FileHash .\Servym_<version>_x64-setup.exe` doit donner l'empreinte indiquée dans `SHA256SUMS.txt`. Les fichiers de la release sont construits par GitHub Actions à partir de ce dépôt (`.github/workflows/release.yml`) ; on peut aussi construire Servym soi-même (voir [Depuis les sources](#depuis-les-sources)).

Désinstaller : Paramètres de Windows › Applications. Seul le programme est retiré ; les données restent dans `%LOCALAPPDATA%\Servym`.

### Linux avec bureau

Trois formats, pour x86_64 :

- **Debian, Ubuntu et dérivés** : `sudo apt install ./Servym_<version>_amd64.deb`, puis Servym dans le menu des applications (ou `servym-desktop`). Les dépendances (WebKitGTK 4.1, GTK 3, libayatana-appindicator) sont installées avec le paquet.
- **Fedora et dérivés** : `sudo dnf install ./Servym-<version>-1.x86_64.rpm`, même chose (dépendances : `webkit2gtk4.1`, `gtk3`, `libayatana-appindicator-gtk3`).
- **Toutes distributions** : `Servym_<version>_amd64.AppImage`, à rendre exécutable (`chmod +x`) puis lancer. Elle embarque WebKitGTK ; les paquets `.deb` et `.rpm` utilisent celui du système et restent préférables.

La fenêtre utilise WebKitGTK. L'icône de la zone de notification passe par AppIndicator : sous GNOME, elle demande l'extension « AppIndicator and KStatusNotifierItem Support » (activée d'office sous Ubuntu ; sous Fedora : `sudo dnf install gnome-shell-extension-appindicator`, puis se reconnecter et l'activer dans Extensions) ; KDE, Xfce, Cinnamon et MATE l'affichent directement. Les notifications passent par `notify-send` (paquet `libnotify-bin`, `libnotify` sous Fedora).

Si rien ne s'affiche : `~/.local/share/servym/logs/servym.log` (journal de Servym) et `servym.stderr.log` (sortie d'erreur de GTK et WebKit), dans le même dossier. Après des plantages répétés au démarrage, un message le signale.

### Linux sans écran (serveur)

`servym_<version>_linux_x86_64.tar.gz` contient `servym`, le serveur seul, sans fenêtre ni dépendance graphique : l'interface s'ouvre dans un navigateur.

```
tar -xzf servym_<version>_linux_x86_64.tar.gz
./servym --data ~/servym
```

Il demande `ss` (paquet iproute2). Paramètres › Général › Démarrer avec la session crée le service systemd utilisateur `servym` ; pour qu'il démarre au boot sans ouvrir de session : `loginctl enable-linger`.

### Depuis les sources

Rust stable (`rustup`), puis :

```
cargo build --release -p servym-server        # target/release/servym : sans écran
cargo install tauri-cli --version "^2" --locked
cd crates/servym-desktop && cargo tauri build  # installateur NSIS, ou .deb, .rpm et AppImage
```

Sous Linux, l'application de bureau demande les paquets de développement de WebKitGTK : `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`.

## Premier lancement

Une présentation propose de démarrer Servym avec Windows (ou la session), de choisir les projets à lancer au démarrage du PC, puis, dans l'application de bureau, où ouvrir Servym : dans sa fenêtre ou dans le navigateur. Le bouton **Ajouter** crée ensuite un premier projet.

- **Fenêtre ou navigateur** : la fenêtre occupe environ 200 Mo tant qu'elle est ouverte ; fermée, elle est détruite et sa mémoire rendue, Servym reste dans la zone de notification. Le navigateur n'ajoute qu'un onglet. Le choix se change dans Paramètres › Général et vaut pour cette machine ; http://localhost:9000 reste toujours valable.
- **Zone de notification** : la liste des projets avec leurs actions (démarrer, arrêter, redémarrer, ouvrir), « Ouvrir Servym », « Ouvrir dans le navigateur », « Démarrer avec Windows » et « Quitter » (les projets continuent de tourner). L'icône passe au rouge quand un projet est en échec. Sous Windows, un clic gauche ouvre Servym ; sous Linux, un clic ouvre le menu.
- **Démarrage avec Windows** : Paramètres › Général ou menu de l'icône. Servym est inscrit dans la clé Run de l'utilisateur (sans droits administrateur) et démarre sans fenêtre à l'ouverture de session ; le Gestionnaire des tâches › Applications de démarrage peut le désactiver, ce que Servym respecte. Sous Linux : service systemd utilisateur `servym`, lancé avec la session graphique.
- **Redémarrer Servym** : Paramètres › Servym. Les projets lancés continuent de tourner et sont repris. L'application de bureau se relance elle-même après un plantage (elle abandonne après 5 plantages rapides de suite).
- **Une seule instance** : relancer Servym ouvre simplement l'interface de celui qui tourne.

## Données

Servym garde tout dans un dossier : `projects.json`, `runtime/` (comptes, état des projets lancés, réglages de la machine), `logs/` et `config/`. Dans l'ordre :

1. `--data <dossier>` sur la ligne de commande ;
2. la variable `SERVYM_HOME` ;
3. mode portable : le dossier de l'exécutable, s'il contient un `projects.json` ;
4. sinon `%LOCALAPPDATA%\Servym` sous Windows, `~/.local/share/servym` sous Linux (`$XDG_DATA_HOME/servym`).

Le binaire sans écran `servym` utilise `--data`, `SERVYM_HOME`, sinon le dossier courant.

Les fichiers de l'ancienne version Node.js sont lus tels quels : il suffit de copier `projects.json`, `runtime/`, `logs/` et `config/` dans le dossier des données. Les projets encore en marche sont repris sans redémarrage.

## Mesures

Même machine (Windows 11, 16 cœurs), le 26/09/2026. Ancienne version : `node server.js` (Node.js 24). Version Rust : `servym` (release). Données vides, moyenne de trois appels pour les routes.

| | Node.js | Rust |
| --- | --- | --- |
| Démarrage jusqu'à la 1re réponse de `/api/projects` | 0,7 à 0,8 s | 0,2 s (0,7 s au tout premier lancement) |
| Mémoire privée, aucune page ouverte | 31 Mo (node, conhost) | 4,5 Mo |
| Mémoire privée, liste affichée (relevés toutes les 5 s) | 109 Mo (node, conhost, sonde PowerShell) | 4,6 Mo |
| Page Réseau : `/api/ports` | 790 ms | 33 ms |
| Page Réseau : `/api/network` | 18 ms (en cache) | 14 ms |
| Paramètres : `/api/autostart` | 2 450 ms (tâche planifiée, PowerShell) | 11 ms (registre) |
| `/api/stats`, premier relevé puis suivants | 191 ms puis 13 ms | 20 ms puis 11 ms |
| Exécutable | node.exe (86 Mo) et les sources | `servym.exe` 6,4 Mo, `servym` Linux 5,8 Mo |

Avec de vrais projets (7 projets, dont MySQL, Bogue et MangaFlow repris ; mesures du 24 au 26/09/2026) :

| | Node.js | Rust |
| --- | --- | --- |
| Processus de Servym | wscript, node, conhost, sonde PowerShell | superviseur et application |
| Mémoire privée, fenêtre ou page fermée | ≈ 98 Mo (wscript 13, node 85) | 8,3 Mo (superviseur 1,4, application 6,9) |
| Pendant l'affichage de la liste | + 92 Mo (sonde PowerShell des relevés) | rien de plus (relevés natifs) |
| Par projet lancé | ≈ 75 Mo (un PowerShell qui attend la fin du projet) | un fil de 1 Mo réservé, quelques Ko utilisés |
| Fenêtre ouverte | — (onglet du navigateur) | ≈ 200 Mo (WebView2), rendus à la fermeture |
| Prérequis | Node.js 22 (node.exe : 86 Mo) et `git clone` | aucun |
| Distribution | dépôt Git et `npm start` | installateur Windows de 3,4 Mo (programme de 11,9 Mo) ; Linux : `.deb` et `.rpm` de 4,8 Mo, AppImage de 83 Mo (GTK et WebKitGTK compris) |

Opérations du système mesurées seules (24/09/2026) : ports en écoute 79 ms (deux `netstat`) contre 0,95 ms (`GetExtendedTcpTable`) ; profils réseau 692 ms (PowerShell) contre 25 ms (COM) ; relevé de 339 processus : requête CIM de 169 ms contre 10,5 ms. Connexion refusée vers un port fermé : Windows réessaie près d'une seconde ; comme Node (libuv), Servym désactive ces réessais sur la boucle locale (0,9 ms), sinon chaque sonde d'un projet arrêté serait lente.

## Projets

Tout se règle depuis l'interface : bouton **Ajouter**, puis l'onglet **Réglages** du volet d'un projet (clic sur sa ligne).

- **Supprimer** : en bas des Réglages du volet, ou menu « … » de la ligne. Le projet est arrêté d'abord, retiré des dépendances des autres ; ses journaux sont gardés. Confirmation par le mot de passe du compte si la protection est active, sinon en retapant le nom du projet.
- **Réordonner** : glisser la poignée à gauche d'une ligne, ou flèches haut et bas quand elle a le focus.

La configuration est enregistrée dans `projects.json`, dans le dossier des données. `projects.example.json` montre un exemple de chaque type de projet. Sans `projects.json`, Servym démarre avec une liste vide.

Le fichier peut aussi se modifier à la main (Paramètres › Configuration › Modifier) : Servym le relit dès qu'il est enregistré et stable depuis une seconde, et inscrit les changements à l'Activité (le bouton Relire fait de même tout de suite). Un projet en marche retiré à la main est gardé jusqu'à son arrêt, puis le changement s'applique. Une modification faite à la main n'est jamais écrasée par l'interface : chaque enregistrement repart du fichier tel qu'il est sur le disque. Si le fichier devient invalide, Servym continue avec la dernière configuration valide (`runtime/projects.valid.json`) et affiche l'erreur, avec sa ligne et sa colonne.

La section `instance` règle Servym lui-même :

| Champ | Rôle |
| --- | --- |
| `port` | Port de la page (9000 par défaut ; la variable `SERVYM_PORT` prime). |
| `name` | Nom affiché de l'instance (barre latérale, page de connexion). |
| `access` | `"local"` (par défaut) ou `"lan"` : page ouverte aussi au réseau local (voir « Sécurité »). |
| `lang` | `"fr"` ou `"en"` : langue des journaux et des messages du serveur. Par défaut : la variable `SERVYM_LANG`, sinon la langue du système. |

Chaque projet :

| Champ | Rôle |
| --- | --- |
| `id` | Identifiant (lettres minuscules, chiffres, tirets), tiré du nom à l'ajout. Nomme le journal : il ne change plus ensuite. |
| `name`, `desc`, `tags`, `icon`, `color` | Affichage. `icon` : nom d'une icône de `public/icons.svg` (Lucide, `book-open`…) ; toute autre valeur affiche l'initiale du nom. |
| `type` | Absent : programme lancé par une commande. `"static"` : Servym sert lui-même les fichiers de `cwd` sur `port`. `"file"` : page ouverte dans le navigateur, sans serveur (`open`). |
| `cwd` | Dossier du projet (chemin absolu). |
| `cmd`, `args` | Commande de lancement et ses arguments. `{port}`, `{host}` et `{servym}` (le dossier des données) sont remplacés. Sous Windows, les arguments passent par `cmd.exe` : `" % & \| < > ^ ( )` et les retours à la ligne y sont refusés. Sous Linux, la commande est lancée directement, sans shell. |
| `env` | Variables d'environnement ajoutées au lancement. `PORT` et `HOST` restent fixés par Servym. |
| `port` | Port du projet, transmis dans la variable `PORT`. Deux projets ne peuvent pas avoir le même. |
| `hostControl` | `env` : le projet lit `HOST`. `args` : l'adresse passe par `{host}`. `local` : le projet écoute toujours en local. |
| `expose` | `false` : écoute sur 127.0.0.1. `true` : sur tout le réseau (confirmation demandée dans l'interface). |
| `url` | Adresse ouverte par « Ouvrir » (`http://localhost:{port}`). |
| `health` | Contrôle « le projet répond » : `{ "path": "/", "expect": "<title>Mon site" }` (page web, texte attendu facultatif) ou `{ "type": "tcp" }` (port ouvert). |
| `startTimeout` | Secondes accordées au démarrage (60 par défaut). |
| `requires` | Projets à démarrer avant celui-ci. Démarrés pour lui, ils s'arrêtent avec lui s'ils ne servent plus à aucun autre projet en marche. Les dépendances en boucle sont refusées. |
| `service` | Rangé dans « Services » (base de données, outil) et dessiné en lune dans la vue orbite. |
| `stop` | Commande d'arrêt propre, essayée avant l'arrêt forcé : `{ "cmd": "mysqladmin", "args": ["shutdown"] }`. |
| `autostart` | Lancé une fois par démarrage du PC (un projet arrêté à la main ne revient pas si Servym redémarre). |
| `restartOnCrash` | Relancé s'il plante (programme seulement, voir « Relance automatique »). |
| `fixedPort` | Texte qui explique pourquoi le port ne doit pas changer ; l'interface le verrouille. Fichier seulement. |
| `open` | Pour `type: "file"` : fichier à ouvrir, relatif à `cwd`. |

## Fonctionnement

- **Lancement** : sous Windows, `CreateProcessW` démarre le projet dans une console cachée, hors du « job » de Servym, en ne lui transmettant que ses propres sorties ; ses sous-processus n'ouvrent aucune fenêtre. Sous Linux, le projet est lancé sans shell, dans son propre groupe de processus. Dans les deux cas, il continue de tourner quand Servym s'arrête : au démarrage suivant, Servym reprend les projets listés dans `runtime/pids.json`, avec l'heure de création de chaque processus (un PID réutilisé par un autre programme n'est jamais repris).
- **Statut** : `stopped`, `starting`, `running`, `stopping`, `failed` ou `conflict` (le port répond, mais ce n'est pas le bon site). Un projet arrêté est sondé toutes les 3 s, un projet en ligne toutes les 15 s.
- **Lancé ailleurs** : un projet qui répond sans avoir été lancé par Servym (tâche planifiée, fichier `.bat`) est affiché « externe ». Pour l'arrêter, il faut confirmer : Servym retrouve alors son PID par le port. Un processus lancé en administrateur ne peut pas être arrêté par Servym.
- **Projet qui se relance lui-même** : s'il quitte avec le code 0 et relance une instance qui garde la même sortie, Servym attend 30 s, retrouve le nouveau PID par le port et continue de le suivre.
- **Lanceur qui quitte pendant le lancement** (code 0) : Servym attend 10 s que le port réponde (serveur laissé en arrière-plan). Sinon, le projet passe en échec avec la ligne de son journal qui l'explique, par exemple le serveur Symfony déjà lancé depuis le même dossier.
- **Relance automatique** (`restartOnCrash`, Réglages du projet, désactivée par défaut) : un projet en ligne qui plante (code de sortie autre que 0) est relancé après 2 s, puis 10 s, puis 30 s. S'il plante encore après ces 3 relances en 5 minutes, abandon : le projet reste en échec, et une notification l'annonce si elles sont activées ; seul l'abandon en envoie une. Jamais après un échec au lancement (port pris, commande introuvable…) ni après un arrêt demandé ; « Arrêter » annule une relance prévue, et un lancement à la main remet le compte à zéro. Chaque relance est inscrite à l'historique. Sous Windows, le code de sortie d'un projet repris après un redémarrage de Servym est connu aussi ; sous Linux, seulement pour les projets lancés par cette instance.
- **Arrêt** : commande `stop` si le projet en déclare une, puis arrêt de toute l'arborescence de processus (Windows : les plus récents d'abord ; Linux : `SIGTERM`, puis `SIGKILL` après 3 s). Arrêter un projet dont d'autres projets en marche dépendent demande une confirmation.
- **Retirer un projet** de `projects.json` pendant qu'il tourne est refusé : il faut l'arrêter d'abord, sinon il continuerait sans surveillance.
- **Processeur et mémoire** : toute l'arborescence de processus d'un projet est comptée, ainsi que le processus qui écoute sur son port. Le processeur est une part de toute la machine, comme dans le Gestionnaire des tâches. Relevé toutes les 5 s, seulement quand une page est visible et qu'un projet tourne, par des appels directs au système (Windows) ou `/proc` (Linux). Le résumé des projets affiche aussi le total de Servym et de ses projets, chaque processus compté une fois.
- **Historique** : chaque changement d'état est écrit dans `logs/servym.log` ; l'onglet « Historique » d'un projet et la page « Activité » le relisent.
- **Réseau** : la page « Réseau » liste tout ce qui écoute sur la machine, rattaché aux projets, avec les adresses de la machine et, sous Windows, le profil (public / privé) de chaque réseau.

Journaux : `logs/servym.log` (le précédent passe dans `logs/servym.prev.log` au-delà de 1 Mo) et `logs/<id>.log` pour chaque projet (lancement précédent dans `logs/<id>.prev.log`).

### Linux

- Demande `ss` (iproute2), et `xdg-open` (xdg-utils) sur un bureau.
- **VS Code et terminal** : `code` dans le `PATH` ; premier terminal trouvé parmi Ptyxis, GNOME Terminal, Konsole, Xfce Terminal, kitty, Alacritty et XTerm.
- **Démarrer avec la session** : service systemd utilisateur `servym` (`~/.config/systemd/user/servym.service`, sans droits administrateur), relancé s'il plante ; l'arrêter laisse tourner les projets (`KillMode=process`). L'application de bureau démarre avec la session graphique (`graphical-session.target`) ; `servym` sans écran, avec la session de l'utilisateur (`default.target`). Un seul des deux à la fois : activer l'un remplace l'autre.
- **Sans écran** (serveur) : rien ne s'ouvre sur la machine ; dossier, VS Code, terminal et fichiers locaux sont masqués.
- Pas de profils réseau comme sous Windows : le pare-feu (firewalld…) se règle à part.

## Langues

L'interface existe en français et en anglais : la langue du navigateur est choisie automatiquement, et Paramètres › Apparence › Langue permet d'en fixer une (gardée par le navigateur, et par le compte si la protection est active). Les messages d'erreur du serveur suivent la langue de la page. Les journaux, l'historique, les états en direct (« Attente de MySQL… ») et le menu de l'icône sont dans la langue du serveur, réglable dans Paramètres › Servym › Langue du serveur (enregistrée dans `instance.lang`) ; par défaut `SERVYM_LANG`, sinon la langue du système.

Textes : `public/locales/` (interface) et `locales/` (serveur), un fichier par langue, sous des clés ; les tests vérifient que toutes les langues ont les mêmes clés et les mêmes paramètres.

## Comptes (facultatif)

Par défaut, pas d'authentification : usage local, par une seule personne. Paramètres › Sécurité › **Activer la protection** crée un compte administrateur ; dès lors, seule la page de connexion répond tant qu'on n'est pas connecté (liste, journaux, ports et actions compris). Le menu de l'icône reste utilisable sans mot de passe, sur la machine seulement (chaque action est inscrite « via la zone de notification »).

- **Permissions**, cochables compte par compte, avec quatre rôles tout prêts :

  | Permission | Lecture seule | Opérateur | Éditeur | Administrateur |
  | --- | --- | --- | --- | --- |
  | Voir (projets, journaux, réseau, activité) | oui | oui | oui | oui |
  | Piloter (démarrer, arrêter, ouvrir) | | oui | oui | oui |
  | Modifier les projets (nom, port, dépendances, ordre…) | | | oui | oui |
  | Commandes (commande, dossier, environnement, exposition, ajout, suppression) | | | au choix | oui |
  | Servym (redémarrage, démarrage avec Windows, nom de l'instance) | | | au choix | oui |
  | Administration (comptes, invitations, sécurité) | | | | oui |

  « Commandes » équivaut à un accès complet à la machine. Le dernier administrateur ne peut être ni supprimé ni rétrogradé.
- **Membres** : invitation par lien (48 h, un seul usage) ; l'invité choisit son nom d'utilisateur et son mot de passe.
- **Sessions** : cookie `HttpOnly`, `SameSite=Strict`. « Rester connecté » : 30 jours ; sinon jusqu'à la fermeture du navigateur (et 12 h sans activité au plus). Mon compte liste les sessions ouvertes ; changer de mot de passe ferme les autres. La fenêtre de l'application a ses propres cookies : on s'y connecte une première fois.
- **Verrouillage après inactivité** (2 min à 1 h, réglé par un administrateur pour tous) : sans souris ni clavier sur la page, le mot de passe est redemandé.
- **Mots de passe** : 8 caractères au moins, hachés par scrypt dans `runtime/auth.json`. 5 essais ratés : attente croissante.
- **Mot de passe oublié** : la **clé de secours** du compte (affichée une seule fois, à usage unique), ou un **code de 15 minutes** donné par un administrateur (Membres › Code), ou créé devant la machine de Servym (« Générer un code sur cette machine » sur la page de connexion), ou en ligne de commande sur cette machine :

  ```
  servym auth list
  servym auth reset-password <nom>
  ```

  Avec l'application de bureau, la même commande s'appelle par son exécutable : `& "$env:LOCALAPPDATA\Servym\Servym.exe" auth reset-password <nom>` sous Windows, `servym-desktop auth reset-password <nom>` sous Linux. Les sessions du compte sont alors fermées. Dernier recours : supprimer `runtime/auth.json` retire la protection et tous les comptes.
- **Connexion avec GitHub** (facultative) : un administrateur crée une OAuth App sur GitHub (adresse de retour : `http://localhost:9000/auth/github/callback`) et colle son identifiant et son secret dans Paramètres › Sécurité ; le secret reste dans `runtime/auth.json` et n'est jamais renvoyé à la page. Chaque membre lie ensuite son compte GitHub dans Mon compte, puis peut se connecter avec « Continuer avec GitHub » ; un compte GitHub non lié est refusé, le mot de passe reste valable. Seul le profil public est demandé à GitHub (identifiant et nom d'utilisateur).
- **Journal d'audit** : connexions, échecs, réglages modifiés, actions (« démarré par alice ») dans l'Activité.
- **Limite en local** : les navigateurs envoient le cookie de session à tout ce qui tourne sur `localhost`, quel que soit le port ; vos autres projets locaux le reçoivent donc. Sur un serveur, donnez à Servym son propre nom de domaine.

## Sécurité

Servym exécute des commandes : c'est son métier, et toute la sécurité consiste à ce que seules les personnes autorisées puissent les lui demander.

- écoute sur 127.0.0.1 seulement, sauf accès réseau demandé (voir ci-dessous) ;
- en-tête `Host` vérifié : `localhost`, et en accès réseau les adresses et le nom de cette machine (bloque le DNS rebinding) ;
- écritures en POST, PATCH, PUT ou DELETE avec l'en-tête `X-Servym-Request` et un contrôle de l'`Origin` (bloque le CSRF : une page tierce ne peut pas poser cet en-tête) ; même en-tête exigé sur les GET qui font travailler la machine (`free-port`, `git`, `ports`, `network`, `autostart`, `stats`) ;
- CSP stricte, aucun script inline ; seule l'interface (`public/`, embarquée dans l'exécutable) est servie ;
- la fenêtre de l'application charge la même page que le navigateur, sans aucune fonction native ouverte à la page (aucune IPC Tauri) ; les liens vers d'autres sites partent dans le navigateur ;
- commandes et arguments validés : aucun métacaractère de `cmd.exe`.

Par défaut, Servym n'écoute que sur ce PC. **Accès depuis le réseau privé** (Paramètres › Servym › Accès à la page › Réseau local, réservé aux administrateurs) : la page s'ouvre aussi depuis les autres appareils du réseau (Wi-Fi, Tailscale), aux adresses affichées. Conditions : la protection par mot de passe doit être active (sans elle, Servym reste sur ce PC même si `projects.json` dit `"access": "lan"`), le mot de passe est redemandé, et la protection ne peut plus être retirée tant que l'accès réseau est ouvert. Appliqué au redémarrage ; Windows peut alors demander d'autoriser Servym dans son pare-feu : autorisez les réseaux privés seulement. Sans HTTPS, les mots de passe circulent en clair sur le réseau : réservez cet accès à un réseau de confiance, ou à Tailscale, qui chiffre tout. Depuis un autre appareil, les actions qui ouvriraient une fenêtre sur la machine de Servym (dossier, VS Code, terminal, fichier local) sont refusées, et seuls les projets exposés s'ouvrent.

## API

| Méthode | Route | |
| --- | --- | --- |
| GET | `/api/projects` | configuration et état |
| POST | `/api/projects` | ajoute un projet (mêmes champs que `projects.json`, `id` facultatif) |
| PUT | `/api/projects` | nouvel ordre : `{ "order": ["id", …] }`, chaque projet une fois |
| PATCH | `/api/projects/:id` | modifie un projet (`null` retire un champ facultatif) |
| DELETE | `/api/projects/:id` | supprime un projet : `{ "password": "…" }` si la protection est active, sinon `{ "confirm": "<nom exact>" }` ; arrêté d'abord, retiré des `requires` des autres, journaux gardés |
| POST | `/api/projects/:id/duplicate` | copie placée juste après le projet : nom « (copie) », port libre suivant, sans lancement au démarrage, exposition au réseau ni port fixe (permission `commands`) |
| GET | `/api/stats` | processeur et mémoire de chaque projet en marche, de Servym seul (`self`) et des deux (`total`) ; relevé partagé, au plus un toutes les 2 s |
| GET | `/api/events` | flux SSE : `snapshot`, `state` à chaque changement, `instance` (erreur de configuration) |
| POST | `/api/projects/:id/start` · `stop` · `restart` | |
| POST | `/api/projects/:id/open` | ouvre le fichier d'un projet « file » |
| GET | `/api/projects/:id/logs?prev` | journal du lancement précédent (`{ "lines": [] }` s'il n'y en a pas) |
| POST | `/api/projects/:id/folder` · `code` · `terminal` | gestionnaire de fichiers, VS Code, terminal |
| GET | `/api/projects/:id/logs` | journal en SSE (`?tail` : 30 dernières lignes en JSON) |
| GET | `/api/projects/:id/history` | changements d'état, du plus récent au plus ancien |
| GET | `/api/history` | changements d'état de tous les projets |
| GET | `/api/git` | branche, modifications, dernier commit de chaque projet |
| GET | `/api/free-port?from=3500` | premier port libre |
| GET | `/api/ports` | ports en écoute sur la machine, programmes, projets, réseaux |
| GET | `/api/network` | adresses de la machine et profil des réseaux connectés |
| GET · PUT | `/api/autostart` | démarrage avec la session (clé Run ou service systemd) : `{ "enabled": true }` l'active, `false` le retire |
| POST | `/api/reload` | relit `projects.json` |
| POST | `/api/instance/config` · `log` · `folder` | ouvre `projects.json` ou `servym.log` (VS Code, à défaut le Bloc-notes ou l'éditeur par défaut), ou le dossier des données |
| POST | `/api/instance/onboarded` | présentation vue (`runtime/ui.json`) |
| PUT | `/api/instance/opener` | application de bureau : `{ "opener": "window" }` ou `"browser"`, sur la machine seulement |
| POST | `/api/instance/browser` | application de bureau : ouvre Servym dans le navigateur (`{ "closeWindow": true }` ferme la fenêtre) |
| POST | `/api/restart` | redémarre Servym (409 sans superviseur : `servym` lancé à la main) |
| PATCH | `/api/instance` | nom de l'instance |
| PUT | `/api/instance/lang` | langue du serveur : `{ "lang": "fr" }`, `"en"` ou `null` (celle du système) ; appliquée tout de suite |
| PUT | `/api/instance/notify` | notifications du système quand un projet plante : `{ "notify": true }` ou `false` |
| GET | `/api/config/export` | tout `projects.json`, en fichier à télécharger (variables d'environnement comprises) |
| POST | `/api/config/import` | `{ "config": {…} }` : aperçu des projets ajoutés, modifiés et retirés ; avec `"apply": true`, remplace les projets (la section `instance` de cette machine est gardée) |
| POST | `/api/instance/notify-test` | envoie une notification d'essai ; `{ "sent": false }` si le système n'en a pas |
| PUT | `/api/instance/access` | `{ "access": "lan", "password": "…" }` ou `{ "access": "local" }` ; administrateur, appliqué au redémarrage |
| … | `/api/auth/…` | `status`, `setup`, `enable`, `login`, `logout`, `lock`, `unlock`, `activity`, `recover`, `reset`, `local-code`, `invite`, `me` (nom, préférences, mot de passe, clé, sessions, comptes liés), `users`, `invites`, `settings`, `providers`, `disable`, `github/start` |

Les messages d'erreur (`{ "error": "…" }`) sont dans la langue de l'en-tête `Accept-Language`, sinon dans celle du serveur.

## Interface

- **Projets** : liste par défaut ; **Orbite** en option (Servym au centre, les projets en ligne tournent, un service requis est une lune de l'application qui en dépend, un projet exposé porte un halo en pointillés).
- **Ressources** : processeur et mémoire dans la liste, et en courbes (5 dernières minutes) dans l'onglet Infos du volet.
- **Export et import** (Paramètres › Configuration) : tout `projects.json` en fichier, pour retrouver ses projets sur une autre machine. L'import montre d'abord les projets ajoutés, modifiés et retirés, garde les réglages de Servym de cette machine (port, accès, nom) et refuse de retirer un projet en marche.
- **Notifications du système** (désactivées par défaut, Paramètres › Général) : une notification quand un projet plante ou ne démarre pas, même la page fermée. Sous Windows, notification native au nom de Servym ; sous Linux, `notify-send`.
- **Activité**, **Réseau**, **Paramètres** (démarrage, fenêtre ou navigateur, langue, thème — système, clair, sombre, Solarized clair ou sombre —, `projects.json`, redémarrage de Servym, raccourcis).
- Raccourcis : `1`–`9` démarrer ou ouvrir un projet · `/` rechercher · `,` paramètres · `V` liste ou orbite · `P` pause de l'orbite · `Échap` fermer.
- Icônes : [Lucide](https://lucide.dev) (licence ISC). Polices : Inter et JetBrains Mono (licence OFL), servies localement. Textes des licences : `THIRD-PARTY-NOTICES.md`.

## Développement

| Crate | Rôle |
| --- | --- |
| `servym-core` | configuration, gestionnaire de processus, journaux, statistiques, comptes (sans accès direct au système) |
| `servym-sys` | accès au système derrière un trait `Platform` : Windows (Win32, COM, WinRT) et Linux (`/proc`, `ss`, signaux) ; tout le code `unsafe` est là |
| `servym-server` | serveur HTTP (axum), API, SSE, comptes, binaire sans écran `servym` |
| `servym-desktop` | application de bureau (Tauri 2) : fenêtre, zone de notification, superviseur, instance unique |
| `servym-fixtures` | applications factices pour les tests |

```
cargo test --workspace                           # tests Rust
cargo clippy --workspace --all-targets -- -D warnings
node --test                                      # le seul test JavaScript (tests/urls.test.mjs, code de l'interface)
```

La CI (GitHub Actions) les lance sous Windows et Linux à chaque push ; une étiquette `v*` construit les fichiers de la release. L'historique de la réécriture depuis la version Node.js est dans [MIGRATION.md](MIGRATION.md).

## Limites

- **Application de bureau Linux** : construite et testée par la CI, pas encore essayée sur une vraie machine de bureau. Pas de version macOS.
- **Réseau privé seulement** : pas encore de HTTPS intégré ni de pilotage d'une autre machine.

## Licence

MIT, voir `LICENSE`.
