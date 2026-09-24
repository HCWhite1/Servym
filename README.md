# Servynex

**English** · [Français](README.fr.md)

A local dashboard to start, watch and stop your development projects: Node, PHP or Python applications, static sites, databases. One web page on http://localhost:9000, no npm dependencies.

- Start a project (and its dependencies) in one click, open its address as soon as it answers.
- Live log, start and crash history, Git status, CPU and memory.
- Listening ports on the machine, projects exposed on the network, warning on public networks.
- Start with Windows, projects started when the PC boots.
- Optional accounts, interface in English and French.

## Requirements

- **Windows 10 or 11**, or **Linux** (not yet tried on a real machine, see "Linux").
- **Node.js 22 or later.**

## Install

```
git clone <repository address> servynex
cd servynex
npm start
```

Open http://localhost:9000: a short introduction offers to start Servynex with Windows, then the **Add** button creates a first project.

- **Start with Windows**: Settings › General, or `autostart.bat` then `ON` / `OFF`. Creates the "Servynex" scheduled task (no administrator rights): Servynex starts without a window when you sign in (`scripts/start-hidden.vbs`) and restarts if it crashes (it gives up after 5 quick crashes in a row).
- **Restart Servynex**: Settings › Servynex. Running projects keep running and are picked up again. Only available under the scheduled task: started with `npm start`, Servynex is restarted from its terminal.
- **Tests**: `npm test`.

## Linux

The same code runs on Linux; only `src/platform/linux.js` changes (Windows: `src/platform/windows.js`, same functions). **Status: tested on recorded outputs only, not yet on a real Linux machine.**

- Needs `ss` (iproute2), plus `xdg-open` (xdg-utils) on a desktop. Projects start directly, without a shell, in their own process group; stopping sends `SIGTERM` to the whole tree, then `SIGKILL` after 3 s.
- **Start with the session**: Settings › General creates the user systemd service `servynex` (`~/.config/systemd/user/servynex.service`, no administrator rights). It restarts Servynex if it crashes, and stopping it leaves projects running (`KillMode=process`). To start it at boot without signing in: `loginctl enable-linger`.
- **VS Code and terminal**: `code` in the `PATH`; first terminal found among Ptyxis, GNOME Terminal, Konsole, Xfce Terminal, kitty, Alacritty and XTerm.
- **Without a screen** (server): nothing opens on the machine; folder, VS Code, terminal and local files are hidden.
- No network profiles as on Windows: the firewall (firewalld…) is set apart.

## Projects

Everything is set from the interface: the **Add** button, then the **Settings** tab of a project's panel (click its row).

- **Delete**: at the bottom of the panel's Settings, or from the row's "…" menu. The project is stopped first and removed from other projects' dependencies; its logs are kept. Confirmed with the account password if protection is on, otherwise by typing the project name.
- **Reorder**: drag the handle on the left of a row, or use the up and down arrows when it has focus.

The configuration is saved in `projects.json`, at the root. This file is personal and not versioned; `projects.example.json` shows an example of each project type. Without `projects.json`, Servynex starts with an empty list.

The file can also be edited by hand (Settings › Configuration › Edit, then Reload). A hand edit is never overwritten by the interface: each save starts again from the file as it is on disk. If the file becomes invalid, Servynex keeps going with the last valid configuration (`runtime/projects.valid.json`) and shows the error.

The `instance` section sets up Servynex itself:

| Field | Role |
| --- | --- |
| `port` | Port of the page (9000 by default). |
| `name` | Instance name shown in the sidebar and on the sign-in page. |
| `access` | `"local"` (default) or `"lan"`: page also open to the local network (see "Security"). |
| `lang` | `"en"` or `"fr"`: language of logs and server messages. Default: the `SERVYNEX_LANG` variable, otherwise the system language. |

Each project:

