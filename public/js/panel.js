// Volet d'un projet : journal en direct, historique, réglages, infos.
import { STATE_LABELS, ago, api, can, canRestart, canStop, confirmDialog, exposure, fillTile, h, icon, isExternal, primaryOf, safeUrl, statusText, toast, uptime } from './lib.js';
import { ERROR_LINE, WARN_LINE } from './log-patterns.js';
import { createForm } from './project-form.js';
import { auditLine } from './activity.js';

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
  $('#log-copy').addEventListener('click', async () => {
    await navigator.clipboard.writeText(logBox.textContent);
    toast('Journal copié');
  });
  form = createForm({ handlers, submitLabel: 'Enregistrer', onSubmit: save });
  const remove = h('button', { class: 'btn danger', type: 'button' }, icon('trash-2'), 'Supprimer');
  remove.addEventListener('click', () => current() && handlers.remove(current()));
  dangerZone = h('div', { class: 'danger-zone' },
    h('div', { class: 'setting-text' },
      h('p', { class: 'setting-label', text: 'Supprimer ce projet' }),
      h('p', { class: 'setting-desc', text: 'Arrêté s’il tourne, puis retiré de projects.json et des dépendances des autres projets. Ses journaux sont conservés.' })),
    remove);
  $('#pane-settings').replaceChildren(form.element, dangerZone);
}

