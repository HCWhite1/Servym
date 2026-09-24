// Accès à l'API et petites fonctions partagées.
import { lang, t } from './i18n.js';
import { projectUrl } from './urls.js';

// raw : la réponse elle-même quand elle réussit (fichier à télécharger).
export async function api(path, { method = 'GET', body, raw = false } = {}) {
  // Accept-Language : les messages d'erreur du serveur arrivent dans la langue de la page.
  const headers = { 'X-Servynex-Request': '1', 'Accept-Language': lang };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(`/api/${path}`, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  if (raw && res.ok) return res;
  const data = await res.json().catch(() => ({}));
  // Session absente, expirée ou verrouillée : l'écran de connexion prend la main (auth.js).
  if (data.auth && !path.startsWith('auth/')) dispatchEvent(new CustomEvent('servynex:auth', { detail: data }));
  if (!res.ok) throw Object.assign(new Error(data.error ?? t('error.status', { status: res.status })), { status: res.status, auth: data.auth });
  return data;
}

// ---- Compte connecté et permissions ----

// enabled : protection active · user : compte connecté (null si protection désactivée) ·
// methods : moyens de connexion proposés ({ password: true, github: false }).
export const session = { enabled: false, user: null, lockAfter: 0, methods: {} };

// Protection désactivée : tout est permis. Sinon, permission du compte (admin : tout).
export function can(permission) {
  const perms = session.user?.permissions;
  return !session.enabled || Boolean(perms && (perms.includes('admin') || perms.includes(permission)));
}

// Permission → [nom, description]. Calculé à l'affichage : certains textes
// dépendent du système de la machine, connu après le chargement.
export const permissionLabels = () => Object.fromEntries(['view', 'operate', 'edit', 'commands', 'instance', 'admin']
  .map((p) => [p, [t(`perm.${p}`), t(`perm.${p}.desc`)]]));
export const ROLE_LABELS = Object.fromEntries(['admin', 'editor', 'operator', 'readonly', 'custom'].map((r) => [r, t(`roleLabel.${r}`)]));
export const ROLE_PERMISSIONS = { readonly: ['view'], operator: ['view', 'operate'], editor: ['view', 'operate', 'edit'], admin: ['view', 'operate', 'edit', 'commands', 'instance', 'admin'] };

export function initials(user) {
  const words = (user?.name || user?.username || '?').trim().split(/\s+/);
  return ((words[0]?.[0] ?? '') + (words[1]?.[0] ?? '')).toUpperCase() || '?';
}

// Qui regarde la page : local (sur la machine de Servynex) ou depuis un autre
// appareil du réseau (main.js le règle d'après le serveur).
export const viewer = { local: true, hostname: location.hostname };
// Adresse à ouvrir d'ici, ou null si le projet n'est pas joignable d'ici (voir urls.js).
export const urlOf = (p) => projectUrl(p, viewer);

export const act = (id, action) => api(`projects/${encodeURIComponent(id)}/${action}`, { method: 'POST' });

// ---- États ----

export const STATE_LABELS = Object.fromEntries(['stopped', 'starting', 'running', 'stopping', 'failed', 'conflict', 'file']
  .map((s) => [s, t(`state.${s}`)]));

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
  if (p.type === 'file') return { text: p.open ?? t('common.file'), exposed: false };
  const net = exposure(p);
  return { text: net.exposed ? net.address : `localhost:${p.activePort ?? p.port}`, exposed: net.exposed };
}

// Action principale d'un projet : libellé, icône, style (primary : à faire).
export function primaryOf(p) {
  if (p.type === 'file') return { label: t('common.open'), icon: 'external-link', disabled: !viewer.local, title: viewer.local ? '' : t('remote.file') };
  if (p.service) {
    if (p.state === 'stopped' || p.state === 'failed') return { label: t('action.start'), icon: 'play', primary: true };
    if (p.state === 'running') return { label: t('action.running'), hidden: true };
    return { label: STATE_LABELS[p.state] ?? '…', disabled: true };
  }
  switch (p.state) {
    case 'running': return { label: t('common.open'), icon: 'external-link', disabled: !urlOf(p), title: p.url && !urlOf(p) ? t('remote.notExposed') : '' };
    case 'starting': return { label: STATE_LABELS.starting, icon: 'external-link' };
    case 'stopping': return { label: STATE_LABELS.stopping, disabled: true };
    case 'failed': return { label: t('action.relaunch'), icon: 'rotate-cw', primary: true };
    case 'conflict': return { label: t('action.viewLog'), icon: 'scroll-text' };
    default: return { label: t('action.start'), icon: 'play', primary: true };
  }
}

