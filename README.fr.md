# Servynex

[English](README.md) · **Français**

Tableau de bord local pour lancer, surveiller et arrêter vos projets de développement : applications Node, PHP ou Python, sites statiques, bases de données. Une page web sur http://localhost:9000, sans dépendance npm.

- Lancer un projet (et ses dépendances) en un clic, ouvrir son adresse dès qu'il répond.
- Journal en direct, historique des démarrages et des plantages, état Git, processeur et mémoire.
- Ports en écoute sur la machine, projets exposés sur le réseau, alerte sur les réseaux publics.
- Démarrage avec Windows, projets lancés au démarrage du PC.
- Comptes facultatifs, interface en français et en anglais.

## Prérequis

- **Windows 10 ou 11**, ou **Linux** (pas encore essayé sur une vraie machine, voir « Linux »).
- **Node.js 22 ou plus.**

## Installation

```
git clone <adresse du dépôt> servynex
cd servynex
npm start
```

Ouvrir http://localhost:9000 : une présentation propose de démarrer Servynex avec Windows, puis le bouton **Ajouter** crée un premier projet.

- **Démarrage avec Windows** : Paramètres › Général, ou `autostart.bat` puis `ON` / `OFF`. Crée la tâche planifiée « Servynex » (sans droits administrateur) : Servynex démarre sans fenêtre à l'ouverture de session (`scripts/start-hidden.vbs`) et se relance s'il plante (il abandonne après 5 plantages rapides de suite).
- **Redémarrer Servynex** : Paramètres › Servynex. Les projets lancés continuent de tourner et sont repris. Disponible seulement sous la tâche planifiée : lancé par `npm start`, Servynex se relance depuis son terminal.
- **Tests** : `npm test`.

## Linux

Le même code tourne sous Linux ; seul `src/platform/linux.js` change (Windows : `src/platform/windows.js`, mêmes fonctions). **État : testé sur des sorties enregistrées seulement, pas encore sur une vraie machine Linux.**

- Demande `ss` (iproute2), et `xdg-open` (xdg-utils) sur un bureau. Les projets sont lancés directement, sans shell, dans leur propre groupe de processus ; l'arrêt envoie `SIGTERM` à toute l'arborescence, puis `SIGKILL` après 3 s.
- **Démarrer avec la session** : Paramètres › Général crée le service systemd utilisateur `servynex` (`~/.config/systemd/user/servynex.service`, sans droits administrateur). Il relance Servynex s'il plante, et l'arrêter laisse tourner les projets (`KillMode=process`). Pour qu'il démarre au boot sans ouvrir de session : `loginctl enable-linger`.
- **VS Code et terminal** : `code` dans le `PATH` ; premier terminal trouvé parmi Ptyxis, GNOME Terminal, Konsole, Xfce Terminal, kitty, Alacritty et XTerm.
- **Sans écran** (serveur) : rien ne s'ouvre sur la machine ; dossier, VS Code, terminal et fichiers locaux sont masqués.
- Pas de profils réseau comme sous Windows : le pare-feu (firewalld…) se règle à part.

## Projets

Tout se règle depuis l'interface : bouton **Ajouter**, puis l'onglet **Réglages** du volet d'un projet (clic sur sa ligne).

- **Supprimer** : en bas des Réglages du volet, ou menu « … » de la ligne. Le projet est arrêté d'abord, retiré des dépendances des autres ; ses journaux sont gardés. Confirmation par le mot de passe du compte si la protection est active, sinon en retapant le nom du projet.
- **Réordonner** : glisser la poignée à gauche d'une ligne, ou flèches haut et bas quand elle a le focus.

La configuration est enregistrée dans `projects.json`, à la racine. Ce fichier est personnel et n'est pas versionné ; `projects.example.json` montre un exemple de chaque type de projet. Sans `projects.json`, Servynex démarre avec une liste vide.

Le fichier peut aussi se modifier à la main (Paramètres › Configuration › Modifier, puis Relire). Une modification faite à la main n'est jamais écrasée par l'interface : chaque enregistrement repart du fichier tel qu'il est sur le disque. Si le fichier devient invalide, Servynex continue avec la dernière configuration valide (`runtime/projects.valid.json`) et affiche l'erreur.

La section `instance` règle Servynex lui-même :

