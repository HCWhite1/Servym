// Petits outils communs aux modules de chaque système (windows.js, linux.js).
import { execFile, spawn } from 'node:child_process';
import { promisify } from 'node:util';

export const run = promisify(execFile);

export function runQuiet(cmd, args, options = {}) {
  return run(cmd, args, { windowsHide: true, timeout: 15000, ...options });
}

export function isAlive(pid) {
  if (!pid) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === 'EPERM';
  }
}

export function detach(cmd, args, options = {}) {
  const child = spawn(cmd, args, { detached: true, stdio: 'ignore', ...options });
  child.on('error', () => {});
  child.unref();
}

// Enfants lancés sans les variables de l'éditeur ou du terminal qui a démarré Servym.
export function cleanEnv(extra = {}) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^(ELECTRON_|VSCODE_|CLAUDE|SERVYM_SUPERVISOR$)/i.test(key)) delete env[key];
  }
  return { ...env, ...extra };
}
