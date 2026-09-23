import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { createResetCode } from '../src/auth.js';
import { createApp } from '../server.js';

function request(port, { method = 'GET', path: p = '/', cookie, body, headers = {} } = {}) {
  return new Promise((resolve, reject) => {
    const h = { host: `localhost:${port}`, ...headers };
    if (method !== 'GET') h['x-servynex-request'] = '1';
    if (body !== undefined) Object.assign(h, { 'content-type': 'application/json', 'content-length': Buffer.byteLength(JSON.stringify(body)) });
    if (cookie) h.cookie = cookie;
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers: h }, (res) => {
      let data = '';
      res.on('data', (c) => (data += c));
      res.on('end', () => {
        let json = null;
        try {
          json = JSON.parse(data);
        } catch {}
        const set = res.headers['set-cookie']?.[0];
        resolve({ status: res.statusCode, json, cookie: set ? set.split(';')[0] : null, setCookie: set });
      });
    });
    req.on('error', reject);
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
}

describe('authentification', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-auth-'));
  let app;
  let port;
  let now = 1_800_000_000_000;
  let admin; // cookie de l'administrateur
  let recovery;
  const call = (method, p, opts = {}) => request(port, { method, path: p, ...opts });

  before(async () => {
    const config = { instance: { port: 1 }, projects: [
      { id: 'site', name: 'Site', cwd: dir, type: 'file', open: 'index.html' },
      { id: 'extra', name: 'Extra', cwd: dir, type: 'file', open: 'index.html' },
    ] };
    fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify(config));
    app = createApp({
      configFile: path.join(dir, 'projects.json'),
      logsDir: path.join(dir, 'logs'),
      runtimeDir: path.join(dir, 'runtime'),
      task: { enabled: async () => false, set: async () => {} },
      clock: () => now,
    });
    await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
    port = app.server.address().port;
  });

  after(async () => {
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    await app.manager.close();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('désactivée par défaut : tout est ouvert', async () => {
    assert.equal((await call('GET', '/api/projects')).status, 200);
    const { json } = await call('GET', '/api/auth/status');
    assert.equal(json.enabled, false);
    assert.equal(json.hasAccounts, false);
  });

  test('activation : compte administrateur, clé de secours, session', async () => {
    assert.equal((await call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'court' } })).status, 400);
    assert.equal((await call('POST', '/api/auth/setup', { body: { username: 'A B', password: 'assez long' } })).status, 400);
    const res = await call('POST', '/api/auth/setup', { body: { username: 'Alice', name: 'Alice M.', password: 'mot de passe 1' } });
    assert.equal(res.status, 201);
    assert.match(res.json.recovery, /^([A-Z2-9]{4}-){4}[A-Z2-9]{4}$/);
    assert.match(res.setCookie, /^servynex_session=[^;]+; .*HttpOnly; SameSite=Strict/);
    assert.equal(res.json.user.username, 'alice');
    assert.equal(res.json.user.role, 'admin');
    admin = res.cookie;
    recovery = res.json.recovery;
    const stored = fs.readFileSync(path.join(dir, 'runtime', 'auth.json'), 'utf8');
    assert.ok(!stored.includes('mot de passe 1') && !stored.includes(recovery), 'rien en clair');
    assert.equal((await call('POST', '/api/auth/setup', { body: { username: 'autre', password: 'mot de passe 2' } })).status, 409);
  });

  test('sans session : seule la page de connexion répond', async () => {
    const res = await call('GET', '/api/projects');
    assert.equal(res.status, 401);
    assert.equal(res.json.auth, 'login');
    assert.equal((await call('GET', '/api/events')).status, 401);
    assert.equal((await call('POST', '/api/projects/site/start')).status, 401);
    assert.equal((await call('GET', '/')).status, 200, "l'interface elle-même reste servie");
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).status, 200);
  });

  test('connexion, erreurs et limite des essais', async () => {
    assert.equal((await call('POST', '/api/auth/login', { body: { username: 'alice', password: 'faux' } })).status, 401);
    const ok = await call('POST', '/api/auth/login', { body: { username: 'ALICE', password: 'mot de passe 1', remember: true } });
    assert.equal(ok.status, 200);
    assert.match(ok.setCookie, /Max-Age=2592000/, 'rester connecté : 30 jours');
    for (let i = 0; i < 5; i++) await call('POST', '/api/auth/login', { body: { username: 'zoe', password: 'faux' } });
    const blocked = await call('POST', '/api/auth/login', { body: { username: 'zoe', password: 'faux' } });
    assert.equal(blocked.status, 429);
    assert.match(blocked.json.error, /Trop d’essais/);
  });

  test('invitations et permissions', async () => {
    const invite = async (permissions) => (await call('POST', '/api/auth/invites', { cookie: admin, body: { permissions } })).json.token;
    const join = async (token, username) => call('POST', '/api/auth/invite', { body: { token, username, password: 'mot de passe 3' } });

    const t1 = await invite(['operate']);
    assert.equal((await call('GET', `/api/auth/invite/${t1}`)).json.role, 'operator');
    const op = (await join(t1, 'op')).cookie;
    assert.equal((await join(t1, 'op2')).status, 410, 'une invitation ne sert qu’une fois');
    assert.equal((await call('POST', '/api/projects/site/start', { cookie: op })).status, 409, 'autorisé, mais un fichier ne se lance pas');
    assert.equal((await call('PATCH', '/api/projects/site', { cookie: op, body: { desc: 'x' } })).status, 403);
    assert.equal((await call('GET', '/api/auth/users', { cookie: op })).status, 403);

    const ed = (await join(await invite(['edit']), 'ed')).cookie;
    assert.equal((await call('PATCH', '/api/projects/site', { cookie: ed, body: { desc: 'modifié' } })).status, 200);
    assert.equal((await call('PATCH', '/api/projects/site', { cookie: ed, body: { cwd: 'C:/Windows' } })).status, 403, 'dossier : permission Commandes');
    assert.equal((await call('POST', '/api/projects', { cookie: ed, body: { name: 'X', cwd: dir, type: 'file', open: 'a.html' } })).status, 403);
    assert.equal((await call('POST', '/api/restart', { cookie: ed })).status, 403, 'redémarrer Servynex : permission Servynex');

    const ro = (await join(await invite([]), 'lecteur')).cookie;
    assert.equal((await call('GET', '/api/projects', { cookie: ro })).status, 200);
    assert.equal((await call('POST', '/api/projects/site/start', { cookie: ro })).status, 403);
  });

  test("journal d'audit : qui a fait quoi", async () => {
    const { json } = await call('GET', '/api/history', { cookie: admin });
    const audit = json.entries.filter((e) => e.kind === 'audit');
    assert.ok(audit.some((e) => e.user === 'ed' && e.id === 'site' && /Réglages modifiés : desc/.test(e.text)));
    assert.ok(audit.some((e) => e.user === 'alice' && /Protection par mot de passe activée/.test(e.text)));
    assert.ok(audit.some((e) => /Échec de connexion/.test(e.text)));
  });

  test("verrouillage après inactivité, commun à tous", async () => {
    assert.equal((await call('PATCH', '/api/auth/settings', { cookie: admin, body: { lockAfter: 45 } })).status, 400);
    assert.equal((await call('PATCH', '/api/auth/settings', { cookie: admin, body: { lockAfter: 120 } })).status, 200);
    now += 100_000;
    assert.equal((await call('POST', '/api/auth/activity', { cookie: admin })).status, 200);
    now += 100_000;
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).status, 200, 'activité il y a 100 s');
    now += 140_000;
    const locked = await call('GET', '/api/projects', { cookie: admin });
    assert.equal(locked.status, 401);
    assert.equal(locked.json.auth, 'locked');
    assert.equal((await call('POST', '/api/auth/unlock', { cookie: admin, body: { password: 'faux' } })).status, 401);
    assert.equal((await call('POST', '/api/auth/unlock', { cookie: admin, body: { password: 'mot de passe 1' } })).status, 200);
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).status, 200);
    assert.equal((await call('POST', '/api/auth/lock', { cookie: admin })).status, 200, 'verrouiller tout de suite');
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).json.auth, 'locked');
    await call('POST', '/api/auth/unlock', { cookie: admin, body: { password: 'mot de passe 1' } });
    await call('PATCH', '/api/auth/settings', { cookie: admin, body: { lockAfter: 0 } });
  });

  test('dernier administrateur protégé', async () => {
    const { json } = await call('GET', '/api/auth/users', { cookie: admin });
    const me = json.users.find((u) => u.username === 'alice');
    assert.equal((await call('DELETE', `/api/auth/users/${me.id}`, { cookie: admin })).status, 409);
    assert.equal((await call('PATCH', `/api/auth/users/${me.id}`, { cookie: admin, body: { permissions: ['operate'] } })).status, 409);
    const ed = json.users.find((u) => u.username === 'ed');
    const res = await call('PATCH', `/api/auth/users/${ed.id}`, { cookie: admin, body: { permissions: ['commands', 'instance'] } });
    assert.deepEqual(res.json.user.permissions, ['view', 'operate', 'edit', 'commands', 'instance']);
    assert.equal(res.json.user.role, 'custom');
  });

  test('réinitialisation par code (npm run auth) : sessions fermées', async () => {
    const old = (await call('POST', '/api/auth/login', { body: { username: 'op', password: 'mot de passe 3' } })).cookie;
    const code = createResetCode(app.auth.store, 'op', now);
    assert.equal((await call('POST', '/api/auth/reset', { body: { username: 'op', code: 'AAAA-AAAA', password: 'nouveau mdp 1' } })).status, 401);
    const res = await call('POST', '/api/auth/reset', { body: { username: 'op', code: code.toLowerCase(), password: 'nouveau mdp 1' } });
    assert.equal(res.status, 200);
    assert.equal((await call('GET', '/api/projects', { cookie: old })).status, 401, 'ancienne session fermée');
    assert.equal((await call('POST', '/api/auth/reset', { body: { username: 'op', code, password: 'encore autre 1' } })).status, 401, 'code à usage unique');
    now += 16 * 60_000;
    const late = createResetCode(app.auth.store, 'op', now - 16 * 60_000);
    assert.equal((await call('POST', '/api/auth/reset', { body: { username: 'op', code: late, password: 'encore autre 1' } })).status, 401, 'expiré après 15 min');
  });

  test('clé de secours : une fois, puis une nouvelle', async () => {
    const res = await call('POST', '/api/auth/recover', { body: { username: 'alice', key: recovery, password: 'mot de passe 2' } });
    assert.equal(res.status, 200);
    assert.notEqual(res.json.recovery, recovery);
    admin = res.cookie;
    assert.equal((await call('POST', '/api/auth/recover', { body: { username: 'alice', key: recovery, password: 'mot de passe 3' } })).status, 401);
  });

  test('mon compte : mot de passe, sessions, préférences', async () => {
    const other = (await call('POST', '/api/auth/login', { body: { username: 'alice', password: 'mot de passe 2' } })).cookie;
    const { json } = await call('GET', '/api/auth/me/sessions', { cookie: admin });
    assert.ok(json.sessions.length >= 2 && json.sessions.some((s) => s.current));
    assert.equal((await call('POST', '/api/auth/me/password', { cookie: admin, body: { current: 'faux', password: 'mot de passe 4' } })).status, 401);
    const changed = await call('POST', '/api/auth/me/password', { cookie: admin, body: { current: 'mot de passe 2', password: 'mot de passe 4' } });
    assert.ok(changed.json.closed >= 1);
    assert.equal((await call('GET', '/api/projects', { cookie: other })).status, 401, 'autres sessions fermées');
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).status, 200, 'la session courante reste');
    const prefs = await call('PATCH', '/api/auth/me', { cookie: admin, body: { prefs: { theme: 'solarized-dark' } } });
    assert.equal(prefs.json.user.prefs.theme, 'solarized-dark');
    assert.equal((await call('POST', '/api/instance/onboarded', { cookie: admin })).status, 200);
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).json.instance.onboarded, true, 'présentation vue : par compte');
  });

  test("nom de l'instance", async () => {
    assert.equal((await call('PATCH', '/api/instance', { cookie: admin, body: { name: 'Serveur maison' } })).status, 200);
    assert.equal((await call('GET', '/api/projects', { cookie: admin })).json.instance.name, 'Serveur maison');
  });

  test('supprimer : permission « Commandes » et mot de passe ; réordonner : permission « Modifier »', async () => {
    const invite = async (permissions) => (await call('POST', '/api/auth/invites', { cookie: admin, body: { permissions } })).json.token;
    const join = async (permissions, username) => (await call('POST', '/api/auth/invite', { body: { token: await invite(permissions), username, password: 'mot de passe 5' } })).cookie;
    const pilote = await join(['operate'], 'pilote');
    const editeur = await join(['edit'], 'editeur');
    const commandes = await join(['commands'], 'commandes');
    const ids = () => JSON.parse(fs.readFileSync(path.join(dir, 'projects.json'), 'utf8')).projects.map((p) => p.id);

    assert.equal((await call('PUT', '/api/projects', { cookie: pilote, body: { order: ['extra', 'site'] } })).status, 403);
    assert.equal((await call('PUT', '/api/projects', { cookie: editeur, body: { order: ['extra', 'site'] } })).status, 200);
    assert.deepEqual(ids(), ['extra', 'site']);

    assert.equal((await call('DELETE', '/api/projects/extra', { cookie: editeur, body: { password: 'mot de passe 5' } })).status, 403, 'permission Commandes');
    assert.equal((await call('DELETE', '/api/projects/extra', { cookie: commandes, body: { confirm: 'Extra' } })).status, 403, 'protection active : le nom ne suffit pas');
    assert.equal((await call('DELETE', '/api/projects/extra', { cookie: commandes, body: { password: 'faux' } })).status, 403);
    assert.deepEqual(ids(), ['extra', 'site'], 'rien de supprimé');
    assert.equal((await call('DELETE', '/api/projects/extra', { cookie: commandes, body: { password: 'mot de passe 5' } })).status, 200);
    assert.deepEqual(ids(), ['site']);
    const { json } = await call('GET', '/api/history', { cookie: admin });
    assert.ok(json.entries.some((e) => e.kind === 'audit' && e.user === 'commandes' && e.id === 'extra' && /Projet supprimé/.test(e.text)));
  });

  test('désactiver puis réactiver : comptes conservés', async () => {
    assert.equal((await call('POST', '/api/auth/disable', { cookie: admin, body: { password: 'faux' } })).status, 401);
    assert.equal((await call('POST', '/api/auth/disable', { cookie: admin, body: { password: 'mot de passe 4' } })).status, 200);
    assert.equal((await call('GET', '/api/projects')).status, 200, 'de nouveau ouvert');
    assert.equal((await call('POST', '/api/auth/enable', { body: { username: 'ed', password: 'mot de passe 3' } })).status, 401, 'un administrateur seulement');
    const res = await call('POST', '/api/auth/enable', { body: { username: 'alice', password: 'mot de passe 4' } });
    assert.equal(res.status, 200);
    assert.equal((await call('GET', '/api/projects')).status, 401);
  });

  test('fichier des comptes illisible : tout est refusé', async () => {
    fs.writeFileSync(path.join(dir, 'runtime', 'auth.json'), '{ cassé');
    const res = await call('GET', '/api/projects', { cookie: admin });
    assert.equal(res.status, 503);
    assert.match(res.json.error, /illisible/);
  });
});