| Champ | Rôle |
| --- | --- |
| `port` | Port de la page (9000 par défaut). |
| `name` | Nom affiché de l'instance (barre latérale, page de connexion). |
| `access` | `"local"` (par défaut) ou `"lan"` : page ouverte aussi au réseau local (voir « Sécurité »). |
| `lang` | `"fr"` ou `"en"` : langue des journaux et des messages du serveur. Par défaut : la variable `SERVYNEX_LANG`, sinon la langue du système. |

Chaque projet :

| Champ | Rôle |
| --- | --- |
| `id` | Identifiant (lettres minuscules, chiffres, tirets), tiré du nom à l'ajout. Nomme le journal : il ne change plus ensuite. |
| `name`, `desc`, `tags`, `icon`, `color` | Affichage. `icon` : nom d'une icône de `public/icons.svg` (Lucide, `book-open`…) ; toute autre valeur affiche l'initiale du nom. |
| `type` | Absent : programme lancé par une commande. `"static"` : Servynex sert lui-même les fichiers de `cwd` sur `port`. `"file"` : page ouverte dans le navigateur, sans serveur (`open`). |
| `cwd` | Dossier du projet (chemin absolu). |
| `cmd`, `args` | Commande de lancement et ses arguments. `{port}`, `{host}` et `{servynex}` sont remplacés. Les arguments passent par `cmd.exe` : `" % & \| < > ^ ( )` et les retours à la ligne y sont refusés. |
| `env` | Variables d'environnement ajoutées au lancement. `PORT` et `HOST` restent fixés par Servynex. |
| `port` | Port du projet, transmis dans la variable `PORT`. Deux projets ne peuvent pas avoir le même. |
| `hostControl` | `env` : le projet lit `HOST`. `args` : l'adresse passe par `{host}`. `local` : le projet écoute toujours en local. |
| `expose` | `false` : écoute sur 127.0.0.1. `true` : sur tout le réseau (confirmation demandée dans l'interface). |
| `url` | Adresse ouverte par « Ouvrir » (`http://localhost:{port}`). |
| `health` | Contrôle « le projet répond » : `{ "path": "/", "expect": "<title>Mon site" }` (page web, texte attendu facultatif) ou `{ "type": "tcp" }` (port ouvert). |
| `startTimeout` | Secondes accordées au démarrage (60 par défaut). |
| `requires` | Projets à démarrer avant celui-ci. Démarrés pour lui, ils s'arrêtent avec lui s'ils ne servent plus à aucun autre projet en marche. Les dépendances en boucle sont refusées. |
| `service` | Rangé dans « Services » (base de données, outil) et dessiné en lune dans la vue orbite. |
| `stop` | Commande d'arrêt propre, essayée avant l'arrêt forcé : `{ "cmd": "mysqladmin", "args": ["shutdown"] }`. |
| `autostart` | Lancé une fois par démarrage du PC (un projet arrêté à la main ne revient pas si Servynex redémarre). |
| `fixedPort` | Texte qui explique pourquoi le port ne doit pas changer ; l'interface le verrouille. Fichier seulement. |
| `open` | Pour `type: "file"` : fichier à ouvrir, relatif à `cwd`. |

## Fonctionnement

- **Lancement** : `src/launch.ps1` démarre le projet dans une console cachée (`Start-Process`). Le projet sort ainsi du « job » de Servynex : il continue de tourner quand Servynex redémarre, et ses sous-processus n'ouvrent aucune fenêtre. Au redémarrage, Servynex reprend les projets listés dans `runtime/pids.json`.
- **Statut** : `stopped`, `starting`, `running`, `stopping`, `failed` ou `conflict` (le port répond, mais ce n'est pas le bon site). Un projet arrêté est sondé toutes les 3 s, un projet en ligne toutes les 15 s.
- **Lancé ailleurs** : un projet qui répond sans avoir été lancé par Servynex (tâche planifiée, fichier `.bat`) est affiché « externe ». Pour l'arrêter, il faut confirmer : Servynex retrouve alors son PID par le port. Un processus lancé en administrateur ne peut pas être arrêté par Servynex.
- **Projet qui se relance lui-même** : s'il quitte avec le code 0 et relance une instance qui garde la même sortie, Servynex attend 30 s, retrouve le nouveau PID par le port et continue de le suivre.
- **Arrêt** : commande `stop` si le projet en déclare une, sinon `taskkill /T /F` sur tout l'arbre de processus. Arrêter un projet dont d'autres projets en marche dépendent demande une confirmation.
- **Retirer un projet** de `projects.json` pendant qu'il tourne est refusé : il faut l'arrêter d'abord, sinon il continuerait sans surveillance.
- **Processeur et mémoire** : toute l'arborescence de processus d'un projet est comptée (`cmd.exe`, puis `node` et ses enfants), ainsi que le processus qui écoute sur son port. Le processeur est une part de toute la machine, comme dans le Gestionnaire des tâches. Relevé toutes les 5 s, seulement quand une page est visible et qu'un projet tourne ; un PowerShell reste ouvert pour ces relevés et se ferme après une minute sans demande. Le résumé des projets affiche aussi le total de Servynex et de ses projets, chaque processus compté une fois ; son info-bulle donne Servynex seul (son propre processus, les relevés et les lanceurs des projets).
- **Historique** : chaque changement d'état est écrit dans `logs/servynex.log` ; l'onglet « Historique » d'un projet et la page « Activité » le relisent.
- **Réseau** : la page « Réseau » liste tout ce qui écoute sur la machine (netstat + tasklist), rattaché aux projets, avec les adresses du PC et le profil (public / privé) de chaque réseau.

Journaux : `logs/servynex.log` (le précédent passe dans `logs/servynex.prev.log` au-delà de 1 Mo) et `logs/<id>.log` pour chaque projet (lancement précédent dans `logs/<id>.prev.log`).

## Langues

L'interface existe en français et en anglais : la langue du navigateur est choisie automatiquement, et Paramètres › Apparence › Langue permet d'en fixer une (gardée par le navigateur, et par le compte si la protection est active). Les messages d'erreur du serveur suivent la langue de la page. Les journaux, l'historique et les états en direct (« Attente de MySQL… ») sont dans la langue du serveur, réglable dans Paramètres › Servynex › Langue du serveur (enregistrée dans `instance.lang`) ; par défaut `SERVYNEX_LANG`, sinon la langue du système.

Textes : `public/locales/` (interface) et `src/locales/` (serveur), un fichier par langue, sous des clés ; `npm test` vérifie que toutes les langues ont les mêmes clés et les mêmes paramètres.

## Comptes (facultatif)

Par défaut, pas d'authentification : usage local, par une seule personne. Paramètres › Sécurité › **Activer la protection** crée un compte administrateur ; dès lors, seule la page de connexion répond tant qu'on n'est pas connecté (liste, journaux, ports et actions compris).

- **Permissions**, cochables compte par compte, avec quatre rôles tout prêts :

  | Permission | Lecture seule | Opérateur | Éditeur | Administrateur |
  | --- | --- | --- | --- | --- |
  | Voir (projets, journaux, réseau, activité) | oui | oui | oui | oui |
  | Piloter (démarrer, arrêter, ouvrir) | | oui | oui | oui |
  | Modifier les projets (nom, port, dépendances, ordre…) | | | oui | oui |
  | Commandes (commande, dossier, environnement, exposition, ajout, suppression) | | | au choix | oui |
  | Servynex (redémarrage, démarrage avec Windows, nom de l'instance) | | | au choix | oui |
  | Administration (comptes, invitations, sécurité) | | | | oui |

  « Commandes » équivaut à un accès complet à la machine. Le dernier administrateur ne peut être ni supprimé ni rétrogradé.
- **Membres** : invitation par lien (48 h, un seul usage) ; l'invité choisit son nom d'utilisateur et son mot de passe.
- **Sessions** : cookie `HttpOnly`, `SameSite=Strict`. « Rester connecté » : 30 jours ; sinon jusqu'à la fermeture du navigateur (et 12 h sans activité au plus). Mon compte liste les sessions ouvertes ; changer de mot de passe ferme les autres.
- **Verrouillage après inactivité** (2 min à 1 h, réglé par un administrateur pour tous) : sans souris ni clavier sur la page, le mot de passe est redemandé.
- **Mots de passe** : 8 caractères au moins, hachés par scrypt dans `runtime/auth.json` (jamais versionné). 5 essais ratés : attente croissante.
- **Mot de passe oublié** : la **clé de secours** du compte (affichée une seule fois, à usage unique), ou un **code de 15 minutes** donné par un administrateur (Membres › Code) ou obtenu sur la machine :

  ```
  npm run auth -- list
  npm run auth -- reset-password <nom>
  ```

  Les sessions du compte sont alors fermées. Dernier recours : supprimer `runtime/auth.json` retire la protection et tous les comptes.
- **Connexion avec GitHub** (facultative) : un administrateur crée une OAuth App sur GitHub (adresse de retour : `http://localhost:9000/auth/github/callback`) et colle son identifiant et son secret dans Paramètres › Sécurité ; le secret reste dans `runtime/auth.json` et n'est jamais renvoyé à la page. Chaque membre lie ensuite son compte GitHub dans Mon compte, puis peut se connecter avec « Continuer avec GitHub » ; un compte GitHub non lié est refusé, le mot de passe reste valable. Seul le profil public est demandé à GitHub (identifiant et nom d'utilisateur).
- **Journal d'audit** : connexions, échecs, réglages modifiés, actions (« démarré par alice ») dans l'Activité.
- **Limite en local** : les navigateurs envoient le cookie de session à tout ce qui tourne sur `localhost`, quel que soit le port ; vos autres projets locaux le reçoivent donc. Sur un serveur, donnez à Servynex son propre nom de domaine.

## Sécurité

Servynex exécute des commandes : c'est son métier, et toute la sécurité consiste à ce que seules les personnes autorisées puissent les lui demander.

- écoute sur 127.0.0.1 seulement, sauf accès réseau demandé (voir ci-dessous) ;
- en-tête `Host` vérifié : `localhost`, et en accès réseau les adresses et le nom de cette machine (bloque le DNS rebinding) ;
- écritures en POST, PATCH, PUT ou DELETE avec l'en-tête `X-Servynex-Request` et un contrôle de l'`Origin` (bloque le CSRF : une page tierce ne peut pas poser cet en-tête) ; même en-tête exigé sur les GET qui font travailler la machine (`free-port`, `git`, `ports`, `network`, `autostart`, `stats`) ;
- CSP stricte, aucun script inline ; seul `public/` est servi ;
- commandes et arguments validés : aucun métacaractère de `cmd.exe`.

Par défaut, Servynex n'écoute que sur ce PC. **Accès depuis le réseau privé** (Paramètres › Servynex › Accès à la page › Réseau local, réservé aux administrateurs) : la page s'ouvre aussi depuis les autres appareils du réseau (Wi-Fi, Tailscale), aux adresses affichées. Conditions : la protection par mot de passe doit être active (sans elle, Servynex reste sur ce PC même si `projects.json` dit `"access": "lan"`), le mot de passe est redemandé, et la protection ne peut plus être retirée tant que l'accès réseau est ouvert. Appliqué au redémarrage ; Windows peut alors demander d'autoriser Node.js dans son pare-feu : autorisez les réseaux privés seulement. Sans HTTPS, les mots de passe circulent en clair sur le réseau : réservez cet accès à un réseau de confiance, ou à Tailscale, qui chiffre tout. Depuis un autre appareil, les actions qui ouvriraient une fenêtre sur la machine de Servynex (dossier, VS Code, terminal, fichier local) sont refusées, et seuls les projets exposés s'ouvrent.

## API

| Méthode | Route | |
| --- | --- | --- |
| GET | `/api/projects` | configuration et état |
| POST | `/api/projects` | ajoute un projet (mêmes champs que `projects.json`, `id` facultatif) |
| PUT | `/api/projects` | nouvel ordre : `{ "order": ["id", …] }`, chaque projet une fois |
| PATCH | `/api/projects/:id` | modifie un projet (`null` retire un champ facultatif) |
| DELETE | `/api/projects/:id` | supprime un projet : `{ "password": "…" }` si la protection est active, sinon `{ "confirm": "<nom exact>" }` ; arrêté d'abord, retiré des `requires` des autres, journaux gardés |
| GET | `/api/stats` | processeur et mémoire de chaque projet en marche, de Servynex seul (`self`) et des deux (`total`) ; relevé partagé, au plus un toutes les 2 s |
| GET | `/api/events` | flux SSE : `snapshot`, `state` à chaque changement, `instance` (erreur de configuration) |
| POST | `/api/projects/:id/start` · `stop` · `restart` | |
| POST | `/api/projects/:id/open` | ouvre le fichier d'un projet « file » |
| POST | `/api/projects/:id/folder` · `code` · `terminal` | Explorateur, VS Code, Windows Terminal |
| GET | `/api/projects/:id/logs` | journal en SSE (`?tail` : 30 dernières lignes en JSON) |
| GET | `/api/projects/:id/history` | changements d'état, du plus récent au plus ancien |
| GET | `/api/history` | changements d'état de tous les projets |
| GET | `/api/git` | branche, modifications, dernier commit de chaque projet |
| GET | `/api/free-port?from=3500` | premier port libre |
| GET | `/api/ports` | ports en écoute sur la machine, programmes, projets, réseaux |
| GET | `/api/network` | adresses du PC et profil des réseaux connectés |
| GET · PUT | `/api/autostart` | tâche planifiée « Servynex » : `{ "enabled": true }` la crée, `false` la supprime |
| POST | `/api/reload` | relit `projects.json` |
| POST | `/api/instance/config` · `log` · `folder` | ouvre `projects.json` ou `servynex.log` (VS Code, Bloc-notes à défaut), ou le dossier de Servynex |
| POST | `/api/instance/onboarded` | présentation vue (`runtime/ui.json`) |
| POST | `/api/restart` | redémarre Servynex (409 s'il n'est pas lancé par `start-hidden.vbs`) |
| PATCH | `/api/instance` | nom de l'instance |
| PUT | `/api/instance/lang` | langue du serveur : `{ "lang": "fr" }`, `"en"` ou `null` (celle du système) ; appliquée tout de suite |
| PUT | `/api/instance/notify` | notifications du système quand un projet plante : `{ "notify": true }` ou `false` |
| GET | `/api/config/export` | tout `projects.json`, en fichier à télécharger (variables d'environnement comprises) |
| POST | `/api/config/import` | `{ "config": {…} }` : aperçu des projets ajoutés, modifiés et retirés ; avec `"apply": true`, remplace les projets (la section `instance` de ce PC est gardée) |
| POST | `/api/instance/notify-test` | envoie une notification d'essai ; `{ "sent": false }` si le système n'en a pas |
| PUT | `/api/instance/access` | `{ "access": "lan", "password": "…" }` ou `{ "access": "local" }` ; administrateur, appliqué au redémarrage |
| … | `/api/auth/…` | `status`, `setup`, `enable`, `login`, `logout`, `lock`, `unlock`, `activity`, `recover`, `reset`, `invite`, `me` (nom, préférences, mot de passe, clé, sessions, comptes liés), `users`, `invites`, `settings`, `providers`, `disable`, `github/start` |

Les messages d'erreur (`{ "error": "…" }`) sont dans la langue de l'en-tête `Accept-Language`, sinon dans celle du serveur.

## Interface

- **Projets** : liste par défaut ; **Orbite** en option (Servynex au centre, les projets en ligne tournent, un service requis est une lune de l'application qui en dépend, un projet exposé porte un halo en pointillés).
- **Ressources** : processeur et mémoire dans la liste, et en courbes (5 dernières minutes) dans l'onglet Infos du volet.
- **Export et import** (Paramètres › Configuration) : tout `projects.json` en fichier, pour retrouver ses projets sur un autre PC. L'import montre d'abord les projets ajoutés, modifiés et retirés, garde les réglages de Servynex de ce PC (port, accès, nom) et refuse de retirer un projet en marche.
- **Notifications du système** (désactivées par défaut, Paramètres › Général) : une notification Windows quand un projet plante ou ne démarre pas, même la page fermée. Les textes passent à PowerShell par des variables d'environnement, jamais par la ligne de commande. Sous Linux, `notify-send` s'il est installé.
- **Activité**, **Réseau**, **Paramètres** (démarrage, langue, thème — système, clair, sombre, Solarized clair ou sombre —, `projects.json`, redémarrage de Servynex, raccourcis).
- Raccourcis : `1`–`9` démarrer ou ouvrir un projet · `/` rechercher · `,` paramètres · `V` liste ou orbite · `P` pause de l'orbite · `Échap` fermer.
- Icônes : [Lucide](https://lucide.dev) (licence ISC). Polices : Inter et JetBrains Mono (licence OFL), servies localement. Textes des licences : `THIRD-PARTY-NOTICES.md`.

## Limites

- **Linux** pas encore essayé sur une vraie machine (voir « Linux »). Pas de version macOS.
- **Réseau privé seulement** : pas encore de HTTPS intégré ni de pilotage d'une autre machine.

## Licence

MIT, voir `LICENSE`.