| Field | Role |
| --- | --- |
| `id` | Identifier (lowercase letters, digits, hyphens), derived from the name when added. It names the log, so it never changes afterwards. |
| `name`, `desc`, `tags`, `icon`, `color` | Display. `icon`: name of an icon in `public/icons.svg` (Lucide, `book-open`…); any other value shows the first letter of the name. |
| `type` | Absent: a program started by a command. `"static"`: Servynex itself serves the files of `cwd` on `port`. `"file"`: a page opened in the browser, without a server (`open`). |
| `cwd` | Project folder (absolute path). |
| `cmd`, `args` | Start command and its arguments. `{port}`, `{host}` and `{servynex}` are replaced. Arguments go through `cmd.exe`: `" % & \| < > ^ ( )` and line breaks are refused. |
| `env` | Environment variables added at launch. `PORT` and `HOST` stay set by Servynex. |
| `port` | Project port, passed in the `PORT` variable. Two projects cannot share one. |
| `hostControl` | `env`: the project reads `HOST`. `args`: the address goes through `{host}`. `local`: the project always listens locally. |
| `expose` | `false`: listens on 127.0.0.1. `true`: on the whole network (the interface asks for confirmation). |
| `url` | Address opened by "Open" (`http://localhost:{port}`). |
| `health` | "The project answers" check: `{ "path": "/", "expect": "<title>My site" }` (web page, optional expected text) or `{ "type": "tcp" }` (open port). |
| `startTimeout` | Seconds allowed to start (60 by default). |
| `requires` | Projects to start before this one. Started for it, they stop with it if no other running project uses them. Dependency loops are refused. |
| `service` | Listed under "Services" (database, tool) and drawn as a moon in the orbit view. |
| `stop` | Clean stop command, tried before a forced stop: `{ "cmd": "mysqladmin", "args": ["shutdown"] }`. |
| `autostart` | Started once per PC boot (a project stopped by hand does not come back if Servynex restarts). |
| `fixedPort` | Text explaining why the port must not change; the interface locks it. File only. |
| `open` | For `type: "file"`: file to open, relative to `cwd`. |

## How it works

- **Launch**: `src/launch.ps1` starts the project in a hidden console (`Start-Process`). The project thus leaves Servynex's "job": it keeps running when Servynex restarts, and its child processes open no window. On restart, Servynex picks up the projects listed in `runtime/pids.json`.
- **Status**: `stopped`, `starting`, `running`, `stopping`, `failed` or `conflict` (the port answers, but it is not the right site). A stopped project is probed every 3 s, a running one every 15 s.
- **Started elsewhere**: a project that answers without having been started by Servynex (scheduled task, `.bat` file) is shown as "external". Stopping it needs a confirmation: Servynex then finds its PID through the port. A process started as administrator cannot be stopped by Servynex.
- **Project that restarts itself**: if it exits with code 0 and starts a new instance that keeps the same output, Servynex waits 30 s, finds the new PID through the port and keeps following it.
- **Stop**: the `stop` command if the project declares one, otherwise `taskkill /T /F` on the whole process tree. Stopping a project that other running projects depend on asks for confirmation.
- **Removing a project** from `projects.json` while it runs is refused: stop it first, or it would keep running unwatched.
- **CPU and memory**: the whole process tree of a project is counted (`cmd.exe`, then `node` and its children), plus the process listening on its port. CPU is a share of the whole machine, as in the Task Manager. Sampled every 5 s, only while a page is visible and a project runs; one PowerShell stays open for these samples and closes after a minute without requests. The project summary also shows the total for Servynex and its projects, each process counted once; its tooltip gives Servynex alone (its own process, the samples and the project launchers).
- **History**: every state change is written to `logs/servynex.log`; a project's "History" tab and the "Activity" page read it back.
- **Network**: the "Network" page lists everything listening on the machine (netstat + tasklist), matched to projects, with the PC's addresses and the profile (public / private) of each network.

Logs: `logs/servynex.log` (the previous one moves to `logs/servynex.prev.log` beyond 1 MB) and `logs/<id>.log` for each project (previous run in `logs/<id>.prev.log`).

## Languages

The interface exists in English and French: the browser language is picked automatically, and Settings › Appearance › Language sets one (kept by the browser, and by the account if protection is on). Server error messages follow the page language. Logs, history and live states ("Waiting for MySQL…") use the server language, set in Settings › Servynex › Server language (saved as `instance.lang`); by default `SERVYNEX_LANG`, otherwise the system language.

Texts: `public/locales/` (interface) and `src/locales/` (server), one file per language, under keys; `npm test` checks that every language has the same keys and the same parameters.

## Accounts (optional)

No authentication by default: local use, by one person. Settings › Security › **Turn on protection** creates an administrator account; from then on, only the sign-in page answers until you sign in (list, logs, ports and actions included).

