// Journaux des projets : un fichier par projet, lu par la fin et suivi en direct.
import fs from 'node:fs';
import { StringDecoder } from 'node:string_decoder';
import { ERROR_LINE } from '../public/js/log-patterns.js';

const ANSI = /\x1b\[[0-9;?]*[ -\/]*[@-~]/g;
const MAX_LINES = 500;

const clean = (line) => line.replace(ANSI, '').replace(/[\x00-\x08\x0b-\x1f]/g, '');

function readEnd(file, bytes) {
  let fd;
  try {
    fd = fs.openSync(file, 'r');
    const { size, ino } = fs.fstatSync(fd);
    const start = Math.max(0, size - bytes);
    const buf = Buffer.alloc(size - start);
    fs.readSync(fd, buf, 0, buf.length, start);
    let lines = buf.toString('utf8').split(/\r?\n/);
    if (start > 0) lines.shift(); // première ligne coupée
    if (lines.at(-1) === '') lines.pop();
    return { lines: lines.map(clean), size, ino };
  } catch {
    return { lines: [], size: 0, ino: null };
  } finally {
    if (fd !== undefined) fs.closeSync(fd);
  }
}

export const tailLines = (file, count = MAX_LINES) => readEnd(file, 96_000).lines.slice(-count);

// Ligne la plus parlante pour expliquer un arrêt.
export function lastError(file) {
  const lines = tailLines(file, 60).filter((l) => l.trim() && !l.startsWith('[servynex]'));
  return (lines.findLast((l) => ERROR_LINE.test(l)) ?? lines.at(-1))?.trim().slice(0, 300) ?? null;
}

// Historique des changements d'état d'un projet (de tous, sans id), lu dans
// servynex.log et le précédent : lignes « <date> [state] <id> <avant> → <après>[ · détail] »
// ([état] dans les journaux écrits avant la traduction).
const TRANSITION = /^(\S+) \[(?:state|état)\] ([a-z0-9-]+) (\w+) → (\w+)(?: · (.*))?$/;
// Journal d'audit : « <date> [audit] <projet ou -> <compte ou -> · <texte> ».
const AUDIT = /^(\S+) \[audit\] ([a-z0-9-]+|-) (\S+) · (.*)$/;

export function history(files, id = null, max = 100) {
  const entries = [];
  for (const file of files) {
    for (const line of readEnd(file, 1 << 20).lines) {
      const m = line.match(TRANSITION);
      if (m && (!id || m[2] === id)) {
        entries.push({ at: m[1], id: m[2], from: m[3], to: m[4], detail: m[5] ?? '' });
        continue;
      }
      const a = line.match(AUDIT);
      if (a && (!id || a[2] === id)) entries.push({ at: a[1], id: a[2] === '-' ? null : a[2], kind: 'audit', user: a[3] === '-' ? null : a[3], text: a[4] });
    }
  }
  return entries.slice(-max).reverse();
}

// Appelle onEvent('reset', lignes) puis onEvent('lines', nouvelles lignes).
// Renvoie la fonction d'arrêt.
export function followLog(file, onEvent, intervalMs = 500) {
  let pos = 0;
  let ino = null;
  let decoder;
  let rest = '';

  const reset = () => {
    const end = readEnd(file, 96_000);
    pos = end.size;
    ino = end.ino;
    decoder = new StringDecoder('utf8');
    rest = '';
    onEvent('reset', end.lines.slice(-MAX_LINES));
  };

  const poll = () => {
    let info;
    try {
      info = fs.statSync(file);
    } catch {
      if (ino !== null) reset();
      return;
    }
    if (info.ino !== ino || info.size < pos) return reset();
    if (info.size === pos) return;
    const fd = fs.openSync(file, 'r');
    try {
      const buf = Buffer.alloc(Math.min(info.size - pos, 1 << 20));
      fs.readSync(fd, buf, 0, buf.length, pos);
      pos += buf.length;
      const parts = (rest + decoder.write(buf)).split(/\r?\n/);
      rest = parts.pop();
      if (parts.length) onEvent('lines', parts.slice(-MAX_LINES).map(clean));
    } finally {
      fs.closeSync(fd);
    }
  };

  reset();
  const timer = setInterval(poll, intervalMs);
  return () => clearInterval(timer);
}
