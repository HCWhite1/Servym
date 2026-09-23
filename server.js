// Servynex : portail local qui lance, surveille et arrête des projets.
// Aucune dépendance : node:http suffit pour une page et une API JSON.
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { ConfigError, createProject, DEFAULT_CONFIG, ROOT_DIR, loadConfig, patchProject, saveConfig } from './src/config.js';
import { Auth, AuthError } from './src/auth.js';
import { followLog, history, tailLines } from './src/logs.js';
import { HttpError, Manager } from './src/processes.js';
import { staticHandler } from './src/static.js';
import * as sys from './src/system.js';

const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);
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

async function readJson(req) {
  if (!String(req.headers['content-type']).startsWith('application/json')) throw new HttpError(415, 'JSON attendu');
  let raw = '';
  for await (const chunk of req) {
    raw += chunk;
    if (raw.length > 16_000) throw new HttpError(413, 'Requête trop grosse');
  }
  try {
    return JSON.parse(raw || '{}');
  } catch {
    throw new HttpError(400, 'JSON invalide');
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
const COSTLY_GET = /^\/api\/(free-port|git|ports|network|autostart)(\?|$)/;

// Host : bloque le DNS rebinding. En-tête X-Servynex-Request et Origin : bloquent le
// CSRF (un site tiers ne peut pas poser l'en-tête sans pré-vol CORS, refusé).
function guard(req) {
  const host = String(req.headers.host ?? '').replace(/:\d+$/, '').toLowerCase();
  if (!LOOPBACK.has(host)) throw new HttpError(421, 'Hôte refusé');
  const site = req.headers['sec-fetch-site'];
  if (req.url.startsWith('/api/') && site && site !== 'same-origin' && site !== 'none') throw new HttpError(403, 'Requête inter-sites refusée');
  if ((req.method === 'GET' || req.method === 'HEAD') && !COSTLY_GET.test(req.url)) return;
  if (req.headers['x-servynex-request'] !== '1') throw new HttpError(403, 'Requête non autorisée');
  const { origin } = req.headers;
  if (origin === undefined) return;
  let sameHost = false;
  try {
    sameHost = new URL(origin).host === req.headers.host;
  } catch {}
  if (!sameHost) throw new HttpError(403, 'Origine refusée');
}

// Permission exigée par une route de l'API (voir src/auth.js).
const OPERATE = new Set(['start', 'stop', 'restart', 'open', 'folder', 'code', 'terminal']);
const INSTANCE_ACTION_PERMISSION = { config: 'commands', log: 'instance', folder: 'instance', onboarded: 'view' };
function permissionFor(section, id, action, method) {
  if (method === 'GET' || method === 'HEAD') return 'view';
  if (section === 'projects' && !id && method === 'POST') return 'commands';
  if (section === 'projects' && id && !action && method === 'PATCH') return 'edit';
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
  restart = null, // arrêt de Servynex pour qu'un superviseur le relance (null : impossible)
  task = sys.autostartTask, // tâche planifiée qui démarre Servynex avec la session
  clock = () => Date.now(), // horloge de l'authentification (remplacée dans les tests)
  ...options
} = {}) {
  // projects.json invalide au démarrage : Servynex démarre quand même, avec la
  // dernière configuration valide, et l'interface affiche l'erreur (mode dégradé).
  const validCopy = path.join(runtimeDir, 'projects.valid.json');
  const events = new EventEmitter();
  let configError = null;
  let initial;
  try {
    initial = loadConfig(configFile);
  } catch (err) {
    if (!(err instanceof ConfigError)) throw err;
    configError = err.message;
    try {
      initial = loadConfig(validCopy);
    } catch {
      initial = structuredClone(DEFAULT_CONFIG);
    }
  }
  const manager = new Manager({ config: initial, logsDir, runtimeDir, ...options });
  const serveUi = staticHandler(path.join(ROOT_DIR, 'public'));
  const gitCache = new Map();
  const ownLog = path.join(logsDir, 'servynex.log');
  const ownPrevLog = path.join(logsDir, 'servynex.prev.log');
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

  const setConfigError = (message) => {
    if (message === configError) return;
    configError = message;
    events.emit('instance', { configError });
  };
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
        setConfigError(err.message);
        throw new ConfigError(`projects.json contient une erreur, à corriger d'abord : ${err.message}`);
      }
      throw err;
    }
  }
  function applyConfig(next) {
    manager.assertCanApply(next);
    saveConfig(configFile, next);
    manager.setConfig(next);
    rememberValid(next);
    setConfigError(null);
  }

  const log = (...parts) => {
    const line = `${new Date().toISOString()} ${parts.join(' ')}\n`;
    process.stdout.write(line);
    // Même rotation que les journaux de projets : servynex.prev.log garde le précédent.
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

  // Historique : chaque changement d'état est écrit dans servynex.log, pour pouvoir
  // expliquer après coup un arrêt survenu en l'absence de l'utilisateur.
  manager.on('transition', ({ id, from, to, managed, exitCode, error, detail }) => {
    const details = [];
    if (to === 'stopping' && detail) details.push(detail);
    const actor = actors.get(id);
    if (actor && ['starting', 'stopping'].includes(to) && clock() - actor.at < 10_000) details.push(`par ${actor.user}`);
    if (to === 'running' && !managed) details.push('lancé hors de Servynex');
    if (exitCode !== null && ['stopped', 'failed'].includes(to)) details.push(`code ${exitCode}`);
    if (error) details.push(error.replace(/\s+/g, ' '));
    log(`[état] ${id} ${from} → ${to}${details.length ? ` · ${details.join(' · ')}` : ''}`);
  });

  // Flux SSE ouverts par session : fermés quand la session est révoquée ou verrouillée.
  const streams = new Map();
  const auth = new Auth({
    file: path.join(runtimeDir, 'auth.json'),
    log,
    now: clock,
    instanceName: () => manager.config.instance.name ?? null,
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

  manager.on('warning', (message) => log(message));
  if (configError) log(`[config] projects.json invalide, dernière configuration valide utilisée : ${configError}`);
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

  // Actions sur Servynex lui-même (page Paramètres). Chemins fixes : rien ne vient de la requête.
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
    throw new HttpError(404, 'Aucun port libre trouvé');
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
    const audit = (project, text) => log(`[audit] ${project} ${who} · ${text}`);

    if (section === 'projects' && !id && method === 'GET') {
      const instance = {
        port: manager.config.instance.port,
        version: VERSION,
        node: process.version,
        dir: ROOT_DIR,
        config: configFile,
        tools: sys.tools,
        lan: sys.lanAddresses(),
        restartable: Boolean(restart),
        name: manager.config.instance.name ?? null,
        // Présentation vue : par compte si la protection est active, sinon pour Servynex.
        onboarded: user ? Boolean(user.prefs?.onboarded) : Boolean(readUi().onboarded),
        configError,
      };
      return send(res, 200, { instance, projects: manager.snapshot() });
    }
    if (section === 'projects' && !id && method === 'POST') {
      const { config: next, id: created } = createProject(diskConfig(), await readJson(req));
      applyConfig(next);
      log(`[config] ${created} ajouté`);
      audit(created, 'Projet ajouté');
      return send(res, 201, manager.view(manager.project(created)));
    }
    if (section === 'history' && !id && method === 'GET') {
      return send(res, 200, { entries: history([ownPrevLog, ownLog], null, 300) });
    }
    if (section === 'autostart' && !id) {
      if (method === 'GET') return send(res, 200, { enabled: await task.enabled() });
      if (method === 'PUT') {
        const { enabled } = await readJson(req);
        if (typeof enabled !== 'boolean') throw new HttpError(400, 'enabled doit valoir true ou false');
        await task.set(enabled);
        log(`[autostart] démarrage de Servynex avec la session ${enabled ? 'activé' : 'désactivé'}`);
        return send(res, 200, { enabled: await task.enabled() });
      }
    }
    if (section === 'instance' && method === 'POST' && id === 'onboarded' && user) {
      auth.setPrefs(user.id, { onboarded: true });
      return send(res, 200, { ok: true });
    }
    if (section === 'instance' && method === 'POST' && Object.hasOwn(instanceActions, id)) {
      await instanceActions[id]();
      return send(res, 200, { ok: true });
    }
    if (section === 'instance' && !id && method === 'PATCH') {
      // Nom de l'instance, affiché dans la barre latérale (« Servynex · Serveur maison »).
      const { name } = await readJson(req);
      if (name !== null && (typeof name !== 'string' || name.trim().length > 40)) throw new HttpError(400, 'Nom trop long (40 caractères au plus)');
      const next = diskConfig();
      if (name?.trim()) next.instance.name = name.trim();
      else delete next.instance.name;
      applyConfig(next);
      audit('-', `Nom de l’instance : ${name?.trim() || 'Servynex'}`);
      return send(res, 200, { name: next.instance.name ?? null });
    }
    if (section === 'events' && method === 'GET') {
      const { emit, done } = openStream(req, res);
      trackStream(req, res);
      const onChange = (project) => emit('state', project);
      const onConfig = () => emit('snapshot', manager.snapshot());
      const onInstance = (info) => emit('instance', info);
      emit('snapshot', manager.snapshot());
      emit('instance', { configError });
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
    if (section === 'network' && method === 'GET') return send(res, 200, { lan: sys.lanAddresses(), networks: await networkProfiles() });
    if (section === 'restart' && method === 'POST') {
      if (!restart) throw new HttpError(409, 'Servynex lancé à la main (npm start) : relancez-le depuis son terminal.');
      log(`[restart] Servynex${user ? ` · par ${who}` : ''}`);
      send(res, 200, { ok: true });
      return void setImmediate(restart);
    }
    if (section === 'reload' && method === 'POST') {
      let next;
      try {
        next = loadConfig(configFile);
      } catch (err) {
        if (err instanceof ConfigError) setConfigError(err.message);
        throw err;
      }
      manager.setConfig(next);
      rememberValid(next);
      setConfigError(null);
      log('[config] rechargée depuis le fichier');
      return send(res, 200, { ok: true });
    }
    if (section !== 'projects' || !id) throw new HttpError(404, 'Route inconnue');

    const project = manager.project(id);
    if (!action && method === 'PATCH') {
      const patch = await readJson(req);
      const sensitive = Object.keys(patch ?? {}).filter((k) => SENSITIVE.includes(k));
      if (sensitive.length && !auth.can(user, 'commands')) {
        throw new AuthError(403, 'Modifier la commande, le dossier, l’environnement ou l’exposition demande la permission « Commandes ».');
      }
      const next = patchProject(diskConfig(), id, patch);
      applyConfig(next);
      log(`[config] ${id} modifié`);
      audit(id, `Réglages modifiés : ${Object.keys(patch).join(', ')}`);
      return send(res, 200, manager.view(manager.project(id)));
    }
    if (action === 'logs' && method === 'GET') {
      const file = manager.logFile(id);
      if (url.searchParams.has('tail')) return send(res, 200, { lines: tailLines(file, 30) });
      const { emit, done } = openStream(req, res);
      trackStream(req, res);
      const stop = followLog(file, emit);
      await done;
      return stop();
    }
    if (action === 'history' && method === 'GET') {
      return send(res, 200, { entries: history([ownPrevLog, ownLog], id) });
    }
    if (method === 'POST' && Object.hasOwn(actions, action)) {
      log(`[${action}] ${id}${user ? ` · par ${who}` : ''}`);
      if (user) actors.set(id, { user: who, at: clock() });
      await actions[action](project);
      return send(res, 200, { ok: true });
    }
    throw new HttpError(404, 'Route inconnue');
  }

  const server = http.createServer(async (req, res) => {
    for (const [k, v] of Object.entries(HEADERS)) res.setHeader(k, v);
    try {
      guard(req);
      const url = new URL(req.url, 'http://localhost');
      if (url.pathname.startsWith('/api/')) return await api(req, res, url);
      if (await serveUi(req, res)) return;
      throw new HttpError(404, 'Introuvable');
    } catch (err) {
      const status = err instanceof HttpError || err instanceof ConfigError || err instanceof AuthError ? err.status : 500;
      if (status === 500) log('[erreur]', req.method, req.url, err.stack ?? err.message);
      if (res.headersSent) return res.end();
      send(res, status, { error: err.message, ...(err.extra ?? {}) });
    }
  });

  return { server, manager, log, auth };
}

// ---- Démarrage ----

// Code de sortie qui demande à scripts/start-hidden.vbs une relance immédiate.
const RESTART_CODE = 75;

if (import.meta.filename === path.resolve(process.argv[1] ?? '')) {
  let app;
  // Redémarrage depuis l'interface : seulement sous le superviseur, qui relance
  // Servynex ; lancé par npm start, personne ne le relancerait.
  const supervised = process.env.SERVYNEX_SUPERVISOR === 'vbs';
  try {
    app = createApp({ restart: supervised ? () => shutdown(RESTART_CODE) : null });
  } catch (err) {
    // Lancé sans fenêtre : l'erreur doit rester lisible dans le journal.
    const logsDir = path.join(ROOT_DIR, 'logs');
    fs.mkdirSync(logsDir, { recursive: true });
    fs.appendFileSync(path.join(logsDir, 'servynex.log'), `${new Date().toISOString()} [config] ${err.message}\n`);
    console.error(`[config] ${err.message}`);
    process.exit(1);
  }
  const { server, manager, log } = app;
  const port = Number(process.env.SERVYNEX_PORT) || manager.config.instance.port;
  server.on('error', (err) => {
    log('[écoute]', err.code === 'EADDRINUSE' ? `le port ${port} est déjà utilisé (une autre instance de Servynex tourne ?)` : err.message);
    process.exit(1);
  });
  // Local uniquement : le panneau lance des programmes, il ne doit pas être
  // joignable depuis le réseau.
  server.listen(port, '127.0.0.1', async () => {
    log(`Servynex → http://localhost:${port}`);
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
    log('[plantage]', err.stack);
    process.exit(1);
  });
}
