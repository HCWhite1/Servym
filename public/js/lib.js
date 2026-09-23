// Accès à l'API et petites fonctions partagées.

export async function api(path, { method = 'GET', body } = {}) {
  const headers = { 'X-Servynex-Request': '1' };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(`/api/${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const data = await res.json().catch(() => ({}));
  // Session absente, expirée ou verrouillée : l'écran de connexion prend la main (auth.js).
  if (data.auth && !path.startsWith('auth/')) dispatchEvent(new CustomEvent('servynex:auth', { detail: data }));
  if (!res.ok) throw Object.assign(new Error(data.error ?? `Erreur ${res.status}`), { status: res.status, auth: data.auth });
  return data;
}

// ---- Compte connecté et permissions ----

// enabled : protection active · user : compte connecté (null si protection désactivée).
export const session = { enabled: false, user: null, lockAfter: 0 };

// Protection désactivée : tout est permis. Sinon, permission du compte (admin : tout).
export function can(permission) {
  const perms = session.user?.permissions;
  return !session.enabled || Boolean(perms && (perms.includes('admin') || perms.includes(permission)));
}

export const PERMISSION_LABELS = {
  view: ['Voir', 'Projets, journaux, réseau et activité.'],
  operate: ['Piloter', 'Démarrer, arrêter, redémarrer et ouvrir les projets.'],
  edit: ['Modifier les projets', 'Nom, port, dépendances, adresse, vérification…'],
  commands: ['Commandes', 'Commande, dossier, environnement, exposition, ajout de projet. Équivaut à un accès complet à la machine.'],
  instance: ['Servynex', 'Redémarrer Servynex, démarrage avec Windows, nom de l’instance.'],
  admin: ['Administration', 'Comptes, invitations et sécurité. Donne toutes les permissions.'],
};
export const ROLE_LABELS = { admin: 'Administrateur', editor: 'Éditeur', operator: 'Opérateur', readonly: 'Lecture seule', custom: 'Personnalisé' };
export const ROLE_PERMISSIONS = { readonly: ['view'], operator: ['view', 'operate'], editor: ['view', 'operate', 'edit'], admin: ['view', 'operate', 'edit', 'commands', 'instance', 'admin'] };

export function initials(user) {
  const words = (user?.name || user?.username || '?').trim().split(/\s+/);
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? '')).toUpperCase() || '?';
}

export const act = (id, action) => api(`projects/${encodeURIComponent(id)}/${action}`, { method: 'POST' });

// ---- États ----

export const STATE_LABELS = {
  stopped: 'Arrêté',
  starting: 'Démarrage…',
  running: 'En ligne',
  stopping: 'Arrêt…',
  failed: 'En échec',
  conflict: 'Port occupé',
  file: 'Fichier local',
};

export function statusText(p) {
  if (p.detail) return p.detail;
  return STATE_LABELS[p.state] ?? p.state;
}

export const isExternal = (p) => p.state === 'running' && !p.managed;

export const canStop = (p) =>
  ['running', 'starting', 'failed', 'conflict'].includes(p.state) && (p.managed || p.state === 'running' || p.state === 'starting');

export const canRestart = (p) => p.state === 'running' || (p.state === 'failed' && p.managed);

// Adresse d'écoute effective (celle du lancement en cours, sinon celle demandée).
export function exposure(p) {
  const host = p.activeHost ?? p.host;
  return { address: `${host}:${p.activePort ?? p.port}`, exposed: host === '0.0.0.0' };
}

// Adresse affichée : localhost:3400, 0.0.0.0:3100 si exposé, ou le fichier ouvert.
export function addressOf(p) {
  if (p.type === 'file') return { text: p.open ?? 'fichier', exposed: false };
  const net = exposure(p);
  return { text: net.exposed ? net.address : `localhost:${p.activePort ?? p.port}`, exposed: net.exposed };
}

// Action principale d'un projet : libellé, icône, style (primary : à faire).
export function primaryOf(p) {
  if (p.type === 'file') return { label: 'Ouvrir', icon: 'external-link' };
  if (p.service) {
    if (p.state === 'stopped' || p.state === 'failed') return { label: 'Démarrer', icon: 'play', primary: true };
    if (p.state === 'running') return { label: 'En marche', hidden: true };
    return { label: STATE_LABELS[p.state] ?? '…', disabled: true };
  }
  switch (p.state) {
    case 'running': return { label: 'Ouvrir', icon: 'external-link', disabled: !p.url };
    case 'starting': return { label: 'Démarrage…', icon: 'external-link' };
    case 'stopping': return { label: 'Arrêt…', disabled: true };
    case 'failed': return { label: 'Relancer', icon: 'rotate-cw', primary: true };
    case 'conflict': return { label: 'Voir le journal', icon: 'scroll-text' };
    default: return { label: 'Démarrer', icon: 'play', primary: true };
  }
}

// ---- Temps ----

const rtf = new Intl.RelativeTimeFormat('fr', { numeric: 'auto' });
const STEPS = [[60, 'second'], [60, 'minute'], [24, 'hour'], [30, 'day'], [12, 'month'], [Infinity, 'year']];

export function ago(ms) {
  let value = (ms - Date.now()) / 1000;
  for (const [size, unit] of STEPS) {
    if (Math.abs(value) < size) return rtf.format(Math.round(value), unit);
    value /= size;
  }
}

export function uptime(ms) {
  const min = Math.floor((Date.now() - ms) / 60000);
  if (min < 1) return 'moins d’une minute';
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60);
  return h < 24 ? `${h} h ${String(min % 60).padStart(2, '0')}` : `${Math.floor(h / 24)} j ${h % 24} h`;
}

export function gitSummary(g) {
  if (!g) return null;
  if (!g.repo) return { branch: null, text: 'Non versionné', dirty: false };
  const bits = [];
  if (g.changes) bits.push(`${g.changes} modif.`);
  if (g.ahead) bits.push(`↑${g.ahead}`);
  if (g.behind) bits.push(`↓${g.behind}`);
  return { branch: g.branch ?? 'détachée', text: bits.join(' · '), dirty: g.changes > 0 };
}

// ---- DOM ----

export function h(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === undefined || v === null || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k === 'dataset') Object.assign(node.dataset, v);
    else if (k === 'text') node.textContent = v;
    else if (k === 'style') for (const [prop, value] of Object.entries(v)) node.style.setProperty(prop, value);
    else node.setAttribute(k, v === true ? '' : v);
  }
  node.append(...children.flat().filter((c) => c !== null && c !== undefined && c !== false));
  return node;
}

// Icône du fichier public/icons.svg (Lucide).
export function icon(name) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
  svg.setAttribute('aria-hidden', 'true');
  use.setAttribute('href', `icons.svg#${name}`);
  svg.append(use);
  return svg;
}

// Icônes proposées pour un projet (réglages). Toute autre valeur, comme un ancien
// emoji, est remplacée par l'initiale du nom.
export const PROJECT_ICONS = [
  'target', 'graduation-cap', 'school', 'globe', 'book-open', 'gamepad-2', 'image', 'database',
  'music', 'film', 'camera', 'briefcase', 'layout-dashboard', 'message-square', 'mail', 'newspaper',
  'shopping-cart', 'calendar', 'chart-line', 'users', 'house', 'notebook', 'palette', 'code-xml',
  'server', 'box', 'cloud', 'shield', 'zap', 'flask-conical', 'bot',
];

export function glyph(p) {
  return PROJECT_ICONS.includes(p.icon) ? icon(p.icon) : document.createTextNode((p.name.trim()[0] ?? '?').toUpperCase());
}

// Pastille d'un projet, teintée par sa couleur.
export function tile(p, size = '') {
  const node = h('span', { class: `tile ${size}`.trim(), 'aria-hidden': 'true' }, glyph(p));
  node.style.setProperty('--c', p.color);
  return node;
}

export function fillTile(node, p) {
  node.style.setProperty('--c', p.color);
  node.replaceChildren(glyph(p));
}

export function toast(message, type = 'info') {
  const node = h('div', { class: `toast ${type}` }, icon(type === 'error' ? 'triangle-alert' : 'check'), h('span', { text: message }));
  document.getElementById('toasts').append(node);
  setTimeout(() => node.remove(), type === 'error' ? 8000 : 4000);
}

// Fenêtre de confirmation (remplace confirm()) : renvoie true si l'utilisateur confirme.
export function confirmDialog({ title, body, confirm = 'Confirmer', danger = false }) {
  const dialog = document.getElementById('confirm');
  dialog.querySelector('h2').textContent = title;
  dialog.querySelector('.modal-body p').textContent = body;
  const ok = dialog.querySelector('[value="ok"]');
  ok.textContent = confirm;
  ok.className = `btn ${danger ? 'danger-fill' : 'primary'}`;
  dialog.returnValue = '';
  dialog.showModal();
  ok.focus();
  return new Promise((resolve) => dialog.addEventListener('close', () => resolve(dialog.returnValue === 'ok'), { once: true }));
}

export function safeUrl(url) {
  try {
    return ['http:', 'https:'].includes(new URL(url).protocol) ? url : null;
  } catch {
    return null;
  }
}

export const store = {
  get(key, fallback) {
    try {
      return JSON.parse(localStorage.getItem(`servynex.${key}`)) ?? fallback;
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(`servynex.${key}`, JSON.stringify(value));
    } catch {}
  },
};

// Thème : « system » suit Windows, sinon un thème de THEMES (voir aussi theme.js).
export const THEMES = ['light', 'dark', 'solarized-light', 'solarized-dark'];
export function applyTheme(theme) {
  if (THEMES.includes(theme)) document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
}
