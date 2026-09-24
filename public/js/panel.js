// Volet d'un projet : journal en direct, historique, réglages, infos.
import { STATE_LABELS, ago, api, can, canRestart, canStop, confirmDialog, exposure, fillTile, formatBytes, formatCpu, h, icon, isExternal, primaryOf, statusText, toast, uptime, urlOf, viewer } from './lib.js';
import { ERROR_LINE, WARN_LINE } from './log-patterns.js';
import { createForm } from './project-form.js';
import { auditLine } from './activity.js';
import { lang, t, tn } from './i18n.js';
import * as stats from './stats.js';

const $ = (sel) => document.querySelector(sel);
const dialog = $('#panel');
const logBox = $('#log');
const MAX_LINES = 2000;

let handlers;
let currentId = null;
let logStream = null;
let tab = 'logs';
let infoTimer = null;
let shownState = null; // état au dernier affichage de l'historique
let form = null; // formulaire des Réglages
let dangerZone = null; // « Supprimer ce projet », sous le formulaire

const current = () => handlers.get(currentId);

// ---- Ouverture ----

export function init(hs) {
  handlers = hs;
  dialog.addEventListener('close', onClose);
  dialog.addEventListener('click', (e) => {
    if (e.target === dialog || e.target.closest('[data-close]')) dialog.close();
  });
  $('#panel-actions').addEventListener('click', onAction);
  for (const button of dialog.querySelectorAll('[role="tab"]')) {
    button.addEventListener('click', () => showTab(button.id.slice(4)));
  }
  $('#log-errors').addEventListener('change', (e) => logBox.classList.toggle('errors-only', e.target.checked));
  $('#log-search').addEventListener('input', (e) => search(e.target.value));
  $('#log-copy').addEventListener('click', async () => {
    await navigator.clipboard.writeText(logBox.textContent);
    toast(t('panel.logCopied'));
  });
  form = createForm({ handlers, submitLabel: t('common.save'), onSubmit: save });
  const remove = h('button', { class: 'btn danger', type: 'button' }, icon('trash-2'), t('action.delete'));
  remove.addEventListener('click', () => current() && handlers.remove(current()));
  dangerZone = h('div', { class: 'danger-zone' },
    h('div', { class: 'setting-text' },
      h('p', { class: 'setting-label', text: t('delete.zone') }),
      h('p', { class: 'setting-desc', text: t('delete.zoneDesc') })),
    remove);
  $('#pane-settings').replaceChildren(form.element, dangerZone);
}

async function save(body, p) {
  if (body.expose && !p.source.expose) {
    const port = body.port ?? p.port;
    const ok = await confirmDialog({
      title: t('expose.title', { name: p.name }),
      body: t('expose.body', { name: p.name, port, reachable: reachableAt(port) }),
      confirm: t('expose.ok'),
      danger: true,
    });
    if (!ok) throw new Error(t('panel.notSaved'));
  }
  return api(`projects/${encodeURIComponent(p.id)}`, { method: 'PATCH', body });
}

// La liste des projets a changé : les dépendances proposées suivent.
export function projectsChanged() {
  form?.projectsChanged();
  if (currentId && !current() && dialog.open) dialog.close(); // projet supprimé
}

export function open(id, which = 'logs') {
  const p = handlers.get(id);
  if (!p) return;
  if (p.type === 'file' && (which === 'logs' || which === 'history')) which = 'info';
  if (currentId !== id) {
    currentId = id;
    form.fill(p);
    if (p.type === 'file') stopLog();
    else startLog();
  }
  update(p);
  showTab(which);
  if (!dialog.open) dialog.showModal();
}

function stopLog() {
  logStream?.close();
  logStream = null;
  logBox.replaceChildren();
}

function onClose() {
  stopLog();
  clearInterval(infoTimer);
  currentId = null;
}

function showTab(name) {
  tab = name;
  for (const button of dialog.querySelectorAll('[role="tab"]')) {
    const on = button.id === `tab-${name}`;
    button.setAttribute('aria-selected', String(on));
    $(`#pane-${button.id.slice(4)}`).hidden = !on;
  }
  clearInterval(infoTimer);
  if (name === 'info') {
    renderInfo(current(), true);
    infoTimer = setInterval(() => renderInfo(current()), 30_000);
  } else if (name === 'history') {
    renderHistory(current());
  }
}

