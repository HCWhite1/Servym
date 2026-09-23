// Page Activité : changements d'état de tous les projets (lus dans servynex.log), groupés par jour.
import { api, h, initials, tile } from './lib.js';
import { transition } from './panel.js';

const $ = (sel) => document.querySelector(sel);
const view = $('#view-activity');
const select = $('#activity-filter');

let handlers;
let entries = [];
let reloadTimer = null;

export function init(hs) {
  handlers = hs;
  select.addEventListener('change', render);
  $('#activity-failures').addEventListener('change', render);
  $('#activity-steps').addEventListener('change', render);
  $('#activity-audit').addEventListener('change', render);
}

// Liste des projets du filtre (après chaque changement de configuration).
export function build() {
  const value = select.value;
  select.replaceChildren(
    h('option', { value: '', text: 'Tous les projets' }),
    ...handlers.all().filter((p) => p.type !== 'file').map((p) => h('option', { value: p.id, text: p.name })));
  select.value = handlers.get(value) ? value : '';
}

// « [AM] alice · Réglages modifiés : port » (compte absent : protection désactivée).
export function auditLine(e) {
  return h('span', { class: 'audit-line' },
    e.user ? h('span', { class: 'avatar xs', text: initials({ username: e.user }) }) : null,
    e.user ? h('strong', { text: e.user }) : null,
    h('span', { text: e.text }));
}

export function show() {
  load();
}

// Un projet a changé d'état : nouvelle ligne dans servynex.log.
export function changed() {
  if (view.hidden) return;
  clearTimeout(reloadTimer);
  reloadTimer = setTimeout(load, 400);
}

async function load() {
  try {
    ({ entries } = await api('history'));
  } catch {
    entries = [];
  }
  render();
}

const dayFormat = new Intl.DateTimeFormat('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' });
const timeFormat = new Intl.DateTimeFormat('fr-FR', { hour: '2-digit', minute: '2-digit' });

function dayLabel(date) {
  const start = (d) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const diff = Math.round((start(new Date()) - start(date)) / 86_400_000);
  if (diff === 0) return 'Aujourd’hui';
  if (diff === 1) return 'Hier';
  const text = dayFormat.format(date);
  return text[0].toUpperCase() + text.slice(1);
}

function render() {
  const project = select.value;
  const failures = $('#activity-failures').checked;
  // « Démarrage… » et « Arrêt… » précèdent de quelques secondes l'état qui compte : masqués par défaut.
  const steps = $('#activity-steps').checked;
  const audit = $('#activity-audit').checked;
  const list = entries.filter((e) => (!project || e.id === project)
    && (audit || e.kind !== 'audit')
    && (!failures || ['failed', 'conflict'].includes(e.to))
    && (steps || !['starting', 'stopping'].includes(e.to)));
  if (!list.length) {
    $('#activity').replaceChildren(h('div', { class: 'card empty', text: entries.length ? 'Aucun évènement pour ce filtre.' : 'Aucune activité enregistrée pour l’instant.' }));
    return;
  }
  const groups = new Map();
  for (const e of list) {
    const label = dayLabel(new Date(e.at));
    if (!groups.has(label)) groups.set(label, []);
    groups.get(label).push(e);
  }
  $('#activity').replaceChildren(...[...groups].flatMap(([label, items]) => [
    h('h2', { class: 'day', text: label }),
    h('div', { class: 'card card-pad' }, h('ol', { class: 'timeline activity' }, ...items.map(item))),
  ]));
}

function item(e) {
  const p = handlers.get(e.id);
  const date = new Date(e.at);
  const who = p
    ? h('button', { class: 'who', type: 'button', title: `Ouvrir ${p.name}` }, tile(p, 'sm'), p.name)
    : h('span', { class: 'who', text: e.id });
  if (p) who.addEventListener('click', () => handlers.openPanel(p.id, 'history'));
  const time = h('time', { datetime: e.at, title: date.toLocaleString('fr-FR'), text: timeFormat.format(date) });
  if (e.kind === 'audit') {
    return h('li', { class: 'audit' }, time,
      h('span', { class: 'what' }, e.id ? who : null, auditLine(e)));
  }
  return h('li', {}, time,
    h('span', { class: 'what' }, who, ...transition(e)),
    e.detail ? h('span', { class: 'detail', text: e.detail }) : null);
}
