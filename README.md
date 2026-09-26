# Servym

**English** · [Français](README.fr.md)

A local dashboard to start, watch and stop your development projects: Node, PHP or Python applications, static sites, databases. A lightweight desktop app for Windows and Linux, written in Rust with [Tauri](https://tauri.app), and the same interface in the browser on http://localhost:9000.

- Start a project (and its dependencies) in one click, open its address as soon as it answers.
- Live log, start and crash history, Git status, CPU and memory.
- Listening ports on the machine, projects exposed on the network, warning on public networks.
- Tray icon: start, stop, open a project without opening the window.
- Start with Windows or with the Linux session, projects started when the PC boots.
- Optional accounts, interface in English and French.

Servym was first written in Node.js (never released); this Rust rewrite keeps the same interface and configuration files, with a single executable, no prerequisites and a much smaller memory footprint (see [Measurements](#measurements)).

## Install

The files are attached to each [release](https://github.com/HCWhite1/Servym/releases), with their SHA-256 checksums (`SHA256SUMS.txt`).

### Windows 10 and 11

Download and run `Servym_<version>_x64-setup.exe`. It installs for the current user, without administrator rights: the program and its data go to `%LOCALAPPDATA%\Servym`. WebView2, the engine of the window, already ships with Windows 10 and 11.

**SmartScreen warning.** The installer is not signed (a code signing certificate costs money): on first run, Windows shows "Windows protected your PC". Click **More info**, then **Run anyway**. To check the file before doing so: `Get-FileHash .\Servym_<version>_x64-setup.exe` must match the checksum in `SHA256SUMS.txt`. Release files are built by GitHub Actions from this repository (`.github/workflows/release.yml`); you can also build Servym yourself (see [From source](#from-source)).

Uninstall: Windows Settings › Apps. Only the program is removed; your data stays in `%LOCALAPPDATA%\Servym`.

### Linux desktop

Three formats, for x86_64:

- **Debian, Ubuntu and derivatives**: `sudo apt install ./Servym_<version>_amd64.deb`, then Servym in the application menu (or `servym-desktop`). Dependencies (WebKitGTK 4.1, GTK 3, libayatana-appindicator) are installed with the package.
- **Fedora and derivatives**: `sudo dnf install ./Servym-<version>-1.x86_64.rpm`, same thing (dependencies: `webkit2gtk4.1`, `gtk3`, `libayatana-appindicator-gtk3`).
- **Any distribution**: `Servym_<version>_amd64.AppImage`, made executable (`chmod +x`) then run. It bundles WebKitGTK; the `.deb` and `.rpm` packages use the system's and remain preferable.

The window uses WebKitGTK. The tray icon goes through AppIndicator: on GNOME it needs the "AppIndicator and KStatusNotifierItem Support" extension (enabled by default on Ubuntu; on Fedora: `sudo dnf install gnome-shell-extension-appindicator`, then sign in again and turn it on in Extensions); KDE, Xfce, Cinnamon and MATE show it directly. Notifications go through `notify-send` (package `libnotify-bin`, `libnotify` on Fedora).

If nothing shows up: `~/.local/share/servym/logs/servym.log` (Servym's log) and `servym.stderr.log` (GTK and WebKit error output), in the same folder. After repeated crashes at startup, a message says so.

### Linux without a screen (server)

`servym_<version>_linux_x86_64.tar.gz` contains `servym`, the server alone, with no window and no graphical dependency: the interface opens in a browser.

```
tar -xzf servym_<version>_linux_x86_64.tar.gz
./servym --data ~/servym
```

It needs `ss` (iproute2 package). Settings › General › Start with the session creates the `servym` user systemd service; to start it at boot without signing in: `loginctl enable-linger`.

### From source

Stable Rust (`rustup`), then:

```
cargo build --release -p servym-server        # target/release/servym: no screen
cargo install tauri-cli --version "^2" --locked
cd crates/servym-desktop && cargo tauri build  # NSIS installer, or .deb, .rpm and AppImage
```

On Linux, the desktop app needs the WebKitGTK development packages: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev`.

## First run

A short introduction offers to start Servym with Windows (or the session), to pick the projects started when the PC boots, then, in the desktop app, where to open Servym: in its window or in the browser. The **Add** button then creates a first project.

- **Window or browser**: the window uses about 200 MB while it is open; when closed it is destroyed and its memory released, and Servym stays in the tray. The browser only adds a tab. The choice can be changed in Settings › General and applies to this machine; http://localhost:9000 always works.
- **Tray**: the list of projects with their actions (start, stop, restart, open), "Open Servym", "Open in browser", "Start with Windows" and "Quit" (projects keep running). The icon turns red when a project has failed. On Windows, a left click opens Servym; on Linux, a click opens the menu.
- **Start with Windows**: Settings › General or the tray menu. Servym is added to the user's Run key (no administrator rights) and starts without a window when you sign in; Task Manager › Startup apps can disable it, and Servym respects that. On Linux: the `servym` user systemd service, started with the graphical session.
- **Restart Servym**: Settings › Servym. Running projects keep running and are picked up again. The desktop app restarts itself after a crash (it gives up after 5 quick crashes in a row).
- **Single instance**: starting Servym again just opens the interface of the running one.

## Data

Servym keeps everything in one folder: `projects.json`, `runtime/` (accounts, state of started projects, settings of this machine), `logs/` and `config/`. In order:

1. `--data <folder>` on the command line;
2. the `SERVYM_HOME` variable;
3. portable mode: the executable's folder, if it contains a `projects.json`;
4. otherwise `%LOCALAPPDATA%\Servym` on Windows, `~/.local/share/servym` on Linux (`$XDG_DATA_HOME/servym`).

The headless `servym` binary uses `--data`, `SERVYM_HOME`, otherwise the current folder.

Files from the former Node.js version are read as they are: copy `projects.json`, `runtime/`, `logs/` and `config/` into the data folder. Projects that are still running are picked up without a restart.

## Measurements

Same machine (Windows 11, 16 cores), on 26/09/2026. Former version: `node server.js` (Node.js 24). Rust version: `servym` (release). Empty data, average of three calls for the routes.

| | Node.js | Rust |
| --- | --- | --- |
| Start until the first `/api/projects` answer | 0.7 to 0.8 s | 0.2 s (0.7 s on the very first run) |
| Private memory, no page open | 31 MB (node, conhost) | 4.5 MB |
| Private memory, list shown (samples every 5 s) | 109 MB (node, conhost, PowerShell probe) | 4.6 MB |
| Network page: `/api/ports` | 790 ms | 33 ms |
| Network page: `/api/network` | 18 ms (cached) | 14 ms |
| Settings: `/api/autostart` | 2,450 ms (scheduled task, PowerShell) | 11 ms (registry) |
| `/api/stats`, first sample then next ones | 191 ms then 13 ms | 20 ms then 11 ms |
| Executable | node.exe (86 MB) plus the sources | `servym.exe` 6.4 MB, Linux `servym` 5.8 MB |

With real projects (7 projects, including MySQL, Bogue and MangaFlow picked up; measured from 24 to 26/09/2026):

| | Node.js | Rust |
| --- | --- | --- |
| Servym processes | wscript, node, conhost, PowerShell probe | supervisor and app |
| Private memory, window or page closed | ≈ 98 MB (wscript 13, node 85) | 8.3 MB (supervisor 1.4, app 6.9) |
| While the list is shown | + 92 MB (PowerShell probe for the samples) | nothing more (native sampling) |
| Per started project | ≈ 75 MB (a PowerShell waiting for the project to end) | a thread with 1 MB reserved, a few KB used |
| Window open | — (browser tab) | ≈ 200 MB (WebView2), released on close |
| Prerequisites | Node.js 22 (node.exe: 86 MB) and `git clone` | none |
| Distribution | Git repository and `npm start` | 3.4 MB Windows installer (11.9 MB program); Linux: 4.8 MB `.deb` and `.rpm`, 83 MB AppImage (GTK and WebKitGTK included) |

System operations measured alone (24/09/2026): listening ports 79 ms (two `netstat`) vs 0.95 ms (`GetExtendedTcpTable`); network profiles 692 ms (PowerShell) vs 25 ms (COM); sampling 339 processes: 169 ms CIM query vs 10.5 ms. Refused connection to a closed port: Windows retries for almost a second; like Node (libuv), Servym turns these retries off on the loopback (0.9 ms), otherwise every probe of a stopped project would be slow.

## Projects

Everything is set from the interface: **Add** button, then the **Settings** tab of a project's panel (click its row).

- **Delete**: at the bottom of the panel's Settings, or the row's "…" menu. The project is stopped first and removed from the dependencies of the others; its logs are kept. Confirmed with the account password if protection is on, otherwise by typing the project's name again.
- **Reorder**: drag the handle on the left of a row, or up and down arrows when it has focus.

The configuration is saved in `projects.json`, in the data folder. `projects.example.json` shows an example of each project type. Without `projects.json`, Servym starts with an empty list.

The file can also be edited by hand (Settings › Configuration › Edit): Servym reloads it as soon as it is saved and stable for one second, and logs the changes in Activity (the Reload button does the same right away). A running project removed by hand is kept until it stops, then the change applies. A change made by hand is never overwritten by the interface: each save starts again from the file as it is on disk. If the file becomes invalid, Servym keeps going with the last valid configuration (`runtime/projects.valid.json`) and shows the error, with its line and column.

The `instance` section sets Servym itself:

| Field | Role |
| --- | --- |
| `port` | Port of the page (9000 by default; the `SERVYM_PORT` variable wins). |
| `name` | Displayed name of the instance (sidebar, sign-in page). |
| `access` | `"local"` (default) or `"lan"`: page also open to the local network (see "Security"). |
| `lang` | `"fr"` or `"en"`: language of the logs and server messages. By default: the `SERVYM_LANG` variable, otherwise the system language. |

Each project:

| Field | Role |
| --- | --- |
| `id` | Identifier (lowercase letters, digits, dashes), taken from the name when added. Names the log: it never changes afterwards. |
| `name`, `desc`, `tags`, `icon`, `color` | Display. `icon`: name of an icon in `public/icons.svg` (Lucide, `book-open`…); any other value shows the name's initial. |
| `type` | Absent: program started by a command. `"static"`: Servym serves the files of `cwd` on `port` itself. `"file"`: page opened in the browser, with no server (`open`). |
| `cwd` | Project folder (absolute path). |
| `cmd`, `args` | Start command and its arguments. `{port}`, `{host}` and `{servym}` (the data folder) are replaced. On Windows, arguments go through `cmd.exe`: `" % & \| < > ^ ( )` and line breaks are refused. On Linux, the command runs directly, without a shell. |
| `env` | Environment variables added at start. `PORT` and `HOST` stay set by Servym. |
| `port` | Project port, passed in the `PORT` variable. Two projects cannot share one. |
| `hostControl` | `env`: the project reads `HOST`. `args`: the address goes through `{host}`. `local`: the project always listens locally. |
| `expose` | `false`: listens on 127.0.0.1. `true`: on the whole network (confirmation asked in the interface). |
| `url` | Address opened by "Open" (`http://localhost:{port}`). |
| `health` | "The project answers" check: `{ "path": "/", "expect": "<title>My site" }` (web page, optional expected text) or `{ "type": "tcp" }` (open port). |
| `startTimeout` | Seconds allowed for starting (60 by default). |
| `requires` | Projects to start before this one. Started for it, they stop with it if no other running project needs them. Circular dependencies are refused. |
| `service` | Listed under "Services" (database, tool) and drawn as a moon in the orbit view. |
| `stop` | Clean stop command, tried before the forced stop: `{ "cmd": "mysqladmin", "args": ["shutdown"] }`. |
| `autostart` | Started once per PC boot (a project stopped by hand does not come back if Servym restarts). |
| `restartOnCrash` | Restarted if it crashes (programs only, see "Automatic restart"). |
| `fixedPort` | Text explaining why the port must not change; the interface locks it. File only. |
| `open` | For `type: "file"`: file to open, relative to `cwd`. |

## How it works

- **Start**: on Windows, `CreateProcessW` starts the project in a hidden console, outside Servym's "job", passing it only its own outputs; its child processes open no window. On Linux, the project starts without a shell, in its own process group. Either way, it keeps running when Servym stops: at the next start, Servym picks up the projects listed in `runtime/pids.json`, with the creation time of each process (a PID reused by another program is never picked up).
- **Status**: `stopped`, `starting`, `running`, `stopping`, `failed` or `conflict` (the port answers, but it is not the right site). A stopped project is probed every 3 s, a running one every 15 s.
- **Started elsewhere**: a project that answers without having been started by Servym (scheduled task, `.bat` file) is shown as "external". Stopping it needs a confirmation: Servym then finds its PID through the port. A process started as administrator cannot be stopped by Servym.
- **Project that restarts itself**: if it exits with code 0 and starts a new instance that keeps the same output, Servym waits 30 s, finds the new PID through the port and keeps following it.
- **Launcher that exits while starting** (code 0): Servym waits 10 s for the port to answer (server left in the background). Otherwise, the project fails with the log line that explains it, for example the Symfony server already started from the same folder.
- **Automatic restart** (`restartOnCrash`, project Settings, off by default): a running project that crashes (exit code other than 0) is restarted after 2 s, then 10 s, then 30 s. If it crashes again after these 3 restarts within 5 minutes, Servym gives up: the project stays failed, and a notification says so if they are on; only giving up sends one. Never after a failure to start (port taken, command not found…) or after a requested stop; "Stop" cancels a planned restart, and starting by hand resets the count. Each restart is logged in the history. On Windows, the exit code of a project picked up after a Servym restart is known too; on Linux, only for projects started by this instance.
- **Stop**: the `stop` command if the project declares one, then the whole process tree is stopped (Windows: newest first; Linux: `SIGTERM`, then `SIGKILL` after 3 s). Stopping a project that other running projects depend on asks for a confirmation.
- **Removing a project** from `projects.json` while it runs is refused: it must be stopped first, otherwise it would keep running unwatched.
- **CPU and memory**: the whole process tree of a project is counted, plus the process listening on its port. CPU is a share of the whole machine, as in Task Manager. Sampled every 5 s, only while a page is visible and a project is running, through direct system calls (Windows) or `/proc` (Linux). The project summary also shows the total of Servym and its projects, each process counted once.
- **History**: each state change is written to `logs/servym.log`; a project's "History" tab and the "Activity" page read it back.
- **Network**: the "Network" page lists everything listening on the machine, linked to projects, with the machine's addresses and, on Windows, the profile (public / private) of each network.

Logs: `logs/servym.log` (the previous one moves to `logs/servym.prev.log` above 1 MB) and `logs/<id>.log` for each project (previous start in `logs/<id>.prev.log`).

### Linux

- Needs `ss` (iproute2), plus `xdg-open` (xdg-utils) on a desktop.
- **VS Code and terminal**: `code` in the `PATH`; first terminal found among Ptyxis, GNOME Terminal, Konsole, Xfce Terminal, kitty, Alacritty and XTerm.
- **Start with the session**: the `servym` user systemd service (`~/.config/systemd/user/servym.service`, no administrator rights), restarted if it crashes; stopping it leaves projects running (`KillMode=process`). The desktop app starts with the graphical session (`graphical-session.target`); headless `servym` with the user session (`default.target`). Only one of them at a time: turning one on replaces the other.
- **Without a screen** (server): nothing opens on the machine; folder, VS Code, terminal and local files are hidden.
- No network profiles as on Windows: the firewall (firewalld…) is set apart.

## Languages

The interface exists in English and French: the browser's language is picked automatically, and Settings › Appearance › Language sets one (kept by the browser, and by the account if protection is on). Server error messages follow the page's language. Logs, history, live states ("Waiting for MySQL…") and the tray menu are in the server's language, set in Settings › Servym › Server language (saved in `instance.lang`); by default `SERVYM_LANG`, otherwise the system language.

Texts: `public/locales/` (interface) and `locales/` (server), one file per language, under keys; the tests check that every language has the same keys and the same parameters.

## Accounts (optional)

By default, no authentication: local use, by a single person. Settings › Security › **Turn on protection** creates an administrator account; from then on, only the sign-in page answers until you sign in (list, logs, ports and actions included). The tray menu stays usable without a password, on the machine only (each action is logged "via the tray").

- **Permissions**, set account by account, with four ready-made roles:

  | Permission | Read only | Operator | Editor | Administrator |
  | --- | --- | --- | --- | --- |
  | View (projects, logs, network, activity) | yes | yes | yes | yes |
  | Operate (start, stop, open) | | yes | yes | yes |
  | Edit projects (name, port, dependencies, order…) | | | yes | yes |
  | Commands (command, folder, environment, exposure, add, delete) | | | optional | yes |
  | Servym (restart, start with Windows, instance name) | | | optional | yes |
  | Administration (accounts, invitations, security) | | | | yes |

  "Commands" amounts to full access to the machine. The last administrator can be neither deleted nor demoted.
- **Members**: invitation by link (48 h, single use); the guest picks their username and password.
- **Sessions**: `HttpOnly`, `SameSite=Strict` cookie. "Stay signed in": 30 days; otherwise until the browser closes (and 12 h without activity at most). My account lists open sessions; changing the password closes the others. The app window has its own cookies: sign in there once.
- **Lock after inactivity** (2 min to 1 h, set by an administrator for everyone): with no mouse or keyboard on the page, the password is asked again.
- **Passwords**: at least 8 characters, hashed with scrypt in `runtime/auth.json`. 5 failed attempts: growing wait.
- **Forgotten password**: the account's **recovery key** (shown only once, single use), or a **15-minute code** given by an administrator (Members › Code), or created in front of Servym's machine ("Generate a code on this machine" on the sign-in page), or on the command line on that machine:

  ```
  servym auth list
  servym auth reset-password <name>
  ```

  With the desktop app, the same command goes through its executable: `& "$env:LOCALAPPDATA\Servym\Servym.exe" auth reset-password <name>` on Windows, `servym-desktop auth reset-password <name>` on Linux. The account's sessions are then closed. Last resort: deleting `runtime/auth.json` removes protection and all accounts.
- **Sign in with GitHub** (optional): an administrator creates an OAuth App on GitHub (callback address: `http://localhost:9000/auth/github/callback`) and pastes its client ID and secret in Settings › Security; the secret stays in `runtime/auth.json` and is never sent back to the page. Each member then links their GitHub account in My account, and can sign in with "Continue with GitHub"; an unlinked GitHub account is refused, and the password still works. Only the public profile is requested from GitHub (ID and username).
- **Audit log**: sign-ins, failures, changed settings, actions ("started by alice") in Activity.
- **Local limit**: browsers send the session cookie to everything running on `localhost`, whatever the port; your other local projects therefore receive it. On a server, give Servym its own domain name.

## Security

Servym runs commands: that is its job, and all of its security is about making sure only authorized people can ask it to.

- listens on 127.0.0.1 only, unless network access is requested (see below);
- `Host` header checked: `localhost`, and with network access the addresses and name of this machine (blocks DNS rebinding);
- writes use POST, PATCH, PUT or DELETE with the `X-Servym-Request` header and an `Origin` check (blocks CSRF: a third-party page cannot set this header); the same header is required on GETs that make the machine work (`free-port`, `git`, `ports`, `network`, `autostart`, `stats`);
- strict CSP, no inline script; only the interface (`public/`, embedded in the executable) is served;
- the app window loads the same page as the browser, with no native function open to the page (no Tauri IPC); links to other sites go to the browser;
- commands and arguments validated: no `cmd.exe` metacharacters.

By default, Servym only listens on this PC. **Access from the private network** (Settings › Servym › Page access › Local network, administrators only): the page also opens from the other devices on the network (Wi-Fi, Tailscale), at the displayed addresses. Conditions: password protection must be on (without it, Servym stays on this PC even if `projects.json` says `"access": "lan"`), the password is asked again, and protection cannot be turned off while network access is open. Applied on restart; Windows may then ask to allow Servym through its firewall: allow private networks only. Without HTTPS, passwords travel in clear text on the network: keep this access for a trusted network, or Tailscale, which encrypts everything. From another device, actions that would open a window on Servym's machine (folder, VS Code, terminal, local file) are refused, and only exposed projects open.

## API

| Method | Route | |
| --- | --- | --- |
| GET | `/api/projects` | configuration and state |
| POST | `/api/projects` | adds a project (same fields as `projects.json`, `id` optional) |
| PUT | `/api/projects` | new order: `{ "order": ["id", …] }`, each project once |
| PATCH | `/api/projects/:id` | edits a project (`null` removes an optional field) |
| DELETE | `/api/projects/:id` | deletes a project: `{ "password": "…" }` if protection is on, otherwise `{ "confirm": "<exact name>" }`; stopped first, removed from the others' `requires`, logs kept |
| POST | `/api/projects/:id/duplicate` | copy placed right after the project: name "(copy)", next free port, no start at boot, network exposure or fixed port (`commands` permission) |
| GET | `/api/stats` | CPU and memory of each running project, of Servym alone (`self`) and of both (`total`); shared sampling, at most one every 2 s |
| GET | `/api/events` | SSE stream: `snapshot`, `state` on each change, `instance` (configuration error) |
| POST | `/api/projects/:id/start` · `stop` · `restart` | |
| POST | `/api/projects/:id/open` | opens the file of a "file" project |
| GET | `/api/projects/:id/logs?prev` | log of the previous start (`{ "lines": [] }` if there is none) |
| POST | `/api/projects/:id/folder` · `code` · `terminal` | file manager, VS Code, terminal |
| GET | `/api/projects/:id/logs` | log over SSE (`?tail`: last 30 lines as JSON) |
| GET | `/api/projects/:id/history` | state changes, newest first |
| GET | `/api/history` | state changes of all projects |
| GET | `/api/git` | branch, changes, last commit of each project |
| GET | `/api/free-port?from=3500` | first free port |
| GET | `/api/ports` | listening ports on the machine, programs, projects, networks |
| GET | `/api/network` | machine addresses and profile of connected networks |
| GET · PUT | `/api/autostart` | start with the session (Run key or systemd service): `{ "enabled": true }` turns it on, `false` removes it |
| POST | `/api/reload` | reloads `projects.json` |
| POST | `/api/instance/config` · `log` · `folder` | opens `projects.json` or `servym.log` (VS Code, otherwise Notepad or the default editor), or the data folder |
| POST | `/api/instance/onboarded` | introduction seen (`runtime/ui.json`) |
| PUT | `/api/instance/opener` | desktop app: `{ "opener": "window" }` or `"browser"`, on the machine only |
| POST | `/api/instance/browser` | desktop app: opens Servym in the browser (`{ "closeWindow": true }` closes the window) |
| POST | `/api/restart` | restarts Servym (409 without a supervisor: `servym` started by hand) |
| PATCH | `/api/instance` | instance name |
| PUT | `/api/instance/lang` | server language: `{ "lang": "fr" }`, `"en"` or `null` (the system's); applied right away |
| PUT | `/api/instance/notify` | system notifications when a project crashes: `{ "notify": true }` or `false` |
| GET | `/api/config/export` | the whole `projects.json`, as a file to download (environment variables included) |
| POST | `/api/config/import` | `{ "config": {…} }`: preview of added, changed and removed projects; with `"apply": true`, replaces the projects (this machine's `instance` section is kept) |
| POST | `/api/instance/notify-test` | sends a test notification; `{ "sent": false }` if the system has none |
| PUT | `/api/instance/access` | `{ "access": "lan", "password": "…" }` or `{ "access": "local" }`; administrator, applied on restart |
| … | `/api/auth/…` | `status`, `setup`, `enable`, `login`, `logout`, `lock`, `unlock`, `activity`, `recover`, `reset`, `local-code`, `invite`, `me` (name, preferences, password, key, sessions, linked accounts), `users`, `invites`, `settings`, `providers`, `disable`, `github/start` |

Error messages (`{ "error": "…" }`) are in the language of the `Accept-Language` header, otherwise the server's.

## Interface

- **Projects**: list by default; **Orbit** as an option (Servym in the middle, running projects orbit around it, a required service is a moon of the app that depends on it, an exposed project has a dotted halo).
- **Resources**: CPU and memory in the list, and as charts (last 5 minutes) in the panel's Info tab.
- **Export and import** (Settings › Configuration): the whole `projects.json` as a file, to get your projects back on another machine. Import first shows the added, changed and removed projects, keeps this machine's Servym settings (port, access, name) and refuses to remove a running project.
- **System notifications** (off by default, Settings › General): a notification when a project crashes or fails to start, even with the page closed. On Windows, a native notification in Servym's name; on Linux, `notify-send`.
- **Activity**, **Network**, **Settings** (startup, window or browser, language, theme — system, light, dark, Solarized light or dark —, `projects.json`, Servym restart, shortcuts).
- Shortcuts: `1`–`9` start or open a project · `/` search · `,` settings (`↑` `↓` between sections, `→` `←` to enter and leave a section) · `V` list or orbit · `Esc` close.
- Icons: [Lucide](https://lucide.dev) (ISC license). Fonts: Inter and JetBrains Mono (OFL license), served locally. License texts: `THIRD-PARTY-NOTICES.md`.

## Development

| Crate | Role |
| --- | --- |
| `servym-core` | configuration, process manager, logs, statistics, accounts (no direct system access) |
| `servym-sys` | system access behind a `Platform` trait: Windows (Win32, COM, WinRT) and Linux (`/proc`, `ss`, signals); all `unsafe` code lives here |
| `servym-server` | HTTP server (axum), API, SSE, accounts, headless `servym` binary |
| `servym-desktop` | desktop app (Tauri 2): window, tray, supervisor, single instance |
| `servym-fixtures` | fake applications for the tests |

```
cargo test --workspace                           # Rust tests
cargo clippy --workspace --all-targets -- -D warnings
node --test                                      # the only JavaScript test (tests/urls.test.mjs, interface code)
```

CI (GitHub Actions) runs them on Windows and Linux on every push; a `v*` tag builds the release files. The history of the rewrite from the Node.js version is in [MIGRATION.md](MIGRATION.md) (in French).

## Limits

- **Linux desktop app**: built and tested by CI, not yet tried on a real desktop machine. No macOS version.
- **Private network only**: no built-in HTTPS yet, and no control of another machine.

## License

MIT, see `LICENSE`.
