// Gestionnaire de processus : lance, surveille et arrête les projets.
//
// États : stopped, starting, running, stopping, failed, conflict.
// « managed » : Servym connaît le PID (lancé par lui). Un projet qui répond
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
//                (projet qui se relance lui-même), launcherMs si elle survient
//                pendant le lancement (lanceur qui laisse un serveur en arrière-plan) ;
//                #refresh() y cherche le nouveau PID par le port. start() est
//                refusé pendant ce temps.
// - #checkedAt : date de la dernière sonde, pour la cadence (3 s arrêté, 15 s en ligne).
// - #ticking   : anti-réentrance de tick() (une sonde lente ne se superpose pas).
// - #statics   : fonction d'arrêt des projets « static » servis par Servym.
// - #watched   : PID lancés par cette instance, dont la sortie arrivera avec son code
//                (onExit) ; #refresh() l'attend un peu avant de conclure sans lui.
// - #retries   : relance automatique après un plantage (restartOnCrash) : dates des
//                plantages des restartWindowMs dernières ms, minuterie de la
//                prochaine relance, et « dependency » du lancement qui a planté.
// - #ready     : faux jusqu'à la première sonde ; l'état trouvé au démarrage de
//                Servym n'est pas émis comme « transition » (historique).
// Seuls les projets managed sont persistés dans runtime/pids.json (#savePids).
// « dependency » : démarré automatiquement pour un projet qui en dépend (une base
// de données pour une application) ; il s'arrête avec le dernier projet qui l'utilisait.
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { expand, hostOf, urlOf } from './config.js';
import { LocalError, msg, serverLang, tr } from './i18n.js';
import { lastError, startupReason } from './logs.js';
import { startStaticSite } from './static-server.js';
import * as sys from './system.js';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Message : clé de src/locales/<langue>.js et ses paramètres (« raw » : texte déjà traduit).
export class HttpError extends LocalError {}

