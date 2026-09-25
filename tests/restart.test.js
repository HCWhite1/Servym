import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { createApp } from '../server.js';
import { patchProject, validate } from '../src/config.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYM_LANG = 'fr';
setServerLang('fr');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(check, ms = 20000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await check()) return;
    await sleep(100);
  }
  throw new Error('délai dépassé');
}
function freePort() {
  return new Promise((resolve) => {
    const srv = net.createServer().listen(0, '127.0.0.1', () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}
function request(port, method, p) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: { host: `localhost:${port}`, 'x-servym-request': '1' } }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => resolve({ status: res.statusCode, json: JSON.parse(data || 'null') }));
    });
    req.on('error', reject);
    req.end();
  });
}

// Servym de test. « api » répond, puis plante 1,5 s après chaque lancement tant
// qu'il n'a pas planté CRASHES fois (compté dans runs.txt) ; « dead » plante
// avant de répondre (échec au lancement).
async function makeApp({ crashes = 99, restartOnCrash = true, ...options } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-restart-'));
  const app = path.join(dir, 'app');
  fs.mkdirSync(app);
  fs.writeFileSync(path.join(app, 'server.js'), `
const fs = require('node:fs');
const runs = Number(fs.existsSync('runs.txt') ? fs.readFileSync('runs.txt', 'utf8') : 0) + 1;
fs.writeFileSync('runs.txt', String(runs));
require('node:http').createServer((q, s) => s.end('<title>Factice</title>')).listen(process.env.PORT, process.env.HOST);
if (runs <= Number(process.env.CRASHES)) setTimeout(() => { console.error('Error: plantage ' + runs); process.exit(3); }, 1500);
`);
  fs.writeFileSync(path.join(app, 'dead.js'), `console.error('Error: base introuvable'); process.exit(3);`);
  fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify({ instance: { port: 1, notify: true }, projects: [
    { id: 'api', name: 'Mon API', cwd: app, cmd: 'node', args: ['server.js'], port: await freePort(), hostControl: 'env', env: { CRASHES: String(crashes) }, health: { expect: 'Factice' }, ...(restartOnCrash ? { restartOnCrash } : {}) },
    { id: 'dead', name: 'Morte', cwd: app, cmd: 'node', args: ['dead.js'], port: await freePort(), hostControl: 'env', restartOnCrash: true },
  ] }));
  const sent = [];
  const servym = createApp({
    configFile: path.join(dir, 'projects.json'),
    logsDir: path.join(dir, 'logs'),
    runtimeDir: path.join(dir, 'runtime'),
    task: { enabled: async () => false, set: async () => {} },
    notify: async (n) => sent.push(n),
    watchMs: 0,
    tickMs: 500,
    runningTickMs: 500,
    startTimeoutMs: 8000,
    restartDelaysMs: [100, 200, 300],
    ...options,
  });
  await new Promise((r) => servym.server.listen(0, '127.0.0.1', r));
  await servym.manager.init();
  const port = servym.server.address().port;
  const call = (method, p) => request(port, method, p);
  const close = async () => {
    if (servym.manager.state('api').managed) await servym.manager.stop('api').catch(() => {});
    await servym.manager.close();
    servym.server.closeAllConnections();
    await new Promise((r) => servym.server.close(r));
    // Sous Windows, le dossier peut rester verrouillé un instant après l'arrêt du projet.
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
  };
  const runs = () => Number(fs.readFileSync(path.join(app, 'runs.txt'), 'utf8'));
  const history = async (id) => (await call('GET', `/api/projects/${id}/history`)).json.entries;
  return { manager: servym.manager, call, close, sent, runs, history, state: (id) => servym.manager.state(id) };
}

describe('relancer s’il plante : configuration', () => {
  const base = { instance: { port: 9000 }, projects: [{ id: 'a', name: 'A', cwd: 'C:/a', cmd: 'node', args: [], port: 3001, hostControl: 'env' }] };

  test('booléen, pour un programme seulement', () => {
    assert.equal(patchProject(base, 'a', { restartOnCrash: true }).projects[0].restartOnCrash, true);
    const conf = structuredClone(base);
    conf.projects[0].restartOnCrash = 'oui';
    assert.throws(() => validate(conf), /restartOnCrash/);
    conf.projects[0] = { id: 'a', name: 'A', cwd: 'C:/a', type: 'static', port: 3001, restartOnCrash: true };
    assert.throws(() => validate(conf), /restartOnCrash/);
  });

  test('passer en site statique ou en fichier retire le réglage', () => {
    const on = patchProject(base, 'a', { restartOnCrash: true });
    assert.equal(patchProject(on, 'a', { type: 'static' }).projects[0].restartOnCrash, undefined);
    assert.equal(patchProject(on, 'a', { type: 'file', open: 'index.html' }).projects[0].restartOnCrash, undefined);
  });
});

