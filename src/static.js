// Fichiers statiques : l'interface de Servynex (public/) et le portfolio.
import { createReadStream } from 'node:fs';
import { stat } from 'node:fs/promises';
import path from 'node:path';

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
  '.avif': 'image/avif',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.ttf': 'font/ttf',
  '.pdf': 'application/pdf',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.webmanifest': 'application/manifest+json',
};

async function fileAt(file) {
  const info = await stat(file).catch(() => null);
  if (info?.isFile()) return { file, info };
  if (info?.isDirectory()) return fileAt(path.join(file, 'index.html'));
  return null;
}

// Résout le chemin demandé sous root, ou null s'il en sort.
export function resolveInside(root, urlPath) {
  let rel;
  try {
    rel = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  // « : » ouvrirait les flux alternatifs NTFS (fichier::$DATA) ; les segments
  // en point cachent .git, .env…
  if (rel.includes('\0') || rel.includes(':') || rel.split(/[\\/]/).some((s) => s.startsWith('.'))) return null;
  // Sans les barres de tête : « //serveur » serait lu comme un chemin réseau.
  const file = path.join(root, path.normalize(rel.replace(/^[\\/]+/, '')));
  return file === root || file.startsWith(root + path.sep) ? file : null;
}

// cleanUrls : /stages sert stages.html (comme GitHub Pages).
export function staticHandler(root, { cleanUrls = false } = {}) {
  root = path.resolve(root);
  return async function serve(req, res) {
    if (req.method !== 'GET' && req.method !== 'HEAD') return false;
    const { pathname } = new URL(req.url, 'http://x');
    const file = resolveInside(root, pathname);
    if (!file) return false;
    let found = await fileAt(file);
    if (!found && cleanUrls && !path.extname(file)) found = await fileAt(`${file}.html`);
    if (!found) return false;
    res.writeHead(200, {
      'Content-Type': TYPES[path.extname(found.file).toLowerCase()] ?? 'application/octet-stream',
      'Content-Length': found.info.size,
      'Cache-Control': 'no-cache',
      'Last-Modified': found.info.mtime.toUTCString(),
    });
    if (req.method === 'HEAD') res.end();
    else createReadStream(found.file).pipe(res);
    return true;
  };
}
