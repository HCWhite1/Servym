// Pilote l'essai Tauri par le protocole de débogage de WebView2 (port 9223).
// Usage : node drive.mjs <étape> ; étapes : info, ipc, open-external, open-wait, export, github, memory
import { execFileSync } from 'node:child_process';

const step = process.argv[2];
const arg = process.argv[3];

async function page(match = 'localhost:9000') {
  const list = await (await fetch('http://127.0.0.1:9223/json')).json();
  const target = list.find((t) => t.type === 'page' && t.url.includes(match));
  if (!target) throw new Error(`aucune page ${match} : ${list.map((t) => t.url).join(', ')}`);
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) pending.get(m.id)(m);
  };
  const send = (method, params = {}) => new Promise((r) => {
    pending.set(++id, r);
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async (expression) => {
    const m = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: true });
    return m.result?.exceptionDetails ? `EXCEPTION ${m.result.exceptionDetails.exception?.description}` : m.result?.result?.value;
  };
  return { evaluate, close: () => ws.close(), url: target.url };
}

function memory(pid) {
  const script = `$all = Get-CimInstance Win32_Process; $tree = @(${pid}); do { $n = $tree.Count; $tree = @($tree + ($all | ? { $tree -contains $_.ParentProcessId -and $tree -notcontains $_.ProcessId } | % ProcessId)) } while ($tree.Count -ne $n);`
    + ` $all | ? { $tree -contains $_.ProcessId } | % { '{0,-24} {1,6} {2,8:N1} Mo {3,8:N1} Mo privés' -f $_.Name, $_.ProcessId, ($_.WorkingSetSize/1MB), ((Get-Process -Id $_.ProcessId).PrivateMemorySize64/1MB) }`;
  const out = execFileSync('powershell.exe', ['-NoProfile', '-Command', script], { encoding: 'utf8' });
  console.log(out.trim());
  const total = out.trim().split('\n').reduce((s, l) => s + Number(l.match(/(\d[\d\s,]*) Mo /)?.[1].replace(/\s/g, '').replace(',', '.') ?? 0), 0);
  console.log(`total mémoire de travail : ${total.toFixed(1)} Mo`);
}

const p = ['memory', 'github-check'].includes(step) ? null : await page();
switch (step) {
  case 'info':
    console.log(p.url, await p.evaluate('document.title + " · tuiles : " + document.querySelectorAll("[data-id]").length'));
    break;
  case 'ipc':
    console.log('__TAURI_INTERNALS__ :', await p.evaluate('typeof window.__TAURI_INTERNALS__'));
    console.log('__TAURI__ :', await p.evaluate('typeof window.__TAURI__'));
    console.log('invoke opener :', await p.evaluate(`(async () => { try { await window.__TAURI_INTERNALS__.invoke('plugin:opener|open_url', { url: 'https://example.com' }); return 'ACCEPTÉ'; } catch (e) { return 'refusé : ' + e; } })()`));
    break;
  case 'open-external':
    console.log('window.open (externe) :', await p.evaluate(`String(window.open(${JSON.stringify(arg)}, '_blank', 'noopener'))`));
    break;
  case 'open-wait':
    console.log('window.open (attente) :', await p.evaluate(`(() => { const tab = window.open('/wait.html?id=${arg}', '_blank'); window.__tab = tab; return tab ? 'fenêtre obtenue' : 'null'; })()`));
    break;
  case 'export':
    console.log('export :', await p.evaluate(`(async () => {
      const res = await fetch('/api/config/export', { headers: { 'X-Servym-Request': '1' } });
      const name = /filename="([^"]+)"/.exec(res.headers.get('content-disposition') ?? '')?.[1] ?? 'servym-projects.json';
      const a = document.createElement('a');
      a.href = URL.createObjectURL(await res.blob());
      a.download = name;
      document.body.append(a);
      a.click();
      a.remove();
      return res.status + ' ' + name;
    })()`));
    break;
  case 'github':
    console.log('cookie posé :', await p.evaluate(`document.cookie = 'servym_essai=1; path=/; SameSite=Lax'; document.cookie`));
    await p.evaluate(`location.href = 'https://github.com/login/oauth/authorize?client_id=servym-essai&redirect_uri=http%3A%2F%2Flocalhost%3A9000%2F'`);
    break;
  case 'github-check': {
    const g = await page('github.com');
    console.log('sur GitHub :', g.url, '·', await g.evaluate('document.title'));
    await g.evaluate(`location.href = 'http://localhost:9000/'`);
    g.close();
    break;
  }
  case 'github-back':
    console.log('retour :', p.url, '· cookie :', await p.evaluate('document.cookie'));
    break;
  case 'memory':
    memory(arg);
    break;
}
p?.close();
