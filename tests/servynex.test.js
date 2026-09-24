import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { createProject, describeChanges, formatConfig, loadConfig, patchProject, removeProject, reorderProjects, validate } from '../src/config.js';
import { lastError } from '../src/logs.js';
import { resolveInside } from '../src/static.js';
import { Sampler, treeOf, usageOf } from '../src/stats.js';
import { Manager } from '../src/processes.js';
import { createApp } from '../server.js';
import { setServerLang } from '../src/i18n.js';

// Messages attendus en français, quelle que soit la langue de la machine qui lance les tests.
process.env.SERVYNEX_LANG = 'fr';
setServerLang('fr');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function freePort() {
  const server = net.createServer().listen(0, '127.0.0.1');
  await new Promise((r) => server.once('listening', r));
  const { port } = server.address();
  await new Promise((r) => server.close(r));
  return port;
}

async function until(check, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const value = await check();
    if (value) return value;
    await sleep(150);
  }
  throw new Error('délai dépassé');
}

function request(port, { method = 'GET', path: p = '/', headers = {}, body } = {}) {
  return new Promise((resolve, reject) => {
    const length = body === undefined ? {} : { 'content-length': Buffer.byteLength(body) }; // DELETE : Node n'envoie pas la longueur tout seul
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: { host: `localhost:${port}`, ...length, ...headers } }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: data }));
    });
    req.on('error', reject);
    req.end(body);
  });
}

const base = {
  instance: { port: 9000 },
  projects: [
    { id: 'a', name: 'A', cwd: 'C:/a', cmd: 'node', args: ['a.js'], port: 3001, hostControl: 'env' },
    { id: 'b', name: 'B', cwd: 'C:/b', type: 'static', port: 3002, fixedPort: 'fixe' },
    { id: 'c', name: 'C', cwd: 'C:/c', cmd: 'node', args: [], port: 3003, hostControl: 'local' },
  ],
};

