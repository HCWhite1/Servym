// Windows : ports (netstat), processus (tasklist, WMI), arrêt (taskkill), lancement
// (launch.ps1 + cmd.exe), fenêtres (Explorer, VS Code, Windows Terminal), démarrage
// avec la session (tâche planifiée). Mêmes fonctions que linux.js.
import { lstatSync, existsSync } from 'node:fs';
import { spawn } from 'node:child_process';
import path from 'node:path';
import { ROOT_DIR } from '../config.js';
import { tr } from '../i18n.js';
import { cleanEnv, detach, isAlive, run, runQuiet } from './shared.js';

const PS = ['-NoLogo', '-NoProfile', '-NonInteractive'];

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

// Réseaux connectés et leur profil Windows (public : Wi-Fi d'école, café… ;
// private : réseau déclaré de confiance). Le pare-feu applique ses règles par profil.
const CATEGORY = { 0: 'public', 1: 'private', 2: 'domain', Public: 'public', Private: 'private', DomainAuthenticated: 'domain' };
export async function networkProfiles() {
  const script = '[Console]::OutputEncoding = [Text.Encoding]::UTF8; '
    + 'ConvertTo-Json -Compress -InputObject @(Get-NetConnectionProfile | Select-Object Name, InterfaceAlias, NetworkCategory)';
  try {
    const { stdout } = await runQuiet('powershell.exe', [...PS, '-Command', script]);
    return JSON.parse(stdout).map((n) => ({ name: n.Name, interface: n.InterfaceAlias, category: CATEGORY[n.NetworkCategory] ?? String(n.NetworkCategory) }));
  } catch {
    return [];
  }
}

export async function killTree(pid) {
  await run('taskkill', ['/PID', String(pid), '/T', '/F'], { windowsHide: true }).catch(() => {});
  for (let i = 0; i < 20 && isAlive(pid); i++) await new Promise((r) => setTimeout(r, 100));
  if (isAlive(pid)) {
    throw new Error(tr('system.killFailed', { pid }));
  }
}

// Lance argv dans une console cachée (src/launch.ps1), sortie ajoutée au journal.
// → PID de cmd.exe, racine de l'arborescence du projet ; onExit(pid, code) à sa fin.
const LAUNCHER = path.join(ROOT_DIR, 'src', 'launch.ps1');
export function launch(argv, { cwd, env, log, onExit }) {
  const child = spawn('powershell.exe', [...PS, '-ExecutionPolicy', 'Bypass', '-File', LAUNCHER], {
    env: {
      ...env,
      SERVYM_CMDLINE: argv.map((a) => `"${a}"`).join(' '),
      SERVYM_LOG: log,
      SERVYM_CWD: cwd,
    },
    windowsHide: true,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  return new Promise((resolve, reject) => {
    let out = '';
    let err = '';
    let pid = null;
    child.stdout.setEncoding('utf8').on('data', (d) => {
      out += d;
      const m = !pid && out.match(/^SERVYM_PID=(\d+)\r?\n/m); // écrit par launch.ps1
      if (m) resolve((pid = Number(m[1])));
    });
    child.stderr.setEncoding('utf8').on('data', (d) => (err += d));
    child.on('error', reject);
    child.on('exit', (code) => {
      if (pid) onExit(pid, code);
      else reject(new Error(err.trim().split('\n')[0] || tr('process.launchFailed', { code })));
    });
  });
}

const VSCODE = [
  path.join(process.env.LOCALAPPDATA ?? '', 'Programs/Microsoft VS Code/Code.exe'),
  path.join(process.env.ProgramFiles ?? '', 'Microsoft VS Code/Code.exe'),
].find((p) => existsSync(p));

// wt.exe est un alias d'application (lien d'exécution) : stat() le refuse
// (EACCES) et existsSync() répond donc toujours false ; lstat() le voit.
const WT = path.join(process.env.LOCALAPPDATA ?? '', 'Microsoft/WindowsApps/wt.exe');
const hasWT = () => Boolean(lstatSync(WT, { throwIfNoEntry: false }));

// terminal : toujours disponible (PowerShell à défaut de Windows Terminal), on
// indique lequel s'ouvrira. desktop : une session avec écran (toujours sous Windows).
export const tools = {
  desktop: true,
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

// Fichier texte (projects.json, servym.log) : VS Code, sinon le Bloc-notes.
export function openInEditor(file) {
  if (VSCODE) detach(VSCODE, [path.normalize(file)], { env: cleanEnv() });
  else detach('notepad.exe', [path.normalize(file)], { windowsHide: false });
}

export function openTerminal(dir) {
  if (hasWT()) detach(WT, ['-d', path.normalize(dir)], { env: cleanEnv() });
  else detach('powershell.exe', ['-NoExit'], { cwd: dir, env: cleanEnv(), windowsHide: false });
}

// Tâche planifiée « Servym » (scripts/install-autostart.ps1) : Servym démarre à
// l'ouverture de session. Même effet que autostart.bat ON / OFF.
export const autostartTask = {
  async enabled() {
    const script = "$t = Get-ScheduledTask -TaskName Servym -ErrorAction SilentlyContinue; if ($t -and $t.State -ne 'Disabled') { 'on' } else { 'off' }";
    const { stdout } = await runQuiet('powershell.exe', [...PS, '-Command', script]);
    return stdout.trim() === 'on';
  },
  async set(on) {
    if (on) {
      await runQuiet('powershell.exe', [...PS, '-ExecutionPolicy', 'Bypass', '-File', path.join(ROOT_DIR, 'scripts', 'install-autostart.ps1')], { timeout: 30000 });
    } else {
      await runQuiet('powershell.exe', [...PS, '-Command', 'if (Get-ScheduledTask -TaskName Servym -ErrorAction SilentlyContinue) { Unregister-ScheduledTask -TaskName Servym -Confirm:$false }']);
    }
  },
};

// ---- Processeur et mémoire (src/stats.js) ----

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
