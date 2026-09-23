// Page principale : navigation, liste des projets (ou orbite), état en direct, raccourcis.
import { ROLE_LABELS, act, addressOf, api, applyTheme, can, canRestart, canStop, confirmDialog, fillTile, gitSummary, h, icon, initials, isExternal, primaryOf, safeUrl, session, statusText, store, toast } from './lib.js';
import { gate, lockNow, logout } from './auth.js';
import * as activity from './activity.js';
import * as menu from './menu.js';
import * as network from './network.js';
import * as onboarding from './onboarding.js';
import * as orbit from './orbit.js';
import * as panel from './panel.js';
import { createForm } from './project-form.js';
import * as settings from './settings.js';

const $ = (sel) => document.querySelector(sel);
const search = $('#search');
const rowTemplate = $('#row-tpl');

const projects = new Map();
let git = {};
let instance = { tools: {}, lan: [] };
let connected = true;
let built = false; // la liste a été construite au moins une fois
const prefs = { layout: store.get('layout', 'list'), motion: store.get('motion', true) };

const apps = () => [...projects.values()].filter((p) => !p.service);
const services = () => [...projects.values()].filter((p) => p.service);

// ---- Actions ----

async function run(p, action, button) {
  const users = [...projects.values()].filter((x) => x.requires.includes(p.id) && ['running', 'starting'].includes(x.state));
  if (action === 'stop' && users.length) {
    const names = users.map((x) => x.name).join(', ');
    const ok = await confirmDialog({
      title: `Arrêter ${p.name} ?`,
      body: `${names} ${users.length > 1 ? 'en ont' : 'en a'} besoin et ne fonctionnera plus correctement sans lui.`,
      confirm: 'Arrêter quand même',
      danger: true,
    });
    if (!ok) return;
  } else if ((action === 'stop' || action === 'restart') && isExternal(p)) {
    const stop = action === 'stop';
    const ok = await confirmDialog({
      title: `${stop ? 'Arrêter' : 'Redémarrer'} ${p.name} ?`,
      body: `${p.name} a été lancé en dehors de Servynex (tâche planifiée, fichier .bat…). Servynex retrouvera son processus par le port ${p.activePort ?? p.port}.`,
      confirm: stop ? 'Arrêter' : 'Redémarrer',
      danger: stop,
    });
    if (!ok) return;
  }
  if (button) button.disabled = true;
  try {
    await act(p.id, action);
  } catch (err) {
    toast(err.message, 'error');
  } finally {
    if (button) update(projects.get(p.id));
  }
}

// L'action principale demande la permission « Piloter », sauf ouvrir l'adresse
// d'un projet déjà en ligne ou consulter le journal d'un conflit.
const needsOperate = (p) => !((p.state === 'running' && p.type !== 'file' && !p.service) || p.state === 'conflict');

async function primary(p, button) {
  if (!p) return;
  if (needsOperate(p) && !can('operate')) return toast('Votre compte ne peut pas démarrer ni ouvrir ce projet.', 'error');
  if (p.type === 'file') return run(p, 'open');
  const url = safeUrl(p.url);
  if (p.state === 'running') {
    if (url) window.open(url, '_blank', 'noopener');
    return;
  }
  if (p.state === 'conflict') return panel.open(p.id, 'logs');
  const waitPage = url ? `/wait.html?id=${encodeURIComponent(p.id)}` : null;
  if (p.state === 'starting') {
    if (waitPage) window.open(waitPage, '_blank');
    return;
  }
  if (p.state !== 'stopped' && p.state !== 'failed') return;
  // Onglet ouvert pendant le clic (sinon le navigateur le bloque), redirigé quand le projet répond.
  const tab = waitPage ? window.open(waitPage, '_blank') : null;
  if (button) button.disabled = true;
  try {
    await act(p.id, 'start');
  } catch (err) {
    tab?.close();
    toast(err.message, 'error');
    update(p);
  }
}