// ---- En-tête et actions ----

export function update(p) {
  if (!p || p.id !== currentId) return;
  fillTile($('#panel-tile'), p);
  $('#panel-name').textContent = p.name;
  const status = $('#panel-status');
  status.dataset.state = p.state;
  const where = p.type === 'file' ? p.open : `port ${p.activePort ?? p.port}`;
  status.replaceChildren(h('i', { class: 'dot' }), [statusText(p), isExternal(p) ? t('panel.startedOutside') : null, where].filter(Boolean).join(' · '));
  const error = $('#panel-error');
  error.textContent = p.error ?? '';
  error.hidden = !p.error;
  dangerZone.hidden = !can('commands');

  const main = $('#panel-actions [data-act="main"]');
  const { label, icon: name, disabled, primary, hidden, title } = primaryOf(p);
  main.replaceChildren(...(name ? [icon(name)] : []), label);
  main.disabled = Boolean(disabled) || p.state === 'conflict';
  main.title = title ?? '';
  main.hidden = Boolean(hidden) || (!can('operate') && !(p.state === 'running' && p.type !== 'file' && !p.service));
  main.classList.toggle('primary', Boolean(primary));
  const file = p.type === 'file';
  $('#panel-actions [data-act="stop"]').hidden = file || !canStop(p) || !can('operate');
  $('#panel-actions [data-act="restart"]').hidden = file || !canRestart(p) || !can('operate');
  // Dossier, VS Code, terminal : s'ouvrent sur la machine de Servynex, pour qui est devant.
  for (const act of ['folder', 'terminal']) $(`#panel-actions [data-act="${act}"]`).hidden = !can('operate') || !viewer.local;
  $('#tab-logs').hidden = file;
  $('#tab-history').hidden = file;
  $('#panel-actions [data-act="code"]').hidden = !handlers.tools().code || !can('operate') || !viewer.local;
  $('#panel-actions [data-act="terminal"]').title = t('panel.terminalHere', { terminal: handlers.tools().terminal ?? 'Terminal' });
  logNote(p);
  if (tab === 'info') renderInfo(p);
  // Un changement d'état ajoute une ligne à l'historique.
  if (tab === 'history' && p.state !== shownState) renderHistory(p);
  form.refresh(p);
}

function onAction(e) {
  const button = e.target.closest('[data-act]');
  const p = current();
  if (!button || !p) return;
  if (button.dataset.act === 'main') handlers.primary(p, button);
  else handlers.run(p, button.dataset.act, button);
}

// ---- Journal ----

function logNote(p) {
  const note = $('#log-note');
  let text = '';
  if (isExternal(p) && p.hasLog) text = t('log.externalOld');
  else if (isExternal(p)) text = t('log.external');
  else if (!p.hasLog) text = t('log.none');
  note.textContent = text;
  note.hidden = !text;
}

function lineNode(line) {
  const cls = line.startsWith('[servynex]') ? 'own' : ERROR_LINE.test(line) ? 'err' : WARN_LINE.test(line) ? 'warn' : '';
  const node = h('span', cls ? { class: cls } : {});
  node.line = line;
  showMatch(node);
  return node;
}

// ---- Recherche dans le journal : lignes qui contiennent le texte, surligné ----

let query = '';

// Texte de la ligne, avec <mark> autour de chaque occurrence ; « miss » la masque.
function showMatch(node) {
  const { line } = node;
  const at = query ? line.toLowerCase().indexOf(query) : -1;
  node.classList.toggle('miss', Boolean(query) && at < 0);
  if (at < 0) return node.replaceChildren(`${line}\n`);
  const parts = [];
  let from = 0;
  for (let i = at; i >= 0; i = line.toLowerCase().indexOf(query, from)) {
    parts.push(line.slice(from, i), h('mark', { text: line.slice(i, i + query.length) }));
    from = i + query.length;
  }
  node.replaceChildren(...parts, `${line.slice(from)}\n`);
}

function search(text) {
  query = text.trim().toLowerCase();
  for (const node of logBox.children) showMatch(node);
  countMatches();
}