// ---- Temps ----

const rtf = new Intl.RelativeTimeFormat(lang, { numeric: 'auto' });
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
  if (min < 1) return t('time.underMinute');
  if (min < 60) return t('time.minutes', { n: min });
  const h = Math.floor(min / 60);
  return h < 24 ? t('time.hoursMinutes', { h, m: String(min % 60).padStart(2, '0') }) : t('time.daysHours', { d: Math.floor(h / 24), h: h % 24 });
}

export function gitSummary(g) {
  if (!g) return null;
  if (!g.repo) return { branch: null, text: t('git.none'), dirty: false };
  const bits = [];
  if (g.changes) bits.push(t('git.changes', { n: g.changes }));
  if (g.ahead) bits.push(`↑${g.ahead}`);
  if (g.behind) bits.push(`↓${g.behind}`);
  return { branch: g.branch ?? t('git.detached'), text: bits.join(' · '), dirty: g.changes > 0 };
}

// ---- Ressources ----

const decimal = new Intl.NumberFormat(lang, { maximumFractionDigits: 1 });

// Part du processeur de toute la machine : une décimale sous 10 %.
export function formatCpu(cpu) {
  if (cpu === null || cpu === undefined) return '—';
  return `${decimal.format(cpu < 10 ? cpu : Math.round(cpu))}\u00a0%`;
}

export function formatBytes(bytes) {
  const mb = bytes / 1048576;
  return mb < 1000 ? t('unit.mb', { n: Math.round(mb) }) : t('unit.gb', { n: decimal.format(mb / 1024) });
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
export function confirmDialog({ title, body, confirm = t('common.confirm'), danger = false }) {
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

// Fenêtre qui demande un ou plusieurs champs ; renvoie les valeurs, ou null si annulée.
export function ask({ title, body, fields, confirm = t('common.ok'), danger = false, submit, valid = null }) {
  const inputs = fields.map((f) => ({ ...f, el: h('input', { class: 'input', type: f.type ?? 'text', autocomplete: f.autocomplete ?? 'off', placeholder: f.placeholder ?? '' }) }));
  const error = h('p', { class: 'hint warn', hidden: true });
  const ok = h('button', { class: `btn ${danger ? 'danger-fill' : 'primary'}`, type: 'submit', text: confirm });
  // valid(values) : le bouton reste grisé tant que la saisie ne convient pas (nom à retaper…).
  const values = () => Object.fromEntries(inputs.map((f) => [f.name, f.el.value]));
  const check = () => (ok.disabled = Boolean(valid && !valid(values())));
  for (const f of inputs) f.el.addEventListener('input', check);
  check();
  const cancel = h('button', { class: 'btn', type: 'button', text: t('common.cancel') });
  const form = h('form', { class: 'form' },
    body ? h('p', { class: 'muted', text: body }) : null,
    ...inputs.map((f) => h('label', { class: 'field' }, h('span', { text: f.label }), f.el)),
    error);
  const dialog = h('dialog', { class: 'modal' },
    h('header', { class: 'modal-head' }, h('h2', { text: title })),
    h('div', { class: 'modal-body' }, form),
    h('footer', { class: 'modal-foot' }, cancel, ok));
  document.body.append(dialog);
  dialog.showModal();
  inputs[0]?.el.focus();
  return new Promise((resolve) => {
    const close = (value) => {
      dialog.close();
      dialog.remove();
      resolve(value);
    };
    cancel.addEventListener('click', () => close(null));
    dialog.addEventListener('cancel', () => close(null));
    const run = async (e) => {
      e?.preventDefault();
      if (ok.disabled) return;
      ok.disabled = true;
      try {
        close(submit ? await submit(values()) : values());
      } catch (err) {
        error.textContent = err.message;
        error.hidden = false;
        check();
      }
    };
    form.addEventListener('submit', run);
    ok.addEventListener('click', run);
  });
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
