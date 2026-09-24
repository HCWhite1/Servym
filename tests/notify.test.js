import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { describe, test } from 'node:test';
import { promisify } from 'node:util';
import { createApp } from '../server.js';
import { validate } from '../src/config.js';
import { setServerLang } from '../src/i18n.js';
import { notifier } from '../src/notify.js';

process.env.SERVYNEX_LANG = 'fr';
setServerLang('fr');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(check, ms = 10000) {
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

function request(port, { method = 'GET', path: p = '/', cookie, body } = {}) {
  return new Promise((resolve, reject) => {
    const headers = { host: `localhost:${port}`, 'x-servynex-request': '1' };
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

// Servynex de test avec un projet qui plante et un qui tourne ; notify : notificateur factice.
async function makeApp(notify) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'servynex-notify-'));
  fs.mkdirSync(path.join(dir, 'app'));
  fs.writeFileSync(path.join(dir, 'app', 'crash.js'), `console.error('Error: base introuvable'); process.exit(3);`);
  fs.writeFileSync(path.join(dir, 'app', 'server.js'), `require('node:http').createServer((q, s) => s.end('<title>Factice</title>')).listen(process.env.PORT, process.env.HOST);`);
  fs.writeFileSync(path.join(dir, 'projects.json'), JSON.stringify({ instance: { port: 1 }, projects: [
    { id: 'crash', name: 'Mon API', cwd: path.join(dir, 'app'), cmd: 'node', args: ['crash.js'], port: await freePort(), hostControl: 'env' },
    { id: 'ok', name: 'Factice', cwd: path.join(dir, 'app'), cmd: 'node', args: ['server.js'], port: await freePort(), hostControl: 'env', health: { expect: 'Factice' } },
  ] }));
  const app = createApp({
    configFile: path.join(dir, 'projects.json'),
    logsDir: path.join(dir, 'logs'),
    runtimeDir: path.join(dir, 'runtime'),
    task: { enabled: async () => false, set: async () => {} },
    tickMs: 500,
    runningTickMs: 500,
    startTimeoutMs: 8000,
    notify,
  });
  await new Promise((r) => app.server.listen(0, '127.0.0.1', r));
  await app.manager.init();
  const port = app.server.address().port;
  const close = async () => {
    if (app.manager.state('ok').managed) await app.manager.stop('ok').catch(() => {});
    await app.manager.close();
    app.server.closeAllConnections();
    await new Promise((r) => app.server.close(r));
    fs.rmSync(dir, { recursive: true, force: true });
  };
  const call = (method, p, opts = {}) => request(port, { method, path: p, ...opts });
  return { app, dir, close, call, state: (id) => app.manager.state(id) };
}

describe('notificateur du système', () => {
  const title = 'Servynex · « Mon API »';
  const body = `Error: $(Remove-Item x) ' " <b>&amp;`;

  test('Windows : PowerShell caché, les textes par variables d’environnement', async () => {
    const calls = [];
    const send = notifier({ platform: 'win32', run: async (cmd, args, options) => calls.push({ cmd, args, options }) });
    assert.equal(await send({ title, body, source: 'Serveur maison' }), true);
    const [{ cmd, args, options }] = calls;
    assert.equal(cmd, 'powershell.exe');
    assert.equal(args.at(-2), '-File');
    assert.ok(args.at(-1).endsWith(path.join('src', 'notify.ps1')));
    assert.equal(options.windowsHide, true);
    assert.equal(options.env.SERVYNEX_NOTIFY_TITLE, title);
    assert.equal(options.env.SERVYNEX_NOTIFY_BODY, body);
    assert.equal(options.env.SERVYNEX_NOTIFY_SOURCE, 'Serveur maison');
    assert.ok(!args.join(' ').includes('Mon API'), 'jamais dans la ligne de commande');
  });

  test('Linux : notify-send, sans shell', async () => {
    const calls = [];
    const send = notifier({ platform: 'linux', run: async (cmd, args, options) => calls.push({ cmd, args, options }) });
    assert.equal(await send({ title, body }), true);
    assert.equal(calls[0].cmd, 'notify-send');
    assert.deepEqual(calls[0].args, ['--app-name=Servynex', title, body]);
  });

  test('ailleurs : rien n’est envoyé', async () => {
    const calls = [];
    const send = notifier({ platform: 'darwin', run: async (...a) => calls.push(a) });
    assert.equal(await send({ title, body }), false);
    assert.equal(calls.length, 0);
  });

  test('notify.ps1 : textes insérés comme nœuds XML, script valide', { skip: process.platform !== 'win32' }, async () => {
    const file = path.join(import.meta.dirname, '..', 'src', 'notify.ps1');
    const script = fs.readFileSync(file, 'utf8');
    assert.match(script, /CreateTextNode\(\$env:SERVYNEX_NOTIFY_TITLE\)/);
    assert.match(script, /CreateTextNode\(\$env:SERVYNEX_NOTIFY_BODY\)/);
    assert.doesNotMatch(script, /LoadXml\([^)]*\$env/, 'jamais concaténés dans le XML');
    // Analyse seulement : aucune notification n'est affichée.
    const check = `$e = $null; [void][System.Management.Automation.Language.Parser]::ParseFile('${file.replaceAll("'", "''")}', [ref]$null, [ref]$e); $e.Count`;
    const { stdout } = await promisify(execFile)('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', check], { windowsHide: true });
    assert.equal(stdout.trim(), '0');
  });
});