function countMatches() {
  const count = $('#log-count');
  count.hidden = !query;
  if (query) count.textContent = tn('log.matches', logBox.querySelectorAll('span:not(.miss)').length);
}

function appendLines(lines, reset) {
  const follow = $('#log-follow').checked;
  if (reset) logBox.replaceChildren();
  logBox.append(...lines.map(lineNode));
  while (logBox.childElementCount > MAX_LINES) logBox.firstElementChild.remove();
  if (query) countMatches();
  if (follow) logBox.scrollTop = logBox.scrollHeight;
}

function startLog() {
  logStream?.close();
  logBox.replaceChildren();
  logStream = new EventSource(`/api/projects/${encodeURIComponent(currentId)}/logs`);
  logStream.addEventListener('reset', (e) => appendLines(JSON.parse(e.data), true));
  logStream.addEventListener('lines', (e) => appendLines(JSON.parse(e.data), false));
}

// « , par exemple http://192.168.1.12:3100 » : adresses de ce PC sur les réseaux.
function reachableAt(port) {
  const urls = (handlers.instance().lan ?? []).map((a) => `http://${a.address}:${port}`);
  return urls.length ? t('expose.forExample', { urls: urls.join(', ') }) : '';
}

// ---- Ressources (en tête des Infos) ----

const SVG = 'http://www.w3.org/2000/svg';
const POINTS = 60; // 5 minutes, un relevé toutes les 5 s (stats.js)

export function statsChanged() {
  if (tab === 'info' && current()) renderResources(current());
}

// Courbe des derniers relevés, calée à droite ; null : pas de mesure (premier relevé).
function sparkline(values, max) {
  const svg = document.createElementNS(SVG, 'svg');
  svg.setAttribute('viewBox', '0 0 240 40');
  svg.setAttribute('preserveAspectRatio', 'none');
  svg.setAttribute('aria-hidden', 'true');
  const step = 240 / (POINTS - 1);
  let pts = values
    .map((v, i) => (v === null ? null : [240 - (values.length - 1 - i) * step, 39 - (v / max) * 36]))
    .filter(Boolean);
  if (pts.length === 1) pts = [[pts[0][0] - step, pts[0][1]], pts[0]];
  if (!pts.length) return svg;
  const line = pts.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' ');
  const area = document.createElementNS(SVG, 'polygon');
  area.setAttribute('points', `${pts[0][0].toFixed(1)},40 ${line} ${pts.at(-1)[0].toFixed(1)},40`);
  const stroke = document.createElementNS(SVG, 'polyline');
  stroke.setAttribute('points', line);
  svg.append(area, stroke);
  return svg;
}

function resCard(label, value, chart, foot) {
  return h('div', { class: 'res-card' },
    h('div', { class: 'res-head' }, h('span', { text: label }), h('strong', { text: value })),
    chart,
    h('p', { class: 'res-foot', text: foot }));
}

function renderResources(p) {
  const box = $('#res');
  const s = ['running', 'starting', 'stopping'].includes(p.state) ? stats.get(p.id) : null;
  box.hidden = !s;
  if (!s) return box.replaceChildren();
  if (s.inProcess) {
    return box.replaceChildren(h('p', { class: 'res-note', text: t('res.builtInNote') }));
  }
  const list = stats.historyOf(p.id);
  const cpus = list.map((e) => e.cpu);
  const mems = list.map((e) => e.memory);
  const peakCpu = Math.max(0, ...cpus.filter((v) => v !== null));
  const peakMem = Math.max(0, ...mems);
  box.replaceChildren(
    resCard(t('res.cpu'), formatCpu(s.cpu), sparkline(cpus, Math.min(100, Math.max(5, peakCpu * 1.2))), t('res.max', { value: formatCpu(peakCpu) })),
    resCard(t('res.memory'), formatBytes(s.memory), sparkline(mems, peakMem * 1.2 || 1), t('res.max', { value: formatBytes(peakMem) })),
    h('p', { class: 'res-note', text: tn('res.note', s.processes) }));
}

// ---- Infos ----

