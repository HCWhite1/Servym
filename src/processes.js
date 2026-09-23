// Gestionnaire de processus : lance, surveille et arrête les projets.
//
// États : stopped, starting, running, stopping, failed, conflict.
// « managed » : Servynex connaît le PID (lancé par lui). Un projet qui répond
// sans être managed a été lancé ailleurs (tâche planifiée, start.bat…).
//
// Structures d'état du Manager (toutes indexées par id de projet) :
// - #states    : état affiché (IDLE + champs). Écrit uniquement par #set(), qui
//                émet « change » (et « transition » si l'état nommé change).
// - #ops       : action en cours, sous forme de jeton ({}). Posé par start() et
//                stop() ; chaque étape asynchrone vérifie que le jeton est
//                toujours le sien (#ops.get(id) === op), sinon elle abandonne.
//                Tant qu'il existe, tick() ne sonde pas le projet, et #refresh()
//                le revérifie après la sonde (une action a pu commencer entre-temps).
// - #relaunch  : fenêtre de relaunchMs après une sortie propre non demandée
//                (projet qui se relance lui-même) ; #refresh() y cherche le
//                nouveau PID par le port. start() est refusé pendant ce temps.
// - #checkedAt : date de la dernière sonde, pour la cadence (3 s arrêté, 15 s en ligne).
// - #ticking   : anti-réentrance de tick() (une sonde lente ne se superpose pas).
// - #statics   : fonction d'arrêt des projets « static » servis par Servynex.
// - #ready     : faux jusqu'à la première sonde ; l'état trouvé au démarrage de
//                Servynex n'est pas émis comme « transition » (historique).
// Seuls les projets managed sont persistés dans runtime/pids.json (#savePids).
// « dependency » : démarré automatiquement pour un projet qui en dépend (une base
// de données pour une application) ; il s'arrête avec le dernier projet qui l'utilisait.
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expand, hostOf, urlOf } from './config.js';
import { lastError } from './logs.js';
import { startStaticSite } from './static-server.js';
import * as sys from './system.js';

const LAUNCHER = path.join(import.meta.dirname, 'launch.ps1');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

export class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

const IDLE = { state: 'stopped', managed: false, pid: null, startedAt: null, exitCode: null, error: null, detail: null, activePort: null, activeHost: null, dependency: false };
const ACTIVE = ['running', 'starting', 'stopping'];

export class Manager extends EventEmitter {
  #config;
  #states = new Map();
  #ops = new Map(); // action en cours par projet (jeton)
  #statics = new Map();
  #timer = null;
  #ticking = false;
  #checkedAt = new Map();
  #relaunch = new Map(); // projets sortis proprement, peut-être en train de se relancer
  #ready = false; // première sonde faite : les changements d'état sont réels

  constructor({ config, logsDir, runtimeDir, tickMs = 3000, runningTickMs = 15000, startTimeoutMs = 60000, relaunchMs = 30000, bootTime = Date.now() - os.uptime() * 1000 }) {
    super();
    this.#config = config;
    this.logsDir = logsDir;
    this.pidsFile = path.join(runtimeDir, 'pids.json');
    this.bootFile = path.join(runtimeDir, 'autostart.json');
    this.bootTime = bootTime;
    this.relaunchMs = relaunchMs;
    this.tickMs = tickMs;
    this.runningTickMs = runningTickMs;
    this.startTimeoutMs = startTimeoutMs;
    fs.mkdirSync(logsDir, { recursive: true });
    fs.mkdirSync(runtimeDir, { recursive: true });
  }

  get config() {
    return this.#config;
  }

  // Refuse de retirer un projet que Servynex fait tourner : il continuerait sans
  // surveillance, en gardant son port.
  assertCanApply(config) {
    const kept = new Set(config.projects.map((p) => p.id));
    const orphans = this.#config.projects.filter((p) => !kept.has(p.id) && (this.#statics.has(p.id) || (this.state(p.id).managed && ACTIVE.includes(this.state(p.id).state))));
    if (orphans.length) {
      const names = orphans.map((p) => p.name).join(', ');
      throw new HttpError(409, `${names} tourne encore : arrêtez-le avant de le retirer de projects.json.`);
    }
  }

  setConfig(config) {
    this.assertCanApply(config);
    this.#config = config;
    for (const id of this.#states.keys()) {
      if (!config.projects.some((p) => p.id === id)) this.#states.delete(id);
    }
    this.emit('config');
  }