async function save(body, p) {
  if (body.expose && !p.source.expose) {
    const port = body.port ?? p.port;
    const ok = await confirmDialog({
      title: `Exposer ${p.name} sur le réseau ?`,
      body: `${p.name} écoutera sur 0.0.0.0:${port} : toute machine du réseau actuel pourra s’y connecter${reachableAt(port)}.`,
      confirm: 'Exposer',
      danger: true,
    });
    if (!ok) throw new Error('Rien n’a été enregistré.');
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
  status.replaceChildren(h('i', { class: 'dot' }), [statusText(p), isExternal(p) ? 'lancé hors de Servynex' : null, where].filter(Boolean).join(' · '));
  const error = $('#panel-error');
  error.textContent = p.error ?? '';
  error.hidden = !p.error;
  dangerZone.hidden = !can('commands');

  const main = $('#panel-actions [data-act="main"]');
  const { label, icon: name, disabled, primary, hidden } = primaryOf(p);
  main.replaceChildren(...(name ? [icon(name)] : []), label);
  main.disabled = Boolean(disabled) || p.state === 'conflict';
  main.hidden = Boolean(hidden) || (!can('operate') && !(p.state === 'running' && p.type !== 'file' && !p.service));
  main.classList.toggle('primary', Boolean(primary));
  const file = p.type === 'file';
  $('#panel-actions [data-act="stop"]').hidden = file || !canStop(p) || !can('operate');
  $('#panel-actions [data-act="restart"]').hidden = file || !canRestart(p) || !can('operate');
  for (const act of ['folder', 'terminal']) $(`#panel-actions [data-act="${act}"]`).hidden = !can('operate');
  $('#tab-logs').hidden = file;
  $('#tab-history').hidden = file;
  $('#panel-actions [data-act="code"]').hidden = !handlers.tools().code || !can('operate');
  $('#panel-actions [data-act="terminal"]').title = `${handlers.tools().terminal ?? 'Terminal'} dans ce dossier`;
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
  if (isExternal(p) && p.hasLog) text = 'Lancé en dehors de Servynex (tâche planifiée, .bat…) : sa sortie actuelle n’est pas visible ici. Le journal ci-dessous date du dernier lancement par Servynex.';
  else if (isExternal(p)) text = 'Lancé en dehors de Servynex (tâche planifiée, .bat…) : sa sortie n’est pas visible ici.';
  else if (!p.hasLog) text = 'Aucun journal pour l’instant : il apparaîtra au premier lancement depuis Servynex.';
  note.textContent = text;
  note.hidden = !text;
}

function lineNode(line) {
  const cls = line.startsWith('[servynex]') ? 'own' : ERROR_LINE.test(line) ? 'err' : WARN_LINE.test(line) ? 'warn' : '';
  return h('span', cls ? { class: cls } : {}, `${line}\n`);
}

function appendLines(lines, reset) {
  const follow = $('#log-follow').checked;
  if (reset) logBox.replaceChildren();
  logBox.append(...lines.map(lineNode));
  while (logBox.childElementCount > MAX_LINES) logBox.firstElementChild.remove();
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
  return urls.length ? `, par exemple ${urls.join(', ')}` : '';
}

// ---- Infos ----

async function renderInfo(p, fresh = false) {
  if (!p) return;
  const url = safeUrl(p.url);
  const tags = p.tags.length ? p.tags.join(', ') : '—';
  if (p.type === 'file') {
    return renderRows(p, [['Fichier', p.file], ['Ouverture', 'Dans le navigateur par défaut, sans serveur'], ['Dossier', p.cwd], ['Étiquettes', tags]], fresh);
  }
  const net = exposure(p);
  const env = Object.entries(p.env ?? {}).map(([k, v]) => `${k}=${v}`).join(' ');
  const rows = [
    ['Adresse', url ? h('a', { href: url, target: '_blank', rel: 'noopener', text: url }) : '—'],
    ['Écoute', net.exposed
      ? h('span', { class: 'exposed', text: `${net.address} — joignable depuis le réseau${reachableAt(p.activePort ?? p.port)}` })
      : `${net.address} (ce PC seulement)`],
    ['Commande', h('code', { text: p.command })],
    ...(env ? [['Environnement', h('code', { text: env })]] : []),
    ['Dossier', p.cwd],
    ['Processus', p.managed ? (p.pid ? `PID ${p.pid} · lancé par Servynex` : 'Lancé par Servynex') : p.state === 'running' ? 'Lancé en dehors de Servynex' : '—'],
    ['Démarré', p.startedAt ? `${ago(p.startedAt)} (depuis ${uptime(p.startedAt)})` : '—'],
    ['Dernier code', p.exitCode ?? '—'],
    ['Démarrage auto', p.autostart ? 'Oui, à l’ouverture de session' : 'Non'],
    ['Dépend de', p.requires.length ? p.requires.map((id) => handlers.get(id)?.name ?? id).join(', ') : '—'],
    ['Requis par', handlers.all().filter((x) => x.requires.includes(p.id)).map((x) => x.name).join(', ') || '—'],
    ...(p.dependency && p.state === 'running' ? [['Lancement', 'Démarré automatiquement comme dépendance : s’arrêtera avec le dernier projet qui l’utilise']] : []),
    ['Étiquettes', tags],
  ];
  return renderRows(p, rows, fresh);
}

async function renderRows(p, rows, fresh) {
  let g = handlers.git()[p.id];
  if (fresh) g = (await api('git?fresh').catch(() => ({})))[p.id] ?? g;
  if (g?.repo) {
    rows.push(['Branche', `${g.branch ?? 'détachée'}${g.upstream ? '' : ' (aucun dépôt distant)'}`]);
    rows.push(['Modifications', g.changes ? `${g.changes} fichier(s) non commité(s)` : 'Aucune']);
    if (g.ahead || g.behind) rows.push(['Synchro', `${g.ahead} commit(s) à pousser, ${g.behind} à récupérer`]);
    if (g.last) rows.push(['Dernier commit', `${g.last.hash} · ${g.last.subject} · ${ago(g.last.date)}`]);
  } else if (g) {
    rows.push(['Git', 'Dossier non versionné']);
  }
  if (p.id !== currentId) return;
  $('#info').replaceChildren(...rows.flatMap(([k, v]) => [h('dt', { text: k }), h('dd', {}, typeof v === 'object' ? v : String(v))]));
}

// ---- Historique ----

const label = (s) => (STATE_LABELS[s] ?? s).replace('…', '');

export function transition(e) {
  return [
    h('span', { class: 'status', dataset: { state: e.from } }, h('i', { class: 'dot' }), label(e.from)),
    h('span', { class: 'arrow', 'aria-label': 'puis' }, icon('arrow-right')),
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
        h('time', { datetime: e.at, title: new Date(e.at).toLocaleString('fr-FR'), text: ago(Date.parse(e.at)) }),
        h('span', { class: 'what' }, ...(e.kind === 'audit' ? [auditLine(e)] : transition(e))),
        e.detail ? h('span', { class: 'detail', text: e.detail }) : null)),
  );
  $('#history-empty').hidden = entries.length > 0;
}
