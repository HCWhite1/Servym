// Linux : ports (ss), processus (/proc), arrêt (SIGTERM puis SIGKILL), lancement
// direct sans shell, fenêtres (xdg-open, VS Code, terminal du bureau), démarrage
// avec la session (service systemd utilisateur). Mêmes fonctions que windows.js.
// Les fonctions reçoivent leurs dépendances (dossier /proc, commandes…) pour être
// testées sur des sorties enregistrées.
import { spawn } from 'node:child_process';
import { constants } from 'node:os';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { ROOT_DIR } from '../config.js';
import { tr } from '../i18n.js';
import { treeOf } from '../stats.js';
import { cleanEnv, detach, isAlive, runQuiet } from './shared.js';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- Ports ----

// Adresse de ss ramenée à la forme de netstat sous Windows : « 127.0.0.53%lo » →
// « 127.0.0.53 », « [::ffff:127.0.0.1] » → « 127.0.0.1 », « * » (toutes) → « 0.0.0.0 ».
function address(raw) {
  const plain = raw.replace(/%[^\]]*/, '');
  if (plain === '*') return '0.0.0.0';
  const mapped = plain.match(/^\[::ffff:(\d+\.\d+\.\d+\.\d+)\]$/);
  return mapped ? mapped[1] : plain;
}

// Sortie de « ss -Hltnp » : [{ address, port, pid }]. pid null quand le processus
// appartient à un autre compte (ss ne le montre qu'à root).
export function parseSs(text) {
  const list = [];
  for (const line of text.split('\n')) {
    const cols = line.trim().split(/\s+/);
    if (cols[0] !== 'LISTEN' || cols.length < 5) continue;
    const local = cols[3];
    const i = local.lastIndexOf(':');
    const pid = line.match(/pid=(\d+)/);
    list.push({ address: address(local.slice(0, i)), port: Number(local.slice(i + 1)), pid: pid ? Number(pid[1]) : null });
  }
  return list;
}

export async function listeners() {
  const { stdout } = await runQuiet('ss', ['-Hltnp']).catch(() => ({ stdout: '' }));
  return parseSs(stdout);
}

// ---- Processus (/proc) ----

// Ligne de /proc/<pid>/stat. Le nom (2e champ) peut contenir espaces et
// parenthèses : on repart de la dernière « ) ». ticks : temps processeur
// (utime + stime), startTicks : démarrage depuis le boot, rss : pages en mémoire.
export function parseStat(line) {
  const close = line.lastIndexOf(')');
  const rest = line.slice(close + 2).split(' ');
  return {
    pid: Number(line.slice(0, line.indexOf(' '))),
    ppid: Number(rest[1]),
    ticks: Number(rest[11]) + Number(rest[12]),
    startTicks: Number(rest[19]),
    rss: Number(rest[21]),
  };
}

const pids = async (proc) => (await fs.promises.readdir(proc)).filter((name) => /^\d+$/.test(name));

// Lit un fichier de chaque processus ; un processus terminé entre-temps est ignoré.
async function readEach(proc, file) {
  const entries = await Promise.all((await pids(proc)).map((pid) =>
    fs.promises.readFile(path.join(proc, pid, file), 'utf8').then((text) => [Number(pid), text], () => null)));
  return entries.filter(Boolean);
}

// Valeurs du noyau (getconf), lues une fois : 100 et 4096 presque partout.
let kernel = null;
async function kernelValues() {
  kernel ??= Promise.all(['CLK_TCK', 'PAGESIZE'].map((name) =>
    runQuiet('getconf', [name]).then((r) => Number(r.stdout.trim()) || null, () => null)))
    .then(([ticks, pageSize]) => ({ ticks: ticks ?? 100, pageSize: pageSize ?? 4096 }));
  return kernel;
}

