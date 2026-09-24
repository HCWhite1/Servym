import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { createApp } from '../server.js';
import { duplicateProject } from '../src/config.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYNEX_LANG = 'fr';
setServerLang('fr');

const base = {
  instance: { port: 9000 },
  projects: [
    { id: 'api', name: 'API', cwd: 'C:/api', cmd: 'node', args: ['server.js'], port: 3001, hostControl: 'env', env: { TOKEN: 'x' }, autostart: true, expose: true, fixedPort: 'codé en dur', tags: ['Node'] },
    { id: 'doc', name: 'Doc', type: 'file', cwd: 'C:/doc', open: 'index.html' },
    { id: 'web', name: 'Web', cwd: 'C:/web', cmd: 'node', args: [], port: 3002, hostControl: 'env', requires: ['api'] },
  ],
};

describe('dupliquer un projet', () => {
  test('copie juste après l’original : nouvel identifiant, nom et port', () => {
    const { config, id } = duplicateProject(base, 'api', { name: 'API (copie)', port: 3100 });
    assert.equal(id, 'api-2');
    assert.deepEqual(config.projects.map((p) => p.id), ['api', 'api-2', 'doc', 'web']);
    const copy = config.projects[1];
    assert.equal(copy.name, 'API (copie)');
    assert.equal(copy.port, 3100);
    assert.deepEqual([copy.cmd, copy.args, copy.cwd, copy.env, copy.tags], ['node', ['server.js'], 'C:/api', { TOKEN: 'x' }, ['Node']]);
    assert.equal(copy.autostart, undefined, 'pas lancé au démarrage du PC sans le demander');
    assert.equal(copy.expose, undefined, 'pas exposé au réseau sans le demander');
    assert.equal(copy.fixedPort, undefined, 'le port fixe est celui de l’original');
    assert.equal(base.projects.length, 3, 'original intact');
    assert.equal(duplicateProject(config, 'api', { name: 'API (copie)', port: 3101 }).id, 'api-3');
  });

  test('dépendances gardées ; page locale sans port', () => {
    assert.deepEqual(duplicateProject(base, 'web', { name: 'Web 2', port: 3200 }).config.projects[3].requires, ['api']);
    const { config } = duplicateProject(base, 'doc', { name: 'Doc (copie)' });
    assert.equal(config.projects[2].port, undefined);
    assert.equal(config.projects[2].open, 'index.html');
  });

  test('projet inconnu', () => {
    assert.throws(() => duplicateProject(base, 'zz', { name: 'x', port: 1 }), /inconnu/i);
  });
});

describe('dupliquer : API', () => {
  async function makeApp() {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-dup-'));
    const config = {
      instance: { port: 1 },
      projects: [
        { id: 'api', name: 'API', cwd: dir, cmd: 'node', args: ['server.js'], port: 39881, hostControl: 'env' },
        { id: 'doc', name: 'Doc', type: 'file', cwd: dir, open: 'index.html' },
      ],
    };
    fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify(config));
    const app = createApp({ configFile: path.join(dir, 'projects.json'), logsDir: path.join(dir, 'logs'), runtimeDir: path.join(dir, 'runtime'), task: { enabled: async () => false, set: async () => {} }, notify: async () => true });
    await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
    const port = app.server.address().port;
    const call = (method, p, { cookie, body } = {}) => new Promise((resolve, reject) => {
      const headers = { host: `localhost:${port}`, 'x-servynex-request': '1' };
      const raw = body === undefined ? undefined : JSON.stringify(body);
      if (raw) Object.assign(headers, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(raw) });
      if (cookie) headers.cookie = cookie;
      const req = http.request({ host: '127.0.0.1', port, method, path: p, headers }, (res) => {
        let data = '';
        res.on('data', (c) => (data += c));
        res.on('end', () => {
          const set = (res.headers['set-cookie'] ?? []).map((c) => c.split(';')[0]).find((c) => c.startsWith('servynex_session='));
          resolve({ status: res.statusCode, json: JSON.parse(data || 'null'), cookie: set ?? null });
        });
      });
      req.on('error', reject);
      req.end(raw);
    });
    const close = async () => {
      await app.manager.close();
      app.server.closeAllConnections();
      await new Promise((r) => app.server.close(r));
      fs.rmSync(dir, { recursive: true, force: true });
    };
    return { call, close, dir };
  }

  test('copie créée, avec un port libre, inscrite à l’activité', async () => {
    const s = await makeApp();
    try {
      const res = await s.call('POST', '/api/projects/api/duplicate');
      assert.equal(res.status, 201);
      assert.equal(res.json.id, 'api-2');
      assert.equal(res.json.name, 'API (copie)');
      assert.ok(res.json.port > 39881, 'port suivant libre');
      const file = JSON.parse(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8'));
      assert.deepEqual(file.projects.map((p) => p.id), ['api', 'api-2', 'doc']);
      const audit = (await s.call('GET', '/api/history')).json.entries.filter((e) => e.kind === 'audit').map((e) => e.text).join('\n');
      assert.match(audit, /Dupliqué depuis API/);
      assert.equal((await s.call('POST', '/api/projects/doc/duplicate')).json.port, undefined);
      assert.equal((await s.call('POST', '/api/projects/zz/duplicate')).status, 404);
    } finally {
      await s.close();
    }
  });

  test('permission « commandes »', async () => {
    const s = await makeApp();
    try {
      const admin = (await s.call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'mot de passe 1' } })).cookie;
      const join = async (permissions, username) => {
        const { token } = (await s.call('POST', '/api/auth/invites', { cookie: admin, body: { permissions } })).json;
        return (await s.call('POST', '/api/auth/invite', { body: { token, username, password: 'mot de passe 2' } })).cookie;
      };
      assert.equal((await s.call('POST', '/api/projects/api/duplicate', { cookie: await join(['edit'], 'ed') })).status, 403);
      assert.equal((await s.call('POST', '/api/projects/api/duplicate', { cookie: await join(['commands'], 'cmd') })).status, 201);
    } finally {
      await s.close();
    }
  });
});