describe('configuration', () => {
  test('refuse les ports en double, y compris celui de Servynex', () => {
    const conf = structuredClone(base);
    conf.projects[1].port = 3001;
    assert.throws(() => validate(conf), /Port 3001/);
    conf.projects[1].port = 9000;
    assert.throws(() => validate(conf), /Servynex/);
  });

  test('refuse guillemets, % et métacaractères de cmd.exe dans les arguments', () => {
    const conf = structuredClone(base);
    for (const arg of ['a.js" & calc', '%PATH%', 'a & calc', 'a | calc', 'a > b', 'a ^& b', '(a)']) {
      conf.projects[0].args = [arg];
      assert.throws(() => validate(conf), /args/, arg);
      assert.throws(() => patchProject(base, 'a', { args: [arg] }), /args/, `interface : ${arg}`);
    }
    conf.projects[0].args = ['--port={port}', '--defaults-file={servynex}/config/mysql.ini', 'server/server.js'];
    assert.doesNotThrow(() => validate(conf));
  });

  test('stop et env validés comme la commande', () => {
    const conf = structuredClone(base);
    conf.projects[0].stop = { cmd: 'mysqladmin', args: ['--port={port}', 'shutdown'] };
    assert.doesNotThrow(() => validate(conf));
    for (const args of [[3306], ['a & b'], 'shutdown']) {
      conf.projects[0].stop.args = args;
      assert.throws(() => validate(conf), /stop/, JSON.stringify(args));
    }
    delete conf.projects[0].stop;
    conf.projects[0].env = { SHELL_VERBOSITY: '1' };
    assert.doesNotThrow(() => validate(conf));
    for (const env of [{ 'A B': '1' }, { A: 1 }, { A: '%PATH%' }, ['A=1']]) {
      conf.projects[0].env = env;
      assert.throws(() => validate(conf), /env/, JSON.stringify(env));
    }
  });

  test("l'interface modifie commande, dossier et environnement, jamais l'identifiant", () => {
    const next = patchProject(base, 'a', { cmd: 'npm', args: ['run', 'dev'], cwd: 'D:/app', env: { A: '1' }, stop: { cmd: 'x', args: [] } });
    assert.deepEqual(next.projects[0].args, ['run', 'dev']);
    assert.equal(next.projects[0].cwd, 'D:/app');
    assert.equal(patchProject(next, 'a', { env: null, stop: null }).projects[0].env, undefined, 'null retire le champ');
    assert.throws(() => patchProject(base, 'a', { id: 'z' }), /non modifiable/);
    assert.throws(() => patchProject(base, 'a', { cwd: null }), /obligatoire/);
    assert.throws(() => patchProject(base, 'a', { cwd: 'relatif' }), /absolu/);
    assert.throws(() => patchProject(base, 'a', JSON.parse('{"__proto__": {"cmd": "calc"}}')), /non modifiable/);
  });

  test('projet « file » : sans port, chemin relatif ; un changement de type retire les champs sans objet', () => {
    const conf = structuredClone(base);
    conf.projects.push({ id: 'f', name: 'F', type: 'file', cwd: 'C:/f', open: 'index.html' });
    assert.doesNotThrow(() => validate(conf));
    assert.equal(patchProject(conf, 'f', { name: 'G' }).projects[3].name, 'G');
    for (const open of ['../secret.html', 'C:/Windows/win.ini', 'a/../../b.html', '']) {
      conf.projects[3].open = open;
      assert.throws(() => validate(conf), /fichier à ouvrir/, open);
    }
    conf.projects[3].open = 'index.html';
    conf.projects[3].port = 4000;
    assert.throws(() => validate(conf), /pas de port/);
    const asFile = patchProject(base, 'a', { type: 'file', open: 'index.html' }).projects[0];
    assert.equal(asFile.cmd, undefined);
    assert.equal(asFile.port, undefined);
  });

  test('autostart : booléen, interdit pour un fichier', () => {
    const conf = structuredClone(base);
    conf.projects[0].autostart = 'oui';
    assert.throws(() => validate(conf), /autostart/);
    assert.equal(patchProject(base, 'a', { autostart: true }).projects[0].autostart, true);
  });

  test("icône : nom d'icône ou court texte", () => {
    assert.equal(patchProject(base, 'a', { icon: 'graduation-cap' }).projects[0].icon, 'graduation-cap');
    assert.equal(patchProject(base, 'a', { icon: '🎯' }).projects[0].icon, '🎯');
    assert.throws(() => patchProject(base, 'a', { icon: 'un texte beaucoup trop long' }), /icon/);
    assert.throws(() => patchProject(base, 'a', { icon: 42 }), /icon/);
  });

  test('port fixe, port pris, exposition impossible', () => {
    assert.throws(() => patchProject(base, 'b', { port: 4000 }), /fixe/);
    assert.throws(() => patchProject(base, 'a', { port: 3003 }), /Port 3003/);
    assert.throws(() => patchProject(base, 'c', { expose: true }), /local/);
    assert.throws(() => patchProject(base, 'a', { port: 70000 }), /port/);
    const next = patchProject(base, 'a', { port: 4000, expose: true, tags: [' x '] });
    assert.equal(next.projects[0].port, 4000);
    assert.deepEqual(next.projects[0].tags, ['x']);
    assert.equal(base.projects[0].port, 3001, "l'original n'est pas modifié");
  });

  test('dépendances : inconnues, fichier ou en boucle refusées', () => {
    assert.throws(() => patchProject(base, 'a', { requires: ['zz'] }), /inconnu/);
    assert.throws(() => patchProject(base, 'a', { requires: ['a'] }), /lui-même/);
    const loop = patchProject(base, 'a', { requires: ['c'] });
    assert.throws(() => patchProject(loop, 'c', { requires: ['a'] }), /boucle/);
    const conf = structuredClone(base);
    conf.projects.push({ id: 'f', name: 'F', type: 'file', cwd: 'C:/f', open: 'index.html' });
    assert.throws(() => patchProject(conf, 'a', { requires: ['f'] }), /fichier/);
  });

  test('ajout : identifiant tiré du nom, champs par défaut, validation complète', () => {
    const { config, id } = createProject(base, { name: 'Mon Été', cwd: 'C:/m', cmd: 'node', args: ['s.js'], port: 3500 });
    assert.equal(id, 'mon-ete');
    assert.equal(config.projects.at(-1).hostControl, 'env');
    assert.equal(createProject(config, { name: 'Mon été', type: 'file', cwd: 'C:/m', open: 'index.html' }).id, 'mon-ete-2');
    assert.throws(() => createProject(base, { name: 'X', cwd: 'C:/x', cmd: 'node', port: 3001 }), /Port 3001/);
    assert.throws(() => createProject(base, { name: 'X', cwd: 'C:/x', port: 3600 }), /commande/);
    assert.throws(() => createProject(base, { name: 'X', cwd: 'C:/x', cmd: 'a & calc', port: 3600 }), /commande/);
    assert.equal(base.projects.length, 3, "l'original n'est pas modifié");
  });

  test('suppression : retirée des dépendances des autres projets', () => {
    const conf = structuredClone(base);
    conf.projects.push({ id: 'd', name: 'D', cwd: 'C:/d', cmd: 'node', args: [], port: 3004, hostControl: 'env', requires: ['a', 'c'] });
    conf.projects.push({ id: 'e', name: 'E', cwd: 'C:/e', cmd: 'node', args: [], port: 3005, hostControl: 'env', requires: ['a'] });
    const next = removeProject(conf, 'a');
    assert.deepEqual(next.projects.map((p) => p.id), ['b', 'c', 'd', 'e']);
    assert.deepEqual(next.projects.find((p) => p.id === 'd').requires, ['c']);
    assert.equal('requires' in next.projects.find((p) => p.id === 'e'), false, 'liste vide : champ retiré');
    assert.equal(conf.projects.length, 5, "l'original n'est pas modifié");
    assert.throws(() => removeProject(conf, 'absent'), (err) => err.status === 404);
  });

  test('différences de réglages : valeurs lisibles, jamais celles de env', () => {
    const before = { id: 'a', name: 'A', port: 1, env: { K: 'secret' }, tags: ['x'] };
    const after = { id: 'a', name: 'B', port: 1, env: { K: 'autre', L: '2' }, tags: ['x', 'y'], url: 'http://a' };
    assert.equal(describeChanges(before, after), 'name "A" → "B" · env K=••• → K=•••, L=••• · tags ["x"] → ["x","y"] · url — → "http://a"');
    assert.equal(describeChanges(before, before), '');
    assert.match(describeChanges({ desc: 'x'.repeat(200) }, {}), /^desc "x{40,}…" → —$/, 'valeurs longues coupées');
  });

  test('réordonner : tous les identifiants, chacun une fois', () => {
    const next = reorderProjects(base, ['c', 'a', 'b']);
    assert.deepEqual(next.projects.map((p) => p.id), ['c', 'a', 'b']);
    assert.deepEqual(base.projects.map((p) => p.id), ['a', 'b', 'c'], "l'original n'est pas modifié");
    for (const order of [['c', 'a'], ['c', 'a', 'b', 'x'], ['a', 'a', 'b'], 'a,b,c', null, ['a', 'b', 3]]) {
      assert.throws(() => reorderProjects(base, order), (err) => err.status === 400, JSON.stringify(order));
    }
  });

  test('langue des messages du serveur (instance.lang) : fr ou en', () => {
    for (const lang of ['fr', 'en']) assert.doesNotThrow(() => validate({ ...base, instance: { port: 9000, lang } }));
    assert.throws(() => validate({ ...base, instance: { port: 9000, lang: 'de' } }), /instance.lang/);
  });

  test('fichier absent : configuration vide ; mise en forme compacte conservée', () => {
    const missing = path.join(os.tmpdir(), `servynex-absent-${process.pid}.json`);
    assert.deepEqual(loadConfig(missing), { instance: { port: 9000 }, projects: [] });
    const text = formatConfig({ instance: { port: 9000 }, projects: [{ id: 'a', name: 'A', tags: ['x', 'y'], health: { path: '/' } }] });
    assert.match(text, /"tags": \["x", "y"\]/);
    assert.match(text, /"health": \{ "path": "\/" \}/);
    assert.match(text, /"instance": \{ "port": 9000 \}/);
  });
});

