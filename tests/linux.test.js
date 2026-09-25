// Module Linux, testé ici sur des sorties enregistrées (ss, /proc) et des appels
// système remplacés : il tourne aussi sous Windows.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { createApp, isSupervised } from '../server.js';
import { setServerLang } from '../src/i18n.js';
import * as linux from '../src/platform/linux.js';
import * as windows from '../src/platform/windows.js';

process.env.SERVYM_LANG = 'fr';
setServerLang('fr');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Faux /proc : { pid: { comm, ppid, utime, stime, start, rss } }, btime en secondes.
function fakeProc(processes, btime = 1_700_000_000) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-proc-'));
  fs.writeFileSync(path.join(dir, 'stat'), `cpu  1 2 3 4\ncpu0 1 2 3 4\nbtime ${btime}\nprocesses 1234\n`);
  fs.mkdirSync(path.join(dir, 'self'));
  fs.mkdirSync(path.join(dir, 'sys'));
  for (const [pid, p] of Object.entries(processes)) {
    fs.mkdirSync(path.join(dir, pid));
    const fields = ['S', p.ppid, pid, pid, 0, -1, 4194560, 100, 0, 0, 0, p.utime ?? 0, p.stime ?? 0, 0, 0, 20, 0, 1, 0, p.start ?? 0, 1000000, p.rss ?? 0, 18446744073709551615];
    fs.writeFileSync(path.join(dir, pid, 'stat'), `${pid} (${p.comm}) ${fields.join(' ')}\n`);
    fs.writeFileSync(path.join(dir, pid, 'comm'), `${p.comm}\n`);
  }
  return dir;
}

describe('Linux : mêmes fonctions que Windows', () => {
  test('chaque fonction du module Windows existe sous Linux', () => {
    for (const [name, value] of Object.entries(windows)) {
      assert.ok(name in linux, `${name} manque sous Linux`);
      assert.equal(typeof linux[name], typeof value, name);
    }
  });
});

describe('Linux : ports en écoute (ss)', () => {
  // ss -Hltnp sur Fedora, compte utilisateur : les PID des autres comptes sont absents.
  const SS = [
    'LISTEN 0      511          127.0.0.1:3990       0.0.0.0:*    users:(("node",pid=4242,fd=21))',
    'LISTEN 0      4096     127.0.0.53%lo:53         0.0.0.0:*',
    'LISTEN 0      511                  *:9000             *:*    users:(("node",pid=1111,fd=18))',
    'LISTEN 0      128               [::]:22            [::]:*',
    'LISTEN 0      511              [::1]:5173          [::]:*    users:(("node",pid=5555,fd=30),("node",pid=5556,fd=30))',
    'LISTEN 0      80  [::ffff:127.0.0.1]:3306             *:*    users:(("mysqld",pid=777,fd=12))',
    'LISTEN 0      511            0.0.0.0:8080       0.0.0.0:*    users:(("php",pid=888,fd=4))',
  ].join('\n');

  test('adresse, port et PID ; adresses ramenées à la forme de Windows', () => {
    assert.deepEqual(linux.parseSs(SS), [
      { address: '127.0.0.1', port: 3990, pid: 4242 },
      { address: '127.0.0.53', port: 53, pid: null },
      { address: '0.0.0.0', port: 9000, pid: 1111 },
      { address: '[::]', port: 22, pid: null },
      { address: '[::1]', port: 5173, pid: 5555 },
      { address: '127.0.0.1', port: 3306, pid: 777 },
      { address: '0.0.0.0', port: 8080, pid: 888 },
    ]);
  });

  test('avec la ligne d’en-tête (ancien ss sans -H)', () => {
    const withHeader = `State  Recv-Q Send-Q Local Address:Port Peer Address:Port Process\n${SS}`;
    assert.equal(linux.parseSs(withHeader).length, 7);
  });
});

