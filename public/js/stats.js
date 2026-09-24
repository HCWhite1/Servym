// Processeur et mémoire des projets en marche : un relevé toutes les 5 secondes,
// seulement quand la page est visible et qu'un projet tourne. Les 5 dernières
// minutes de chaque projet sont gardées ici pour les courbes du volet.
import { api } from './lib.js';

const EVERY = 5000;
const KEEP = 60; // 5 minutes

let handlers;
let onUpdate = () => {};
let latest = {};
let sums = null; // { self, total } : Servynex seul, et avec ses projets
const history = new Map(); // id → [{ at, cpu, memory }]
let timer = null;

export function init(hs, callback) {
  handlers = hs;
  onUpdate = callback;
  document.addEventListener('visibilitychange', schedule);
  schedule();
}

export const get = (id) => latest[id] ?? null;
export const historyOf = (id) => history.get(id) ?? [];
export const totals = () => sums;

const wanted = () => document.visibilityState === 'visible'
  && handlers.all().some((p) => p.type !== 'file' && ['running', 'starting', 'stopping'].includes(p.state));

// Relancé aussi quand un projet démarre (main.js) : pas d'attente de 5 s.
export function schedule() {
  clearTimeout(timer);
  timer = null;
  if (document.visibilityState === 'visible') timer = setTimeout(tick, wanted() ? 0 : EVERY);
}

async function tick() {
  if (wanted()) {
    try {
      const { projects, self, total } = await api('stats');
      latest = projects;
      sums = total ? { self, total } : null;
      const at = Date.now();
      for (const [id, s] of Object.entries(projects)) {
        if (s.inProcess) continue;
        const list = history.get(id) ?? [];
        list.push({ at, cpu: s.cpu, memory: s.memory });
        if (list.length > KEEP) list.shift();
        history.set(id, list);
      }
    } catch {
      latest = {};
      sums = null;
    }
  } else {
    latest = {};
    sums = null;
  }
  // Projet arrêté : sa courbe repart de zéro au prochain lancement.
  for (const id of history.keys()) if (!latest[id]) history.delete(id);
  onUpdate();
  clearTimeout(timer);
  timer = document.visibilityState === 'visible' ? setTimeout(tick, EVERY) : null;
}