  project(id) {
    const p = this.#config.projects.find((x) => x.id === id);
    if (!p) throw new HttpError(404, 'Projet inconnu');
    return p;
  }

  state(id) {
    return this.#states.get(id) ?? IDLE;
  }

  logFile(id) {
    return path.join(this.logsDir, `${id}.log`);
  }

  view(p) {
    return {
      id: p.id,
      name: p.name,
      icon: p.icon ?? 'box',
      color: p.color ?? '#8b5cf6',
      desc: p.desc ?? '',
      tags: p.tags ?? [],
      type: p.type ?? 'process',
      service: Boolean(p.service),
      autostart: Boolean(p.autostart),
      port: p.port,
      expose: Boolean(p.expose),
      host: hostOf(p),
      hostControl: p.type === 'static' ? 'internal' : p.type === 'file' ? 'none' : p.hostControl,
      fixedPort: p.fixedPort ?? null,
      requires: p.requires ?? [],
      cwd: p.cwd,
      url: urlOf(p),
      command: p.type === 'static' ? 'Serveur de fichiers intégré à Servynex' : p.type === 'file' ? 'Aucune : ouvert dans le navigateur' : [p.cmd, ...p.args].map((a) => expand(a, p)).join(' '),
      env: p.env ?? {},
      open: p.open ?? null,
      file: p.type === 'file' ? path.join(p.cwd, p.open) : null,
      hasLog: fs.existsSync(this.logFile(p.id)),
      source: p, // configuration brute, pour le formulaire de l'interface
      ...(p.type === 'file' ? { ...IDLE, state: 'file' } : this.state(p.id)),
    };
  }

  snapshot() {
    return this.#config.projects.map((p) => this.view(p));
  }

