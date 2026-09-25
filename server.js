// Servym : portail local qui lance, surveille et arrête des projets.
// Aucune dépendance : node:http suffit pour une page et une API JSON.
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { ConfigError, createProject, DEFAULT_CONFIG, describeChanges, duplicateProject, ROOT_DIR, loadConfig, patchProject, removeProject, reorderProjects, saveConfig, validate } from './src/config.js';
import { Auth, AuthError } from './src/auth.js';
import { LANGS, LocalError, langOf, msg, serverLang, setServerLang, tr } from './src/i18n.js';
import { followLog, history, tailLines } from './src/logs.js';
import { notifier } from './src/notify.js';
import { HttpError, Manager } from './src/processes.js';
import { staticHandler } from './src/static.js';
import { Sampler } from './src/stats.js';
import * as sys from './src/system.js';

const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);
const LOOPBACK_ADDRESSES = new Set(['127.0.0.1', '::1', '::ffff:127.0.0.1']);

// Adresse d'écoute : tout le réseau seulement si l'accès réseau est demandé ET la
// protection par mot de passe active ; sinon n'importe qui sur le réseau aurait la main.
export function bindHost(instance, protectedByPassword) {
  return instance.access === 'lan' && protectedByPassword ? '0.0.0.0' : '127.0.0.1';
}
const { version: VERSION } = JSON.parse(fs.readFileSync(path.join(ROOT_DIR, 'package.json'), 'utf8'));

const HEADERS = {
  'Content-Security-Policy': "default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
  'X-Content-Type-Options': 'nosniff',
  'Referrer-Policy': 'no-referrer',
  'X-Frame-Options': 'DENY',
};

// ---- Réponses ----

const send = (res, status, body) => {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', 'Cache-Control': 'no-store' });
  res.end(JSON.stringify(body));
};

// Contenu d'un fichier, ou null s'il est illisible (absent, en cours d'écriture…).
function readText(file) {
  try {
    return fs.readFileSync(file, 'utf8');
  } catch {
    return null;
  }
}

async function readJson(req, limit = 16_000) {
  if (!String(req.headers['content-type']).startsWith('application/json')) throw new HttpError(415, 'http.jsonExpected');
  let raw = '';
  for await (const chunk of req) {
    raw += chunk;
    if (raw.length > limit) throw new HttpError(413, 'http.tooLarge');
  }
  try {
    return JSON.parse(raw || '{}');
  } catch {
    throw new HttpError(400, 'http.badJson');
  }
}

function openStream(req, res) {
  res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-store' });
  const ping = setInterval(() => res.write(': ping\n\n'), 25_000);
  const emit = (event, data) => res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  const done = new Promise((resolve) => res.on('close', resolve)).then(() => clearInterval(ping));
  return { emit, done };
}

// ---- Sécurité ----

// GET qui font travailler la machine (scan de ports, netstat, git, PowerShell) : même
// en-tête exigé que pour les écritures, pour qu'un <img> tiers ne puisse pas
// les déclencher sur un navigateur qui n'envoie pas Sec-Fetch-Site. Les flux
// SSE (EventSource ne pose pas d'en-tête) restent libres : ils ne font que lire.
const COSTLY_GET = /^\/api\/(free-port|git|ports|network|autostart|stats|config\/export)(\?|$)/;

// Host : bloque le DNS rebinding (seuls les noms de cette machine sont acceptés, voir
// hostAllowed). En-tête X-Servym-Request et Origin : bloquent le CSRF (un site
// tiers ne peut pas poser l'en-tête sans pré-vol CORS, refusé).
function guard(req, hostAllowed) {
  const host = String(req.headers.host ?? '').replace(/:\d+$/, '').toLowerCase();
  if (!hostAllowed(host)) throw new HttpError(421, 'http.badHost');
  const site = req.headers['sec-fetch-site'];
  if (req.url.startsWith('/api/') && site && site !== 'same-origin' && site !== 'none') throw new HttpError(403, 'http.crossSite');
  if ((req.method === 'GET' || req.method === 'HEAD') && !COSTLY_GET.test(req.url)) return;
  if (req.headers['x-servym-request'] !== '1') throw new HttpError(403, 'http.forbidden');
  const { origin } = req.headers;
  if (origin === undefined) return;
  let sameHost = false;
  try {
    sameHost = new URL(origin).host === req.headers.host;
  } catch {}
  if (!sameHost) throw new HttpError(403, 'http.badOrigin');
}