describe('notifications système', () => {
  test('instance.notify : oui ou non', () => {
    const conf = (notify) => ({ instance: { port: 9000, notify }, projects: [] });
    assert.doesNotThrow(() => validate(conf(true)));
    assert.doesNotThrow(() => validate(conf(false)));
    assert.throws(() => validate(conf('oui')), /instance\.notify/);
  });

  test('désactivées par défaut ; activées, un plantage prévient, un arrêt demandé non', async () => {
    const sent = [];
    const s = await makeApp(async (n) => sent.push(n));
    try {
      assert.equal((await s.call('GET', '/api/projects')).json.instance.notify, false);
      await s.call('POST', '/api/projects/crash/start');
      await until(() => s.state('crash').state === 'failed');
      await sleep(300);
      assert.equal(sent.length, 0, 'désactivées par défaut');

      assert.equal((await s.call('PUT', '/api/instance/notify', { body: { notify: 'oui' } })).status, 400);
      assert.equal((await s.call('PUT', '/api/instance/notify', { body: { notify: true } })).status, 200);
      assert.equal(JSON.parse(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8')).instance.notify, true);
      assert.equal((await s.call('GET', '/api/projects')).json.instance.notify, true);

      assert.equal((await s.call('POST', '/api/projects/crash/start')).status, 200);
      await until(() => sent.length === 1);
      assert.match(sent[0].title, /Mon API/);
      assert.equal(sent[0].body, 'Error: base introuvable');
      assert.equal(sent[0].source, 'Servynex');

      await s.call('POST', '/api/projects/ok/start');
      await until(() => s.state('ok').state === 'running');
      await s.call('POST', '/api/projects/ok/stop');
      await until(() => s.state('ok').state === 'stopped');
      await sleep(300);
      assert.equal(sent.length, 1, 'arrêt demandé : pas de notification');

      assert.equal((await s.call('PUT', '/api/instance/notify', { body: { notify: false } })).status, 200);
      assert.equal('notify' in JSON.parse(fs.readFileSync(path.join(s.dir, 'projects.json'), 'utf8')).instance, false);
      const audit = (await s.call('GET', '/api/history')).json.entries.filter((e) => e.kind === 'audit');
      assert.ok(audit.some((e) => /notification/i.test(e.text)), 'inscrit au journal d’audit');
    } finally {
      await s.close();
    }
  });

  test('essai : envoyé même désactivées ; indisponible signalé', async () => {
    const sent = [];
    let supported = true;
    const s = await makeApp(async (n) => {
      sent.push(n);
      return supported;
    });
    try {
      const res = await s.call('POST', '/api/instance/notify-test');
      assert.equal(res.status, 200);
      assert.equal(res.json.sent, true);
      assert.equal(sent.length, 1);
      assert.match(sent[0].title, /Servynex/);
      supported = false;
      assert.equal((await s.call('POST', '/api/instance/notify-test')).json.sent, false);
    } finally {
      await s.close();
    }
  });

  test('notificateur en échec : consigné, sans effet sur Servynex', async () => {
    const s = await makeApp(async () => {
      throw new Error('toast refusé');
    });
    try {
      await s.call('PUT', '/api/instance/notify', { body: { notify: true } });
      await s.call('POST', '/api/projects/crash/start');
      await until(() => s.state('crash').state === 'failed');
      await until(() => fs.readFileSync(path.join(s.dir, 'logs', 'servynex.log'), 'utf8').includes('toast refusé'));
      assert.equal((await s.call('GET', '/api/projects')).status, 200);
      const res = await s.call('POST', '/api/instance/notify-test');
      assert.equal(res.status, 502);
      assert.match(res.json.error, /toast refusé/);
    } finally {
      await s.close();
    }
  });

  test('permission « Servynex »', async () => {
    const s = await makeApp(async () => true);
    try {
      const admin = (await s.call('POST', '/api/auth/setup', { body: { username: 'alice', password: 'mot de passe 1' } })).cookie;
      const join = async (permissions, username) => {
        const { token } = (await s.call('POST', '/api/auth/invites', { cookie: admin, body: { permissions } })).json;
        return (await s.call('POST', '/api/auth/invite', { body: { token, username, password: 'mot de passe 2' } })).cookie;
      };
      const editor = await join(['edit'], 'ed');
      assert.equal((await s.call('PUT', '/api/instance/notify', { cookie: editor, body: { notify: true } })).status, 403);
      assert.equal((await s.call('POST', '/api/instance/notify-test', { cookie: editor })).status, 403);
      assert.equal((await s.call('PUT', '/api/instance/notify', { cookie: await join(['instance'], 'inst'), body: { notify: true } })).status, 200);
    } finally {
      await s.close();
    }
  });
});
