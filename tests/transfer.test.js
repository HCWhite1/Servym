import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { createApp } from '../server.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYM_LANG = 'fr';
setServerLang('fr');

function request(port, { method = 'GET', path: p = '/', cookie, body, header = true } = {}) {
  return new Promise((resolve, reject) => {
    const headers = { host: `localhost:${port}` };
    if (header) headers['x-servym-request'] = '1';
    const raw = body === undefined ? undefined : JSON.stringify(body);
    if (raw !== undefined) Object.assign(headers, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(raw) });
    if (cookie) headers.cookie = cookie;
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => {
        let json = null;
        try {
          json = JSON.parse(data);
        } catch {}
        const set = (res.headers['set-cookie'] ?? []).map((c) => c.split(';')[0]).find((c) => c.startsWith('servym_session='));
        resolve({ status: res.statusCode, headers: res.headers, body: data, json, cookie: set ?? null });
      });
    });
    req.on('error', reject);
    req.end(raw);
  });
}

// Servym de test : un site statique (démarrable sans processus), une page, une API.
async function makeApp() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-transfer-'));
  fs.mkdirSync(path.join(dir, 'site'));
  fs.writeFileSync(path.join(dir, 'site', 'index.html'), 'ok');
  const site = path.join(dir, 'site');
  const config = {
    instance: { port: 1, name: 'Maison' },
    projects: [
      { id: 'site', name: 'Site', type: 'static', cwd: site, port: 39871, desc: 'Avant' },
      { id: 'doc', name: 'Doc', type: 'file', cwd: site, open: 'index.html' },
      { id: 'api', name: 'API', cwd: site, cmd: 'node', args: ['server.js'], port: 39872, hostControl: 'env', env: { TOKEN: 'secret' } },
    ],
  };
  fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify(config, null, 2));
  const app = createApp({
    configFile: path.join(dir, 'projects.json'),
    logsDir: path.join(dir, 'logs'),
    runtimeDir: path.join(dir, 'runtime'),
    task: { enabled: async () => false, set: async () => {} },
    notify: async () => true,
  });
  await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
  await app.manager.init();
  const port = app.server.address().port;
  const close = async () => {
    await app.manager.close();
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    fs.rmSync(dir, { recursive: true, force: true });
  };
  const disk = () => JSON.parse(fs.readFileSync(path.join(dir, 'projects.json'), 'utf8'));
  return { app, dir, site, config, close, disk, call: (method, p, opts = {}) => request(port, { method, path: p, ...opts }) };
}