// Permission exigée par une route de l'API (voir src/auth.js).
const OPERATE = new Set(['start', 'stop', 'restart', 'open', 'folder', 'code', 'terminal']);
const INSTANCE_ACTION_PERMISSION = { config: 'commands', log: 'instance', folder: 'instance', onboarded: 'view', lang: 'instance', notify: 'instance', 'notify-test': 'instance' };
function permissionFor(section, id, action, method) {
  if (section === 'config') return 'commands'; // export et import : tout projects.json, commandes comprises
  if (method === 'GET' || method === 'HEAD') return 'view';
  if (section === 'projects' && !id && method === 'POST') return 'commands';
  if (section === 'projects' && !id && method === 'PUT') return 'edit'; // ordre des projets
  if (section === 'projects' && id && !action && method === 'PATCH') return 'edit';
  if (section === 'projects' && id && !action && method === 'DELETE') return 'commands';
  if (section === 'projects' && action === 'duplicate' && method === 'POST') return 'commands'; // copie sa commande
  if (section === 'projects' && OPERATE.has(action) && method === 'POST') return 'operate';
  if (section === 'reload') return 'edit';
  if (section === 'instance' && id) return INSTANCE_ACTION_PERMISSION[id] ?? 'admin';
  if (section === 'instance' || section === 'restart' || section === 'autostart') return 'instance';
  return 'admin';
}
// Champs dont la modification revient à choisir ce qui s'exécute (ou s'expose).
const SENSITIVE = ['type', 'cwd', 'cmd', 'args', 'env', 'stop', 'expose'];

// ---- Application ----

