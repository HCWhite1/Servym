// Projets « static » : Servym sert lui-même les fichiers de cwd sur un port,
// avec des URL propres (comme GitHub Pages) et un journal des requêtes.
import fs from 'node:fs';
import http from 'node:http';
import { serverLang, tr } from './i18n.js';
import { staticHandler } from './static.js';

// Renvoie la fonction d'arrêt une fois le port ouvert.
export async function startStaticSite({ root, port, host, logFile }) {
  const serve = staticHandler(root, { cleanUrls: true });
  const log = fs.createWriteStream(logFile, { flags: 'w' });
  log.write(`[servym] ${new Date().toLocaleString(serverLang())} · ${tr('process.staticFiles', { root })} · ${host}:${port}\n`);
  const server = http.createServer(async (req, res) => {
    res.setHeader('X-Content-Type-Options', 'nosniff');
    res.on('finish', () => log.write(`${res.statusCode} ${req.method} ${req.url}\n`));
    try {
      if (!(await serve(req, res))) res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' }).end('Introuvable');
    } catch {
      res.destroy();
    }
  });
  try {
    await new Promise((resolve, reject) => {
      server.once('error', reject);
      // 0.0.0.0 : toutes les interfaces (undefined laisse Node écouter aussi en IPv6).
      server.listen(port, host === '0.0.0.0' ? undefined : host, resolve);
    });
  } catch (err) {
    log.end();
    throw err;
  }
  return async function stop() {
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
    log.end();
  };
}
