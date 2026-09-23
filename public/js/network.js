// Page Réseau : réseaux connectés, ports en écoute sur ce PC (projets de Servynex et
// reste de la machine), et détection d'un projet exposé sur un réseau public.
import { api, exposure, h, icon, tile, toast } from './lib.js';

const $ = (sel) => document.querySelector(sel);
// Plage dynamique de Windows (RPC, services système) : masquée par défaut.
const DYNAMIC = 49152;

let handlers;
let data = null;
let networks = [];
let loading = false;

export function init(hs) {
  handlers = hs;
  $('#ports-refresh').addEventListener('click', load);
  $('#ports-dynamic').addEventListener('change', render);
}

export function show() {
  if (!data) $('#ports-rows').replaceChildren(h('tr', {}, h('td', { colspan: '5', text: 'Lecture des ports…' })));
  load();
}

async function load() {
  if (loading) return;
  loading = true;
  $('#ports-refresh').disabled = true;
  try {
    data = await api('ports');
    networks = data.networks;
    render();
    handlers.renderBanners();
  } catch (err) {
    toast(err.message, 'error');
  } finally {
    loading = false;
    $('#ports-refresh').disabled = false;
  }
}

function render() {
  if (!data) return;
  const all = $('#ports-dynamic').checked;
  const rows = data.ports.filter((r) => all || r.port < DYNAMIC || r.project);
  const exposed = rows.filter((r) => r.exposed).length;
  $('#ports-summary').textContent = `${rows.length} port${rows.length > 1 ? 's' : ''} en écoute · ${exposed} joignable${exposed > 1 ? 's' : ''} depuis le réseau`;
  $('#networks').replaceChildren(...networkCards());
  $('#ports-rows').replaceChildren(...rows.map(row));
}

function row(r) {
  const p = r.project ? handlers.get(r.project) : null;
  const owner = r.port === data.instance
    ? h('span', { class: 'owner', text: 'Servynex' })
    : p ? h('span', { class: 'owner' }, tile(p, 'sm'), p.name) : '';
  return h('tr', { class: r.exposed ? 'exposed' : '' },
    h('td', { class: 'num', text: String(r.port) }),
    h('td', {}, h('span', { class: 'mono', text: r.addresses.join(', ') }), r.exposed ? h('span', { class: 'badge warn', text: 'Réseau' }) : null),
    h('td', { text: r.process ?? '?' }),
    h('td', { class: 'num', text: String(r.pid) }),
    h('td', {}, owner));
}

function networkCards() {
  const cards = networks.map((n) => {
    const pub = n.category === 'public';
    return h('div', { class: 'card net-card' },
      h('h3', {}, icon(/wi-?fi|wlan|sans fil/i.test(n.interface) ? 'radio-tower' : 'network'), n.name,
        h('span', { class: `badge${pub ? ' warn' : ''}`, text: `Réseau ${n.category}` })),
      h('p', { text: n.interface }),
      pub ? h('p', { class: 'hint warn', text: 'Non déclaré de confiance dans Windows : un projet exposé y est joignable par tous.' }) : null);
  });
  const lan = data?.lan ?? [];
  if (lan.length) {
    cards.push(h('div', { class: 'card net-card' },
      h('h3', {}, icon('globe'), 'Adresses de ce PC'),
      ...lan.map((a) => h('p', {}, h('span', { class: 'mono', text: a.address }), ` · ${a.name}`))));
  }
  if (!cards.length) cards.push(h('div', { class: 'card net-card' }, h('p', { text: 'Aucun réseau connecté.' })));
  return cards;
}

// ---- Avertissement : projet exposé alors qu'un réseau connecté est public ----

export async function refreshNetwork() {
  try {
    networks = (await api('network')).networks;
  } catch {}
}

export function publicExposure() {
  const projects = handlers.all().filter((p) => p.type !== 'file' && ['running', 'starting'].includes(p.state) && exposure(p).exposed);
  const pub = networks.filter((n) => n.category === 'public');
  return projects.length && pub.length ? { projects, networks: pub } : null;
}
