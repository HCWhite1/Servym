// Notifications du système (Paramètres › Général, désactivées par défaut) : un
// projet qui plante prévient même la page fermée. Windows : notification par
// src/notify.ps1 ; Linux : notify-send. Les textes passent par des variables
// d'environnement ou des arguments sans shell, jamais par une ligne de commande.
import { execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';
import { ROOT_DIR } from './config.js';

const exec = promisify(execFile);
const runFile = (cmd, args, options) => exec(cmd, args, { timeout: 15000, ...options });

// send({ title, body, source }) → true si envoyée, false si le système n'en a pas.
export function notifier({ platform = process.platform, run = runFile } = {}) {
  return async ({ title, body = '', source = 'Servynex' }) => {
    if (platform === 'win32') {
      await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', path.join(ROOT_DIR, 'src', 'notify.ps1')], {
        windowsHide: true,
        env: { ...process.env, SERVYNEX_NOTIFY_TITLE: title, SERVYNEX_NOTIFY_BODY: body, SERVYNEX_NOTIFY_SOURCE: source },
      });
      return true;
    }
    if (platform === 'linux') {
      try {
        await run('notify-send', ['--app-name=Servynex', title, body], {});
      } catch (err) {
        if (err.code === 'ENOENT') return false; // notify-send absent (libnotify)
        throw err;
      }
      return true;
    }
    return false;
  };
}