// Relevé : PID → { pid, ppid, start (ms), cpu (ms cumulées), memory (octets) },
// comme sample.ps1 sous Windows.
export async function sampleProcesses({ proc = '/proc', ticks, pageSize } = {}) {
  if (!ticks || !pageSize) {
    const k = await kernelValues();
    ticks ||= k.ticks;
    pageSize ||= k.pageSize;
  }
  const btime = Number((await fs.promises.readFile(path.join(proc, 'stat'), 'utf8')).match(/^btime (\d+)/m)?.[1] ?? 0);
  const processes = new Map();
  for (const [pid, text] of await readEach(proc, 'stat')) {
    const s = parseStat(text.trim());
    processes.set(pid, {
      pid,
      ppid: s.ppid,
      start: btime * 1000 + Math.round((s.startTicks * 1000) / ticks),
      cpu: Math.round((s.ticks * 1000) / ticks),
      memory: s.rss * pageSize,
    });
  }
  return processes;
}

// Nom de chaque processus (PID → « node »).
export async function processNames({ proc = '/proc' } = {}) {
  return new Map((await readEach(proc, 'comm')).map(([pid, text]) => [pid, text.trim()]));
}

// Pas de profils réseau comme sous Windows : le pare-feu (firewalld…) se règle à part.
export async function networkProfiles() {
  return [];
}

// ---- Arrêt ----

// SIGTERM au groupe du projet (launch le crée) et à tous ses descendants, qui
// peuvent s'arrêter proprement ; SIGKILL à ceux qui restent après grace ms.
export async function killTree(pid, { proc = '/proc', kill = process.kill.bind(process), alive = isAlive, grace = 3000, wait = 2000 } = {}) {
  const snapshot = await sampleProcesses({ proc, ticks: 100, pageSize: 4096 }).catch(() => new Map());
  const tree = [...treeOf(snapshot, [pid])];
  if (!tree.includes(pid)) tree.push(pid);
  const send = (targets, signal) => {
    for (const target of targets) {
      try {
        kill(target, signal);
      } catch {}
    }
  };
  const living = () => tree.filter((p) => alive(p));
  send([-pid, ...tree], 'SIGTERM');
  for (let t = 0; t < grace && living().length; t += 50) await sleep(50);
  send(living(), 'SIGKILL');
  for (let t = 0; t < wait && alive(pid); t += 50) await sleep(50);
  if (alive(pid)) throw new Error(tr('system.killFailed', { pid }));
}

// ---- Lancement ----

// Code de sortie d'un processus tué par un signal : 128 + numéro, comme un shell
// (SIGKILL → 137). null reste « inconnu ».
export function exitCode(code, signal) {
  if (code !== null && code !== undefined) return code;
  return signal ? 128 + (constants.signals[signal] ?? 0) : null;
}

// Lance argv directement, sans shell : aucun caractère n'est interprété. Nouveau
// groupe de processus (detached) : le projet survit à un redémarrage de Servynex
// et killTree l'arrête avec ses enfants. Sortie ajoutée au journal.
// → PID du projet ; onExit(pid, code) à sa fin.
export function launch(argv, { cwd, env, log, onExit }) {
  return new Promise((resolve, reject) => {
    const out = fs.openSync(log, 'a');
    let child;
    try {
      child = spawn(argv[0], argv.slice(1), { cwd, env, detached: true, windowsHide: true, stdio: ['ignore', out, out] });
    } finally {
      fs.closeSync(out);
    }
    child.once('error', (err) => {
      reject(new Error(err.code === 'ENOENT' ? tr('process.commandNotFound', { cmd: argv[0] }) : err.message));
    });
    child.once('spawn', () => {
      child.unref();
      resolve(child.pid);
    });
    // Après un échec de lancement, « exit » peut arriver sans PID : rien à signaler.
    child.once('exit', (code, signal) => child.pid && onExit(child.pid, exitCode(code, signal)));
  });
}

// ---- Fenêtres du bureau ----

// Terminaux reconnus, dans l'ordre de préférence : [commande, nom, arguments(dossier)].
const TERMINALS = [
  ['ptyxis', 'Ptyxis', (dir) => ['--new-window', `--working-directory=${dir}`]], // Fedora 41 et suivantes
  ['gnome-terminal', 'GNOME Terminal', (dir) => [`--working-directory=${dir}`]],
  ['konsole', 'Konsole', (dir) => ['--workdir', dir]],
  ['xfce4-terminal', 'Xfce Terminal', (dir) => [`--working-directory=${dir}`]],
  ['kitty', 'kitty', (dir) => ['--directory', dir]],
  ['alacritty', 'Alacritty', (dir) => ['--working-directory', dir]],
  ['xterm', 'XTerm', () => []],
];

