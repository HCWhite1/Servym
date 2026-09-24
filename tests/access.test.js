import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { bindHost, createApp } from '../server.js';
import { validate } from '../src/config.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYNEX_LANG = 'fr';
setServerLang('fr');

function request(port, { method = 'GET', path: p = '/', cookie, body, host } = {}) {
  return new Promise((resolve, reject) => {
    const headers = { host: host ?? `localhost:${port}` };
    if (method !== 'GET') headers['x-servynex-request'] = '1';
    if (body !== undefined) Object.assign(headers, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(JSON.stringify(body)) });
    if (cookie) headers.cookie = cookie;
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => {
        let json = null;
        try {
          json = JSON.parse(data);
        } catch {}
        const set = (res.headers['set-cookie'] ?? []).map((c) => c.split(';')[0]).find((c) => c.startsWith('servynex_session='));
        resolve({ status: res.statusCode, json, cookie: set ?? null });
      });
    });
    req.on('error', reject);
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
}

// Servynex de test : options de createApp en plus (visiteur distant, noms de la machine).
async function makeApp(options = {}, instance = { port: 1 }) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-access-'));
  fs.mkdirSync(path.join(dir, 'site'));
  fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify({ instance, projects: [{ id: 'doc', name: 'Doc', type: 'file', cwd: path.join(dir, 'site'), open: 'index.html' }] }));
  const app = createApp({
    configFile: path.join(dir, 'projects.json'),
    logsDir: path.join(dir, 'logs'),
    runtimeDir: path.join(dir, 'runtime'),
    task: { enabled: async () => false, set: async () => {} },
    ...options,
  });
  await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
  const port = app.server.address().port;
  const close = async () => {
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    await app.manager.close();
    fs.rmSync(dir, { recursive: true, force: true });
  };
  return { app, port, dir, close, call: (method, p, opts = {}) => request(port, { method, path: p, ...opts }) };
}

describe('accès depuis le réseau privé', () => {
  test('instance.access : « local » ou « lan »', () => {
    const conf = (access) => ({ instance: { port: 9000, access }, projects: [] });
    assert.doesNotThrow(() => validate(conf('local')));
    assert.doesNotThrow(() => validate(conf('lan')));
    assert.throws(() => validate(conf('internet')), /instance\.access/);
  });

  test('adresse d’écoute : le réseau seulement avec la protection par mot de passe', () => {
    assert.equal(bindHost({ access: 'lan' }, true), '0.0.0.0');
    assert.equal(bindHost({ access: 'lan' }, false), '127.0.0.1', 'sans protection : ce PC seulement');
    assert.equal(bindHost({ access: 'local' }, true), '127.0.0.1');
    assert.equal(bindHost({}, true), '127.0.0.1', 'par défaut : ce PC seulement');
  });

  describe('réglage depuis l’interface', () => {
    let s;
    let admin;
    before(async () => {
      s = await makeApp();
    });
    after(() => s.close());

    test('refusé tant que la protection n’est pas active', async () => {
      const res = await s.call('PUT', '/api/instance/access', { body: { access: 'lan', password: 'x' } });
      assert.equal(res.status, 409);
      assert.match(res.json.error, /protection/);
    });

    test('administrateur et mot de passe ; appliqué au prochain démarrage', async () => {
      admin = (await s.call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'mot de passe 1' } })).cookie;
      const { token } = (await s.call('POST', '/api/auth/invites', { cookie: admin, body: { permissions: ['instance'] } })).json;
      const bob = (await s.call('POST', '/api/auth/invite', { body: { token, username: 'bob', password: 'mot de passe 2' } })).cookie;
      assert.equal((await s.call('PUT', '/api/instance/access', { cookie: bob, body: { access: 'lan', password: 'mot de passe 2' } })).status, 403, 'administrateur seulement');
      assert.equal((await s.call('PUT', '/api/instance/access', { cookie: admin, body: { access: 'lan', password: 'faux' } })).status, 403);
      assert.equal((await s.call('PUT', '/api/instance/access', { cookie: admin, body: { access: 'partout', password: 'mot de passe 1' } })).status, 400);
      const res = await s.call('PUT', '/api/instance/access', { cookie: admin, body: { access: 'lan', password: 'mot de passe 1' } });
      assert.equal(res.status, 200);
      assert.equal(JSON.parse(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8')).instance.access, 'lan');
      const { instance } = (await s.call('GET', '/api/projects', { cookie: admin })).json;
      assert.equal(instance.access, 'lan', 'réglage enregistré');
      assert.equal(instance.listening, 'local', 'toujours sur ce PC jusqu’au redémarrage');
      assert.equal(instance.viewerLocal, true);
      const audit = (await s.call('GET', '/api/history', { cookie: admin })).json.entries.filter((e) => e.kind === 'audit');
      assert.ok(audit.some((e) => /réseau/i.test(e.text)), 'inscrit au journal d’audit');
    });

    test('la protection ne peut pas être retirée tant que l’accès réseau est ouvert', async () => {
      const res = await s.call('POST', '/api/auth/disable', { cookie: admin, body: { password: 'mot de passe 1' } });
      assert.equal(res.status, 409);
      assert.equal((await s.call('PUT', '/api/instance/access', { cookie: admin, body: { access: 'local' } })).status, 200, 'revenir à ce PC seulement : sans mot de passe');
      assert.equal((await s.call('POST', '/api/auth/disable', { cookie: admin, body: { password: 'mot de passe 1' } })).status, 200);
    });
  });

  test('adresses acceptées : celles de la machine en accès réseau, jamais un autre nom', async () => {
    const names = () => ['192.168.1.50', 'mon-pc'];
    const lan = await makeApp({ hostnames: names, listening: 'lan' }, { port: 1, access: 'lan' });
    try {
      assert.equal((await lan.call('GET', '/api/auth/status', { host: `192.168.1.50:${lan.port}` })).status, 200);
      assert.equal((await lan.call('GET', '/api/auth/status', { host: `MON-PC:${lan.port}` })).status, 200);
      assert.equal((await lan.call('GET', '/api/auth/status', { host: `evil.example:${lan.port}` })).status, 421, 'DNS rebinding');
    } finally {
      await lan.close();
    }
    const local = await makeApp({ hostnames: names });
    try {
      assert.equal((await local.call('GET', '/api/auth/status', { host: `192.168.1.50:${local.port}` })).status, 421, 'ce PC seulement : localhost uniquement');
    } finally {
      await local.close();
    }
  });

  test('visiteur distant : rien ne s’ouvre sur la machine de Servynex', async () => {
    const s = await makeApp({ isLocalViewer: () => false });
    try {
      assert.equal((await s.call('GET', '/api/projects')).json.instance.viewerLocal, false);
      for (const action of ['folder', 'code', 'terminal', 'open']) {
        const res = await s.call('POST', `/api/projects/doc/${action}`);
        assert.equal(res.status, 403, action);
        assert.match(res.json.error, /machine/);
      }
      for (const action of ['folder', 'config', 'log']) assert.equal((await s.call('POST', `/api/instance/${action}`)).status, 403, `instance/${action}`);
    } finally {
      await s.close();
    }
  });
});