- **Permissions**, checkable account by account, with four ready-made roles:

  | Permission | Read-only | Operator | Editor | Administrator |
  | --- | --- | --- | --- | --- |
  | View (projects, logs, network, activity) | yes | yes | yes | yes |
  | Operate (start, stop, open) | | yes | yes | yes |
  | Edit projects (name, port, dependencies, order…) | | | yes | yes |
  | Commands (command, folder, environment, exposure, add, delete) | | | optional | yes |
  | Servynex (restart, start with Windows, instance name) | | | optional | yes |
  | Administration (accounts, invitations, security) | | | | yes |

  "Commands" amounts to full access to the machine. The last administrator can be neither deleted nor demoted.
- **Members**: invitation by link (48 h, single use); the invited person picks their username and password.
- **Sessions**: `HttpOnly`, `SameSite=Strict` cookie. "Stay signed in": 30 days; otherwise until the browser closes (and 12 h without activity at most). My account lists open sessions; changing the password closes the others.
- **Lock after inactivity** (2 min to 1 h, set by an administrator for everyone): with no mouse or keyboard on the page, the password is asked again.
- **Passwords**: 8 characters at least, hashed with scrypt in `runtime/auth.json` (never versioned). 5 failed attempts: increasing wait.
- **Forgot password**: the account's **recovery key** (shown once, single use), or a **15-minute code** given by an administrator (Members › Code) or obtained on the machine:

  ```
  npm run auth -- list
  npm run auth -- reset-password <name>
  ```

  The account's sessions are then closed. Last resort: deleting `runtime/auth.json` removes the protection and every account.
- **Sign-in with GitHub** (optional): an administrator creates an OAuth App on GitHub (callback address: `http://localhost:9000/auth/github/callback`) and pastes its Client ID and secret in Settings › Security; the secret stays in `runtime/auth.json` and is never sent back to the page. Each member then links their GitHub account in My account and can sign in with "Continue with GitHub"; an unlinked GitHub account is refused, and the password keeps working. Only the public profile is requested from GitHub (ID and username).
- **Audit log**: sign-ins, failures, changed settings, actions ("started by alice") in Activity.
- **Local limit**: browsers send the session cookie to everything running on `localhost`, whatever the port; your other local projects therefore receive it. On a server, give Servynex its own domain name.

## Security

Servynex runs commands: that is its job, and all of its security is about making sure only authorized people can ask it to.

- listens on 127.0.0.1 only, unless network access is requested (see below);
- `Host` header checked: `localhost`, plus this machine's addresses and name when network access is on (blocks DNS rebinding);
- writes use POST, PATCH, PUT or DELETE with the `X-Servynex-Request` header and an `Origin` check (blocks CSRF: a third-party page cannot set this header); the same header is required on GETs that make the machine work (`free-port`, `git`, `ports`, `network`, `autostart`, `stats`);
- strict CSP, no inline script; only `public/` is served;
- commands and arguments validated: no `cmd.exe` metacharacters.

By default, Servynex only listens on this PC. **Access from the private network** (Settings › Servynex › Access to the page › Local network, administrators only): the page also opens from other devices on the network (Wi-Fi, Tailscale), at the addresses shown. Conditions: password protection must be on (without it, Servynex stays on this PC even if `projects.json` says `"access": "lan"`), the password is asked again, and protection cannot be turned off while network access is open. Applied at restart; Windows may then ask to allow Node.js through its firewall: allow private networks only. Without HTTPS, passwords travel unencrypted on the network: keep this access for a trusted network, or Tailscale, which encrypts everything. From another device, actions that would open a window on the Servynex machine (folder, VS Code, terminal, local file) are refused, and only exposed projects open.

## API