async function renderInfo(p, fresh = false) {
  if (!p) return;
  renderResources(p);
  const url = urlOf(p);
  const tags = p.tags.length ? p.tags.join(', ') : '—';
  if (p.type === 'file') {
    return renderRows(p, [[t('info.file'), p.file], [t('info.opening'), t('info.openingValue')], [t('info.folder'), p.cwd], [t('info.tags'), tags]], fresh);
  }
  const net = exposure(p);
  const env = Object.entries(p.env ?? {}).map(([k, v]) => `${k}=${v}`).join(' ');
  const rows = [
    [t('col.address'), url ? h('a', { href: url, target: '_blank', rel: 'noopener', text: url }) : '—'],
    [t('info.listen'), net.exposed
      ? h('span', { class: 'exposed', text: t('info.listenExposed', { address: net.address, reachable: reachableAt(p.activePort ?? p.port) }) })
      : t('info.listenLocal', { address: net.address })],
    [t('info.command'), h('code', { text: p.command })],
    ...(env ? [[t('info.env'), h('code', { text: env })]] : []),
    [t('info.folder'), p.cwd],
    [t('info.process'), p.managed ? (p.pid ? t('info.processPid', { pid: p.pid }) : t('info.processOwn')) : p.state === 'running' ? t('info.processExternal') : '—'],
    [t('info.started'), p.startedAt ? t('info.startedValue', { ago: ago(p.startedAt), uptime: uptime(p.startedAt) }) : '—'],
    [t('info.exitCode'), p.exitCode ?? '—'],
    [t('info.autostart'), p.autostart ? t('info.autostartYes') : t('common.no')],
    [t('info.requires'), p.requires.length ? p.requires.map((id) => handlers.get(id)?.name ?? id).join(', ') : '—'],
    [t('info.requiredBy'), handlers.all().filter((x) => x.requires.includes(p.id)).map((x) => x.name).join(', ') || '—'],
    ...(p.dependency && p.state === 'running' ? [[t('info.launch'), t('info.asDependency')]] : []),
    [t('info.tags'), tags],
  ];
  return renderRows(p, rows, fresh);
}

async function renderRows(p, rows, fresh) {
  let g = handlers.git()[p.id];
  if (fresh) g = (await api('git?fresh').catch(() => ({})))[p.id] ?? g;
  if (g?.repo) {
    const branch = g.branch ?? t('git.detached');
    rows.push([t('info.branch'), g.upstream ? branch : t('info.branchNoRemote', { branch })]);
    rows.push([t('info.changes'), g.changes ? tn('info.changesValue', g.changes) : t('info.changesNone')]);
    if (g.ahead || g.behind) rows.push([t('info.sync'), t('info.syncValue', { ahead: g.ahead, behind: g.behind })]);
    if (g.last) rows.push([t('info.lastCommit'), `${g.last.hash} · ${g.last.subject} · ${ago(g.last.date)}`]);
  } else if (g) {
    rows.push(['Git', t('info.notVersioned')]);
  }
  if (p.id !== currentId) return;
  $('#info').replaceChildren(...rows.flatMap(([k, v]) => [h('dt', { text: k }), h('dd', {}, typeof v === 'object' ? v : String(v))]));
}

// ---- Historique ----

const label = (s) => (STATE_LABELS[s] ?? s).replace('…', '');

export function transition(e) {
  return [
    h('span', { class: 'status', dataset: { state: e.from } }, h('i', { class: 'dot' }), label(e.from)),
    h('span', { class: 'arrow', 'aria-label': t('history.then') }, icon('arrow-right')),
    h('span', { class: 'status', dataset: { state: e.to } }, h('i', { class: 'dot' }), label(e.to)),
  ];
}

async function renderHistory(p) {
  if (!p || p.type === 'file') return;
  shownState = p.state;
  const { entries } = await api(`projects/${encodeURIComponent(p.id)}/history`).catch(() => ({ entries: [] }));
  if (p.id !== currentId) return;
  $('#history').replaceChildren(
    ...entries.map((e) =>
      h('li', { class: e.kind === 'audit' ? 'audit' : '' },
        h('time', { datetime: e.at, title: new Date(e.at).toLocaleString(lang), text: ago(Date.parse(e.at)) }),
        h('span', { class: 'what' }, ...(e.kind === 'audit' ? [auditLine(e)] : transition(e))),
        e.detail ? h('span', { class: 'detail', text: e.detail }) : null)),
  );
  $('#history-empty').hidden = entries.length > 0;
}