describe('processeur et mémoire', () => {
  // Relevé : PID → { pid, ppid, start (ms), cpu (ms cumulées), memory (octets) }.
  const proc = (pid, ppid, start, cpu, memory) => [pid, { pid, ppid, start, cpu, memory }];
  const snap = (at, list) => ({ at, processes: new Map(list) });

  test('arborescence : enfants et petits-enfants, jamais un PID réutilisé', () => {
    const { processes } = snap(0, [proc(10, 1, 100, 0, 0), proc(11, 10, 110, 0, 0), proc(12, 11, 120, 0, 0), proc(13, 10, 50, 0, 0), proc(20, 1, 100, 0, 0)]);
    assert.deepEqual([...treeOf(processes, [10])].sort(), [10, 11, 12], "13 a démarré avant 10 : son parent n'est pas ce 10-là");
    assert.deepEqual([...treeOf(processes, [10, 11])].sort(), [10, 11, 12], 'racines qui se recouvrent : chaque processus une fois');
    assert.equal(treeOf(processes, [99]).size, 0);
  });

  test('somme de l’arborescence, en part de toute la machine', () => {
    const prev = snap(10_000, [proc(10, 1, 100, 1_000, 0), proc(11, 10, 110, 3_000, 0), proc(12, 11, 120, 500, 0)]);
    const next = snap(15_000, [proc(10, 1, 100, 1_200, 10e6), proc(11, 10, 110, 3_600, 50e6), proc(12, 11, 12_000, 200, 5e6), proc(14, 11, 13_000, 200, 1e6)]);
    // 12 a été remplacé (autre début) et 14 est nouveau : comptés depuis leur démarrage.
    // (200 + 600 + 200 + 200) ms sur 5 000 ms × 4 cœurs = 6 %.
    assert.deepEqual(usageOf(prev, next, [10], 4), { cpu: 6, memory: 66e6, processes: 4 });
    assert.equal(usageOf(null, next, [10], 4).cpu, null, 'premier relevé : processeur inconnu');
    assert.equal(usageOf(null, next, [10], 4).memory, 66e6);
    assert.equal(usageOf(prev, next, [99], 4), null, 'processus introuvable');
  });

  test('relevés partagés : des lectures rapprochées ne relancent pas la mesure', async () => {
    let calls = 0;
    let now = 0;
    const sampler = new Sampler({ sample: async () => new Map([proc(10, 1, 0, ++calls * 1000, 5e6)]), now: () => now, cores: 1, maxAge: 2000 });
    const first = await sampler.measure({ a: [10], b: [99] });
    assert.deepEqual(first, { a: { cpu: null, memory: 5e6, processes: 1 } }, 'b introuvable : absent');
    now = 1000;
    assert.deepEqual(await sampler.measure({ a: [10] }), first, 'moins de 2 s : même relevé');
    assert.equal(calls, 1);
    now = 5000;
    await Promise.all([sampler.measure({ a: [10] }), sampler.measure({ a: [10] })]);
    assert.equal(calls, 2, 'deux onglets en même temps : une seule mesure');
    assert.equal((await sampler.measure({ a: [10] })).a.cpu, 20, '(2 000 − 1 000) ms sur 5 000 ms');
  });
});

test('total : Servynex compris, chaque processus compté une fois', async () => {
  // Servynex (1) lance le projet par un PowerShell (2) → cmd.exe (3, racine du projet) → node (4) ;
  // le PowerShell des relevés (5) est aussi son enfant.
  const snap = new Map([[1, 0, 100], [2, 1, 110], [3, 2, 120], [4, 3, 130], [5, 1, 140]].map(([pid, ppid, start]) => [pid, { pid, ppid, start, cpu: 0, memory: pid * 1e6 }]));
  const sampler = new Sampler({ sample: async () => snap, now: () => 0, cores: 1 });
  const res = await sampler.measureAll({ self: [3] }, 1);
  assert.equal(res.projects.self.processes, 2, 'un projet peut s’appeler « self »');
  assert.equal(res.projects.self.memory, 7e6);
  assert.deepEqual(res.self, { cpu: null, memory: 8e6, processes: 3 }, 'Servynex sans ses projets');
  assert.deepEqual(res.total, { cpu: null, memory: 15e6, processes: 5 }, 'chacun une fois');
});

describe('fichiers statiques', () => {
  const root = path.resolve('C:/site');
  test('reste dans le dossier', () => {
    assert.equal(resolveInside(root, '/css/a.css'), path.join(root, 'css/a.css'));
    assert.equal(resolveInside(root, '/'), root);
    assert.equal(resolveInside(root, '/../secret'), null);
    assert.equal(resolveInside(root, '/%2e%2e/%2e%2e/Windows/win.ini'), null);
    assert.equal(resolveInside(root, '/..%5c..%5cWindows'), null);
    assert.equal(resolveInside(root, '//serveur/partage/x'), path.join(root, 'serveur/partage/x'));
    assert.equal(resolveInside(root, '/%5c%5cserveur%5cpartage'), path.join(root, 'serveur/partage'));
  });
  test('refuse fichiers cachés, flux NTFS et octets nuls', () => {
    assert.equal(resolveInside(root, '/.git/config'), null);
    assert.equal(resolveInside(root, '/.env'), null);
    assert.equal(resolveInside(root, '/index.html::$DATA'), null);
    assert.equal(resolveInside(root, '/a%00.html'), null);
    assert.equal(resolveInside(root, '/%E0%A4%A'), null);
  });
});

test("lastError choisit la ligne qui explique l'arrêt, pas un avertissement", () => {
  const file = path.join(os.tmpdir(), `servynex-test-${process.pid}.log`);
  fs.writeFileSync(file, '[servynex] lancement\nprêt\nError: listen EADDRINUSE :::3000\n    at Server.listen\n(node) Warning: deprecated\n');
  assert.equal(lastError(file), 'Error: listen EADDRINUSE :::3000');
  fs.writeFileSync(file, 'Connexion impossible à la base\nfin\n');
  assert.equal(lastError(file), 'Connexion impossible à la base');
  fs.rmSync(file);
});