function setLayout(layout, save = true) {
  const before = prefs.layout;
  prefs.layout = layout === 'orbit' ? 'orbit' : 'list';
  store.set('layout', prefs.layout);
  if (save && before !== prefs.layout) settings.savePref({ layout: prefs.layout });
  const isOrbit = prefs.layout === 'orbit' && projects.size > 0;
  $('#list-view').hidden = isOrbit;
  $('#orbit-view').hidden = !isOrbit;
  for (const b of document.querySelectorAll('[data-layout]')) b.setAttribute('aria-checked', String(b.dataset.layout === prefs.layout));
  if (isOrbit) orbit.fit();
  settings.sync();
  applyFilter();
}

function setMotion(on, save = true) {
  prefs.motion = Boolean(on);
  store.set('motion', prefs.motion);
  if (save) settings.savePref({ motion: prefs.motion });
  orbit.setMotion(prefs.motion);
  settings.sync();
}

export const handlers = {
  primary,
  run,
  get: (id) => projects.get(id),
  all: () => [...projects.values()],
  apps,
  services,
  git: () => git,
  tools: () => instance.tools,
  instance: () => instance,
  prefs: () => prefs,
  setLayout,
  setMotion,
  openPanel: (id, tab) => panel.open(id, tab),
  addProject: () => openAdd(),
  refreshGit: () => refreshGit(true),
  renderBanners: () => renderBanners(),
  onboarding: () => onboarding.open(),
};

// ---- Liste ----

const lanTitle = (p) => {
  const urls = (instance.lan ?? []).map((a) => `http://${a.address}:${p.activePort ?? p.port}`);
  return `Joignable depuis le réseau${urls.length ? ` : ${urls.join(', ')}` : ''}`;
};

function fillMain(button, p) {
  const { label, icon: name, disabled, primary: strong, hidden } = primaryOf(p);
  button.replaceChildren(...(name ? [icon(name)] : []), label);
  button.disabled = Boolean(disabled);
  button.hidden = Boolean(hidden) || (needsOperate(p) && !can('operate'));
  button.classList.toggle('primary', Boolean(strong));
}

function buildRows() {
  for (const [container, list] of [[$('#apps'), apps()], [$('#services'), services()]]) {
    for (const node of container.querySelectorAll('.row')) node.remove();
    container.append(...list.map((p) => {
      const node = rowTemplate.content.firstElementChild.cloneNode(true);
      node.dataset.id = p.id;
      return node;
    }));
  }
  $('#services-group').hidden = services().length === 0;
}

function updateRow(p) {
  const row = document.querySelector(`.row[data-id="${CSS.escape(p.id)}"]`);
  if (!row) return;
  row.dataset.state = p.state;
  row.setAttribute('aria-label', `${p.name}, ${statusText(p)}`);
  fillTile(row.querySelector('.tile'), p);

  const name = row.querySelector('.name');
  name.querySelector('.name-text').textContent = p.name;
  for (const badge of name.querySelectorAll('.badge')) badge.remove();
  if (isExternal(p)) name.append(h('span', { class: 'badge', title: 'Lancé en dehors de Servynex (tâche planifiée, .bat…)', text: 'Externe' }));
  if (p.autostart) name.append(h('span', { class: 'badge', title: 'Lancé au démarrage du PC', text: 'Auto' }));
  const desc = row.querySelector('.desc');
  desc.textContent = p.desc;
  desc.title = p.desc;

  const status = row.querySelector('.status');
  status.dataset.state = p.state;
  status.querySelector('.status-text').textContent = statusText(p);

  const addr = addressOf(p);
  const cell = row.querySelector('.cell-addr');
  cell.classList.toggle('exposed', addr.exposed);
  cell.querySelector('.mono').textContent = addr.text;
  cell.querySelector('.badge')?.remove();
  if (addr.exposed) cell.append(h('span', { class: 'badge warn', text: 'Réseau' }));
  cell.title = addr.exposed ? lanTitle(p) : '';

  const g = gitSummary(git[p.id]);
  const gitCell = row.querySelector('.cell-git');
  if (!g || p.service) gitCell.replaceChildren();
  else if (!g.branch) gitCell.replaceChildren(h('span', { class: 'faint', text: g.text }));
  else gitCell.replaceChildren(icon('git-branch'), h('span', { text: g.branch }), g.text ? h('span', { class: g.dirty ? 'dirty' : '', text: `· ${g.text}` }) : '');

  let error = row.querySelector('.row-error');
  if (p.error && !error) row.append((error = h('p', { class: 'row-error' })));
  if (error) {
    error.textContent = p.error ?? '';
    error.title = p.error ?? '';
    error.hidden = !p.error;
  }

  fillMain(row.querySelector('[data-act="main"]'), p);
  row.querySelector('[data-act="restart"]').hidden = p.type === 'file' || !canRestart(p) || !can('operate');
  row.querySelector('[data-act="stop"]').hidden = p.type === 'file' || !canStop(p) || !can('operate');
}

