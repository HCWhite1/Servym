// Page principale : navigation, liste des projets (ou orbite), état en direct, raccourcis.
import { ROLE_LABELS, act, addressOf, api, applyTheme, ask, can, canRestart, canStop, confirmDialog, fillTile, formatBytes, formatCpu, gitSummary, h, icon, initials, isExternal, primaryOf, session, statusText, store, toast, urlOf, viewer } from './lib.js';
import { gate, lockNow, logout, takeNotice } from './auth.js';
import { choice, setChoice, setPlatform, t, tn, translate } from './i18n.js';
import * as activity from './activity.js';
import * as menu from './menu.js';
import * as network from './network.js';
import * as onboarding from './onboarding.js';
import * as orbit from './orbit.js';
import * as panel from './panel.js';
import { createForm } from './project-form.js';
import * as settings from './settings.js';
import * as stats from './stats.js';

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
      title: t('confirm.stop.title', { name: p.name }),
      body: tn('confirm.stopUsed.body', users.length, { names }),
      confirm: t('confirm.stopUsed.ok'),
      danger: true,
    });
    if (!ok) return;
  } else if ((action === 'stop' || action === 'restart') && isExternal(p)) {
    const stop = action === 'stop';
    const ok = await confirmDialog({
      title: t(stop ? 'confirm.stop.title' : 'confirm.restart.title', { name: p.name }),
      body: t('confirm.external.body', { name: p.name, port: p.activePort ?? p.port }),
      confirm: t(stop ? 'action.stop' : 'action.restart'),
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

// Supprimer : confirmation par le mot de passe du compte (protection active),
// sinon en retapant le nom exact du projet. Le serveur arrête le projet d'abord.
async function removeProject(p) {
  if (!can('commands')) return toast(t('delete.needsCommands'), 'error');
  const users = [...projects.values()].filter((x) => x.requires.includes(p.id)).map((x) => x.name);
  const running = ['running', 'starting'].includes(p.state);
  const body = [
    running && (p.managed || p.type === 'static') ? t('delete.stoppedFirst') : null,
    running && isExternal(p) ? t('delete.external') : null,
    users.length ? t('delete.removedWithUsers', { names: users.join(', ') }) : t('delete.removed'),
  ].filter(Boolean).join(' ');
  const field = session.enabled
    ? { name: 'password', label: t('delete.password'), type: 'password', autocomplete: 'current-password' }
    : { name: 'confirm', label: t('delete.typeName', { name: p.name }), placeholder: p.name };
  const done = await ask({
    title: t('delete.title', { name: p.name }),
    body,
    fields: [field],
    confirm: t('action.delete'),
    danger: true,
    valid: (v) => (session.enabled ? v.password.length > 0 : v.confirm === p.name),
    submit: (v) => api(`projects/${encodeURIComponent(p.id)}`, { method: 'DELETE', body: v }),
  });
  if (done) toast(t('delete.done', { name: p.name }));
}

// Dupliquer : la copie arrive juste après l'original (nouveau nom, port libre),
// puis ses Réglages s'ouvrent pour l'ajuster.
async function duplicateProject(p) {
  try {
    const created = await api(`projects/${encodeURIComponent(p.id)}/duplicate`, { method: 'POST' });
    toast(t('duplicate.done', { name: created.name }));
    for (let i = 0; i < 30 && !projects.has(created.id); i++) await new Promise((r) => setTimeout(r, 100));
    panel.open(created.id, 'settings');
  } catch (err) {
    toast(err.message, 'error');
  }
}

// L'action principale demande la permission « Piloter », sauf ouvrir l'adresse
// d'un projet déjà en ligne ou consulter le journal d'un conflit.
const needsOperate = (p) => !((p.state === 'running' && p.type !== 'file' && !p.service) || p.state === 'conflict');

async function primary(p, button) {
  if (!p) return;
  if (needsOperate(p) && !can('operate')) return toast(t('project.cannotOperate'), 'error');
  if (p.type === 'file') return run(p, 'open');
  const url = urlOf(p);
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
  remove: (p) => removeProject(p),
  duplicate: (p) => duplicateProject(p),
  refreshGit: () => refreshGit(true),
  renderBanners: () => renderBanners(),
  onboarding: () => onboarding.open(),
};

// ---- Liste ----

const lanTitle = (p) => {
  const urls = (instance.lan ?? []).map((a) => `http://${a.address}:${p.activePort ?? p.port}`);
  return urls.length ? t('project.reachableAt', { urls: urls.join(', ') }) : t('project.reachable');
};

function fillMain(button, p) {
  const { label, icon: name, disabled, primary: strong, hidden, title } = primaryOf(p);
  button.replaceChildren(...(name ? [icon(name)] : []), label);
  button.disabled = Boolean(disabled);
  button.title = title ?? '';
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

// Processeur et mémoire (stats.js) : seulement pour un projet en marche.
function fillResources(row, p) {
  const cell = row.querySelector('.cell-res');
  const s = ['running', 'starting', 'stopping'].includes(p.state) ? stats.get(p.id) : null;
  if (!s) cell.replaceChildren();
  else if (s.inProcess) cell.replaceChildren(h('span', { class: 'faint', text: t('res.builtIn') }));
  else cell.replaceChildren(h('span', { class: 'cpu', text: formatCpu(s.cpu) }), h('span', { text: formatBytes(s.memory) }));
  cell.title = !s ? '' : s.inProcess
    ? t('res.builtInHint')
    : tn('res.hint', s.processes);
}

function statsChanged() {
  updateSummary();
  for (const p of projects.values()) {
    const row = document.querySelector(`.row[data-id="${CSS.escape(p.id)}"]`);
    if (row) fillResources(row, p);
  }
  panel.statsChanged();
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
  if (isExternal(p)) name.append(h('span', { class: 'badge', title: t('badge.externalHint'), text: t('badge.external') }));
  if (p.autostart) name.append(h('span', { class: 'badge', title: t('badge.autoHint'), text: t('badge.auto') }));
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
  if (addr.exposed) cell.append(h('span', { class: 'badge warn', text: t('badge.network') }));
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

  fillResources(row, p);
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
    ? t('projects.emptyText')
    : t('projects.emptyReadonly');
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
  const parts = [t('summary.online', { up, total: list.length })];
  if (failed) parts.push(t('summary.failed', { n: failed }));
  // Total de la machine pour Servynex et ses projets, pendant qu'un projet tourne.
  const sums = stats.totals();
  const res = sums && projects.size ? h('span', {
    id: 'summary-res',
    text: t('summary.resources', { cpu: formatCpu(sums.total.cpu), memory: formatBytes(sums.total.memory) }),
    title: t('summary.resourcesHint', { cpu: formatCpu(sums.self?.cpu), memory: sums.self ? formatBytes(sums.self.memory) : '—' }),
  }) : null;
  if (!projects.size) $('#summary').textContent = t('summary.none');
  else $('#summary').replaceChildren(parts.join(' · '), ...(res ? [' · ', res] : []));
  $('#nav-count').textContent = list.length ? `${up}/${list.length}` : '';
  orbit.summary(up, list.length);
  document.title = up ? t('summary.title', { n: up }) : 'Servynex';
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
  syncReorder();
  orbit.filter((p) => !query || matches(p, query));
}

// ---- Réordonner : glisser la poignée, ou flèches haut et bas quand elle a le focus ----

// Permission « Modifier les projets », et pas pendant une recherche (lignes masquées).
function syncReorder() {
  $('#list-view').classList.toggle('reorderable', can('edit') && !search.value.trim());
}


async function saveOrder() {
  const order = [...document.querySelectorAll('#apps .row, #services .row')].map((r) => r.dataset.id);
  if (order.join() === [...projects.keys()].join()) return;
  // Même ordre en mémoire : la configuration renvoyée par le serveur ne reconstruit pas la liste.
  const entries = order.map((id) => [id, projects.get(id)]);
  projects.clear();
  for (const [id, p] of entries) projects.set(id, p);
  orbit.build();
  for (const p of projects.values()) orbit.update(p);
  try {
    await api('projects', { method: 'PUT', body: { order } });
  } catch (err) {
    toast(err.message, 'error');
    const list = await api('projects').catch(() => null);
    if (list) {
      projects.clear();
      applySnapshot(list.projects);
    }
  }
}

function moveRow(row, delta) {
  const sibling = delta < 0 ? row.previousElementSibling : row.nextElementSibling;
  if (!sibling?.classList.contains('row')) return;
  if (delta < 0) sibling.before(row);
  else sibling.after(row);
  row.querySelector('.grip').focus(); // déplacer l'élément lui fait perdre le focus
  saveOrder();
}

let dragged = null;
document.addEventListener('pointerdown', (e) => {
  const grip = e.target.closest('.reorderable .grip');
  if (grip) grip.closest('.row').draggable = true;
});
document.addEventListener('dragstart', (e) => {
  const row = e.target.closest?.('.row[draggable="true"]');
  if (!row) return;
  dragged = row;
  row.classList.add('dragging');
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setData('text/plain', row.dataset.id);
});
document.addEventListener('dragover', (e) => {
  const row = e.target.closest?.('.row');
  // Seulement dans le même groupe : une application ne devient pas un service.
  if (!dragged || !row || row.parentElement !== dragged.parentElement) return;
  e.preventDefault();
  if (row === dragged) return;
  const box = row.getBoundingClientRect();
  if (e.clientY > box.top + box.height / 2) row.after(dragged);
  else row.before(dragged);
});
document.addEventListener('drop', (e) => {
  if (dragged) e.preventDefault();
});
document.addEventListener('dragend', () => {
  if (!dragged) return;
  dragged.classList.remove('dragging');
  dragged.draggable = false;
  dragged = null;
  saveOrder();
});

// ---- Bandeaux : connexion perdue, projet exposé sur un réseau public ----

function renderBanners() {
  const list = [];
  if (instance.configError) {
    const edit = h('button', { class: 'alert-action', type: 'button', text: t('banner.editConfig') });
    edit.addEventListener('click', () => api('instance/config', { method: 'POST' }).catch((err) => toast(err.message, 'error')));
    list.push(h('div', { class: 'alert error', role: 'alert' }, icon('triangle-alert'),
      h('span', { text: t('banner.configError', { error: instance.configError }) }), edit));
  }
  if (!connected) {
    list.push(h('div', { class: 'alert error', role: 'alert' }, icon('triangle-alert'), h('span', { text: t('banner.lost') })));
  }
  const pub = network.publicExposure();
  if (pub) {
    const names = pub.projects.map((p) => p.name).join(', ');
    list.push(h('div', { class: 'alert warn', role: 'alert' },
      icon('triangle-alert'),
      h('span', { text: t('banner.public', {
        exposed: tn('banner.exposed', pub.projects.length, { names }),
        onPublic: tn('banner.onPublic', pub.networks.length, { networks: pub.networks.map((n) => t('common.quoted', { text: n.name })).join(', ') }),
      }) }),
      h('a', { class: 'alert-action', href: '#network', text: t('banner.seeNetwork') })));
  }
  const banners = $('#banners');
  const moved = banners.childElementCount !== list.length;
  banners.replaceChildren(...list);
  if (moved) orbit.fit();
  $('#nav-net-alert').hidden = !pub;
  const conn = $('#conn');
  conn.dataset.state = connected ? 'ok' : 'lost';
  $('#conn-text').textContent = connected ? t('conn.version', { version: instance.version ?? '' }) : t('conn.lost');
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
    if (before !== p.state && p.state === 'running') stats.schedule(); // premier relevé sans attendre
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
  if (!row || e.target.closest('.grip')) return;
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
  if ((e.key === 'ArrowUp' || e.key === 'ArrowDown') && e.target.closest?.('.reorderable .grip')) {
    e.preventDefault();
    return moveRow(e.target.closest('.row'), e.key === 'ArrowUp' ? -1 : 1);
  }
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
  submitLabel: t('add.submit'),
  onSubmit: async (body) => {
    if (body.expose) {
      const ok = await confirmDialog({
        title: t('expose.title', { name: body.name }),
        body: t('expose.bodyNew', { port: body.port }),
        confirm: t('expose.ok'),
        danger: true,
      });
      if (!ok) throw new Error(t('add.cancelled'));
    }
    const created = await api('projects', { method: 'POST', body });
    addDialog.close();
    toast(t('add.done', { name: created.name }));
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
  if (!can('commands')) return toast(t('add.needsCommands'), 'error');
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
  syncReorder();
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
  // Langue du compte, choisie sur un autre navigateur : la page se recharge dans cette langue.
  if (p.lang && p.lang !== choice()) {
    setChoice(p.lang);
    location.reload();
  }
  if (p.theme) {
    store.set('theme', p.theme);
    applyTheme(p.theme);
  }
  if (p.layout) prefs.layout = p.layout;
  if (typeof p.motion === 'boolean') prefs.motion = p.motion;
}

// ---- Démarrage ----

// Protection active sans session : l'écran de connexion s'affiche et rien d'autre ne se charge.
translate();
await gate();
applyUserPrefs();
for (const module of [panel, menu, orbit, network, activity, settings, onboarding]) module.init(handlers);

const initial = await api('projects');
instance = initial.instance;
// Textes propres au système (« avec la session » sous Linux).
if (setPlatform(instance.platform)) translate();
viewer.local = instance.viewerLocal !== false;
$('#brand-sub').textContent = location.host;
applySnapshot(initial.projects);
renderUser();
setLayout(prefs.layout, false);
orbit.setMotion(prefs.motion);
showView();
connect();
stats.init(handlers, statsChanged);
refreshGit();
setInterval(refreshGit, 60_000);
network.refreshNetwork().then(renderBanners);
setInterval(() => network.refreshNetwork().then(renderBanners), 60_000);
if (!instance.onboarded) onboarding.open();
// Retour d'une liaison GitHub (auth.js) : message, puis Mon compte.
const notice = takeNotice();
if (notice?.done === 'linked') {
  toast(t('oauth.linked'));
  settings.open('account');
} else if (notice?.error) {
  toast(t(`oauth.error.${notice.error}`), 'error');
}