describe('relancer s’il plante', () => {
  test('désactivé par défaut : un plantage laisse le projet en échec', async () => {
    const s = await makeApp({ restartOnCrash: false, crashes: 1 });
    try {
      assert.equal((await s.call('GET', '/api/projects')).json.projects.find((p) => p.id === 'api').restartOnCrash, false);
      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').state === 'running');
      await until(() => s.state('api').state === 'failed');
      await sleep(800);
      assert.equal(s.state('api').state, 'failed');
      assert.equal(s.state('api').autoRestart, null);
      assert.equal(s.runs(), 1);
      assert.match(s.sent[0]?.title ?? '', /Mon API s'est arrêté/);
    } finally {
      await s.close();
    }
  });

  test('un plantage est relancé, sans notification, et inscrit à l’historique', async () => {
    const s = await makeApp({ crashes: 1 });
    try {
      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').state === 'running');
      await until(() => s.state('api').state === 'failed');
      await until(() => s.runs() === 2 && s.state('api').state === 'running');
      assert.equal(s.state('api').managed, true);
      await sleep(2000);
      assert.equal(s.state('api').state, 'running', 'reste en ligne');
      assert.equal(s.sent.length, 0, 'réparé tout seul : pas de notification');
      const details = (await s.history('api')).map((e) => `${e.from} → ${e.to} · ${e.detail}`);
      assert.ok(details.some((d) => /^running → failed · .*Relance dans 1 s \(1\/3\)/.test(d)), details.join(' | '));
      assert.ok(details.some((d) => /^failed → starting · Relancé automatiquement \(1\/3\)/.test(d)), details.join(' | '));
    } finally {
      await s.close();
    }
  });

  test('plante en boucle : abandon après 3 relances, notifié', async () => {
    const s = await makeApp();
    try {
      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').autoRestart === 'gaveUp', 30000);
      assert.equal(s.state('api').state, 'failed');
      assert.equal(s.runs(), 4, 'lancé, puis relancé 3 fois');
      await sleep(800);
      assert.equal(s.runs(), 4, 'plus de relance');
      assert.equal(s.sent.length, 1);
      assert.equal(s.sent[0].title, 'Mon API plante en boucle, relance abandonnée');
      assert.match(s.sent[0].body, /plantage 4/);
      const entries = await s.history('api');
      assert.match(entries[0].detail, /Relance abandonnée : 3 relances en 5 min/);
      for (const n of [1, 2, 3]) assert.ok(entries.some((e) => e.detail.includes(`Relancé automatiquement (${n}/3)`)), `relance ${n}/3`);

      // Relancé à la main : le compte repart de zéro.
      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').state === 'failed' && s.state('api').autoRestart === 'scheduled');
      await until(() => s.runs() === 6);
    } finally {
      await s.close();
    }
  });

  test('pas après un échec au lancement', async () => {
    const s = await makeApp();
    try {
      await s.call('POST', '/api/projects/dead/start');
      await until(() => s.state('dead').state === 'failed');
      await sleep(800);
      assert.equal(s.state('dead').state, 'failed');
      assert.equal(s.state('dead').autoRestart, null);
      assert.match(s.sent[0]?.title ?? '', /n'a pas pu démarrer/);
    } finally {
      await s.close();
    }
  });

  test('arrêté à la main : pas relancé ; relance prévue annulée par « Arrêter » ou en retirant le réglage', async () => {
    const s = await makeApp({ restartDelaysMs: [1500, 1500, 1500] });
    try {
      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').state === 'running');
      await s.call('POST', '/api/projects/api/stop');
      await until(() => s.state('api').state === 'stopped');

      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').autoRestart === 'scheduled');
      assert.equal((await s.call('POST', '/api/projects/api/stop')).status, 200);
      assert.equal(s.state('api').state, 'failed');
      assert.equal(s.state('api').autoRestart, null);
      assert.equal(s.state('api').detail, null);

      await s.call('POST', '/api/projects/api/start');
      await until(() => s.state('api').autoRestart === 'scheduled');
      const config = structuredClone(s.manager.config);
      delete config.projects[0].restartOnCrash;
      s.manager.setConfig(config);
      assert.equal(s.state('api').autoRestart, null);

      const before = s.runs();
      await sleep(2000);
      assert.equal(s.runs(), before, 'aucune relance');
      assert.equal(s.state('api').state, 'failed');
    } finally {
      await s.close();
    }
  });
});