describe('Linux : processus (/proc)', () => {
  test('ligne stat : un nom avec espaces et parenthèses ne décale rien', () => {
    const line = '4242 (node (worker) 2) S 4000 4242 4242 0 -1 4194560 100 0 0 0 250 50 0 0 20 0 7 0 12345 1000000 2048 18446744073709551615';
    assert.deepEqual(linux.parseStat(line), { pid: 4242, ppid: 4000, ticks: 300, startTicks: 12345, rss: 2048 });
  });

  test('relevé : début, temps processeur et mémoire en ms et octets', async () => {
    const proc = fakeProc({
      1: { comm: 'systemd', ppid: 0, utime: 10, stime: 5, start: 1, rss: 1000 },
      4242: { comm: 'node', ppid: 1, utime: 250, stime: 50, start: 12345, rss: 2048 },
    });
    try {
      const sample = await linux.sampleProcesses({ proc, ticks: 100, pageSize: 4096 });
      assert.deepEqual(sample.get(4242), { pid: 4242, ppid: 1, start: 1_700_000_000_000 + 123_450, cpu: 3000, memory: 2048 * 4096 });
      assert.equal(sample.size, 2, 'les dossiers self, sys… ne sont pas des processus');
    } finally {
      fs.rmSync(proc, { recursive: true, force: true });
    }
  });

  test('un processus disparu pendant la lecture est ignoré', async () => {
    const proc = fakeProc({ 10: { comm: 'a', ppid: 1 } });
    fs.mkdirSync(path.join(proc, '11')); // plus de stat : terminé entre-temps
    try {
      assert.deepEqual([...(await linux.sampleProcesses({ proc, ticks: 100, pageSize: 4096 })).keys()], [10]);
    } finally {
      fs.rmSync(proc, { recursive: true, force: true });
    }
  });

  test('pas de profils réseau comme sous Windows', async () => {
    assert.deepEqual(await linux.networkProfiles(), []);
  });

  test('noms des processus', async () => {
    const proc = fakeProc({ 4242: { comm: 'node', ppid: 1 }, 777: { comm: 'mysqld', ppid: 1 } });
    try {
      const names = await linux.processNames({ proc });
      assert.equal(names.get(4242), 'node');
      assert.equal(names.get(777), 'mysqld');
    } finally {
      fs.rmSync(proc, { recursive: true, force: true });
    }
  });
});

describe('Linux : arrêt de toute l’arborescence', () => {
  test('SIGTERM au groupe et aux descendants, SIGKILL à ceux qui résistent, rien aux autres', async () => {
    const proc = fakeProc({
      100: { comm: 'npm', ppid: 1, start: 10 },
      101: { comm: 'node', ppid: 100, start: 11 },
      102: { comm: 'esbuild', ppid: 101, start: 12 },
      200: { comm: 'bash', ppid: 1, start: 5 },
    });
    const calls = [];
    const alive = new Set([100, 101, 102, 200]);
    const kill = (pid, signal) => {
      calls.push([pid, signal]);
      // esbuild ignore SIGTERM
      if (pid > 0 && (signal === 'SIGKILL' || pid !== 102)) alive.delete(pid);
    };
    try {
      await linux.killTree(100, { proc, kill, alive: (pid) => alive.has(pid), grace: 50 });
      assert.deepEqual(calls.filter(([, s]) => s === 'SIGTERM').map(([p]) => p).sort((a, b) => a - b), [-100, 100, 101, 102]);
      assert.deepEqual(calls.filter(([, s]) => s === 'SIGKILL'), [[102, 'SIGKILL']]);
      assert.ok(alive.has(200), 'processus étranger intact');
    } finally {
      fs.rmSync(proc, { recursive: true, force: true });
    }
  });

  test('un processus impossible à arrêter : erreur', async () => {
    const proc = fakeProc({ 100: { comm: 'bloqué', ppid: 1 } });
    try {
      await assert.rejects(linux.killTree(100, { proc, kill: () => {}, alive: () => true, grace: 20, wait: 20 }), /100/);
    } finally {
      fs.rmSync(proc, { recursive: true, force: true });
    }
  });
});