// autoRestart : 'scheduled' (relance automatique prévue) ou 'gaveUp' (abandonnée), sinon null.
const IDLE = { state: 'stopped', managed: false, pid: null, startedAt: null, exitCode: null, error: null, detail: null, activePort: null, activeHost: null, dependency: false, autoRestart: null };
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
  #watched = new Set(); // PID lancés par cette instance, dont la sortie sera signalée (onExit)
  #retries = new Map(); // relance automatique après un plantage
  #ready = false; // première sonde faite : les changements d'état sont réels

  constructor({ config, logsDir, runtimeDir, tickMs = 3000, runningTickMs = 15000, startTimeoutMs = 60000, relaunchMs = 30000, launcherMs = 10000, restartDelaysMs = [2000, 10000, 30000], restartWindowMs = 300_000, exitGraceMs = 1500, bootTime = Date.now() - os.uptime() * 1000 }) {
    super();
    this.#config = config;
    this.logsDir = logsDir;
    this.pidsFile = path.join(runtimeDir, 'pids.json');
    this.bootFile = path.join(runtimeDir, 'autostart.json');
    this.bootTime = bootTime;
    this.relaunchMs = relaunchMs;
    this.launcherMs = launcherMs;
    this.restartDelaysMs = restartDelaysMs;
    this.restartWindowMs = restartWindowMs;
    this.exitGraceMs = exitGraceMs;
    this.tickMs = tickMs;
    this.runningTickMs = runningTickMs;
    this.startTimeoutMs = startTimeoutMs;
    fs.mkdirSync(logsDir, { recursive: true });
    fs.mkdirSync(runtimeDir, { recursive: true });
  }

  get config() {
    return this.#config;
  }

  // Refuse de retirer un projet que Servym fait tourner : il continuerait sans
  // surveillance, en gardant son port.
  assertCanApply(config) {
    const kept = new Set(config.projects.map((p) => p.id));
    const orphans = this.#config.projects.filter((p) => !kept.has(p.id) && (this.#statics.has(p.id) || (this.state(p.id).managed && ACTIVE.includes(this.state(p.id).state))));
    if (orphans.length) {
      const names = orphans.map((p) => p.name).join(', ');
      throw new HttpError(409, 'process.stillRunning', { names });
    }
  }

  setConfig(config) {
    this.assertCanApply(config);
    this.#config = config;
    for (const id of this.#states.keys()) {
      if (!config.projects.some((p) => p.id === id)) this.#states.delete(id);
    }
    // Relance automatique désactivée (ou projet retiré) : celle qui était prévue n'a plus lieu.
    for (const id of this.#retries.keys()) {
      if (config.projects.find((p) => p.id === id)?.restartOnCrash) continue;
      this.#cancelRestart(id);
      if (this.state(id).autoRestart === 'scheduled') this.#set(id, { detail: null, autoRestart: null });
    }
    this.emit('config');
  }

  project(id) {
    const p = this.#config.projects.find((x) => x.id === id);
    if (!p) throw new HttpError(404, 'project.unknown');
    return p;
  }

  state(id) {
    return this.#states.get(id) ?? IDLE;
  }

  logFile(id) {
    return path.join(this.logsDir, `${id}.log`);
  }

  // Journal du lancement précédent : le journal est renommé ainsi à chaque lancement.
  prevLogFile(id) {
    return path.join(this.logsDir, `${id}.prev.log`);
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
      restartOnCrash: Boolean(p.restartOnCrash),
      port: p.port,
      expose: Boolean(p.expose),
      host: hostOf(p),
      hostControl: p.type === 'static' ? 'internal' : p.type === 'file' ? 'none' : p.hostControl,
      fixedPort: p.fixedPort ?? null,
      requires: p.requires ?? [],
      cwd: p.cwd,
      url: urlOf(p),
      command: p.type === 'static' ? tr('process.builtinServer') : p.type === 'file' ? tr('process.openedInBrowser') : [p.cmd, ...p.args].map((a) => expand(a, p)).join(' '),
      env: p.env ?? {},
      open: p.open ?? null,
      file: p.type === 'file' ? path.join(p.cwd, p.open) : null,
      hasLog: fs.existsSync(this.logFile(p.id)),
      hasPrevLog: fs.existsSync(this.prevLogFile(p.id)),
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
    // trouver un projet déjà en ligne au démarrage de Servym n'est pas un changement.
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
    // Projets lancés par une instance précédente de Servym : on les reprend.
    for (const p of this.#config.projects) {
      const s = saved[p.id];
      if (s && sys.isAlive(s.pid)) this.#set(p.id, { ...s, managed: true, state: 'running' });
    }
    // Projet retiré du fichier pendant que Servym était arrêté : il tourne toujours.
    for (const [id, s] of Object.entries(saved)) {
      if (!this.#config.projects.some((p) => p.id === id) && sys.isAlive(s.pid)) {
        this.emit('warning', `[config] ${tr('process.orphan', { id, pid: s.pid })}`);
      }
    }
    await this.tick();
    this.#ready = true;
    this.#timer = setInterval(() => this.tick(), this.tickMs);
    this.#timer.unref();
    this.#autostart();
  }

  // Lance les projets « autostart » une fois par démarrage du PC : un simple
  // redémarrage de Servym ne relance pas un projet arrêté volontairement.
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
    for (const id of this.#retries.keys()) this.#cancelRestart(id);
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
    if (st.managed && !sys.isAlive(st.pid)) {
      // Lancé par cette instance : sa sortie, avec son code, arrive juste après.
      // Sans elle, un plantage passerait pour une sortie propre (code inconnu).
      if (this.#watched.has(st.pid)) {
        await sleep(this.exitGraceMs);
        if (this.state(p.id).pid !== st.pid) return;
      }
      return this.#exited(p.id, st.pid, null);
    }
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
      if (!relaunch.starting) return this.#set(p.id, { ...IDLE, exitCode: relaunch.code });
      // Sorti pendant son lancement sans que rien ne réponde : c'est un échec, et
      // son journal dit souvent pourquoi (« The local web server is already running »).
      const params = { name: p.name, port: relaunch.activePort ?? p.port, reason: startupReason(this.logFile(p.id)) };
      return this.#set(p.id, { ...IDLE, state: 'failed', exitCode: relaunch.code, error: tr(params.reason ? 'process.exitedDuringStartWhy' : 'process.exitedDuringStart', params) });
    }
    if (health === 'ok') {
      this.#set(p.id, { state: 'running', error: null, detail: null });
    } else if (health === 'mismatch') {
      this.#set(p.id, { state: 'conflict', error: tr('process.wrongService', { port: st.activePort ?? p.port, name: p.name }) });
    } else if (st.managed) {
      // Un projet qui plante ferme son port un peu avant que sa sortie soit connue :
      // on laisse à celle-ci le temps d'arriver (code de sortie, relance automatique).
      if (st.state === 'running') {
        await sleep(this.exitGraceMs);
        if (this.#ops.has(p.id) || this.state(p.id).pid !== st.pid || !sys.isAlive(st.pid)) return;
      }
      this.#set(p.id, { state: 'failed', error: tr('process.notResponding', { port: st.activePort ?? p.port }) });
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
      // Pendant le lancement, c'est un lanceur qui a peut-être laissé tourner le
      // serveur en arrière-plan : attente plus courte, échec expliqué ensuite.
      const starting = st.state === 'starting';
      const port = st.activePort ?? this.project(id).port;
      this.#relaunch.set(id, { until: Date.now() + (starting ? this.launcherMs : this.relaunchMs), code, starting, activePort: st.activePort, activeHost: st.activeHost });
      this.#set(id, { ...IDLE, state: 'starting', detail: starting ? tr('process.launcherDone', { port }) : tr('process.restarting'), activePort: st.activePort, activeHost: st.activeHost });
      return this.#savePids();
    }
    const expected = st.state === 'stopping' || code === 0;
    // Plantage d'un projet en ligne : relancé si demandé. Pas après un échec au
    // lancement (port pris, commande introuvable…) : relancer ne corrigerait rien.
    const retry = !expected && st.state === 'running' ? this.#planRestart(id, st.dependency) : null;
    this.#set(id, {
      ...IDLE,
      state: expected ? 'stopped' : 'failed',
      exitCode: code,
      error: expected ? null : lastError(this.logFile(id)) ?? (code === null ? tr('process.unexpectedExit') : tr('process.unexpectedExitCode', { code })),
      detail: retry?.detail ?? null,
      autoRestart: retry?.kind ?? null,
    });
    // Mort pendant le lancement : celui-ci est fini (#waitHealthy le verra au
    // prochain tour), le projet peut être relancé tout de suite.
    if (st.state === 'starting') this.#ops.delete(id);
    this.#savePids();
  }

  // ---- Relance automatique (restartOnCrash) ----

  // Prévoit la relance d'un projet qui vient de planter, avec un délai qui grandit
  // (restartDelaysMs). Autant de relances en restartWindowMs que de délais : au
  // plantage suivant, abandon, le projet reste en échec. → { kind, detail } ou null.
  #planRestart(id, dependency) {
    if (!this.#config.projects.find((p) => p.id === id)?.restartOnCrash) return null;
    const now = Date.now();
    const retry = this.#retries.get(id) ?? { crashes: [], timer: null };
    retry.crashes = retry.crashes.filter((at) => now - at < this.restartWindowMs);
    const max = this.restartDelaysMs.length;
    if (retry.crashes.length >= max) {
      this.#cancelRestart(id);
      return { kind: 'gaveUp', detail: tr('process.restartGaveUp', { count: max, minutes: Math.round(this.restartWindowMs / 60000) }) };
    }
    retry.crashes.push(now);
    const attempt = retry.crashes.length;
    const delay = this.restartDelaysMs[attempt - 1];
    clearTimeout(retry.timer);
    retry.timer = setTimeout(() => this.#autoRestart(id, attempt, max), delay);
    retry.timer.unref();
    retry.dependency = dependency;
    this.#retries.set(id, retry);
    return { kind: 'scheduled', detail: tr('process.restartIn', { seconds: Math.ceil(delay / 1000), attempt, max }) };
  }

  // Oublie la relance prévue et les plantages comptés (lancement ou arrêt demandé, réglage retiré).
  #cancelRestart(id) {
    clearTimeout(this.#retries.get(id)?.timer);
    this.#retries.delete(id);
  }

  #autoRestart(id, attempt, max) {
    const retry = this.#retries.get(id);
    if (!retry) return;
    retry.timer = null;
    if (this.state(id).autoRestart !== 'scheduled' || this.#ops.has(id)) return;
    try {
      this.start(id, { asDependency: retry.dependency, auto: tr('process.autoRestarted', { attempt, max }) });
    } catch (err) {
      this.#set(id, { detail: null, autoRestart: null });
      this.emit('warning', `[restart] ${id} : ${err.message}`);
    }
  }

  // ---- Actions ----

  // action : start ou stop ; why : clé de la raison (process.why.*).
  #refuse(p, action, why, params = {}) {
    return new HttpError(409, action === 'start' ? 'process.cannotStart' : 'process.cannotStop', { name: p.name, why: msg(why, params) });
  }

  // Lance en arrière-plan : la promesse se termine quand le projet répond
  // (ou a échoué), mais les vérifications immédiates lèvent tout de suite.
  // asDependency : lancé pour un projet qui en dépend (voir « dependency »).
  // auto : relance automatique après un plantage (texte affiché et inscrit à
  // l'historique) ; un lancement demandé remet à zéro le compte des plantages.
  start(id, { asDependency = false, auto = null } = {}) {
    const p = this.project(id);
    if (p.type === 'file') throw this.#refuse(p, 'start', 'process.why.file');
    const st = this.state(id);
    if (this.#ops.has(id) || !['stopped', 'failed'].includes(st.state) || this.#relaunch.has(id)) {
      throw this.#refuse(p, 'start', st.state === 'running' ? 'process.why.running' : 'process.why.busy');
    }
    this.#relaunch.delete(id);
    if (!auto) this.#cancelRestart(id);
    const op = {};
    this.#ops.set(id, op);
    return this.#start(p, op, asDependency, auto)
      .catch((err) => this.#ops.get(id) === op && this.#set(id, { state: 'failed', error: err.message, detail: null }))
      .finally(() => this.#ops.get(id) === op && this.#ops.delete(id));
  }

  async #start(p, op, asDependency, detail) {
    const aborted = () => this.#ops.get(p.id) !== op;
    this.#set(p.id, { ...IDLE, state: 'starting', dependency: asDependency, detail });
    for (const depId of p.requires ?? []) {
      const dep = this.project(depId);
      if (this.state(depId).state === 'running') continue;
      this.#set(p.id, { detail: tr('process.waiting', { name: dep.name }) });
      if (!this.#ops.has(depId)) this.start(depId, { asDependency: true });
      while (this.#ops.has(depId)) await sleep(250);
      if (aborted()) return;
      if (this.state(depId).state !== 'running') throw new Error(tr('process.depFailed', { name: dep.name, reason: this.state(depId).error ?? tr('process.unknownReason') }));
    }
    this.#set(p.id, { detail: null });

    if (await sys.portInUse(p.port)) {
      if ((await this.#health(p, p.port)) === 'ok') return this.#set(p.id, { state: 'running' });
      const pid = await sys.pidOnPort(p.port);
      throw new Error(pid ? tr('process.portTakenBy', { port: p.port, pid }) : tr('process.portTaken', { port: p.port }));
    }
    if (!fs.existsSync(p.cwd)) throw new Error(tr('process.noFolder', { path: p.cwd }));

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
      fs.renameSync(log, this.prevLogFile(p.id));
    } catch {}
    const argv = [expand(p.cmd, p), ...p.args.map((a) => expand(a, p))];
    const stamp = new Date().toLocaleString(serverLang());
    fs.writeFileSync(log, `[servym] ${stamp} · ${argv.join(' ')} · ${hostOf(p)}:${p.port}\n`);

    const env = sys.cleanEnv({ ...p.env, PORT: String(p.port), NO_COLOR: '1' });
    delete env.HOST;
    // HOST vide = toutes les interfaces (IPv4 et IPv6) pour les projets Node.
    if (p.hostControl === 'env' && !p.expose) env.HOST = '127.0.0.1';

    // PID de la racine de l'arborescence du projet (cmd.exe sous Windows).
    const onExit = (pid, code) => {
      this.#watched.delete(pid);
      this.#exited(p.id, pid, code);
    };
    return sys.launch(argv, { cwd: p.cwd, env, log, onExit }).then((pid) => {
      if (sys.isAlive(pid)) this.#watched.add(pid);
      return pid;
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
      if (health === 'mismatch') throw new Error(tr('process.wrongService', { port: p.port, name: p.name }));
      await sleep(600);
    }
    throw new Error(tr('process.timeout', { name: p.name, port: p.port, seconds }));
  }

  // Arrêt : commande d'arrêt propre si le projet en déclare une, sinon (ou
  // si elle ne suffit pas) arrêt forcé de tout l'arbre de processus.
  // cascade : arrête ensuite les dépendances démarrées pour lui et devenues inutiles.
  async stop(id, { cascade = false, reason = null } = {}) {
    const p = this.project(id);
    if (p.type === 'file') throw this.#refuse(p, 'stop', 'process.why.noProcess');
    const st = this.state(id);
    if (this.#ops.has(id) && st.state !== 'starting') throw this.#refuse(p, 'stop', 'process.why.busy');
    // Projet planté dont la relance est prévue : l'arrêter, c'est l'annuler.
    this.#cancelRestart(id);
    if (st.autoRestart === 'scheduled') return this.#set(id, { detail: null, autoRestart: null });
    if (st.state === 'stopped' || (st.state === 'failed' && !st.managed)) throw this.#refuse(p, 'stop', 'process.why.stopped');
    // Sans PID connu, le port désigne le processus : pas question de tuer un programme étranger.
    if (st.state === 'conflict' && !st.managed) throw this.#refuse(p, 'stop', 'process.why.portTaken', { port: p.port });
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
      throw new HttpError(500, 'raw', { text: err.message });
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
      await this.stop(depId, { cascade: true, reason: tr('process.unusedBy', { name: p.name }) }).catch((err) => this.emit('warning', `[stop] ${depId} : ${err.message}`));
    }
  }

  openFile(id) {
    const p = this.project(id);
    if (p.type !== 'file') throw new HttpError(400, 'process.notAFile', { name: p.name });
    const file = path.join(p.cwd, p.open);
    if (!fs.existsSync(file)) throw new HttpError(404, 'process.fileMissing', { path: file });
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