export function createApp({
  configFile = path.join(ROOT_DIR, 'projects.json'),
  logsDir = path.join(ROOT_DIR, 'logs'),
  runtimeDir = path.join(ROOT_DIR, 'runtime'),
  restart = null, // arrêt de Servym pour qu'un superviseur le relance (null : impossible)
  task = sys.autostartTask, // tâche planifiée qui démarre Servym avec la session
  clock = () => Date.now(), // horloge de l'authentification (remplacée dans les tests)
  sample = sys.sampleProcesses, // relevé des processus (processeur et mémoire)
  oauth = {}, // adresses des fournisseurs de connexion (tests : faux fournisseur)
  listening = 'local', // adresse d'écoute réelle : 'local' (127.0.0.1) ou 'lan' (0.0.0.0)
  hostnames = () => [...sys.lanAddresses().map((a) => a.address), os.hostname()], // noms de cette machine sur le réseau
  isLocalViewer = (req) => LOOPBACK_ADDRESSES.has(req.socket.remoteAddress), // la page est-elle ouverte sur cette machine ?
  notify = notifier(), // notification du système (tests : factice)
  platform = process.platform, // système, pour les textes de la page (« avec Windows », « avec la session »)
  desktop = sys.tools.desktop, // session avec écran : sinon rien ne peut s'ouvrir sur cette machine
  watchMs = 1000, // relecture automatique de projects.json : intervalle de surveillance (0 : aucune)
  ...options
} = {}) {
  // projects.json invalide au démarrage : Servym démarre quand même, avec la
  // dernière configuration valide, et l'interface affiche l'erreur (mode dégradé).
  const validCopy = path.join(runtimeDir, 'projects.valid.json');
  const events = new EventEmitter();
  let configError = null; // ConfigError, traduite pour chaque page
  let initial;
  try {
    initial = loadConfig(configFile);
  } catch (err) {
    if (!(err instanceof ConfigError)) throw err;
    configError = err;
    try {
      initial = loadConfig(validCopy);
    } catch {
      initial = structuredClone(DEFAULT_CONFIG);
    }
  }
  setServerLang(initial.instance.lang);
  const manager = new Manager({ config: initial, logsDir, runtimeDir, ...options });
  const serveUi = staticHandler(path.join(ROOT_DIR, 'public'));
  const sampler = new Sampler({ sample, cores: sys.cores });
  const gitCache = new Map();
  const ownLog = path.join(logsDir, 'servym.log');
  const ownPrevLog = path.join(logsDir, 'servym.prev.log');
  // Préférences de l'interface communes à tous les navigateurs (présentation vue ou non).
  const uiFile = path.join(runtimeDir, 'ui.json');
  const readUi = () => {
    try {
      return JSON.parse(fs.readFileSync(uiFile, 'utf8'));
    } catch {
      return {};
    }
  };
  let ownLogSize = fs.statSync(ownLog, { throwIfNoEntry: false })?.size ?? 0;

  const setConfigError = (err) => {
    if (err?.message === configError?.message) return;
    configError = err;
    events.emit('instance');
  };
  const configErrorIn = (lang) => configError?.in(lang) ?? null;
  const rememberValid = (config) => {
    try {
      saveConfig(validCopy, config);
    } catch {}
  };

  // Toute écriture part du fichier tel qu'il est sur le disque (et non de la copie
  // en mémoire) : une modification faite à la main n'est jamais écrasée.
  function diskConfig() {
    try {
      return loadConfig(configFile);
    } catch (err) {
      if (err instanceof ConfigError) {
        setConfigError(err);
        throw new ConfigError('config.fixFirst', { error: msg(err.key, err.params) });
      }
      throw err;
    }
  }
  function applyConfig(next) {
    manager.assertCanApply(next);
    saveConfig(configFile, next);
    known = readText(configFile); // écrit par Servym : pas à relire
    manager.setConfig(next);
    setServerLang(next.instance.lang);
    rememberValid(next);
    setConfigError(null);
  }

  // Relit projects.json (bouton « Relire » ou relecture automatique) ; who : compte
  // qui l'a demandé, « - » sinon. Fichier invalide ou projet en marche retiré :
  // l'erreur est affichée par la page et la configuration actuelle reste en place.
  function reloadConfig(who, logKey) {
    known = readText(configFile);
    let next;
    try {
      next = loadConfig(configFile);
      manager.assertCanApply(next);
    } catch (err) {
      if (err instanceof LocalError) setConfigError(err);
      throw err;
    }
    const previous = manager.config;
    manager.setConfig(next);
    setServerLang(next.instance.lang);
    rememberValid(next);
    setConfigError(null);
    log(`[config] ${tr(logKey)}`);
    // Modifications faites à la main dans le fichier : inscrites comme les autres.
    const audit = (project, key, params) => log(`[audit] ${project} ${who} · ${tr(key, params)}`);
    for (const p of next.projects) {
      const old = previous.projects.find((x) => x.id === p.id);
      if (!old) audit(p.id, 'audit.addedInFile');
      else if (describeChanges(old, p)) audit(p.id, 'audit.changedInFile', { changes: describeChanges(old, p) });
    }
    for (const old of previous.projects) if (!next.projects.some((p) => p.id === old.id)) audit(old.id, 'audit.removedInFile', { name: old.name });
  }

  // Relecture automatique : projects.json modifié à la main (éditeur, git pull…) est
  // relu dès qu'il ne bouge plus (même contenu deux tours de suite), comme avec
  // « Relire ». known : le contenu déjà connu, lu ou écrit par Servym lui-même.
  // Refusé (fichier invalide, projet en marche retiré) : réessayé à chaque tour,
  // pour s'appliquer dès que possible (projet arrêté), et signalé une seule fois.
  let known = readText(configFile);
  let pending = null;
  let failure = null;
  const watcher = watchMs > 0 ? setInterval(() => {
    const text = readText(configFile);
    if (text === null || text === known) {
      pending = null;
      return;
    }
    if (pending !== text) {
      pending = text;
      return;
    }
    try {
      reloadConfig('-', 'log.autoReloaded');
      pending = null;
      failure = null;
    } catch (err) {
      known = null;
      if (err.message !== failure) log(`[config] ${tr('log.autoReloadFailed', { error: err.message })}`);
      failure = err.message;
    }
  }, watchMs) : null;
  watcher?.unref();

  const log = (...parts) => {
    const line = `${new Date().toISOString()} ${parts.join(' ')}\n`;
    process.stdout.write(line);
    // Même rotation que les journaux de projets : servym.prev.log garde le précédent.
    ownLogSize += Buffer.byteLength(line);
    if (ownLogSize > 1_000_000) {
      try {
        fs.renameSync(ownLog, ownPrevLog);
      } catch {}
      ownLogSize = Buffer.byteLength(line);
    }
    try {
      fs.appendFileSync(ownLog, line);
    } catch {}
  };

  // Historique : chaque changement d'état est écrit dans servym.log, pour pouvoir
  // expliquer après coup un arrêt survenu en l'absence de l'utilisateur.
  manager.on('transition', ({ id, from, to, managed, exitCode, error, detail }) => {
    const details = [];
    // Raison d'un arrêt, relance automatique (après l'erreur si elle est prévue ou abandonnée).
    if (['stopping', 'starting'].includes(to) && detail) details.push(detail);
    const actor = actors.get(id);
    if (actor && ['starting', 'stopping'].includes(to) && clock() - actor.at < 10_000) details.push(tr('history.by', { user: actor.user }));
    if (to === 'running' && !managed) details.push(tr('history.external'));
    if (exitCode !== null && ['stopped', 'failed'].includes(to)) details.push(tr('history.exitCode', { code: exitCode }));
    if (error) details.push(error.replace(/\s+/g, ' '));
    if (to === 'failed' && detail) details.push(detail);
    log(`[state] ${id} ${from} → ${to}${details.length ? ` · ${details.join(' · ')}` : ''}`);
  });

  // Flux SSE ouverts par session : fermés quand la session est révoquée ou verrouillée.
  const streams = new Map();
  const auth = new Auth({
    file: path.join(runtimeDir, 'auth.json'),
    log,
    now: clock,
    instanceName: () => manager.config.instance.name ?? null,
    oauth,
    // Protection retirée alors que la page est ouverte au réseau : n'importe qui y aurait la main.
    beforeDisable: () => {
      if (manager.config.instance.access === 'lan') throw new AuthError(409, 'auth.disableLan');
    },
    onRevoke: (ids) => {
      for (const id of ids) for (const res of streams.get(id) ?? []) res.end();
    },
  });
  function trackStream(req, res) {
    const id = auth.enabled ? auth.session(req)?.session.id : null;
    if (!id) return;
    if (!streams.has(id)) streams.set(id, new Set());
    streams.get(id).add(res);
    res.on('close', () => streams.get(id)?.delete(res));
  }
  // Dernière action demandée par un compte sur chaque projet, pour attribuer
  // les changements d'état qui en découlent (« démarré par alice »).
  const actors = new Map();

  // Notification du système quand un projet passe en échec (réglage instance.notify),
  // pour le savoir même la page fermée. Un arrêt demandé n'en envoie pas, ni un
  // plantage que la relance automatique va réparer : seulement son abandon.
  const source = () => manager.config.instance.name ?? 'Servym';
  manager.on('transition', ({ id, from, to, exitCode, error, autoRestart }) => {
    if (to !== 'failed' || autoRestart === 'scheduled' || manager.config.instance.notify !== true) return;
    const name = manager.config.projects.find((p) => p.id === id)?.name ?? id;
    const body = error ?? (exitCode !== null ? tr('history.exitCode', { code: exitCode }) : '');
    const title = autoRestart === 'gaveUp' ? 'notify.gaveUp' : from === 'starting' ? 'notify.failedStart' : 'notify.crashed';
    Promise.resolve()
      .then(() => notify({ title: tr(title, { name }), body, source: source() }))
      .catch((err) => log(`[notify] ${tr('log.notifyFailed', { error: err.message })}`));
  });

  manager.on('warning', (message) => log(message));
  if (configError) log(`[config] ${tr('log.degraded', { error: configError.message })}`);
  else rememberValid(initial);

  const actions = {
    start: (p) => void manager.start(p.id),
    restart: (p) => manager.restart(p.id),
    stop: (p) => manager.stop(p.id, { cascade: true }),
    open: (p) => manager.openFile(p.id),
    folder: (p) => sys.openFolder(p.cwd),
    code: (p) => sys.openInCode(p.cwd),
    terminal: (p) => sys.openTerminal(p.cwd),
  };

  // Accès réseau : l'en-tête Host peut aussi nommer cette machine (adresse sur le
  // réseau, nom d'ordinateur). Ce PC seulement : localhost uniquement.
  const hostAllowed = (host) => LOOPBACK.has(host) || (listening === 'lan' && hostnames().some((n) => n.toLowerCase() === host));
  // Ouvrent quelque chose sur cette machine : réservées à qui est devant elle.
  const HOST_ONLY = new Set(['open', 'folder', 'code', 'terminal', 'config', 'log']);
  // Sans écran (serveur Linux), personne n'est « devant » la machine.
  const viewerLocal = (req) => desktop && isLocalViewer(req);
  const hostOnly = (req, action) => {
    if (!HOST_ONLY.has(action)) return;
    if (!desktop) throw new HttpError(403, 'http.noDesktop');
    if (!isLocalViewer(req)) throw new HttpError(403, 'http.hostOnly');
  };

  // Actions sur Servym lui-même (page Paramètres). Chemins fixes : rien ne vient de la requête.
  const instanceActions = {
    folder: () => sys.openFolder(ROOT_DIR),
    config: () => sys.openInEditor(configFile),
    log: () => sys.openInEditor(ownLog),
    onboarded: () => {
      fs.mkdirSync(runtimeDir, { recursive: true });
      fs.writeFileSync(uiFile, JSON.stringify({ ...readUi(), onboarded: true }));
    },
  };

  async function git(p, fresh) {
    const hit = gitCache.get(p.id);
    if (!fresh && hit && Date.now() - hit.at < 20_000) return hit.info;
    const info = await sys.gitInfo(p.cwd);
    gitCache.set(p.id, { at: Date.now(), info });
    return info;
  }

  async function freePort(from) {
    const taken = new Set([manager.config.instance.port, ...manager.config.projects.map((p) => p.port)]);
    for (let port = Math.max(from, 1024); port < Math.min(from + 500, 65536); port++) {
      if (!taken.has(port) && !(await sys.portInUse(port))) return port;
    }
    throw new HttpError(404, 'http.noFreePort');
  }

  // Profils réseau : une requête PowerShell (~1 s), gardée une minute.
  let network = { at: 0, profiles: [] };
  async function networkProfiles() {
    if (Date.now() - network.at > 60_000) network = { at: Date.now(), profiles: await sys.networkProfiles() };
    return network.profiles;
  }

  // Tous les ports en écoute sur la machine, rattachés aux projets quand c'est le cas.
  async function ports() {
    const [list, names, profiles] = await Promise.all([sys.listeners(), sys.processNames(), networkProfiles()]);
    const rows = new Map();
    for (const { address, port, pid } of list) {
      const key = `${port}/${pid}`;
      if (!rows.has(key)) rows.set(key, { port, pid, process: names.get(pid) ?? null, addresses: [] });
      if (!rows.get(key).addresses.includes(address)) rows.get(key).addresses.push(address);
    }
    // Rattachement par port, puis par PID : les autres ports ouverts par le même
    // processus (un second serveur HTTPS, par exemple) appartiennent au projet.
    const projects = manager.snapshot().filter((p) => p.port !== undefined);
    const byPid = new Map();
    for (const r of rows.values()) {
      r.project = projects.find((p) => (p.activePort ?? p.port) === r.port)?.id ?? null;
      if (r.project && r.pid) byPid.set(r.pid, r.project);
    }
    return {
      instance: manager.config.instance.port,
      lan: sys.lanAddresses(),
      networks: profiles,
      ports: [...rows.values()]
        .map((r) => ({
          ...r,
          exposed: r.addresses.some((a) => !a.startsWith('127.') && a !== '[::1]'),
          project: r.project ?? byPid.get(r.pid) ?? null,
        }))
        .sort((a, b) => a.port - b.port || a.pid - b.pid),
    };
  }

  async function api(req, res, url) {
    const [section, id, action] = url.pathname.split('/').slice(2); // /api/projects/:id/:action
    const { method } = req;

    // Connexion, comptes, sessions : routes à part, publiques ou non (src/auth.js).
    if (section === 'auth') return auth.handle(req, res, url.pathname.split('/').slice(3), { readJson, send });
    // Tout le reste : compte connecté et permission de la route (null : protection désactivée).
    const user = auth.require(req, permissionFor(section, id, action, method));
    const who = user?.username ?? '-';
    const audit = (project, key, params) => log(`[audit] ${project} ${who} · ${tr(key, params)}`);
    const lang = langOf(req);

    if (section === 'projects' && !id && method === 'GET') {
      const instance = {
        port: manager.config.instance.port,
        version: VERSION,
        node: process.version,
        dir: ROOT_DIR,
        config: configFile,
        tools: desktop ? sys.tools : { desktop: false, code: false, terminal: null },
        lan: sys.lanAddresses(),
        restartable: Boolean(restart),
        name: manager.config.instance.name ?? null,
        // Présentation vue : par compte si la protection est active, sinon pour Servym.
        onboarded: user ? Boolean(user.prefs?.onboarded) : Boolean(readUi().onboarded),
        access: manager.config.instance.access ?? 'local', // réglage (appliqué au démarrage)
        listening, // adresse d'écoute réelle
        viewerLocal: viewerLocal(req),
        platform,
        lang: manager.config.instance.lang ?? null, // langue choisie (null : celle du système)
        serverLang: serverLang(), // langue des journaux et des états, en ce moment
        notify: manager.config.instance.notify === true,
        configError: configErrorIn(lang),
      };
      return send(res, 200, { instance, projects: manager.snapshot() });
    }
    if (section === 'projects' && !id && method === 'POST') {
      const { config: next, id: created } = createProject(diskConfig(), await readJson(req));
      applyConfig(next);
      log(`[config] ${tr('log.added', { id: created })}`);
      audit(created, 'audit.added');
      return send(res, 201, manager.view(manager.project(created)));
    }
    if (section === 'projects' && !id && method === 'PUT') {
      const { order } = await readJson(req);
      applyConfig(reorderProjects(diskConfig(), order));
      log(`[config] ${tr('log.reordered')}`);
      audit('-', 'audit.reordered');
      return send(res, 200, { ok: true });
    }
    if (section === 'history' && !id && method === 'GET') {
      return send(res, 200, { entries: history([ownPrevLog, ownLog], null, 300) });
    }
    if (section === 'autostart' && !id) {
      if (method === 'GET') return send(res, 200, { enabled: await task.enabled() });
      if (method === 'PUT') {
        const { enabled } = await readJson(req);
        if (typeof enabled !== 'boolean') throw new HttpError(400, 'http.badEnabled');
        await task.set(enabled);
        log(`[autostart] ${tr(enabled ? 'log.autostartOn' : 'log.autostartOff')}`);
        return send(res, 200, { enabled: await task.enabled() });
      }
    }
    if (section === 'instance' && method === 'POST' && id === 'onboarded' && user) {
      auth.setPrefs(user.id, { onboarded: true });
      return send(res, 200, { ok: true });
    }
    if (section === 'instance' && method === 'POST' && Object.hasOwn(instanceActions, id)) {
      hostOnly(req, id);
      await instanceActions[id]();
      return send(res, 200, { ok: true });
    }
    if (section === 'instance' && id === 'lang' && method === 'PUT') {
      // Langue des journaux, de l'historique et des états : appliquée tout de suite
      // (applyConfig). null : celle du système.
      const { lang: chosen } = await readJson(req);
      if (chosen !== null && !LANGS.includes(chosen)) throw new HttpError(400, 'http.badLang');
      const next = diskConfig();
      if (chosen) next.instance.lang = chosen;
      else delete next.instance.lang;
      applyConfig(next);
      audit('-', 'audit.serverLang', { lang: chosen ?? 'auto' });
      return send(res, 200, { lang: chosen, serverLang: serverLang() });
    }
    if (section === 'instance' && id === 'notify' && method === 'PUT') {
      // Notifications du système quand un projet plante : désactivées par défaut.
      const { notify: enabled } = await readJson(req);
      if (typeof enabled !== 'boolean') throw new HttpError(400, 'http.badNotify');
      const next = diskConfig();
      if (enabled) next.instance.notify = true;
      else delete next.instance.notify;
      applyConfig(next);
      audit('-', enabled ? 'audit.notifyOn' : 'audit.notifyOff');
      return send(res, 200, { notify: enabled });
    }
    if (section === 'instance' && id === 'notify-test' && method === 'POST') {
      // Essai depuis les paramètres, même désactivées. sent false : pas sur ce système.
      let sent;
      try {
        sent = await notify({ title: tr('notify.testTitle'), body: tr('notify.testBody'), source: source() });
      } catch (err) {
        throw new HttpError(502, 'http.notifyFailed', { error: err.message });
      }
      return send(res, 200, { sent });
    }
    if (section === 'instance' && id === 'access' && method === 'PUT') {
      // Ouvrir au réseau : protection active et mot de passe redemandé. Revenir à ce
      // PC seulement ne demande rien. Appliqué au prochain démarrage (bindHost).
      const { access, password } = await readJson(req);
      if (!['local', 'lan'].includes(access)) throw new HttpError(400, 'http.badAccess');
      if (access === 'lan') {
        if (!auth.enabled) throw new HttpError(409, 'http.lanNeedsProtection');
        await auth.confirmPassword(req, user, password);
      }
      const next = diskConfig();
      if (access === 'lan') next.instance.access = 'lan';
      else delete next.instance.access;
      applyConfig(next);
      audit('-', access === 'lan' ? 'audit.accessLan' : 'audit.accessLocal');
      return send(res, 200, { access, listening });
    }
    if (section === 'instance' && !id && method === 'PATCH') {
      // Nom de l'instance, affiché dans la barre latérale (« Servym · Serveur maison »).
      const { name } = await readJson(req);
      if (name !== null && (typeof name !== 'string' || name.trim().length > 40)) throw new HttpError(400, 'http.nameTooLong');
      const next = diskConfig();
      if (name?.trim()) next.instance.name = name.trim();
      else delete next.instance.name;
      applyConfig(next);
      audit('-', 'audit.instanceName', { name: name?.trim() || 'Servym' });
      return send(res, 200, { name: next.instance.name ?? null });
    }
    if (section === 'events' && method === 'GET') {
      const { emit, done } = openStream(req, res);
      trackStream(req, res);
      const onChange = (project) => emit('state', project);
      const onConfig = () => emit('snapshot', manager.snapshot());
      const onInstance = () => emit('instance', { configError: configErrorIn(lang) });
      emit('snapshot', manager.snapshot());
      onInstance();
      manager.on('change', onChange).on('config', onConfig);
      events.on('instance', onInstance);
      await done;
      manager.off('change', onChange).off('config', onConfig);
      events.off('instance', onInstance);
      return;
    }
    if (section === 'git' && method === 'GET') {
      const fresh = url.searchParams.has('fresh');
      const entries = await Promise.all(manager.config.projects.map(async (p) => [p.id, await git(p, fresh)]));
      return send(res, 200, Object.fromEntries(entries));
    }
    if (section === 'free-port' && method === 'GET') {
      return send(res, 200, { port: await freePort(Number(url.searchParams.get('from')) || 3500) });
    }
    if (section === 'ports' && method === 'GET') return send(res, 200, await ports());
    if (section === 'stats' && method === 'GET') {
      // Racines de chaque projet en marche : le PID lancé par Servym et celui qui écoute
      // sur son port (un projet qui se relance lui-même n'est plus sous le premier).
      const running = manager.snapshot().filter((p) => p.type !== 'file' && ['running', 'starting', 'stopping'].includes(p.state));
      const listening = running.some((p) => p.type !== 'static') ? await sys.listeners() : [];
      const roots = {};
      const inProcess = {}; // sites statiques : servis par Servym lui-même
      for (const p of running) {
        if (p.type === 'static') {
          inProcess[p.id] = { inProcess: true };
          continue;
        }
        const port = p.activePort ?? p.port;
        roots[p.id] = [p.managed ? p.pid : null, ...listening.filter((l) => l.port === port).map((l) => l.pid)].filter(Boolean);
      }
      // Servym lui-même (sites statiques compris) et le total de la machine pour Servym.
      const { projects: measured, self, total } = await sampler.measureAll(roots, process.pid);
      return send(res, 200, { cores: sys.cores, projects: { ...measured, ...inProcess }, self, total });
    }
    if (section === 'network' && method === 'GET') return send(res, 200, { lan: sys.lanAddresses(), networks: await networkProfiles() });
    if (section === 'restart' && method === 'POST') {
      if (!restart) throw new HttpError(409, 'http.notRestartable');
      log(`[restart] Servym${user ? ` · ${tr('history.by', { user: who })}` : ''}`);
      send(res, 200, { ok: true });
      return void setImmediate(restart);
    }
    if (section === 'reload' && method === 'POST') {
      reloadConfig(who, 'log.reloaded');
      return send(res, 200, { ok: true });
    }
    if (section === 'config' && id === 'export' && method === 'GET') {
      // Le fichier tel qu'il est sur le disque, à télécharger.
      const day = new Date().toISOString().slice(0, 10);
      res.writeHead(200, {
        'Content-Type': 'application/json; charset=utf-8',
        'Content-Disposition': `attachment; filename="servym-projects-${day}.json"`,
        'Cache-Control': 'no-store',
      });
      return res.end(fs.readFileSync(configFile, 'utf8'));
    }
    if (section === 'config' && id === 'import' && method === 'POST') {
      // Remplace les projets par ceux d'un fichier exporté ; la section instance (port,
      // accès, nom…) reste celle de cette machine. Sans apply : aperçu seulement.
      const { config: imported, apply } = await readJson(req, 512_000);
      if (!Array.isArray(imported?.projects)) throw new ConfigError('config.noProjects');
      const current = diskConfig();
      const next = validate({ ...current, projects: imported.projects });
      const diff = { added: [], removed: [], changed: [], unchanged: 0 };
      for (const p of next.projects) {
        const old = current.projects.find((x) => x.id === p.id);
        const changes = old ? describeChanges(old, p) : '';
        if (!old) diff.added.push({ id: p.id, name: p.name });
        else if (changes) diff.changed.push({ id: p.id, name: p.name, changes });
        else diff.unchanged += 1;
      }
      for (const old of current.projects) if (!next.projects.some((p) => p.id === old.id)) diff.removed.push({ id: old.id, name: old.name });
      if (apply === true) {
        applyConfig(next);
        audit('-', 'audit.imported', { added: diff.added.length, changed: diff.changed.length, removed: diff.removed.length });
        for (const p of diff.added) audit(p.id, 'audit.addedByImport', { name: p.name });
        for (const p of diff.changed) audit(p.id, 'audit.changedByImport', { changes: p.changes });
        for (const p of diff.removed) audit(p.id, 'audit.removedByImport', { name: p.name });
      }
      return send(res, 200, { ...diff, applied: apply === true });
    }
    if (section !== 'projects' || !id) throw new HttpError(404, 'http.unknownRoute');

    const project = manager.project(id);
    if (!action && method === 'PATCH') {
      const patch = await readJson(req);
      const sensitive = Object.keys(patch ?? {}).filter((k) => SENSITIVE.includes(k));
      if (sensitive.length && !auth.can(user, 'commands')) {
        throw new AuthError(403, 'auth.needsCommands');
      }
      const current = diskConfig();
      const next = patchProject(current, id, patch);
      applyConfig(next);
      log(`[config] ${tr('log.changed', { id })}`);
      const changes = describeChanges(current.projects.find((p) => p.id === id), next.projects.find((p) => p.id === id));
      if (changes) audit(id, 'audit.changed', { changes });
      return send(res, 200, manager.view(manager.project(id)));
    }
    if (!action && method === 'DELETE') {
      // Confirmation : mot de passe du compte si la protection est active, sinon le nom exact du projet.
      const { password, confirm } = req.headers['content-type'] ? await readJson(req) : {};
      if (user) await auth.confirmPassword(req, user, password);
      else if (confirm !== project.name) throw new HttpError(400, 'http.confirmName', { name: project.name });
      // Arrêté d'abord (avec les dépendances lancées pour lui) : il ne tournerait plus sous aucun contrôle.
      const st = manager.state(id);
      if ((st.managed || project.type === 'static') && st.state !== 'stopped') {
        await manager.stop(id, { cascade: true, reason: tr('history.deleted') });
      }
      applyConfig(removeProject(diskConfig(), id));
      log(`[config] ${tr('log.deleted', { id })}`);
      audit(id, 'audit.deleted', { name: project.name });
      return send(res, 200, { ok: true });
    }
    if (action === 'logs' && method === 'GET') {
      const file = manager.logFile(id);
      if (url.searchParams.has('tail')) return send(res, 200, { lines: tailLines(file, 30) });
      // Lancement précédent : terminé, envoyé en une fois (sa fin, comme le journal en direct).
      if (url.searchParams.has('prev')) return send(res, 200, { lines: tailLines(manager.prevLogFile(id)) });
      const { emit, done } = openStream(req, res);
      trackStream(req, res);
      const stop = followLog(file, emit);
      await done;
      return stop();
    }
    if (action === 'duplicate' && method === 'POST') {
      // Port libre après celui de l'original ; nom dans la langue du serveur.
      const port = project.type === 'file' ? undefined : await freePort((project.port ?? 3000) + 1);
      const { config: next, id: created } = duplicateProject(diskConfig(), id, { name: tr('project.copyName', { name: project.name }), port });
      applyConfig(next);
      audit(created, 'audit.duplicated', { name: project.name });
      return send(res, 201, manager.view(manager.project(created)));
    }
    if (action === 'history' && method === 'GET') {
      return send(res, 200, { entries: history([ownPrevLog, ownLog], id) });
    }
    if (method === 'POST' && Object.hasOwn(actions, action)) {
      hostOnly(req, action);
      log(`[${action}] ${id}${user ? ` · ${tr('history.by', { user: who })}` : ''}`);
      if (user) actors.set(id, { user: who, at: clock() });
      await actions[action](project);
      return send(res, 200, { ok: true });
    }
    throw new HttpError(404, 'http.unknownRoute');
  }

  const server = http.createServer(async (req, res) => {
    for (const [k, v] of Object.entries(HEADERS)) res.setHeader(k, v);
    try {
      guard(req, hostAllowed);
      const url = new URL(req.url, 'http://localhost');
      if (url.pathname.startsWith('/api/')) return await api(req, res, url);
      // Retour d'un fournisseur de connexion : navigation venue de son site, hors de /api/.
      const [, root, provider, step] = url.pathname.split('/');
      if (root === 'auth' && step === 'callback' && req.method === 'GET') return await auth.callback(req, res, provider, url);
      if (await serveUi(req, res)) return;
      throw new HttpError(404, 'http.notFound');
    } catch (err) {
      const status = err instanceof LocalError ? err.status : 500;
      if (status === 500) log('[error]', req.method, req.url, err.stack ?? err.message);
      if (res.headersSent) return res.end();
      // Message dans la langue de la page (Accept-Language).
      send(res, status, { error: err instanceof LocalError ? err.in(langOf(req)) : err.message, ...(err.extra ?? {}) });
    }
  });

  server.on('close', () => clearInterval(watcher));

  return {
    server,
    manager,
    log,
    auth,
    // Adresse d'écoute choisie au démarrage (bindHost), une fois la configuration lue.
    setListening(value) {
      listening = value;
    },
  };
}