| Method | Route | |
| --- | --- | --- |
| GET | `/api/projects` | configuration and state |
| POST | `/api/projects` | adds a project (same fields as `projects.json`, `id` optional) |
| PUT | `/api/projects` | new order: `{ "order": ["id", …] }`, each project once |
| PATCH | `/api/projects/:id` | changes a project (`null` removes an optional field) |
| DELETE | `/api/projects/:id` | deletes a project: `{ "password": "…" }` if protection is on, otherwise `{ "confirm": "<exact name>" }`; stopped first, removed from the others' `requires`, logs kept |
| GET | `/api/stats` | CPU and memory of each running project, of Servynex alone (`self`) and of both (`total`); shared sample, at most one every 2 s |
| GET | `/api/events` | SSE stream: `snapshot`, `state` on every change, `instance` (configuration error) |
| POST | `/api/projects/:id/start` · `stop` · `restart` | |
| POST | `/api/projects/:id/open` | opens the file of a "file" project |
| POST | `/api/projects/:id/folder` · `code` · `terminal` | File Explorer, VS Code, Windows Terminal |
| GET | `/api/projects/:id/logs` | log as SSE (`?tail`: last 30 lines as JSON) |
| GET | `/api/projects/:id/history` | state changes, newest first |
| GET | `/api/history` | state changes of every project |
| GET | `/api/git` | branch, changes, last commit of each project |
| GET | `/api/free-port?from=3500` | first free port |
| GET | `/api/ports` | listening ports on the machine, programs, projects, networks |
| GET | `/api/network` | the PC's addresses and the profile of connected networks |
| GET · PUT | `/api/autostart` | "Servynex" scheduled task: `{ "enabled": true }` creates it, `false` deletes it |
| POST | `/api/reload` | reloads `projects.json` |
| POST | `/api/instance/config` · `log` · `folder` | opens `projects.json` or `servynex.log` (VS Code, Notepad otherwise), or the Servynex folder |
| POST | `/api/instance/onboarded` | introduction seen (`runtime/ui.json`) |
| POST | `/api/restart` | restarts Servynex (409 if it was not started by `start-hidden.vbs`) |
| PATCH | `/api/instance` | instance name |
| PUT | `/api/instance/lang` | server language: `{ "lang": "en" }`, `"fr"` or `null` (the system one); applied at once |
| PUT | `/api/instance/notify` | system notifications when a project crashes: `{ "notify": true }` or `false` |
| GET | `/api/config/export` | the whole `projects.json`, as a file to download (environment variables included) |
| POST | `/api/config/import` | `{ "config": {…} }`: preview of the added, changed and removed projects; with `"apply": true`, replaces the projects (the `instance` section of this PC is kept) |
| POST | `/api/instance/notify-test` | sends a test notification; `{ "sent": false }` when the system has none |
| PUT | `/api/instance/access` | `{ "access": "lan", "password": "…" }` or `{ "access": "local" }`; administrator, applied at restart |
| … | `/api/auth/…` | `status`, `setup`, `enable`, `login`, `logout`, `lock`, `unlock`, `activity`, `recover`, `reset`, `invite`, `me` (name, preferences, password, key, sessions, linked accounts), `users`, `invites`, `settings`, `providers`, `disable`, `github/start` |

Error messages (`{ "error": "…" }`) use the language of the `Accept-Language` header, otherwise the server language.

## Interface

- **Projects**: list by default; **Orbit** as an option (Servynex in the middle, online projects circle around it, a required service is a moon of the application that depends on it, an exposed project has a dotted halo).
- **Resources**: CPU and memory in the list, and as charts (last 5 minutes) in the panel's Info tab.
- **Export and import** (Settings › Configuration): the whole `projects.json` as a file, to take your projects to another PC. Importing shows the added, changed and removed projects first, keeps this PC's Servynex settings (port, access, name) and refuses to remove a running project.
- **System notifications** (off by default, Settings › General): a Windows notification when a project crashes or fails to start, even with the page closed. The texts reach PowerShell through environment variables, never through the command line. On Linux, `notify-send` when it is installed.
- **Activity**, **Network**, **Settings** (startup, language, theme — system, light, dark, Solarized light or dark —, `projects.json`, restarting Servynex, shortcuts).
- Shortcuts: `1`–`9` start or open a project · `/` search · `,` settings · `V` list or orbit · `P` pause the orbit · `Esc` close.
- Icons: [Lucide](https://lucide.dev) (ISC license). Fonts: Inter and JetBrains Mono (OFL license), served locally. License texts: `THIRD-PARTY-NOTICES.md`.

## Limits

- **Linux** not yet tried on a real machine (see "Linux"). No macOS version.
- **Private network only**: no built-in HTTPS nor control of another machine yet.

## License

MIT, see `LICENSE`.
