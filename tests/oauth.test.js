import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { after, before, describe, test } from 'node:test';
import { createApp } from '../server.js';
import { setServerLang } from '../src/i18n.js';

process.env.SERVYM_LANG = 'fr';
setServerLang('fr');

function request(port, { method = 'GET', path: p = '/', cookie, body } = {}) {
  return new Promise((resolve, reject) => {
    const headers = { host: `localhost:${port}` };
    if (method !== 'GET') headers['x-servym-request'] = '1';
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
        const cookies = res.headers['set-cookie'] ?? [];
        const cookieOf = (name) => cookies.map((c) => c.split(';')[0]).find((c) => c.startsWith(`${name}=`) && c.length > name.length + 1) ?? null;
        resolve({ status: res.statusCode, json, body: data, location: res.headers.location, setCookies: cookies, cookieOf });
      });
    });
    req.on('error', reject);
    req.end(body === undefined ? undefined : JSON.stringify(body));
  });
}

// Faux GitHub : échange du code contre un jeton, puis profil du compte.
const PROFILES = { 'alice-code': { id: 101, login: 'alice-gh', name: 'Alice' }, stranger: { id: 999, login: 'stranger' } };
function fakeGitHub() {
  return http.createServer((req, res) => {
    let raw = '';
    req.on('data', (c) => (raw += c));
    req.on('end', () => {
      res.setHeader('content-type', 'application/json');
      if (req.method === 'POST' && req.url === '/login/oauth/access_token') {
        const body = Object.fromEntries(new URLSearchParams(raw)); // format standard OAuth
        if (body.client_id !== 'cid' || body.client_secret !== 'secret-123' || !PROFILES[body.code]) return res.end(JSON.stringify({ error: 'bad_verification_code' }));
        return res.end(JSON.stringify({ access_token: `tok-${body.code}`, token_type: 'bearer' }));
      }
      if (req.method === 'GET' && req.url === '/user') {
        const code = String(req.headers.authorization ?? '').replace(/^Bearer tok-/, '');
        if (!PROFILES[code] || !req.headers['user-agent']) return res.writeHead(401).end('{}');
        return res.end(JSON.stringify(PROFILES[code]));
      }
      res.writeHead(404).end('{}');
    });
  });
}

