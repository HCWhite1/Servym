# Servynex

Tableau de bord local pour lancer, surveiller et arrêter vos projets de développement : applications Node, PHP ou Python, sites statiques, bases de données. Une page web sur http://localhost:9000, sans dépendance npm.

- Lancer un projet (et ses dépendances) en un clic, ouvrir son adresse dès qu'il répond.
- Journal en direct, historique des démarrages et des plantages, état Git.
- Ports en écoute sur la machine, projets exposés sur le réseau, alerte sur les réseaux publics.
- Démarrage avec Windows, projets lancés au démarrage du PC.

## Prérequis

- **Windows 10 ou 11** (la version Linux est prévue, voir « Limites »).
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

## Projets

Tout se règle depuis l'interface : bouton **Ajouter**, puis l'onglet **Réglages** du volet d'un projet (clic sur sa ligne). La configuration est enregistrée dans `projects.json`, à la racine. Ce fichier est personnel et n'est pas versionné ; `projects.example.json` montre un exemple de chaque type de projet. Sans `projects.json`, Servynex démarre avec une liste vide. **Supprimer** : Réglages du volet (en bas) ou menu « … » de la ligne ; le projet est arrêté d'abord, retiré des dépendances des autres, ses journaux sont gardés. Confirmation par le mot de passe du compte si la protection est active, sinon en retapant le nom du projet. **Réordonner** : glisser la poignée à gauche d'une ligne, ou flèches haut et bas quand elle a le focus. La section `instance` règle Servynex lui-même : `port` (9000 par défaut), `name`, le nom affiché de l'instance, et `lang` (`"fr"` ou `"en"`), la langue des journaux et des messages du serveur (par défaut : la variable `SERVYNEX_LANG`, sinon la langue du système).

Le fichier peut aussi se modifier à la main (Paramètres › Configuration › Modifier, puis Relire). Une modification faite à la main n'est jamais écrasée par l'interface : chaque enregistrement repart du fichier tel qu'il est sur le disque. Si le fichier devient invalide, Servynex continue avec la dernière configuration valide (`runtime/projects.valid.json`) et affiche l'erreur.

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
- **Retirer un projet** qui tourne est refusé : il faut l'arrêter d'abord, sinon il continuerait sans surveillance.
- **Historique** : chaque changement d'état est écrit dans `logs/servynex.log` ; l'onglet « Historique » d'un projet et la page « Activité » le relisent.
- **Réseau** : la page « Réseau » liste tout ce qui écoute sur la machine (netstat + tasklist), rattaché aux projets, avec les adresses du PC et le profil (public / privé) de chaque réseau.

Journaux : `logs/servynex.log` (le précédent passe dans `logs/servynex.prev.log` au-delà de 1 Mo) et `logs/<id>.log` pour chaque projet (lancement précédent dans `logs/<id>.prev.log`).

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
- **Journal d'audit** : connexions, échecs, réglages modifiés, actions (« démarré par alice ») dans l'Activité.
- **Limite en local** : les navigateurs envoient le cookie de session à tout ce qui tourne sur `localhost`, quel que soit le port ; vos autres projets locaux le reçoivent donc. Sur un serveur, donnez à Servynex son propre nom de domaine.

## Sécurité

Servynex exécute des commandes : c'est son métier, et toute la sécurité consiste à ce que seules les personnes autorisées puissent les lui demander.

- écoute sur 127.0.0.1 seulement ;
- en-tête `Host` vérifié (bloque le DNS rebinding) ;
- écritures en POST, PATCH ou PUT avec l'en-tête `X-Servynex-Request` et un contrôle de l'`Origin` (bloque le CSRF : une page tierce ne peut pas poser cet en-tête) ; même en-tête exigé sur les GET qui font travailler la machine (`free-port`, `git`, `ports`, `network`, `autostart`) ;
- CSP stricte, aucun script inline ; seul `public/` est servi ;
- commandes et arguments validés : aucun métacaractère de `cmd.exe`.

Servynex n'écoute qu'en local, avec ou sans comptes. L'accès distant (écoute sur le réseau, HTTPS) viendra plus tard, et exigera la protection par mot de passe.

## API

| Méthode | Route | |
| --- | --- | --- |
| GET | `/api/projects` | configuration et état |
| POST | `/api/projects` | ajoute un projet (mêmes champs que `projects.json`, `id` facultatif) |
| PATCH | `/api/projects/:id` | modifie un projet (`null` retire un champ facultatif) |
| DELETE | `/api/projects/:id` | supprime un projet : `{ "password": "…" }` si la protection est active, sinon `{ "confirm": "<nom exact>" }` ; arrêté d'abord, retiré des `requires` des autres, journaux gardés |
| PUT | `/api/projects` | nouvel ordre : `{ "order": ["id", …] }`, chaque projet une fois |
| GET | `/api/stats` | processeur (part de toute la machine) et mémoire de chaque projet en marche, arborescence de processus comprise ; relevé partagé, au plus un toutes les 2 s |
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
| … | `/api/auth/…` | `status`, `setup`, `enable`, `login`, `logout`, `lock`, `unlock`, `activity`, `recover`, `reset`, `invite`, `me` (nom, préférences, mot de passe, clé, sessions), `users`, `invites`, `settings`, `disable` |

## Interface

- **Projets** : liste par défaut ; **Orbite** en option (Servynex au centre, les projets en ligne tournent, un service requis est une lune de l'application qui en dépend, un projet exposé porte un halo en pointillés).
- **Ressources** : processeur et mémoire de chaque projet en marche, dans la liste et en courbes (5 dernières minutes) dans l'onglet Infos du volet. Toute l'arborescence de processus est comptée (`cmd.exe`, puis `node` et ses enfants). Relevé toutes les 5 s, seulement quand la page est visible et qu'un projet tourne ; un PowerShell reste ouvert pour ces relevés et se ferme après une minute sans demande.
- **Activité**, **Réseau**, **Paramètres** (démarrage, thème — système, clair, sombre, Solarized clair ou sombre —, `projects.json`, redémarrage de Servynex, raccourcis).
- Raccourcis : `1`–`9` démarrer ou ouvrir un projet · `/` rechercher · `,` paramètres · `V` liste ou orbite · `P` pause de l'orbite · `Échap` fermer.
- Icônes : [Lucide](https://lucide.dev) (licence ISC). Polices : Inter et JetBrains Mono (licence OFL), servies localement. Textes des licences : `THIRD-PARTY-NOTICES.md`.

## Limites

- **Windows uniquement** pour l'instant : lancement (`launch.ps1`, `cmd.exe`), arrêt (`taskkill`), ports (`netstat`, `tasklist`), profils réseau et démarrage automatique (tâche planifiée) sont propres à Windows.
- **Local uniquement** : pas encore d'accès distant ni de pilotage d'une autre machine.
- Interface en français.

## Licence

MIT, voir `LICENSE`.