describe('export et import de la configuration', () => {
  test('export : tout projects.json, en fichier à télécharger', async () => {
    const s = await makeApp();
    try {
      assert.equal((await s.call('GET', '/api/config/export', { header: false })).status, 403, 'sans en-tête');
      const res = await s.call('GET', '/api/config/export');
      assert.equal(res.status, 200);
      assert.match(res.headers['content-type'], /application\/json/);
      assert.match(res.headers['content-disposition'], /^attachment; filename="servym-projects-\d{4}-\d{2}-\d{2}\.json"$/);
      assert.deepEqual(res.json, s.disk(), 'le fichier tel qu’il est, variables d’environnement comprises');
    } finally {
      await s.close();
    }
  });

  test('aperçu : ajouts, retraits, modifications, sans rien écrire', async () => {
    const s = await makeApp();
    try {
      const before = fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8');
      const projects = [
        { ...s.config.projects[0], desc: 'Après' },
        s.config.projects[2],
        { id: 'blog', name: 'Blog', type: 'file', cwd: s.site, open: 'index.html' },
      ];
      const res = await s.call('POST', '/api/config/import', { body: { config: { instance: { port: 1 }, projects } } });
      assert.equal(res.status, 200);
      assert.equal(res.json.applied, false);
      assert.deepEqual(res.json.added, [{ id: 'blog', name: 'Blog' }]);
      assert.deepEqual(res.json.removed, [{ id: 'doc', name: 'Doc' }]);
      assert.deepEqual(res.json.changed, [{ id: 'site', name: 'Site', changes: 'desc "Avant" → "Après"' }]);
      assert.equal(res.json.unchanged, 1);
      assert.equal(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8'), before, 'fichier intact');
    } finally {
      await s.close();
    }
  });

  test('appliqué : les projets remplacés, la section instance de cette machine gardée', async () => {
    const s = await makeApp();
    try {
      const projects = [s.config.projects[1], { id: 'blog', name: 'Blog', type: 'file', cwd: s.site, open: 'index.html' }];
      const res = await s.call('POST', '/api/config/import', { body: { config: { instance: { port: 4242, access: 'lan', name: 'Autre PC' }, projects }, apply: true } });
      assert.equal(res.status, 200);
      assert.equal(res.json.applied, true);
      assert.deepEqual(s.disk(), { instance: s.config.instance, projects });
      const live = (await s.call('GET', '/api/projects')).json;
      assert.deepEqual(live.projects.map((p) => p.id), ['doc', 'blog']);
      const audit = (await s.call('GET', '/api/history')).json.entries.filter((e) => e.kind === 'audit').map((e) => e.text).join('\n');
      assert.match(audit, /import/i);
      assert.match(audit, /Blog/);
    } finally {
      await s.close();
    }
  });

  test('refusé : fichier invalide, ou projet en marche retiré', async () => {
    const s = await makeApp();
    try {
      const before = fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8');
      assert.equal((await s.call('POST', '/api/config/import', { body: { config: { projects: 'non' }, apply: true } })).status, 400);
      assert.equal((await s.call('POST', '/api/config/import', { body: { apply: true } })).status, 400);
      const twice = await s.call('POST', '/api/config/import', { body: { config: { projects: [s.config.projects[1], s.config.projects[1]] }, apply: true } });
      assert.equal(twice.status, 400);
      assert.match(twice.json.error, /doc/i);

      await s.call('POST', '/api/projects/site/start');
      const res = await s.call('POST', '/api/config/import', { body: { config: { projects: [s.config.projects[1]] }, apply: true } });
      assert.equal(res.status, 409);
      assert.match(res.json.error, /Site/);
      assert.equal(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8'), before, 'fichier intact');
      await s.call('POST', '/api/projects/site/stop');
    } finally {
      await s.close();
    }
  });

  test('une grande configuration passe', async () => {
    const s = await makeApp();
    try {
      const projects = Array.from({ length: 150 }, (_, i) => ({ id: `page-${i}`, name: `Page ${i}`, type: 'file', cwd: s.site, open: 'index.html' }));
      assert.ok(JSON.stringify(projects).length > 16_000);
      const res = await s.call('POST', '/api/config/import', { body: { config: { projects } } });
      assert.equal(res.status, 200);
      assert.equal(res.json.added.length, 150);
    } finally {
      await s.close();
    }
  });

  test('permission « commandes » pour exporter comme pour importer', async () => {
    const s = await makeApp();
    try {
      const admin = (await s.call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'mot de passe 1' } })).cookie;
      const join = async (permissions, username) => {
        const { token } = (await s.call('POST', '/api/auth/invites', { cookie: admin, body: { permissions } })).json;
        return (await s.call('POST', '/api/auth/invite', { body: { token, username, password: 'mot de passe 2' } })).cookie;
      };
      const editor = await join(['edit'], 'ed');
      assert.equal((await s.call('GET', '/api/config/export', { cookie: editor })).status, 403);
      assert.equal((await s.call('POST', '/api/config/import', { cookie: editor, body: { config: s.config } })).status, 403);
      const commands = await join(['commands'], 'cmd');
      assert.equal((await s.call('GET', '/api/config/export', { cookie: commands })).status, 200);
      assert.equal((await s.call('POST', '/api/config/import', { cookie: commands, body: { config: s.config } })).status, 200);
    } finally {
      await s.close();
    }
  });
});