// ---- Démarrage ----

// Code de sortie qui demande au superviseur une relance immédiate.
const RESTART_CODE = 75;

// Lancé par un superviseur qui le relance : scripts/start-hidden.vbs sous Windows,
// le service systemd utilisateur sous Linux (src/platform/linux.js).
export function isSupervised(env = process.env) {
  return ['vbs', 'systemd'].includes(env.SERVYM_SUPERVISOR);
}

if (import.meta.filename === path.resolve(process.argv[1] ?? '')) {
  let app;
  // Redémarrage depuis l'interface : seulement sous le superviseur, qui relance
  // Servym ; lancé par npm start, personne ne le relancerait.
  const supervised = isSupervised();
  try {
    app = createApp({ restart: supervised ? () => shutdown(RESTART_CODE) : null });
  } catch (err) {
    // Lancé sans fenêtre : l'erreur doit rester lisible dans le journal.
    const logsDir = path.join(ROOT_DIR, 'logs');
    fs.mkdirSync(logsDir, { recursive: true });
    fs.appendFileSync(path.join(logsDir, 'servym.log'), `${new Date().toISOString()} [config] ${err.message}\n`);
    console.error(`[config] ${err.message}`);
    process.exit(1);
  }
  const { server, manager, log, auth } = app;
  const port = Number(process.env.SERVYM_PORT) || manager.config.instance.port;
  const host = bindHost(manager.config.instance, auth.enabled);
  app.setListening(host === '0.0.0.0' ? 'lan' : 'local');
  if (manager.config.instance.access === 'lan' && host !== '0.0.0.0') log(`[listen] ${tr('log.lanRefused')}`);
  server.on('error', (err) => {
    log('[listen]', err.code === 'EADDRINUSE' ? tr('log.portInUse', { port }) : err.message);
    process.exit(1);
  });
  // Ce PC seulement, sauf accès réseau demandé avec la protection active (bindHost) :
  // le panneau lance des programmes.
  server.listen(port, host, async () => {
    log(`Servym → http://localhost:${port}`);
    if (host === '0.0.0.0') for (const a of sys.lanAddresses()) log(`Servym → http://${a.address}:${port} (${a.name})`);
    await manager.init();
  });
  // Les projets lancés continuent de tourner : la prochaine instance les reprendra.
  async function shutdown(code = 0) {
    await manager.close();
    server.closeAllConnections();
    server.close(() => process.exit(code));
  }
  process.on('SIGINT', () => shutdown()).on('SIGTERM', () => shutdown());
  process.on('uncaughtException', (err) => {
    log('[crash]', err.stack);
    process.exit(1);
  });
}
