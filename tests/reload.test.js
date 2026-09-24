import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { createApp } from '../server.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYNEX_LANG = 'fr';
setServerLang('fr');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(check, ms = 5000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await check()) return;
    await sleep(50);
  }
  throw new Error('délai dépassé');
}

// Servynex de test qui surveille projects.json toutes les 50 ms.
async function makeApp() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-reload-'));
  fs.mkdirSync(path.join(dir, 'site'));
  fs.writeFileSync(path.join(dir, 'site', 'index.html'), 'ok');
  const file = path.join(dir, 'projects.json');
  const config = {
    instance: { port: 1 },
    projects: [
      { id: 'site', name: 'Site', type: 'static', cwd: path.join(dir, 'site'), port: 39891 },
      { id: 'doc', name: 'Doc', type: 'file', cwd: path.join(dir, 'site'), open: 'index.html' },
    ],
  };
  fs.writeFileSync(file, JSON.stringify(config, null, 2));
  const app = createApp({ configFile: file, logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime'), task: { enabled: async () => false, set: async () => {} }, notify: async () => true, watchMs: 50 });
  await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
  await app.manager.init();
  const port = app.server.address().port;
  const call = (method, p, body) => new Promise((resolve, reject) => {
    const headers = { host: `localhost:${port}`, 'x-servynex-request': '1' };
    const raw = body === undefined ? undefined : JSON.stringify(body);
    if (raw) Object.assign(headers, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(raw) });
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => resolve({ status: res.statusCode, json: JSON.parse(data || 'null') }));
    });
    req.on('error', reject);
    req.end(raw);
  });
  const edit = (change) => {
    const next = JSON.parse(fs.readFileSync(file, 'utf8'));
    change(next);
    fs.writeFileSync(file, JSON.stringify(next, null, 2));
  };
  const names = async () => (await call('GET', '/api/projects')).json.projects.map((p) => p.name);
  const own = () => fs.readFileSync(path.join(dir, 'logs', 'servynex.log'), 'utf8');
  const close = async () => {
    await app.manager.close();
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    fs.rmSync(dir, { recursive: true, force: true });
  };
  return { call, edit, file, names, own, close };
}

describe('relecture automatique de projects.json', () => {
  test('une modification à la main est appliquée seule, et inscrite à l’activité', async () => {
    const s = await makeApp();
    try {
      s.edit((c) => (c.projects[1].name = 'Documentation'));
      await until(async () => (await s.names()).includes('Documentation'));
      assert.match(s.own(), /relu automatiquement/);
      const audit = (await s.call('GET', '/api/history')).json.entries.filter((e) => e.kind === 'audit').map((e) => e.text).join('\n');
      assert.match(audit, /Modifié dans projects\.json : name "Doc" → "Documentation"/);
    } finally {
      await s.close();
    }
  });

  test('fichier invalide : erreur affichée, dernière configuration gardée ; corrigé, appliqué', async () => {
    const s = await makeApp();
    try {
      const good = fs.readFileSync(s.file, 'utf8');
      fs.writeFileSync(s.file, good.replace('"Doc"', '"Doc'));
      await until(async () => (await s.call('GET', '/api/projects')).json.instance.configError);
      assert.deepEqual(await s.names(), ['Site', 'Doc'], 'configuration gardée');
      fs.writeFileSync(s.file, good.replace('"Doc"', '"Doc corrigée"'));
      await until(async () => (await s.names()).includes('Doc corrigée'));
      assert.equal((await s.call('GET', '/api/projects')).json.instance.configError, null);
    } finally {
      await s.close();
    }
  });

  test('les écritures de Servynex lui-même ne sont pas relues', async () => {
    const s = await makeApp();
    try {
      assert.equal((await s.call('PATCH', '/api/projects/doc', { desc: 'depuis la page' })).status, 200);
      await sleep(400);
      assert.doesNotMatch(s.own(), /relu automatiquement/);
      assert.doesNotMatch(s.own(), /Modifié dans projects\.json/);
    } finally {
      await s.close();
    }
  });

  test('retirer à la main un projet en marche : refusé, signalé', async () => {
    const s = await makeApp();
    try {
      await s.call('POST', '/api/projects/site/start');
      await until(async () => (await s.call('GET', '/api/projects')).json.projects[0].state === 'running');
      s.edit((c) => c.projects.shift());
      await until(async () => (await s.call('GET', '/api/projects')).json.instance.configError);
      const { instance, projects } = (await s.call('GET', '/api/projects')).json;
      assert.match(instance.configError, /Site/);
      assert.equal(projects[0].state, 'running', 'toujours là, toujours en marche');
      await s.call('POST', '/api/projects/site/stop');
      // Arrêté : le retrait est appliqué sans autre modification du fichier.
      await until(async () => !(await s.names()).includes('Site'));
      assert.equal((await s.call('GET', '/api/projects')).json.instance.configError, null);
      assert.equal(s.own().match(/pas appliqué/g).length, 1, 'signalé une seule fois');
    } finally {
      await s.close();
    }
  });
});