describe('Linux : lancement d’un projet', () => {
  test('sans shell, sortie dans le journal, code de sortie transmis', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-launch-'));
    const log = path.join(dir, 'p.log');
    fs.writeFileSync(log, '[servym] en-tête\n');
    try {
      let exit;
      const pid = await linux.launch([process.execPath, '-e', 'console.log("sortie $HOME"); console.error("erreur"); process.exit(3)'], {
        cwd: dir,
        env: { ...process.env },
        log,
        onExit: (p, code) => (exit = { p, code }),
      });
      assert.ok(pid > 0);
      for (let i = 0; i < 100 && !exit; i++) await sleep(50);
      assert.deepEqual(exit, { p: pid, code: 3 });
      const text = fs.readFileSync(log, 'utf8');
      assert.match(text, /^\[servym\] en-tête\n/, 'journal complété, pas remplacé');
      assert.match(text, /sortie \$HOME/, 'aucun shell : $HOME reste tel quel');
      assert.match(text, /erreur/);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('tué par un signal : code 128 + numéro du signal', () => {
    assert.equal(linux.exitCode(null, 'SIGKILL'), 137);
    assert.equal(linux.exitCode(null, 'SIGTERM'), 143);
    assert.equal(linux.exitCode(2, null), 2);
    assert.equal(linux.exitCode(null, null), null);
  });

  test('commande introuvable : message clair', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-launch-'));
    try {
      await assert.rejects(
        linux.launch(['commande-qui-n-existe-pas', 'x'], { cwd: dir, env: { ...process.env }, log: path.join(dir, 'p.log'), onExit: () => {} }),
        /Commande introuvable : commande-qui-n-existe-pas/,
      );
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});

describe('Linux : outils du bureau', () => {
  const has = (list) => (cmd) => list.includes(cmd);

  test('sans écran (serveur) : rien à ouvrir', () => {
    assert.deepEqual(linux.detectTools({ env: {}, has: has(['code', 'gnome-terminal']) }), { desktop: false, code: false, terminal: null });
  });

  test('avec écran : VS Code et le premier terminal trouvé (Ptyxis sur Fedora)', () => {
    assert.deepEqual(linux.detectTools({ env: { WAYLAND_DISPLAY: 'wayland-0' }, has: has(['code', 'ptyxis', 'gnome-terminal']) }), { desktop: true, code: true, terminal: 'Ptyxis' });
    assert.deepEqual(linux.detectTools({ env: { DISPLAY: ':0' }, has: has(['konsole']) }), { desktop: true, code: false, terminal: 'Konsole' });
    assert.deepEqual(linux.detectTools({ env: { DISPLAY: ':0' }, has: has([]) }), { desktop: true, code: false, terminal: null });
  });

  test('terminal ouvert dans le dossier du projet', () => {
    assert.deepEqual(linux.terminalCommand('Ptyxis', '/home/hugo/app'), ['ptyxis', ['--new-window', '--working-directory=/home/hugo/app']]);
    assert.deepEqual(linux.terminalCommand('GNOME Terminal', '/srv/a b'), ['gnome-terminal', ['--working-directory=/srv/a b']]);
    assert.deepEqual(linux.terminalCommand('Konsole', '/srv/app'), ['konsole', ['--workdir', '/srv/app']]);
  });
});

describe('Linux : démarrage avec la session (systemd)', () => {
  test('service utilisateur : relancé s’il plante, les projets survivent à son arrêt', () => {
    const unit = linux.unitFile({ node: '/usr/bin/node', root: '/home/hugo/Servym 100%' });
    assert.match(unit, /^ExecStart="\/usr\/bin\/node" "\/home\/hugo\/Servym 100%%\/server\.js"$/m, '% doublé pour systemd');
    assert.match(unit, /^WorkingDirectory=\/home\/hugo\/Servym 100%%$/m);
    assert.match(unit, /^Environment=SERVYM_SUPERVISOR=systemd$/m);
    assert.match(unit, /^Restart=on-failure$/m);
    assert.match(unit, /^KillMode=process$/m);
    assert.match(unit, /^WantedBy=default\.target$/m);
  });

  test('activer, lire l’état, désactiver', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-systemd-'));
    const calls = [];
    let enabled = false;
    const run = async (cmd, args) => {
      calls.push([cmd, ...args].join(' '));
      if (args.includes('is-enabled')) {
        if (enabled) return { stdout: 'enabled\n' };
        throw Object.assign(new Error('exit 1'), { stdout: 'disabled\n' });
      }
      if (args.includes('enable')) enabled = true;
      if (args.includes('disable')) enabled = false;
      return { stdout: '' };
    };
    const task = linux.systemdTask({ dir, run, node: '/usr/bin/node', root: '/opt/servym' });
    try {
      assert.equal(await task.enabled(), false);
      await task.set(true);
      assert.ok(fs.existsSync(path.join(dir, 'servym.service')));
      assert.equal(await task.enabled(), true);
      await task.set(false);
      assert.equal(await task.enabled(), false);
      assert.ok(!fs.existsSync(path.join(dir, 'servym.service')), 'fichier retiré');
      assert.deepEqual(calls.filter((c) => !c.includes('is-enabled')), [
        'systemctl --user daemon-reload',
        'systemctl --user enable servym.service',
        'systemctl --user disable servym.service',
        'systemctl --user daemon-reload',
      ]);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  test('superviseur reconnu : Windows (vbs) ou systemd', () => {
    assert.equal(isSupervised({ SERVYM_SUPERVISOR: 'vbs' }), true);
    assert.equal(isSupervised({ SERVYM_SUPERVISOR: 'systemd' }), true);
    assert.equal(isSupervised({}), false);
  });
});

describe('Linux : serveur', () => {
  async function makeApp(options) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-linux-'));
    fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify({ instance: { port: 1 }, projects: [{ id: 'doc', name: 'Doc', type: 'file', cwd: dir, open: 'index.html' }] }));
    const app = createApp({ configFile: path.join(dir, 'projects.json'), logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime'), task: { enabled: async () => false, set: async () => {} }, notify: async () => true, ...options });
    await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
    const port = app.server.address().port;
    const call = (method, p) => new Promise((resolve, reject) => {
      const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: { host: `localhost:${port}`, 'x-servym-request': '1' } }, (res) => {
        let data = '';
        res.on('data', (c) => (data += c));
        res.on('end', () => resolve({ status: res.statusCode, json: JSON.parse(data || 'null') }));
      });
      req.on('error', reject);
      req.end();
    });
    const close = async () => {
      await app.manager.close();
      app.server.closeAllConnections();
      await new Promise((r) => app.server.close(r));
      fs.rmSync(dir, { recursive: true, force: true });
    };
    return { call, close };
  }

  test('système indiqué à la page', async () => {
    const s = await makeApp({ platform: 'linux' });
    try {
      assert.equal((await s.call('GET', '/api/projects')).json.instance.platform, 'linux');
    } finally {
      await s.close();
    }
  });

  test('sans écran : rien ne s’ouvre, même devant la machine', async () => {
    const s = await makeApp({ platform: 'linux', desktop: false });
    try {
      const { instance } = (await s.call('GET', '/api/projects')).json;
      assert.equal(instance.viewerLocal, false);
      assert.deepEqual(instance.tools, { desktop: false, code: false, terminal: null }, 'aucun outil annoncé');
      const res = await s.call('POST', '/api/projects/doc/open');
      assert.equal(res.status, 403);
      assert.match(res.json.error, /sans écran/);
    } finally {
      await s.close();
    }
  });
});