function update(p) {
  if (!p) return;
  updateRow(p);
  orbit.update(p);
  panel.update(p);
  settings.update(p);
  menu.update(p);
  updateSummary();
  renderBanners();
}

function build() {
  built = true;
  buildRows();
  orbit.build();
  activity.build();
  settings.build();
  panel.projectsChanged();
  const empty = projects.size === 0;
  $('#no-projects').hidden = !empty;
  $('#no-projects [data-add]').hidden = !can('commands');
  $('#no-projects p').textContent = can('commands')
    ? 'Ajoutez un site, une application ou un service (base de données…) : Servynex pourra le lancer, le surveiller et l’arrêter.'
    : 'Aucun projet n’a encore été ajouté. Un compte avec la permission « Commandes » peut en ajouter.';
  $('#apps').closest('.group').hidden = empty;
  if (empty && prefs.layout === 'orbit') setLayout('list');
  updateSummary();
  renderBanners();
  for (const p of projects.values()) update(p);
  applyFilter();
}

function updateSummary() {
  const list = [...projects.values()].filter((p) => p.type !== 'file');
  const up = list.filter((p) => p.state === 'running').length;
  const failed = list.filter((p) => ['failed', 'conflict'].includes(p.state)).length;
  const parts = [`${up} en ligne sur ${list.length}`];
  if (failed) parts.push(`${failed} en échec`);
  $('#summary').textContent = projects.size ? parts.join(' · ') : 'Aucun projet';
  $('#nav-count').textContent = list.length ? `${up}/${list.length}` : '';
  orbit.summary(up, list.length);
  document.title = up ? `Servynex · ${up} en ligne` : 'Servynex';
}

function matches(p, query) {
  return [p.id, p.name, p.desc, ...p.tags].join(' ').toLowerCase().includes(query);
}

function applyFilter() {
  const query = search.value.trim().toLowerCase();
  let shown = 0;
  for (const row of document.querySelectorAll('.row')) {
    const hit = !query || matches(projects.get(row.dataset.id), query);
    row.classList.toggle('dim', !hit);
    shown += hit;
  }
  for (const group of ['#apps', '#services']) {
    const table = $(group);
    const none = group === '#services' ? !services().length : !apps().length;
    table.closest('.group').hidden = none || (query && !table.querySelector('.row:not(.dim)'));
  }
  $('#empty').hidden = shown > 0 || prefs.layout === 'orbit' || projects.size === 0;
  orbit.filter((p) => !query || matches(p, query));
}

// ---- Bandeaux : connexion perdue, projet exposé sur un réseau public ----

function renderBanners() {
  const list = [];
  if (instance.configError) {
    const edit = h('button', { class: 'alert-action', type: 'button', text: 'Modifier projects.json' });
    edit.addEventListener('click', () => api('instance/config', { method: 'POST' }).catch((err) => toast(err.message, 'error')));
    list.push(h('div', { class: 'alert error', role: 'alert' }, icon('triangle-alert'),
      h('span', { text: `projects.json contient une erreur, la dernière configuration valide reste utilisée. ${instance.configError}` }), edit));
  }
  if (!connected) {
    list.push(h('div', { class: 'alert error', role: 'alert' }, icon('triangle-alert'), h('span', { text: 'Servynex ne répond plus. Nouvelle tentative en cours…' })));
  }
  const pub = network.publicExposure();
  if (pub) {
    const names = pub.projects.map((p) => p.name).join(', ');
    list.push(h('div', { class: 'alert warn', role: 'alert' },
      icon('triangle-alert'),
      h('span', { text: `${names} ${pub.projects.length > 1 ? 'sont exposés' : 'est exposé'} alors que ${pub.networks.length > 1 ? 'les réseaux' : 'le réseau'} ${pub.networks.map((n) => `« ${n.name} »`).join(', ')} ${pub.networks.length > 1 ? 'sont publics' : 'est public'} : toute machine connectée peut s’y connecter si le pare-feu Windows l’autorise.` }),
      h('a', { class: 'alert-action', href: '#network', text: 'Voir le réseau' })));
  }
  const banners = $('#banners');
  const moved = banners.childElementCount !== list.length;
  banners.replaceChildren(...list);
  if (moved) orbit.fit();
  $('#nav-net-alert').hidden = !pub;
  const conn = $('#conn');
  conn.dataset.state = connected ? 'ok' : 'lost';
  $('#conn-text').textContent = connected ? `Connecté · v${instance.version ?? ''}` : 'Servynex injoignable';
}

