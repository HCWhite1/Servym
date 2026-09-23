// Onglet d'attente : ouvert au clic sur « Démarrer », il suit le démarrage et
// redirige vers le projet dès qu'il répond.
import { api, fillTile, safeUrl, statusText } from './lib.js';

const $ = (sel) => document.querySelector(sel);
const id = new URLSearchParams(location.search).get('id');
let finished = false;

async function showFailure(p) {
  $('#spinner').hidden = true;
  $('#hint').hidden = true;
  $('#error').textContent = p.error ?? 'Le projet s’est arrêté.';
  $('#error').hidden = false;
  const { lines } = await api(`projects/${encodeURIComponent(id)}/logs?tail`).catch(() => ({ lines: [] }));
  if (lines.length) {
    $('#log').textContent = lines.join('\n');
    $('#log').hidden = false;
  }
}

function render(p) {
  if (finished || p.id !== id) return;
  fillTile($('#tile'), p);
  $('#name').textContent = p.name;
  $('#status').dataset.state = p.state;
  $('#status-text').textContent = statusText(p);
  document.title = `${p.name} · ${statusText(p)}`;
  const url = safeUrl(p.url);
  if (p.state === 'running' && url) {
    finished = true;
    window.opener = null;
    location.replace(url);
  } else if (p.state === 'failed' || p.state === 'conflict') {
    finished = true;
    events.close();
    showFailure(p);
  }
}

const events = new EventSource('/api/events');
events.addEventListener('snapshot', (e) => {
  const p = JSON.parse(e.data).find((x) => x.id === id);
  if (p) render(p);
  else $('#status-text').textContent = 'Projet inconnu';
});
events.addEventListener('state', (e) => render(JSON.parse(e.data)));
events.addEventListener('error', () => {
  if (!finished) $('#status-text').textContent = 'Servynex injoignable, nouvelle tentative…';
});
