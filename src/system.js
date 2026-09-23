// Petites commandes Windows : ports, processus, réseau, dossiers, git.
import { execFile, spawn } from 'node:child_process';
import { existsSync, lstatSync } from 'node:fs';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { promisify } from 'node:util';
import { ROOT_DIR } from './config.js';
import { tr } from './i18n.js';

const run = promisify(execFile);

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

// Ports TCP en écoute sur la machine : [{ address, port, pid }], IPv4 puis IPv6.
export async function listeners() {
  const outputs = await Promise.all(['TCP', 'TCPv6'].map((proto) =>
    run('netstat', ['-ano', '-p', proto], { windowsHide: true }).then((r) => r.stdout, () => '')));
  const list = [];
  for (const line of outputs.join('\n').split(/\r?\n/)) {
    const cols = line.trim().split(/\s+/);
    // Proto  Adresse locale  Adresse distante  État  PID (l'état est traduit : on vérifie la distante)
    if (cols.length !== 5 || !/:0$/.test(cols[2])) continue;
    const i = cols[1].lastIndexOf(':');
    list.push({ address: cols[1].slice(0, i), port: Number(cols[1].slice(i + 1)), pid: Number(cols[4]) });
  }
  return list;
}

export async function pidOnPort(port) {
  return (await listeners()).find((l) => l.port === port)?.pid ?? null;
}

// Nom de l'exécutable de chaque processus (PID → « node.exe »).
export async function processNames() {
  const { stdout } = await run('tasklist', ['/FO', 'CSV', '/NH'], { windowsHide: true, maxBuffer: 8 << 20 }).catch(() => ({ stdout: '' }));
  const names = new Map();
  for (const line of stdout.split(/\r?\n/)) {
    const m = line.match(/^"([^"]*)","(\d+)"/);
    if (m) names.set(Number(m[2]), m[1]);
  }
  return names;
}

// Adresses IPv4 de ce PC sur les réseaux (hors boucle locale et auto-configuration).
export function lanAddresses() {
  return Object.entries(os.networkInterfaces()).flatMap(([name, list]) =>
    (list ?? [])
      .filter((a) => a.family === 'IPv4' && !a.internal && !a.address.startsWith('169.254.'))
      .map((a) => ({ name, address: a.address })));
}

// Réseaux connectés et leur profil Windows (public : Wi-Fi d'école, café… ;
// private : réseau déclaré de confiance). Le pare-feu applique ses règles par profil.
const CATEGORY = { 0: 'public', 1: 'private', 2: 'domain', Public: 'public', Private: 'private', DomainAuthenticated: 'domain' };
export async function networkProfiles() {
  const script = '[Console]::OutputEncoding = [Text.Encoding]::UTF8; '
    + 'ConvertTo-Json -Compress -InputObject @(Get-NetConnectionProfile | Select-Object Name, InterfaceAlias, NetworkCategory)';
  try {
    const { stdout } = await runQuiet('powershell.exe', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script]);
    return JSON.parse(stdout).map((n) => ({ name: n.Name, interface: n.InterfaceAlias, category: CATEGORY[n.NetworkCategory] ?? String(n.NetworkCategory) }));
  } catch {
    return [];
  }
}

export function isAlive(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

export async function killTree(pid) {
  await run('taskkill', ['/PID', String(pid), '/T', '/F'], { windowsHide: true }).catch(() => {});
  for (let i = 0; i < 20 && isAlive(pid); i++) await new Promise((r) => setTimeout(r, 100));
  if (isAlive(pid)) {
    throw new Error(tr('system.killFailed', { pid }));
  }
}

export function runQuiet(cmd, args, options = {}) {
  return run(cmd, args, { windowsHide: true, timeout: 15000, ...options });
}

function detach(cmd, args, options = {}) {
  const child = spawn(cmd, args, { detached: true, stdio: 'ignore', ...options });
  child.on('error', () => {});
  child.unref();
}

const VSCODE = [
  path.join(process.env.LOCALAPPDATA ?? '', 'Programs/Microsoft VS Code/Code.exe'),
  path.join(process.env.ProgramFiles ?? '', 'Microsoft VS Code/Code.exe'),
].find((p) => existsSync(p));

// wt.exe est un alias d'application (lien d'exécution) : stat() le refuse
// (EACCES) et existsSync() répond donc toujours false ; lstat() le voit.
const WT = path.join(process.env.LOCALAPPDATA ?? '', 'Microsoft/WindowsApps/wt.exe');
const hasWT = () => Boolean(lstatSync(WT, { throwIfNoEntry: false }));

// Enfants lancés sans les variables de l'éditeur ou du terminal qui a démarré Servynex.
export function cleanEnv(extra = {}) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^(ELECTRON_|VSCODE_|CLAUDE|SERVYNEX_SUPERVISOR$)/i.test(key)) delete env[key];
  }
  return { ...env, ...extra };
}