export function terminalCommand(name, dir) {
  const [cmd, , args] = TERMINALS.find(([, n]) => n === name);
  return [cmd, args(dir)];
}

// Commande présente dans le PATH ?
const inPath = (env) => (cmd) => (env.PATH ?? '').split(':').some((dir) => dir && fs.existsSync(path.join(dir, cmd)));

// desktop : une session graphique (sinon, serveur sans écran : rien ne s'ouvre).
export function detectTools({ env = process.env, has = inPath(env) } = {}) {
  if (!env.DISPLAY && !env.WAYLAND_DISPLAY) return { desktop: false, code: false, terminal: null };
  return { desktop: true, code: has('code'), terminal: TERMINALS.find(([cmd]) => has(cmd))?.[1] ?? null };
}

export const tools = detectTools();

// xdg-open : le gestionnaire de fichiers, ou l'application par défaut d'un fichier.
export function openFolder(dir) {
  detach('xdg-open', [dir], { env: cleanEnv() });
}

export function openFile(file) {
  detach('xdg-open', [file], { env: cleanEnv() });
}

export function openInCode(dir) {
  if (!tools.code) throw new Error('VS Code introuvable');
  detach('code', [dir], { env: cleanEnv() });
}

// Fichier texte (projects.json, servynex.log) : VS Code, sinon l'éditeur par défaut.
export function openInEditor(file) {
  if (tools.code) detach('code', [file], { env: cleanEnv() });
  else openFile(file);
}

export function openTerminal(dir) {
  if (!tools.terminal) throw new Error(tr('system.noTerminal'));
  const [cmd, args] = terminalCommand(tools.terminal, dir);
  detach(cmd, args, { cwd: dir, env: cleanEnv() });
}

// ---- Démarrage avec la session : service systemd utilisateur ----

// Service « servynex » : démarre à l'ouverture de session, relancé s'il plante
// (5 échecs rapides : abandon, comme sous Windows). KillMode=process : arrêter
// Servynex laisse tourner les projets, repris au démarrage suivant.
export function unitFile({ node = process.execPath, root = ROOT_DIR } = {}) {
  const q = (text) => text.replaceAll('%', '%%'); // % introduit une variable pour systemd
  return [
    '[Unit]',
    `Description=Servynex (${q(root)})`,
    'After=network.target',
    'StartLimitIntervalSec=300',
    'StartLimitBurst=5',
    '',
    '[Service]',
    'Type=simple',
    `WorkingDirectory=${q(root)}`,
    `ExecStart="${q(node)}" "${q(root)}/server.js"`,
    'Environment=SERVYNEX_SUPERVISOR=systemd',
    '# Code 75 : redémarrage demandé depuis l’interface.',
    'Restart=on-failure',
    'RestartSec=2',
    'KillMode=process',
    '',
    '[Install]',
    'WantedBy=default.target',
    '',
  ].join('\n');
}

export function systemdTask({
  dir = path.join(process.env.XDG_CONFIG_HOME || path.join(os.homedir(), '.config'), 'systemd', 'user'),
  run: exec = runQuiet,
  node,
  root,
} = {}) {
  const file = path.join(dir, 'servynex.service');
  const systemctl = (...args) => exec('systemctl', ['--user', ...args]);
  return {
    async enabled() {
      // is-enabled répond « disabled » avec un code d'erreur : sa sortie suffit.
      const { stdout } = await systemctl('is-enabled', 'servynex.service').catch((err) => ({ stdout: err.stdout ?? '' }));
      return stdout.trim() === 'enabled';
    },
    async set(on) {
      if (on) {
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(file, unitFile({ node, root }));
        await systemctl('daemon-reload');
        await systemctl('enable', 'servynex.service');
      } else {
        await systemctl('disable', 'servynex.service');
        fs.rmSync(file, { force: true });
        await systemctl('daemon-reload');
      }
    },
  };
}

export const autostartTask = systemdTask();