// ---- Vues ----

const VIEWS = ['projects', 'activity', 'network'];

function showView() {
  const view = VIEWS.includes(location.hash.slice(1)) ? location.hash.slice(1) : 'projects';
  for (const v of VIEWS) $(`#view-${v}`).hidden = v !== view;
  for (const link of document.querySelectorAll('.nav [data-view]')) {
    if (link.dataset.view === view) link.setAttribute('aria-current', 'page');
    else link.removeAttribute('aria-current');
  }
  if (view === 'network') network.show();
  if (view === 'activity') activity.show();
  if (view === 'projects' && prefs.layout === 'orbit') orbit.fit();
}

// ---- Données en direct ----

function applySnapshot(list) {
  const ids = [...projects.keys()];
  const sameShape = built && list.length === ids.length && list.every((p, i) => ids[i] === p.id && projects.get(p.id).service === p.service);
  projects.clear();
  for (const p of list) projects.set(p.id, p);
  if (sameShape) list.forEach(update);
  else build();
}

function connect() {
  const events = new EventSource('/api/events');
  events.addEventListener('snapshot', (e) => {
    connected = true;
    applySnapshot(JSON.parse(e.data));
    renderBanners();
  });
  events.addEventListener('state', (e) => {
    const p = JSON.parse(e.data);
    const before = projects.get(p.id)?.state;
    projects.set(p.id, p);
    update(p);
    if (before !== p.state) activity.changed();
  });
  events.addEventListener('instance', (e) => {
    Object.assign(instance, JSON.parse(e.data));
    renderBanners();
  });
  events.addEventListener('error', () => {
    connected = false;
    renderBanners();
  });
}

async function refreshGit(fresh = false) {
  try {
    git = await api(`git${fresh ? '?fresh' : ''}`);
    for (const p of projects.values()) updateRow(p);
  } catch {}
}

// ---- Évènements ----

document.addEventListener('click', (e) => {
  const row = e.target.closest('.row');
  if (!row) return;
  const p = projects.get(row.dataset.id);
  if (!p) return;
  const button = e.target.closest('button[data-act]');
  if (!button) return panel.open(p.id);
  const action = button.dataset.act;
  if (action === 'main') primary(p, button);
  else if (action === 'menu') menu.open(button, p);
  else run(p, action, button);
});

document.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && e.target.classList?.contains('row')) {
    e.preventDefault();
    return panel.open(e.target.dataset.id);
  }
  if (e.ctrlKey || e.metaKey || e.altKey || e.target.closest?.('input, textarea, select') || document.querySelector('dialog[open]')) return;
  const key = e.key.toLowerCase();
  if (key === '/') {
    e.preventDefault();
    if (location.hash && location.hash !== '#projects') location.hash = '#projects';
    search.focus();
  } else if (/^[1-9]$/.test(key)) {
    primary(apps()[Number(key) - 1]);
  } else if (key === ',') {
    settings.open();
  } else if (key === 'v') {
    setLayout(prefs.layout === 'orbit' ? 'list' : 'orbit');
  } else if (key === 'p' && prefs.layout === 'orbit') {
    setMotion(!prefs.motion);
  }
});

for (const button of document.querySelectorAll('[data-layout]')) {
  button.addEventListener('click', () => setLayout(button.dataset.layout));
}