// terminal : toujours disponible (PowerShell à défaut de Windows Terminal), on
// indique lequel s'ouvrira.
export const tools = {
  code: Boolean(VSCODE),
  terminal: hasWT() ? 'Windows Terminal' : 'PowerShell',
};

export function openFolder(dir) {
  detach('explorer.exe', [path.normalize(dir)]);
}

// Explorer ouvre le fichier avec l'application par défaut (le navigateur pour un .html).
export function openFile(file) {
  detach('explorer.exe', [path.normalize(file)]);
}

export function openInCode(dir) {
  if (!VSCODE) throw new Error('VS Code introuvable');
  detach(VSCODE, [path.normalize(dir)], { env: cleanEnv() });
}

// Fichier texte (projects.json, servynex.log) : VS Code, sinon le Bloc-notes.
export function openInEditor(file) {
  if (VSCODE) detach(VSCODE, [path.normalize(file)], { env: cleanEnv() });
  else detach('notepad.exe', [path.normalize(file)], { windowsHide: false });
}

// Tâche planifiée « Servynex » (scripts/install-autostart.ps1) : Servynex démarre à
// l'ouverture de session. Même effet que autostart.bat ON / OFF.
const PS = ['-NoLogo', '-NoProfile', '-NonInteractive'];
export const autostartTask = {
  async enabled() {
    const script = "$t = Get-ScheduledTask -TaskName Servynex -ErrorAction SilentlyContinue; if ($t -and $t.State -ne 'Disabled') { 'on' } else { 'off' }";
    const { stdout } = await runQuiet('powershell.exe', [...PS, '-Command', script]);
    return stdout.trim() === 'on';
  },
  async set(on) {
    if (on) {
      await runQuiet('powershell.exe', [...PS, '-ExecutionPolicy', 'Bypass', '-File', path.join(ROOT_DIR, 'scripts', 'install-autostart.ps1')], { timeout: 30000 });
    } else {
      await runQuiet('powershell.exe', [...PS, '-Command', 'if (Get-ScheduledTask -TaskName Servynex -ErrorAction SilentlyContinue) { Unregister-ScheduledTask -TaskName Servynex -Confirm:$false }']);
    }
  },
};

export function openTerminal(dir) {
  if (hasWT()) detach(WT, ['-d', path.normalize(dir)], { env: cleanEnv() });
  else detach('powershell.exe', ['-NoExit'], { cwd: dir, env: cleanEnv(), windowsHide: false });
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

// PowerShell gardé ouvert (src/sample.ps1) tant que des pages demandent des
// relevés, fermé après une minute sans demande. Relevé : PID → processus.
let probe = null;
process.on('exit', () => probe?.child.kill());

export function sampleProcesses() {
  probe ??= openProbe();
  return probe.sample();
}

function openProbe() {
  const child = spawn('powershell.exe', [...PS, '-ExecutionPolicy', 'Bypass', '-File', path.join(ROOT_DIR, 'src', 'sample.ps1')], {
    windowsHide: true,
    stdio: ['pipe', 'pipe', 'ignore'],
  });
  const waiting = []; // relevés demandés, servis dans l'ordre
  let lines = [];
  let buffer = '';
  let idle = null;
  const close = () => {
    if (probe?.child === child) probe = null;
    clearTimeout(idle);
    child.kill();
    for (const w of waiting.splice(0)) w.reject(new Error(tr('system.sampleStopped')));
  };
  child.on('error', close);
  child.on('exit', close);
  child.stdout.setEncoding('utf8');
  child.stdout.on('data', (chunk) => {
    const parts = (buffer + chunk).split(/\r?\n/);
    buffer = parts.pop();
    for (const line of parts) {
      if (line !== 'END') {
        lines.push(line);
        continue;
      }
      const processes = new Map();
      for (const l of lines) {
        const [pid, ppid, start, cpu, memory] = l.split(' ').map(Number);
        if (Number.isFinite(pid) && Number.isFinite(memory)) processes.set(pid, { pid, ppid, start, cpu, memory });
      }
      lines = [];
      const w = waiting.shift();
      clearTimeout(w?.timer);
      w?.resolve(processes);
    }
  });
  // Ne retient pas Node à la fermeture : seul le serveur le fait.
  child.unref();
  child.stdin.unref?.();
  child.stdout.unref?.();
  return {
    child,
    sample() {
      clearTimeout(idle);
      idle = setTimeout(close, 60_000);
      idle.unref();
      return new Promise((resolve, reject) => {
        const timer = setTimeout(close, 15_000);
        timer.unref();
        waiting.push({ resolve, reject, timer });
        child.stdin.write('x\n');
      });
    },
  };
}