  #set(id, patch) {
    const before = this.state(id);
    const after = { ...before, ...patch };
    if (JSON.stringify(before) === JSON.stringify(after)) return;
    this.#states.set(id, after);
    const p = this.#config.projects.find((x) => x.id === id);
    if (!p) return;
    this.emit('change', this.view(p));
    // Avant la première sonde, l'état « stopped » n'est qu'une valeur par défaut :
    // trouver un projet déjà en ligne au démarrage de Servynex n'est pas un changement.
    // Une action lancée entre-temps (jeton dans #ops) en est bien un.
    const real = this.#ready || this.#ops.has(id);
    if (before.state !== after.state && real) this.emit('transition', { id, from: before.state, to: after.state, ...after });
  }

  // ---- Surveillance ----

  async init() {
    let saved = {};
    try {
      saved = JSON.parse(fs.readFileSync(this.pidsFile, 'utf8'));
    } catch {}
    // Projets lancés par une instance précédente de Servynex : on les reprend.
    for (const p of this.#config.projects) {
      const s = saved[p.id];
      if (s && sys.isAlive(s.pid)) this.#set(p.id, { ...s, managed: true, state: 'running' });
    }
    // Projet retiré du fichier pendant que Servynex était arrêté : il tourne toujours.
    for (const [id, s] of Object.entries(saved)) {
      if (!this.#config.projects.some((p) => p.id === id) && sys.isAlive(s.pid)) {
        this.emit('warning', `[config] « ${id} » ne figure plus dans projects.json mais son processus ${s.pid} tourne encore`);
      }
    }
    await this.tick();
    this.#ready = true;
    this.#timer = setInterval(() => this.tick(), this.tickMs);
    this.#timer.unref();
    this.#autostart();
  }

  // Lance les projets « autostart » une fois par démarrage du PC : un simple
  // redémarrage de Servynex ne relance pas un projet arrêté volontairement.
  #autostart() {
    let last = null;
    try {
      last = JSON.parse(fs.readFileSync(this.bootFile, 'utf8')).bootTime;
    } catch {}
    if (last !== null && Math.abs(last - this.bootTime) < 120_000) return;
    fs.writeFileSync(this.bootFile, JSON.stringify({ bootTime: this.bootTime }));
    for (const p of this.#config.projects) {
      if (!p.autostart || p.type === 'file' || this.state(p.id).state !== 'stopped' || this.#ops.has(p.id)) continue;
      try {
        this.start(p.id);
      } catch {}
    }
  }

  async close() {
    clearInterval(this.#timer);
    await Promise.all([...this.#statics.keys()].map((id) => this.#stopStatic(id)));
  }

  async #stopStatic(id) {
    const stop = this.#statics.get(id);
    this.#statics.delete(id);
    await stop?.();
  }

  async tick() {
    if (this.#ticking) return;
    this.#ticking = true;
    try {
      // Un projet arrêté est sondé souvent (pour voir tout de suite s'il est lancé
      // ailleurs) ; un projet en ligne plus rarement, chaque sonde coûtant une requête.
      const now = Date.now();
      const due = this.#config.projects.filter((p) => {
        if (this.#ops.has(p.id) || p.type === 'static' || p.type === 'file') return false;
        const every = this.state(p.id).state === 'running' ? this.runningTickMs : 0;
        return now - (this.#checkedAt.get(p.id) ?? 0) >= every;
      });
      for (const p of due) this.#checkedAt.set(p.id, now);
      await Promise.all(due.map((p) => this.#refresh(p)));
    } finally {
      this.#ticking = false;
    }
  }

  #health(p, port = this.state(p.id).activePort ?? p.port) {
    if (p.health?.type === 'tcp') return sys.portInUse(port).then((up) => (up ? 'ok' : 'down'));
    return sys.httpHealth(port, { ...p.health, tls: urlOf(p)?.startsWith('https:') });
  }

  async #refresh(p) {
    const st = this.state(p.id);
    if (st.managed && !sys.isAlive(st.pid)) return this.#exited(p.id, st.pid, null);
    const health = await this.#health(p);
    if (this.#ops.has(p.id)) return; // une action a commencé pendant le contrôle
    const relaunch = this.#relaunch.get(p.id);
    if (relaunch) {
      // Le nouveau processus hérite de la sortie de l'ancien : on le reprend
      // (PID trouvé par le port), avec son journal.
      const pid = health === 'ok' ? await sys.pidOnPort(relaunch.activePort ?? p.port) : null;
      if (pid) {
        this.#relaunch.delete(p.id);
        this.#set(p.id, { ...IDLE, state: 'running', managed: true, pid, startedAt: Date.now(), activePort: relaunch.activePort, activeHost: relaunch.activeHost });
        return this.#savePids();
      }
      if (Date.now() < relaunch.until) return;
      this.#relaunch.delete(p.id);
      return this.#set(p.id, { ...IDLE, exitCode: relaunch.code });
    }
    if (health === 'ok') {
      this.#set(p.id, { state: 'running', error: null, detail: null });
    } else if (health === 'mismatch') {
      this.#set(p.id, { state: 'conflict', error: `Le port ${st.activePort ?? p.port} répond, mais ce n'est pas ${p.name}.` });
    } else if (st.managed) {
      this.#set(p.id, { state: 'failed', error: `Le processus tourne mais ne répond plus sur le port ${st.activePort ?? p.port}.` });
    } else if (st.state !== 'failed') {
      this.#set(p.id, { ...IDLE, exitCode: st.exitCode });
    }
  }

  #savePids() {
    const pids = {};
    for (const p of this.#config.projects) {
      const { managed, pid, startedAt, activePort, activeHost, dependency } = this.state(p.id);
      if (managed && p.type !== 'static') pids[p.id] = { pid, startedAt, activePort, activeHost, dependency };
    }
    fs.writeFileSync(this.pidsFile, JSON.stringify(pids, null, 2));
  }

  #exited(id, pid, code) {
    const st = this.state(id);
    if (st.pid !== pid || !st.managed) return;
    if (st.state !== 'stopping' && (code === 0 || code === null)) {
      // Sortie propre sans demande d'arrêt : le projet se relance peut-être
      // lui-même (bouton « Redémarrer » d’une application). On attend un peu.
      this.#relaunch.set(id, { until: Date.now() + this.relaunchMs, code, activePort: st.activePort, activeHost: st.activeHost });
      this.#set(id, { ...IDLE, state: 'starting', detail: 'Redémarrage…', activePort: st.activePort, activeHost: st.activeHost });
      return this.#savePids();
    }
    const expected = st.state === 'stopping' || code === 0;
    this.#set(id, {
      ...IDLE,
      state: expected ? 'stopped' : 'failed',
      exitCode: code,
      error: expected ? null : lastError(this.logFile(id)) ?? `Arrêt inattendu${code === null ? '' : ` (code ${code})`}`,
    });
    this.#savePids();
  }

  // ---- Actions ----

  #refuse(p, verb, why) {
    return new HttpError(409, `Impossible de ${verb} ${p.name} : ${why}.`);
  }

  // Lance en arrière-plan : la promesse se termine quand le projet répond
  // (ou a échoué), mais les vérifications immédiates lèvent tout de suite.
  // asDependency : lancé pour un projet qui en dépend (voir « dependency »).
  start(id, { asDependency = false } = {}) {
    const p = this.project(id);
    if (p.type === 'file') throw this.#refuse(p, 'lancer', 's’ouvre directement, sans serveur');
    const st = this.state(id);
    if (this.#ops.has(id) || !['stopped', 'failed'].includes(st.state) || this.#relaunch.has(id)) {
      throw this.#refuse(p, 'lancer', st.state === 'running' ? 'il tourne déjà' : 'une action est déjà en cours');
    }
    this.#relaunch.delete(id);
    const op = {};
    this.#ops.set(id, op);
    return this.#start(p, op, asDependency)
      .catch((err) => this.#ops.get(id) === op && this.#set(id, { state: 'failed', error: err.message, detail: null }))
      .finally(() => this.#ops.get(id) === op && this.#ops.delete(id));
  }

  async #start(p, op, asDependency) {
    const aborted = () => this.#ops.get(p.id) !== op;
    this.#set(p.id, { ...IDLE, state: 'starting', dependency: asDependency });
    for (const depId of p.requires ?? []) {
      const dep = this.project(depId);
      if (this.state(depId).state === 'running') continue;
      this.#set(p.id, { detail: `Attente de ${dep.name}…` });
      if (!this.#ops.has(depId)) this.start(depId, { asDependency: true });
      while (this.#ops.has(depId)) await sleep(250);
      if (aborted()) return;
      if (this.state(depId).state !== 'running') throw new Error(`${dep.name} n'a pas démarré : ${this.state(depId).error ?? 'raison inconnue'}`);
    }
    this.#set(p.id, { detail: null });

    if (await sys.portInUse(p.port)) {
      if ((await this.#health(p, p.port)) === 'ok') return this.#set(p.id, { state: 'running' });
      const pid = await sys.pidOnPort(p.port);
      throw new Error(`Le port ${p.port} est déjà pris${pid ? ` par le processus ${pid}` : ''}. Choisissez un autre port dans les réglages du projet.`);
    }
    if (!fs.existsSync(p.cwd)) throw new Error(`Dossier introuvable : ${p.cwd}`);

    const active = { activePort: p.port, activeHost: hostOf(p), startedAt: Date.now() };
    if (p.type === 'static') {
      this.#statics.set(p.id, await startStaticSite({ root: p.cwd, port: p.port, host: active.activeHost, logFile: this.logFile(p.id) }));
      return this.#set(p.id, { ...active, state: 'running', managed: true, pid: process.pid });
    }
    if (aborted()) return;
    const pid = await this.#spawn(p);
    if (aborted()) return sys.killTree(pid).catch(() => {}); // arrêt demandé pendant le lancement
    this.#set(p.id, { ...active, managed: true, pid });
    this.#savePids();
    await this.#waitHealthy(p, pid);
  }

  #spawn(p) {
    const log = this.logFile(p.id);
    try {
      fs.renameSync(log, path.join(this.logsDir, `${p.id}.prev.log`));
    } catch {}
    const argv = [expand(p.cmd, p), ...p.args.map((a) => expand(a, p))];
    const stamp = new Date().toLocaleString('fr-FR');
    fs.writeFileSync(log, `[servynex] ${stamp} · ${argv.join(' ')} · ${hostOf(p)}:${p.port}\n`);

    const env = sys.cleanEnv({
      ...p.env,
      PORT: String(p.port),
      NO_COLOR: '1',
      SERVYNEX_CMDLINE: argv.map((a) => `"${a}"`).join(' '),
      SERVYNEX_LOG: log,
      SERVYNEX_CWD: p.cwd,
    });
    delete env.HOST;
    // HOST vide = toutes les interfaces (IPv4 et IPv6) pour les projets Node.
    if (p.hostControl === 'env' && !p.expose) env.HOST = '127.0.0.1';

    const child = spawn('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', LAUNCHER], {
      env,
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    return new Promise((resolve, reject) => {
      let out = '';
      let err = '';
      let pid = null;
      child.stdout.setEncoding('utf8').on('data', (d) => {
        out += d;
        const m = !pid && out.match(/^SERVYNEX_PID=(\d+)\r?\n/m); // écrit par launch.ps1
        if (m) resolve((pid = Number(m[1])));
      });
      child.stderr.setEncoding('utf8').on('data', (d) => (err += d));
      child.on('error', reject);
      child.on('exit', (code) => {
        if (pid) this.#exited(p.id, pid, code);
        else reject(new Error(err.trim().split('\n')[0] || `Lancement impossible (code ${code})`));
      });
    });
  }

  async #waitHealthy(p, pid) {
    const seconds = p.startTimeout ?? this.startTimeoutMs / 1000;
    const deadline = Date.now() + seconds * 1000;
    while (Date.now() < deadline) {
      const st = this.state(p.id);
      if (st.pid !== pid || st.state !== 'starting') return; // arrêté ou mort entre-temps
      const health = await this.#health(p);
      if (health === 'ok') return this.#set(p.id, { state: 'running' });
      if (health === 'mismatch') throw new Error(`Le port ${p.port} répond, mais ce n'est pas ${p.name}.`);
      await sleep(600);
    }
    throw new Error(`${p.name} ne répond pas sur le port ${p.port} après ${seconds} s : voir son journal.`);
  }

  // Arrêt : commande d'arrêt propre si le projet en déclare une, sinon (ou
  // si elle ne suffit pas) arrêt forcé de tout l'arbre de processus.
  // cascade : arrête ensuite les dépendances démarrées pour lui et devenues inutiles.
  async stop(id, { cascade = false, reason = null } = {}) {
    const p = this.project(id);
    if (p.type === 'file') throw this.#refuse(p, 'arrêter', 'aucun processus ne tourne');
    const st = this.state(id);
    if (this.#ops.has(id) && st.state !== 'starting') throw this.#refuse(p, 'arrêter', 'une action est déjà en cours');
    if (st.state === 'stopped' || (st.state === 'failed' && !st.managed)) throw this.#refuse(p, 'arrêter', 'il est déjà arrêté');
    // Sans PID connu, le port désigne le processus : pas question de tuer un programme étranger.
    if (st.state === 'conflict' && !st.managed) throw this.#refuse(p, 'arrêter', `le port ${p.port} est occupé par un autre programme`);
    const op = {};
    this.#ops.set(id, op); // remplace un démarrage en cours, qui s'interrompt
    this.#relaunch.delete(id);
    try {
      this.#set(id, { state: 'stopping', error: null, detail: reason });
      if (p.type === 'static') {
        await this.#stopStatic(id);
        this.#set(id, { ...IDLE });
        if (cascade) await this.#stopUnusedDependencies(p);
        return;
      }
      const pid = st.managed ? st.pid : await sys.pidOnPort(st.activePort ?? p.port);
      if (pid && p.stop) {
        await sys.runQuiet(expand(p.stop.cmd, p), p.stop.args.map((a) => expand(a, p))).catch(() => {});
        for (let i = 0; i < 60 && sys.isAlive(pid); i++) await sleep(250);
      }
      if (pid && sys.isAlive(pid)) await sys.killTree(pid);
      if (st.managed) this.#exited(id, pid, null);
      this.#set(id, { ...IDLE });
      if (cascade) await this.#stopUnusedDependencies(p);
    } catch (err) {
      this.#set(id, { state: st.state === 'starting' ? 'failed' : st.state, error: err.message });
      throw new HttpError(500, err.message);
    } finally {
      if (this.#ops.get(id) === op) this.#ops.delete(id);
    }
  }

  // Dépendances lancées automatiquement pour p, qu'aucun autre projet actif n'utilise.
  async #stopUnusedDependencies(p) {
    for (const depId of p.requires ?? []) {
      const dep = this.#config.projects.find((x) => x.id === depId);
      const st = this.state(depId);
      if (!dep || !st.dependency || !st.managed || st.state !== 'running' || this.#ops.has(depId)) continue;
      const users = this.#config.projects.filter((x) => x.id !== p.id && (x.requires ?? []).includes(depId) && ACTIVE.includes(this.state(x.id).state));
      if (users.length) continue;
      await this.stop(depId, { cascade: true, reason: `Plus utilisé par ${p.name}` }).catch((err) => this.emit('warning', `[stop] ${depId} : ${err.message}`));
    }
  }

  openFile(id) {
    const p = this.project(id);
    if (p.type !== 'file') throw new HttpError(400, `${p.name} n'est pas un fichier à ouvrir`);
    const file = path.join(p.cwd, p.open);
    if (!fs.existsSync(file)) throw new HttpError(404, `Fichier introuvable : ${file}`);
    sys.openFile(file);
  }

  async restart(id) {
    const st = this.state(id);
    if (st.state !== 'stopped' && !(st.state === 'failed' && !st.managed)) await this.stop(id);
    // Volontairement sans await : l'API répond dès le lancement, la suite
    // (démarrage, échec) arrive par le flux d'évènements.
    this.start(id);
  }
}