describe('serveur et processus', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-test-'));
  let app;
  let port;
  let ports;

  const state = (id) => app.manager.state(id);
  const post = (p, headers = {}) => request(port, { method: 'POST', path: p, headers: { 'x-servynex-request': '1', ...headers } });
  const get = (p) => request(port, { path: p, headers: { 'x-servynex-request': '1' } }).then((r) => ({ ...r, json: JSON.parse(r.body) }));

  before(async () => {
    ports = { ok: await freePort(), crash: await freePort(), site: await freePort(), busy: await freePort(), wrong: await freePort(), respawn: await freePort(), auto: await freePort(), db: await freePort(), web: await freePort(), solo: await freePort(), sdb: await freePort() };
    fs.mkdirSync(path.join(dir, 'app'));
    fs.writeFileSync(path.join(dir, 'app', 'server.js'), `
      const http = require('node:http');
      console.log('démarrage sur', process.env.HOST, process.env.PORT, process.env.SERVYNEX_TEST_ENV ?? '');
      http.createServer((q, s) => s.end('<title>Factice</title>')).listen(process.env.PORT, process.env.HOST || undefined);
    `);
    // Comme une application qui se relance elle-même : /restart lance une nouvelle instance (même sortie) puis quitte.
    fs.writeFileSync(path.join(dir, 'app', 'respawn.js'), `
      const http = require('node:http');
      const { spawn } = require('node:child_process');
      console.log('instance', process.pid);
      const server = http.createServer((q, s) => {
        if (q.url !== '/restart') return s.end('<title>Relance</title>');
        s.end('ok');
        server.close(() => {
          spawn(process.execPath, [__filename], { detached: true, stdio: 'inherit' }).unref();
          process.exit(0);
        });
        server.closeAllConnections();
      }).listen(process.env.PORT, process.env.HOST || undefined);
    `);
    fs.writeFileSync(path.join(dir, 'app', 'crash.js'), `console.log('ok'); console.error('Error: base introuvable'); process.exit(3);`);
    fs.mkdirSync(path.join(dir, 'site', 'sub'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'site', 'index.html'), 'accueil');
    fs.writeFileSync(path.join(dir, 'site', 'page.html'), 'page');
    fs.writeFileSync(path.join(dir, 'site', '.env'), 'SECRET');
    const config = {
      instance: { port: 1 },
      projects: [
        { id: 'ok', name: 'Factice', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.ok, hostControl: 'env', health: { expect: 'Factice' }, url: 'http://localhost:{port}', env: { SERVYNEX_TEST_ENV: 'bonjour', PORT: '1' } },
        { id: 'crash', name: 'Crash', cwd: path.join(dir, 'app'), cmd: 'node', args: ['crash.js'], port: ports.crash, hostControl: 'env' },
        { id: 'site', name: 'Site', cwd: path.join(dir, 'site'), type: 'static', port: ports.site },
        { id: 'respawn', name: 'Relance', cwd: path.join(dir, 'app'), cmd: 'node', args: ['respawn.js'], port: ports.respawn, hostControl: 'env', health: { expect: 'Relance' } },
        { id: 'auto', name: 'Auto', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.auto, hostControl: 'env', autostart: true, health: { expect: 'Factice' } },
        { id: 'doc', name: 'Doc', cwd: path.join(dir, 'site'), type: 'file', open: 'absent.html' },
        { id: 'busy', name: 'Occupé', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.busy, hostControl: 'env' },
        { id: 'wrong', name: 'Autre', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.wrong, hostControl: 'env', health: { expect: 'Pas ça' } },
        { id: 'dep', name: 'Dépendant', cwd: path.join(dir, 'app'), cmd: 'node', args: ['crash.js'], port: await freePort(), hostControl: 'env', requires: ['crash'] },
        { id: 'db', name: 'Base', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.db, hostControl: 'env', service: true, health: { expect: 'Factice' } },
        { id: 'web', name: 'Web', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.web, hostControl: 'env', requires: ['db'], health: { expect: 'Factice' } },
        { id: 'sdb', name: 'Base solo', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.sdb, hostControl: 'env', service: true, health: { expect: 'Factice' } },
        { id: 'solo', name: 'Solo', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: ports.solo, hostControl: 'env', requires: ['sdb'], health: { expect: 'Factice' } },
        { id: 'fan', name: 'Fan', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: await freePort(), hostControl: 'env', requires: ['solo', 'db'] },
      ],
    };
    fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify(config));
    app = createApp({ configFile: path.join(dir, 'projects.json'), logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime'), tickMs: 500, runningTickMs: 500, startTimeoutMs: 8000, relaunchMs: 10000, bootTime: 1_000_000 });
    await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
    port = app.server.address().port;
    await app.manager.init();
  });

  after(async () => {
    for (const id of ['ok', 'wrong', 'respawn', 'auto', 'db', 'web', 'nouveau-projet', 'sdb']) if (state(id).managed || state(id).state === 'running') await app.manager.stop(id).catch(() => {});
    await app.manager.close();
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('protections HTTP', async () => {
    assert.equal((await request(port, { path: '/api/projects', headers: { host: 'evil.example' } })).status, 421);
    assert.equal((await request(port, { method: 'POST', path: '/api/projects/ok/start' })).status, 403);
    assert.equal((await post('/api/projects/ok/start', { origin: 'http://evil.example' })).status, 403);
    assert.equal((await post('/api/projects/ok/start', { origin: 'null' })).status, 403);
    assert.equal((await request(port, { path: '/api/projects', headers: { 'sec-fetch-site': 'cross-site' } })).status, 403);
    for (const route of ['/api/free-port?from=1024', '/api/git?fresh', '/api/ports', '/api/network', '/api/autostart']) {
      assert.equal((await request(port, { path: route })).status, 403, `${route} sans en-tête`);
    }
    assert.equal((await request(port, { path: '/server.js' })).status, 404);
    assert.equal((await request(port, { path: '/projects.json' })).status, 404);
    assert.equal((await post('/api/projects/__proto__/start')).status, 404);
    const page = await request(port, { path: '/' });
    assert.equal(page.status, 200);
    assert.match(page.headers['content-security-policy'], /script-src 'self'/);
    assert.equal((await request(port, { method: 'PATCH', path: '/api/projects/ok', headers: { 'x-servynex-request': '1' }, body: '{}' })).status, 415);
    assert.equal((await request(port, { method: 'POST', path: '/api/projects/ok/start', headers: { 'x-hub-request': '1' } })).status, 403, 'ancien en-tête X-Hub-Request refusé');
  });

  test('ressources : processeur et mémoire des projets en marche', async () => {
    assert.equal((await request(port, { path: '/api/stats' })).status, 403, 'sans en-tête : une mesure coûte');
    await post('/api/projects/ok/start');
    await until(() => state('ok').state === 'running');
    const first = (await get('/api/stats')).json;
    assert.ok(first.projects.ok.memory > 1_000_000, 'mémoire du processus node');
    assert.ok(first.projects.ok.processes >= 2, 'cmd.exe puis node : toute l’arborescence');
    assert.equal(first.projects.crash, undefined, 'arrêté : absent');
    assert.ok(first.self.memory > 1_000_000 && first.self.processes >= 1, 'Servynex lui-même');
    assert.ok(first.total.memory >= first.projects.ok.memory + first.self.memory, 'total');
    await sleep(2500);
    const second = (await get('/api/stats')).json;
    assert.equal(typeof second.projects.ok.cpu, 'number');
    assert.ok(second.projects.ok.cpu >= 0 && second.projects.ok.cpu <= 100);
    await post('/api/projects/ok/stop');
    await until(() => state('ok').state === 'stopped');
  });

  test('lance, suit le journal, arrête', async () => {
    assert.equal((await post('/api/projects/ok/start')).status, 200);
    assert.equal((await post('/api/projects/ok/start')).status, 409, 'double lancement refusé');
    await until(() => state('ok').state === 'running');
    assert.ok(state('ok').managed && state('ok').pid);
    const res = await request(ports.ok);
    assert.match(res.body, /Factice/);
    await until(() => fs.readFileSync(app.manager.logFile('ok'), 'utf8').includes('démarrage sur 127.0.0.1'));
    const tail = JSON.parse((await request(port, { path: '/api/projects/ok/logs?tail' })).body);
    assert.ok(tail.lines.some((l) => l.includes('démarrage sur 127.0.0.1')), 'HOST=127.0.0.1 transmis, accents intacts');
    assert.ok(tail.lines.some((l) => l.includes(`${ports.ok} bonjour`)), 'env du projet transmis, sans écraser PORT');
    const { json: overview } = await get('/api/ports');
    const row = overview.ports.find((r) => r.port === ports.ok);
    assert.equal(row?.project, 'ok');
    assert.equal(row.exposed, false);
    assert.equal(row.process, 'node.exe');
    // Servynex redémarré : le projet est repris, sans « stopped → running » dans l'historique.
    const next = new Manager({ config: app.manager.config, logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime'), bootTime: 1_000_000 });
    const transitions = [];
    next.on('transition', (t) => transitions.push(t));
    await next.init();
    assert.equal(next.state('ok').state, 'running');
    assert.ok(next.state('ok').managed, 'repris par son PID');
    assert.deepEqual(transitions.filter((t) => t.id === 'ok'), []);
    await next.close();

    const pid = state('ok').pid;
    assert.equal((await post('/api/projects/ok/stop')).status, 200);
    assert.equal(state('ok').state, 'stopped');
    await until(async () => !(await request(ports.ok).catch(() => null)));
    assert.throws(() => process.kill(pid, 0));
  });

  test("un plantage passe en échec avec la ligne d'erreur", async () => {
    await post('/api/projects/crash/start');
    await until(() => state('crash').state === 'failed');
    assert.equal(state('crash').exitCode, 3);
    assert.equal(state('crash').error, 'Error: base introuvable');
    const { entries } = (await get('/api/projects/crash/history')).json;
    assert.deepEqual(entries[0], { at: entries[0].at, id: 'crash', from: 'starting', to: 'failed', detail: 'code 3 · Error: base introuvable' });
    assert.equal(entries[1].to, 'starting', 'du plus récent au plus ancien');
  });

  test('une dépendance en échec bloque le lancement', async () => {
    await post('/api/projects/dep/start');
    await until(() => state('dep').state === 'failed');
    assert.match(state('dep').error, /Crash n'a pas démarré/);
  });

  test('port déjà pris, ou pris par un autre site', async () => {
    const blocker = net.createServer().listen(ports.busy, '127.0.0.1');
    await new Promise((r) => blocker.once('listening', r));
    await post('/api/projects/busy/start');
    await until(() => state('busy').state === 'failed');
    assert.match(state('busy').error, new RegExp(`port ${ports.busy} est déjà pris`));
    blocker.close();

    await post('/api/projects/wrong/start');
    // « failed » à la fin du lancement, puis « conflict » dès la sonde suivante
    // (le port répond, mais pas le bon site) : les deux sont justes.
    await until(() => ['failed', 'conflict'].includes(state('wrong').state));
    assert.match(state('wrong').error, /ce n'est pas Autre/);
    assert.equal((await post('/api/projects/wrong/stop')).status, 200, 'le processus lancé reste arrêtable');
  });

  test('site statique intégré', async () => {
    await post('/api/projects/site/start');
    await until(() => state('site').state === 'running');
    assert.equal((await request(ports.site, { path: '/' })).body, 'accueil');
    assert.equal((await request(ports.site, { path: '/page' })).body, 'page');
    assert.equal((await request(ports.site, { path: '/.env' })).status, 404);
    await post('/api/projects/site/stop');
    assert.equal(state('site').state, 'stopped');
    await assert.rejects(request(ports.site));
  });

  test('un projet qui se relance lui-même est repris, avec son journal', async () => {
    await post('/api/projects/respawn/start');
    await until(() => state('respawn').state === 'running');
    const first = state('respawn').pid;
    await request(ports.respawn, { path: '/restart' });
    await until(() => state('respawn').state === 'running' && state('respawn').pid !== first);
    assert.ok(state('respawn').managed, 'repris comme lancé par Servynex');
    const log = fs.readFileSync(app.manager.logFile('respawn'), 'utf8');
    assert.equal(log.match(/^instance /gm).length, 2, 'la nouvelle instance écrit dans le même journal');
    assert.equal((await post('/api/projects/respawn/stop')).status, 200);
    assert.equal(state('respawn').state, 'stopped');
    await assert.rejects(request(ports.respawn));
  });

  test('démarrage automatique : une seule fois par démarrage du PC', async () => {
    await until(() => state('auto').state === 'running');
    assert.ok(state('auto').managed);
    await post('/api/projects/auto/stop');
    const config = app.manager.config;
    const runtimeDir = path.join(dir, 'runtime');
    const again = new Manager({ config, logsDir: path.join(dir, 'logs'), runtimeDir, bootTime: 1_000_000 + 30_000 });
    await again.init();
    await sleep(1500);
    assert.equal(again.state('auto').state, 'stopped', 'Servynex redémarré, même démarrage du PC : pas relancé');
    await again.close();
    const reboot = new Manager({ config, logsDir: path.join(dir, 'logs'), runtimeDir, bootTime: 1_000_000 + 3_600_000 });
    await reboot.init();
    await until(() => reboot.state('auto').state === 'running');
    await reboot.stop('auto');
    await reboot.close();
  });

  test('projet « file » : jamais lancé ni arrêté, fichier vérifié avant ouverture', async () => {
    const doc = app.manager.view(app.manager.project('doc'));
    assert.equal(doc.state, 'file');
    assert.equal(doc.file, path.join(dir, 'site', 'absent.html'));
    assert.equal((await post('/api/projects/doc/start')).status, 409);
    assert.equal((await post('/api/projects/doc/stop')).status, 409);
    assert.equal((await post('/api/projects/doc/restart')).status, 409);
    const open = await post('/api/projects/doc/open');
    assert.equal(open.status, 404, "fichier absent : rien n'est lancé");
    assert.match(JSON.parse(open.body).error, /introuvable/);
    assert.equal((await post('/api/projects/ok/open')).status, 400);
  });

  test('réglages : enregistrés dans le fichier, champs protégés', async () => {
    const patch = (body) => request(port, { method: 'PATCH', path: '/api/projects/ok', headers: { 'x-servynex-request': '1', 'content-type': 'application/json' }, body: JSON.stringify(body) });
    assert.equal((await patch({ id: 'autre' })).status, 400);
    assert.equal((await patch({ args: ['a & calc'] })).status, 400);
    assert.equal((await patch({ port: ports.site })).status, 400);
    const res = await patch({ name: 'Renommé', expose: true });
    assert.equal(res.status, 200);
    assert.equal(JSON.parse(res.body).host, '0.0.0.0');
    const saved = JSON.parse(fs.readFileSync(path.join(dir, 'projects.json'), 'utf8'));
    assert.equal(saved.projects[0].name, 'Renommé');
    const { json: free } = await get(`/api/free-port?from=${ports.ok}`);
    assert.ok(free.port >= ports.ok && ![ports.site, ports.crash, ports.busy].includes(free.port));
  });

  test("une modification faite à la main n'est pas écrasée par l'interface", async () => {
    const file = path.join(dir, 'projects.json');
    const onDisk = JSON.parse(fs.readFileSync(file, 'utf8'));
    onDisk.projects.find((p) => p.id === 'site').desc = 'écrit à la main';
    fs.writeFileSync(file, JSON.stringify(onDisk));
    const res = await request(port, { method: 'PATCH', path: '/api/projects/crash', headers: { 'x-servynex-request': '1', 'content-type': 'application/json' }, body: JSON.stringify({ desc: "depuis l'interface" }) });
    assert.equal(res.status, 200);
    const after = JSON.parse(fs.readFileSync(file, 'utf8'));
    assert.equal(after.projects.find((p) => p.id === 'site').desc, 'écrit à la main');
    assert.equal(after.projects.find((p) => p.id === 'crash').desc, "depuis l'interface");
    assert.equal(app.manager.project('site').desc, 'écrit à la main', 'la modification à la main est aussi appliquée');
  });

  test('activité : chaque modification de réglage, avec ses valeurs', async () => {
    const patch = (body) => request(port, { method: 'PATCH', path: '/api/projects/dep', headers: { 'x-servynex-request': '1', 'content-type': 'application/json' }, body: JSON.stringify(body) });
    assert.equal((await patch({ desc: 'avant', startTimeout: 20 })).status, 200);
    assert.equal((await patch({ desc: 'après', startTimeout: 30, env: { SECRET_TOKEN: 'abc123' } })).status, 200);
    const texts = async () => (await get('/api/history')).json.entries.filter((e) => e.kind === 'audit' && e.id === 'dep').map((e) => e.text);
    let all = await texts();
    assert.ok(all.some((t) => /desc "avant" → "après"/.test(t) && /startTimeout 20 → 30/.test(t) && /env — → SECRET_TOKEN=•••/.test(t)), all.join(' | '));
    assert.ok(!all.some((t) => t.includes('abc123')), 'valeurs de env jamais écrites');
    // Modifié à la main, puis relu : la différence est aussi inscrite.
    const file = path.join(dir, 'projects.json');
    const conf = JSON.parse(fs.readFileSync(file, 'utf8'));
    conf.projects.find((p) => p.id === 'dep').desc = 'à la main';
    fs.writeFileSync(file, JSON.stringify(conf));
    assert.equal((await post('/api/reload')).status, 200);
    all = await texts();
    assert.ok(all.some((t) => /projects.json/.test(t) && /desc "après" → "à la main"/.test(t)), all.join(' | '));
  });

  test('ajout depuis l’interface, puis lancement', async () => {
    const newPort = await freePort();
    const body = { name: 'Nouveau projet', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: newPort, health: { expect: 'Factice' } };
    const create = (b) => request(port, { method: 'POST', path: '/api/projects', headers: { 'x-servynex-request': '1', 'content-type': 'application/json' }, body: JSON.stringify(b) });
    assert.equal((await request(port, { method: 'POST', path: '/api/projects', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) })).status, 403, 'sans en-tête');
    assert.equal((await create({ ...body, port: ports.ok })).status, 400, 'port déjà pris');
    const res = await create(body);
    assert.equal(res.status, 201);
    assert.equal(JSON.parse(res.body).id, 'nouveau-projet');
    const saved = JSON.parse(fs.readFileSync(path.join(dir, 'projects.json'), 'utf8'));
    assert.ok(saved.projects.some((p) => p.id === 'nouveau-projet'));
    assert.equal((await post('/api/projects/nouveau-projet/start')).status, 200);
    await until(() => state('nouveau-projet').state === 'running');
    await post('/api/projects/nouveau-projet/stop');
    await until(() => state('nouveau-projet').state === 'stopped');
  });

  test('retirer du fichier un projet qui tourne est refusé', async () => {
    const file = path.join(dir, 'projects.json');
    await post('/api/projects/nouveau-projet/start');
    await until(() => state('nouveau-projet').state === 'running');
    const before = fs.readFileSync(file, 'utf8');
    const conf = JSON.parse(before);
    conf.projects = conf.projects.filter((p) => p.id !== 'nouveau-projet');
    fs.writeFileSync(file, JSON.stringify(conf));
    const res = await post('/api/reload');
    assert.equal(res.status, 409);
    assert.match(JSON.parse(res.body).error, /tourne encore/);
    assert.ok(app.manager.project('nouveau-projet'), 'toujours suivi');
    fs.writeFileSync(file, before);
    await post('/api/projects/nouveau-projet/stop');
    await until(() => state('nouveau-projet').state === 'stopped');
    fs.writeFileSync(file, JSON.stringify(conf));
    assert.equal((await post('/api/reload')).status, 200, 'arrêté, il peut être retiré');
  });

  test('arrêter un projet arrête la dépendance lancée pour lui, pas celle lancée à la main', async () => {
    await post('/api/projects/web/start');
    await until(() => state('web').state === 'running' && state('db').state === 'running');
    assert.equal(state('db').dependency, true);
    await post('/api/projects/web/stop');
    await until(() => state('db').state === 'stopped');

    await post('/api/projects/db/start');
    await until(() => state('db').state === 'running');
    await post('/api/projects/web/start');
    await until(() => state('web').state === 'running');
    await post('/api/projects/web/stop');
    await until(() => state('web').state === 'stopped');
    await sleep(500);
    assert.equal(state('db').state, 'running', 'lancée à la main : reste en marche');
    await post('/api/projects/db/stop');
    await until(() => state('db').state === 'stopped');
  });

  const send = (method, p, body) => request(port, { method, path: p, headers: { 'x-servynex-request': '1', ...(body === undefined ? {} : { 'content-type': 'application/json' }) }, body: body === undefined ? undefined : JSON.stringify(body) })
    .then((r) => ({ ...r, json: r.body ? JSON.parse(r.body) : null }));
  const onDisk = () => JSON.parse(fs.readFileSync(path.join(dir, 'projects.json'), 'utf8'));
  const listening = (p) => new Promise((resolve) => {
    const socket = net.connect(p, '127.0.0.1', () => { socket.destroy(); resolve(true); });
    socket.on('error', () => resolve(false));
  });

  test('supprimer : confirmation par le nom, arrêt d’abord, dépendances nettoyées, journal gardé', async () => {
    await post('/api/projects/solo/start');
    await until(() => state('solo').state === 'running' && state('sdb').state === 'running');
    assert.equal((await send('DELETE', '/api/projects/solo')).status, 400, 'sans confirmation');
    assert.equal((await send('DELETE', '/api/projects/solo', { confirm: 'solo' })).status, 400, "l'identifiant ne suffit pas : le nom exact");
    assert.equal((await send('DELETE', '/api/projects/solo', { confirm: 'Autre' })).status, 400);
    assert.ok(onDisk().projects.some((p) => p.id === 'solo'), 'rien de supprimé');
    assert.equal(state('solo').state, 'running', 'rien arrêté');

    const res = await send('DELETE', '/api/projects/solo', { confirm: 'Solo' });
    assert.equal(res.status, 200);
    assert.equal(onDisk().projects.some((p) => p.id === 'solo'), false, 'retiré de projects.json');
    assert.equal((await get('/api/projects')).json.projects.some((p) => p.id === 'solo'), false, 'retiré de la liste');
    assert.deepEqual(onDisk().projects.find((p) => p.id === 'fan').requires, ['db'], 'retiré des dépendances');
    assert.equal(await listening(ports.solo), false, 'arrêté avant la suppression');
    await until(() => state('sdb').state === 'stopped'); // lancée pour lui : arrêtée avec lui
    assert.ok(fs.existsSync(path.join(dir, 'logs', 'solo.log')), 'journal gardé');
    const audit = (await get('/api/history')).json.entries.filter((e) => e.kind === 'audit');
    assert.ok(audit.some((e) => e.id === 'solo' && /Projet supprimé/.test(e.text)));
    assert.equal((await send('DELETE', '/api/projects/solo', { confirm: 'Solo' })).status, 404, 'déjà supprimé');
  });

  test('erreurs dans la langue demandée par la page (Accept-Language)', async () => {
    const del = (lang) => request(port, { method: 'DELETE', path: '/api/projects/crash', headers: { 'x-servynex-request': '1', 'accept-language': lang, 'content-type': 'application/json' }, body: '{}' })
      .then((r) => JSON.parse(r.body).error);
    assert.match(await del('en-US,en;q=0.9'), /Type "Crash" to confirm/);
    assert.match(await del('fr-FR'), /Tapez « Crash » pour confirmer/);
    const unknown = await request(port, { method: 'POST', path: '/api/projects/absent/start', headers: { 'x-servynex-request': '1', 'accept-language': 'en' } });
    assert.equal(unknown.status, 404);
    assert.equal(JSON.parse(unknown.body).error, 'Unknown project');
  });

  test('réordonner : écrit dans projects.json, refuse une liste incomplète', async () => {
    const ids = onDisk().projects.map((p) => p.id);
    const reversed = [...ids].reverse();
    assert.equal((await send('PUT', '/api/projects', { order: reversed })).status, 200);
    assert.deepEqual(onDisk().projects.map((p) => p.id), reversed);
    assert.deepEqual((await get('/api/projects')).json.projects.map((p) => p.id), reversed, 'la liste suit le nouvel ordre');
    assert.equal((await send('PUT', '/api/projects', { order: reversed.slice(1) })).status, 400, 'identifiant manquant');
    assert.equal((await send('PUT', '/api/projects', { order: [...reversed, 'x'] })).status, 400, 'identifiant inconnu');
    assert.equal((await send('PUT', '/api/projects', {})).status, 400);
    assert.deepEqual(onDisk().projects.map((p) => p.id), reversed, 'refusé : fichier inchangé');
    assert.equal((await send('PUT', '/api/projects', { order: ids })).status, 200);
  });

  test('projects.json invalide au démarrage : Servynex démarre avec la dernière configuration valide', async () => {
    const other = path.join(dir, 'invalide');
    fs.mkdirSync(path.join(other, 'runtime'), { recursive: true });
    fs.writeFileSync(path.join(other, 'runtime', 'projects.valid.json'), JSON.stringify({ instance: { port: 1 }, projects: [{ id: 'v', name: 'Valide', type: 'file', cwd: dir, open: 'x.html' }] }));
    fs.writeFileSync(path.join(other, 'projects.json'), '{ "instance": { "port": 1 }, "projects": [ , ] }');
    const broken = createApp({ configFile: path.join(other, 'projects.json'), logsDir: path.join(other, 'logs'), runtimeDir: path.join(other, 'runtime'), task: { enabled: async () => false, set: async () => {} } });
    await new Promise((r) => broken.server.listen(0, '127.0.0.1', r));
    const p = broken.server.address().port;
    const info = JSON.parse((await request(p, { path: '/api/projects' })).body);
    assert.match(info.instance.configError, /illisible/);
    assert.deepEqual(info.projects.map((x) => x.id), ['v']);
    const patch = await request(p, { method: 'PATCH', path: '/api/projects/v', headers: { 'x-servynex-request': '1', 'content-type': 'application/json' }, body: '{"name":"W"}' });
    assert.equal(patch.status, 400);
    assert.match(JSON.parse(patch.body).error, /à corriger d'abord/);
    broken.server.closeAllConnections();
    await new Promise((r) => broken.server.close(r));
  });

  test('redémarrage de Servynex : refusé sans superviseur, demandé sinon', async () => {
    assert.equal((await request(port, { method: 'POST', path: '/api/restart' })).status, 403, 'sans en-tête');
    const res = await post('/api/restart');
    assert.equal(res.status, 409);
    assert.match(JSON.parse(res.body).error, /npm start/);
    assert.equal((await get('/api/projects')).json.instance.restartable, false);

    let asked = 0;
    const other = createApp({ configFile: path.join(dir, 'projects.json'), logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime-restart'), restart: () => asked++ });
    await new Promise((r) => other.server.listen(0, '127.0.0.1', r));
    const otherPort = other.server.address().port;
    const ok = await request(otherPort, { method: 'POST', path: '/api/restart', headers: { 'x-servynex-request': '1' } });
    assert.equal(ok.status, 200);
    await until(() => asked === 1, 2000);
    const info = JSON.parse((await request(otherPort, { path: '/api/projects' })).body);
    assert.equal(info.instance.restartable, true);
    other.server.closeAllConnections();
    await new Promise((r) => other.server.close(r));
  });

  test('démarrage avec Windows et présentation, depuis les Paramètres', async () => {
    let enabled = false;
    const task = { enabled: async () => enabled, set: async (on) => void (enabled = on) };
    const other = createApp({ configFile: path.join(dir, 'projects.json'), logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime-ui'), task });
    await new Promise((r) => other.server.listen(0, '127.0.0.1', r));
    const otherPort = other.server.address().port;
    const call = (method, p, body) => request(otherPort, { method, path: p, headers: { 'x-servynex-request': '1', ...(body ? { 'content-type': 'application/json' } : {}) }, body: body && JSON.stringify(body) });

    assert.equal((await request(otherPort, { path: '/api/autostart' })).status, 403, 'sans en-tête');
    assert.equal(JSON.parse((await call('GET', '/api/autostart')).body).enabled, false);
    assert.equal((await call('PUT', '/api/autostart', { enabled: 'oui' })).status, 400);
    assert.equal(JSON.parse((await call('PUT', '/api/autostart', { enabled: true })).body).enabled, true);
    assert.equal(enabled, true);

    const instanceInfo = async () => JSON.parse((await call('GET', '/api/projects')).body).instance;
    assert.equal((await instanceInfo()).onboarded, false);
    assert.equal((await call('POST', '/api/instance/onboarded')).status, 200);
    assert.equal((await instanceInfo()).onboarded, true);
    assert.match((await instanceInfo()).version, /^\d+\.\d+\.\d+$/);
    assert.equal((await call('POST', '/api/instance/calc')).status, 404);
    other.server.closeAllConnections();
    await new Promise((r) => other.server.close(r));
  });

  test('activité : changements d’état de tous les projets', async () => {
    const { json } = await get('/api/history');
    const ids = new Set(json.entries.map((e) => e.id));
    assert.ok(ids.has('ok') && ids.has('crash'), [...ids].join(', '));
    assert.ok(json.entries.filter((e) => e.kind !== 'audit').every((e) => e.at && e.from && e.to));
    assert.ok(json.entries.some((e) => e.kind === 'audit' && /Projet ajouté/.test(e.text)), 'audit des réglages même sans protection');
  });

  test("flux d'évènements", async () => {
    const events = await new Promise((resolve) => {
      http.get({ host: '127.0.0.1', port, path: '/api/events', headers: { host: `localhost:${port}` } }, (res) => {
        res.setEncoding('utf8');
        res.once('data', (chunk) => {
          res.destroy();
          resolve(chunk);
        });
      });
    });
    assert.match(events, /^event: snapshot\ndata: \[/);
  });
});