search.addEventListener('input', applyFilter);
search.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    search.value = '';
    applyFilter();
    search.blur();
  } else if (e.key === 'Enter') {
    primary(apps().find((p) => matches(p, search.value.trim().toLowerCase())));
  }
});

$('#open-settings').addEventListener('click', () => settings.open());

// ---- Ajout d'un projet ----

const addDialog = $('#add-dialog');
const addForm = createForm({
  handlers,
  submitLabel: 'Ajouter le projet',
  onSubmit: async (body) => {
    if (body.expose) {
      const ok = await confirmDialog({
        title: `Exposer ${body.name} sur le réseau ?`,
        body: `Il écoutera sur 0.0.0.0:${body.port} : toute machine du réseau actuel pourra s’y connecter.`,
        confirm: 'Exposer',
        danger: true,
      });
      if (!ok) throw new Error('Rien n’a été ajouté.');
    }
    const created = await api('projects', { method: 'POST', body });
    addDialog.close();
    toast(`${created.name} ajouté`);
    // Le nouveau projet arrive par le flux d'évènements ; on ouvre son volet dès qu'il est là.
    for (let i = 0; i < 30 && !projects.has(created.id); i++) await new Promise((r) => setTimeout(r, 100));
    panel.open(created.id, 'info');
  },
});
$('#add-body').append(addForm.element);
addDialog.addEventListener('click', (e) => {
  if (e.target === addDialog || e.target.closest('[data-close]')) addDialog.close();
});

function openAdd() {
  if (!can('commands')) return toast('Ajouter un projet demande la permission « Commandes ».', 'error');
  addForm.fill(null);
  addDialog.showModal();
  addForm.element.elements.name.focus();
}
$('#add-project').addEventListener('click', openAdd);
document.addEventListener('click', (e) => {
  if (e.target.closest('[data-add]')) openAdd();
});
addEventListener('hashchange', showView);
addEventListener('focus', () => refreshGit());

// ---- Compte connecté ----

function renderUser() {
  const u = session.user;
  $('#user-chip').hidden = !u;
  $('#brand-name').textContent = instance.name || 'Servynex';
  $('#add-project').hidden = !can('commands');
  if (!u) return;
  $('#user-avatar').textContent = initials(u);
  $('#user-name').textContent = u.name;
  $('#user-menu-head').textContent = `${u.name} · ${ROLE_LABELS[u.role] ?? u.role}`;
}

const userMenu = $('#user-menu');
$('#user-chip').addEventListener('click', (e) => {
  const r = e.currentTarget.getBoundingClientRect();
  userMenu.showPopover();
  userMenu.style.left = `${r.left}px`;
  userMenu.style.top = `${Math.max(8, r.top - userMenu.offsetHeight - 6)}px`;
});
userMenu.addEventListener('click', (e) => {
  const act = e.target.closest('[data-user-act]')?.dataset.userAct;
  if (!act) return;
  userMenu.hidePopover();
  if (act === 'account') settings.open('account');
  else if (act === 'lock') lockNow();
  else if (act === 'logout') logout();
});
document.addEventListener('servynex:user', renderUser);

// Préférences du compte (thème, vue, animation) : elles priment sur celles du navigateur.
function applyUserPrefs() {
  const p = session.user?.prefs ?? {};
  if (p.theme) {
    store.set('theme', p.theme);
    applyTheme(p.theme);
  }
  if (p.layout) prefs.layout = p.layout;
  if (typeof p.motion === 'boolean') prefs.motion = p.motion;
}

// ---- Démarrage ----

// Protection active sans session : l'écran de connexion s'affiche et rien d'autre ne se charge.
await gate();
applyUserPrefs();
for (const module of [panel, menu, orbit, network, activity, settings, onboarding]) module.init(handlers);

const initial = await api('projects');
instance = initial.instance;
$('#brand-sub').textContent = location.host;
applySnapshot(initial.projects);
renderUser();
setLayout(prefs.layout, false);
orbit.setMotion(prefs.motion);
showView();
connect();
refreshGit();
setInterval(refreshGit, 60_000);
network.refreshNetwork().then(renderBanners);
setInterval(() => network.refreshNetwork().then(renderBanners), 60_000);
if (!instance.onboarded) onboarding.open();