describe('connexion avec GitHub', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servym-oauth-'));
  let app;
  let port;
  let gh;
  let admin;
  let now = 1_800_000_000_000;
  const call = (method, p, opts = {}) => request(port, { method, path: p, ...opts });

  // Démarre une connexion (ou une liaison) et renvoie l'état et le cookie qui l'accompagne.
  async function start(cookie, link = false) {
    const res = await call('GET', `/api/auth/github/start${link ? '?link=1' : ''}`, { cookie });
    assert.equal(res.status, 302, `démarrage : ${res.body}`);
    const url = new URL(res.location);
    return { url, state: url.searchParams.get('state'), cookie: res.cookieOf('servym_oauth'), setCookie: res.setCookies.find((c) => c.startsWith('servym_oauth=')) };
  }
  // Page de retour : Servym redirige par une balise meta (voir la conception dans src/auth.js).
  const target = (res) => res.body.match(/url=([^"]+)"/)?.[1];

  before(async () => {
    gh = fakeGitHub();
    await new Promise((r) => gh.listen(0, '127.0.0.1', r));
    const base = `http://127.0.0.1:${gh.address().port}`;
    fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify({ instance: { port: 1 }, projects: [] }));
    app = createApp({
      configFile: path.join(dir, 'projects.json'),
      logsDir: path.join(dir, 'logs'),
      runtimeDir: path.join(dir, 'runtime'),
      task: { enabled: async () => false, set: async () => {} },
      clock: () => now,
      oauth: { github: { authorizeUrl: `${base}/login/oauth/authorize`, tokenUrl: `${base}/login/oauth/access_token`, userUrl: `${base}/user` } },
    });
    await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
    port = app.server.address().port;
    admin = (await call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'mot de passe 1' } })).cookieOf('servym_session');
  });

  after(async () => {
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    await app.manager.close();
    gh.closeAllConnections();
    await new Promise((r) => gh.close(r));
    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('absente tant qu’un administrateur ne l’a pas configurée', async () => {
    assert.equal((await call('GET', '/api/auth/status')).json.methods.github, false);
    assert.equal((await call('GET', '/api/auth/github/start')).status, 409);
  });

  test('configuration par un administrateur ; le secret n’est jamais renvoyé', async () => {
    const body = { clientId: 'cid', clientSecret: 'secret-123' };
    assert.equal((await call('PUT', '/api/auth/providers/github', { body })).status, 401, 'sans session');
    assert.equal((await call('PUT', '/api/auth/providers/github', { cookie: admin, body: { clientId: 'cid' } })).status, 400, 'secret obligatoire la première fois');
    assert.equal((await call('PUT', '/api/auth/providers/github', { cookie: admin, body })).status, 200);
    const res = await call('GET', '/api/auth/providers', { cookie: admin });
    assert.equal(res.json.github.configured, true);
    assert.equal(res.json.github.clientId, 'cid');
    assert.ok(!res.body.includes('secret-123'), 'secret absent de la réponse');
    assert.ok(!(await call('GET', '/api/auth/status')).body.includes('secret-123'));
    assert.equal((await call('GET', '/api/auth/status')).json.methods.github, true);
    assert.equal((await call('PUT', '/api/auth/providers/github', { cookie: admin, body: { clientId: 'cid' } })).status, 200, 'secret gardé si on ne change que l’identifiant');
  });

  test('lier son compte : état à usage unique, lié au navigateur', async () => {
    const s = await start(admin, true);
    assert.equal(s.url.origin + s.url.pathname, `http://127.0.0.1:${gh.address().port}/login/oauth/authorize`);
    assert.equal(s.url.searchParams.get('client_id'), 'cid');
    assert.equal(s.url.searchParams.get('redirect_uri'), `http://localhost:${port}/auth/github/callback`);
    assert.match(s.state, /^[\w-]{32,}$/);
    assert.match(s.setCookie, /HttpOnly; SameSite=Lax/);
    const back = (query, cookie) => call('GET', `/auth/github/callback?${query}`, { cookie });
    assert.equal(target(await back(`code=alice-code&state=${s.state}`)), '/?auth_error=state', 'sans le cookie du navigateur');
    assert.equal(target(await back(`code=alice-code&state=autre`, s.cookie)), '/?auth_error=state', 'état différent');
    const ok = await back(`code=alice-code&state=${s.state}`, s.cookie);
    assert.equal(ok.status, 200);
    assert.equal(target(ok), '/?auth_done=linked');
    assert.equal(target(await back(`code=alice-code&state=${s.state}`, s.cookie)), '/?auth_error=state', 'usage unique');
    const status = (await call('GET', '/api/auth/status', { cookie: admin })).json;
    assert.deepEqual(status.user.identities, [{ provider: 'github', login: 'alice-gh' }]);
  });

  test('se connecter avec GitHub : seulement un compte lié', async () => {
    let s = await start();
    let res = await call('GET', `/auth/github/callback?code=stranger&state=${s.state}`, { cookie: s.cookie });
    assert.equal(target(res), '/?auth_error=unlinked');
    assert.equal(res.cookieOf('servym_session'), null, 'aucune session');
    s = await start();
    res = await call('GET', `/auth/github/callback?code=alice-code&state=${s.state}`, { cookie: s.cookie });
    assert.equal(target(res), '/');
    const session = res.cookieOf('servym_session');
    assert.ok(session, 'session ouverte');
    assert.equal((await call('GET', '/api/projects', { cookie: session })).status, 200);
    const audit = (await call('GET', '/api/history', { cookie: admin })).json.entries.filter((e) => e.kind === 'audit').map((e) => e.text);
    assert.ok(audit.some((t) => /GitHub/.test(t) && /Connexion/.test(t)), 'connexion inscrite au journal');
    assert.ok(audit.some((t) => /GitHub/.test(t) && /stranger/.test(t)), 'échec inscrit au journal');
  });

  test('état expiré après 10 minutes ; échec chez GitHub', async () => {
    let s = await start();
    now += 11 * 60_000;
    assert.equal(target(await call('GET', `/auth/github/callback?code=alice-code&state=${s.state}`, { cookie: s.cookie })), '/?auth_error=state');
    s = await start();
    assert.equal(target(await call('GET', `/auth/github/callback?code=inconnu&state=${s.state}`, { cookie: s.cookie })), '/?auth_error=failed');
    s = await start();
    assert.equal(target(await call('GET', `/auth/github/callback?error=access_denied&state=${s.state}`, { cookie: s.cookie })), '/?auth_error=denied', 'refusé sur GitHub');
  });

  test('page de retour : rien de la requête n’y est recopié', async () => {
    const s = await start();
    const res = await call('GET', `/auth/github/callback?code=%3Cscript%3E&state=%22%3E%3Cscript%3Ealert(1)%3C%2Fscript%3E`, { cookie: s.cookie });
    assert.ok(!res.body.includes('<script'), 'aucun script');
    assert.ok(!res.body.includes('alert(1)'));
  });

  test('un compte GitHub ne se lie qu’à un seul compte Servym ; délier', async () => {
    const { token } = (await call('POST', '/api/auth/invites', { cookie: admin, body: { permissions: ['operate'] } })).json;
    const bob = (await call('POST', '/api/auth/invite', { body: { token, username: 'bob', password: 'mot de passe 2' } })).cookieOf('servym_session');
    const s = await start(bob, true);
    assert.equal(target(await call('GET', `/auth/github/callback?code=alice-code&state=${s.state}`, { cookie: s.cookie })), '/?auth_error=taken');
    assert.equal((await call('DELETE', '/api/auth/me/identities/github', { cookie: admin })).status, 200);
    assert.deepEqual((await call('GET', '/api/auth/status', { cookie: admin })).json.user.identities, []);
    const again = await start();
    assert.equal(target(await call('GET', `/auth/github/callback?code=alice-code&state=${again.state}`, { cookie: again.cookie })), '/?auth_error=unlinked');
  });

  test('retirer la configuration : bouton et démarrage disparaissent', async () => {
    assert.equal((await call('DELETE', '/api/auth/providers/github', { cookie: admin })).status, 200);
    assert.equal((await call('GET', '/api/auth/status')).json.methods.github, false);
    assert.equal((await call('GET', '/api/auth/github/start')).status, 409);
  });
});
