// Ports, processus, réseau, dossiers, git. Ce qui dépend du système vient de
// src/platform/windows.js ou src/platform/linux.js (mêmes fonctions).
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import os from 'node:os';
import * as linux from './platform/linux.js';
import { runQuiet } from './platform/shared.js';
import * as windows from './platform/windows.js';

export { cleanEnv, isAlive, runQuiet } from './platform/shared.js';

const platform = process.platform === 'win32' ? windows : linux;

// Ports TCP en écoute sur la machine : [{ address, port, pid }].
export const listeners = () => platform.listeners();
// Nom de l'exécutable de chaque processus (PID → « node.exe », « node »).
export const processNames = () => platform.processNames();
// Réseaux connectés et leur profil (Windows seulement : public, private, domain).
export const networkProfiles = () => platform.networkProfiles();
// Arrêt d'un processus et de tous ses descendants.
export const killTree = (pid) => platform.killTree(pid);
// Lancement d'un projet : launch(argv, { cwd, env, log, onExit(pid, code) }) → PID.
export const launch = (argv, options) => platform.launch(argv, options);
// Relevé des processus pour src/stats.js : PID → { pid, ppid, start, cpu, memory }.
export const sampleProcesses = () => platform.sampleProcesses();
// tools : { desktop, code, terminal }, ce qui peut s'ouvrir sur cette machine.
export const { tools, autostartTask } = platform;
export const openFolder = (dir) => platform.openFolder(dir);
export const openFile = (file) => platform.openFile(file);
export const openInCode = (dir) => platform.openInCode(dir);
export const openInEditor = (file) => platform.openInEditor(file);
export const openTerminal = (dir) => platform.openTerminal(dir);

export function tcpOpen(port, host = '127.0.0.1', timeout = 800) {
  return new Promise((resolve) => {
    const socket = net.connect({ port, host });
    const done = (ok) => {
      socket.destroy();
      resolve(ok);
    };
    socket.setTimeout(timeout, () => done(false));
    socket.once('connect', () => done(true));
    socket.once('error', () => done(false));
  });
}

// Un serveur sur « :: » accepte aussi 127.0.0.1 ; ::1 couvre ceux qui
// n'écoutent qu'en IPv6.
export async function portInUse(port) {
  const [v4, v6] = await Promise.all([tcpOpen(port), tcpOpen(port, '::1')]);
  return v4 || v6;
}

// Répond « ok », « down » ou « mismatch » (le port répond, mais pas le bon site).
// Une erreur 5xx compte comme « ok » : le serveur tourne, c'est l'application qui échoue.
// En HTTPS, le certificat local (symfony serve) n'est pas vérifié : on ne teste que la présence.
export function httpHealth(port, { path: pathname = '/', expect = '', tls = false, timeout = 1500 } = {}) {
  return new Promise((resolve) => {
    const options = { host: '127.0.0.1', port, path: pathname, timeout, headers: { host: `localhost:${port}` }, rejectUnauthorized: false };
    const req = (tls ? https : http).get(options, (res) => {
      if (!expect || res.statusCode >= 500) {
        res.resume();
        return resolve('ok');
      }
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (chunk) => {
        body += chunk;
        if (body.includes(expect)) {
          res.destroy();
          resolve('ok');
        } else if (body.length > 65536) {
          res.destroy();
        }
      });
      res.on('close', () => resolve(body.includes(expect) ? 'ok' : 'mismatch'));
    });
    req.on('timeout', () => req.destroy());
    req.on('error', () => resolve('down'));
  });
}

export async function pidOnPort(port) {
  return (await listeners()).find((l) => l.port === port)?.pid ?? null;
}

// Adresses IPv4 de ce PC sur les réseaux (hors boucle locale et auto-configuration).
export function lanAddresses() {
  return Object.entries(os.networkInterfaces()).flatMap(([name, list]) =>
    (list ?? [])
      .filter((a) => a.family === 'IPv4' && !a.internal && !a.address.startsWith('169.254.'))
      .map((a) => ({ name, address: a.address })));
}

export async function gitInfo(dir) {
  const git = (...args) => runQuiet('git', ['-C', dir, ...args], { timeout: 5000 }).then((r) => r.stdout);
  try {
    const status = await git('status', '--porcelain=v2', '--branch');
    const info = { repo: true, branch: null, ahead: 0, behind: 0, changes: 0, upstream: false };
    for (const line of status.split('\n')) {
      if (line.startsWith('# branch.head ')) info.branch = line.slice(14);
      else if (line.startsWith('# branch.upstream ')) info.upstream = true;
      else if (line.startsWith('# branch.ab ')) {
        const [, a, b] = line.match(/\+(\d+) -(\d+)/) ?? [];
        info.ahead = Number(a);
        info.behind = Number(b);
      } else if (line && !line.startsWith('#')) info.changes++;
    }
    const last = (await git('log', '-1', '--format=%h%x09%ct%x09%s').catch(() => '')).trim();
    if (last) {
      const [hash, time, ...subject] = last.split('\t');
      info.last = { hash, date: Number(time) * 1000, subject: subject.join('\t') };
    }
    return info;
  } catch {
    return { repo: false };
  }
}

// ---- Processeur et mémoire (src/stats.js) ----

export const cores = os.availableParallelism?.() ?? os.cpus().length;
