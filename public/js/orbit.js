// Vue « orbite » : Servym au centre, chaque application sur son anneau.
// Seuls les projets en ligne tournent ; un service requis (MySQL) est une lune de
// l'application qui en dépend ; un projet exposé sur le réseau porte un halo.
import { t } from './i18n.js';
import { addressOf, glyph, h, icon, primaryOf, statusText, store, tile } from './lib.js';

const root = document.getElementById('orbit-view');
const GOLDEN = 137.5; // angle de départ entre deux planètes : jamais alignées au chargement

let handlers;
let stage;
let sunCount;
let card;
let motionButton;
let selected = null;
let hovered = null;

export function init(hs) {
  handlers = hs;
  sunCount = h('p', { text: '…' });
  motionButton = h('button', { class: 'icon-btn', type: 'button' });
  motionButton.addEventListener('click', () => handlers.setMotion(!handlers.prefs().motion));
  card = h('div', { class: 'orbit-card' });
  stage = h('div', { class: 'orbit-stage' });
  root.append(stage);

  stage.addEventListener('click', (e) => {
    const body = e.target.closest('[data-body]');
    if (body) select(body.dataset.body === selected ? null : body.dataset.body);
    else if (!e.target.closest('.orbit-card, .orbit-tools')) select(null);
  });
  stage.addEventListener('dblclick', (e) => {
    const body = e.target.closest('[data-body]');
    if (body) handlers.openPanel(body.dataset.body);
  });
  stage.addEventListener('pointerover', (e) => hover(e.target.closest('[data-body]')?.dataset.body ?? null));
  stage.addEventListener('pointerleave', () => hover(null));
  new ResizeObserver(fit).observe(stage);
  addEventListener('resize', fit);
}

function select(id) {
  selected = id;
  for (const b of stage.querySelectorAll('[data-body]')) b.setAttribute('aria-pressed', String(b.dataset.body === id));
  renderCard();
}

function hover(id) {
  if (id === hovered) return;
  hovered = id;
  renderCard();
}

// ---- Construction ----

function body(p, cls) {
  return h('button', { class: cls, type: 'button', 'aria-pressed': 'false', dataset: { body: p.id } }, glyph(p));
}

function moon(s, i) {
  const t = 14 + i * 5;
  return h('div', { class: 'moon-wrap', dataset: { id: s.id }, style: { '--t': `${t}s`, '--d': `${-(((i * 180 + 300) % 360) / 360) * t}s` } },
    h('div', { class: 'moon-lane' }, h('div', { class: 'moon-arm' }, h('div', { class: 'moon-upright' }, body(s, 'moon')))));
}

// Période plus longue pour les anneaux extérieurs (à la manière de Kepler).
const period = (i) => Math.round(36 * ((i + 2) / 2) ** 1.5);

function lane(p, i, moons) {
  const t = period(i);
  const angle = (i * GOLDEN + 250) % 360;
  return h('div', { class: 'lane', dataset: { id: p.id, ring: i }, style: { '--t': `${t}s`, '--d': `${-(angle / 360) * t}s` } },
    h('div', { class: 'arm' },
      h('div', { class: 'upright' },
        body(p, 'planet'),
        h('span', { class: 'planet-label', text: p.name }),
        ...moons.map(moon))));
}

export function build() {
  const apps = handlers.apps();
  const moonsOf = new Map();
  const loose = [];
  for (const s of handlers.services()) {
    const host = apps.find((a) => a.requires.includes(s.id));
    if (host) moonsOf.set(host.id, [...(moonsOf.get(host.id) ?? []), s]);
    else loose.push(s);
  }
  const bodies = [...apps, ...loose];
  const sun = h('div', { class: 'sun', 'aria-hidden': 'true' });
  sun.innerHTML = '<svg class="logo" viewBox="0 0 32 32"><use href="icons.svg#servym-logo"/></svg>';
  sun.append(sunCount);
  stage.replaceChildren(
    ...bodies.map((_, i) => h('div', { class: 'ring', dataset: { ring: i } })),
    sun,
    ...bodies.map((p, i) => lane(p, i, moonsOf.get(p.id) ?? [])),
    h('div', { class: 'orbit-tools' }, motionButton),
    card,
  );
  if (!handlers.get(selected)) selected = null;
  fit();
  renderCard();
}

// Anneaux répartis entre le soleil et le bord de la scène.
export function fit() {
  if (!stage || root.hidden) return;
  // Toute la hauteur restante de la fenêtre, sous l'en-tête et les bandeaux.
  const top = stage.getBoundingClientRect().top + scrollY;
  const height = `${Math.max(460, Math.floor(innerHeight - top - 24))}px`;
  if (stage.style.height !== height) stage.style.height = height;
  const rings = stage.querySelectorAll('.ring');
  const size = Math.min(stage.clientWidth, stage.clientHeight);
  const inner = 104;
  const outer = Math.max(inner, size / 2 - 44);
  const step = rings.length > 1 ? (outer - inner) / (rings.length - 1) : 0;
  for (const node of stage.querySelectorAll('.ring, .lane')) {
    node.style.setProperty('--r', `${inner + Number(node.dataset.ring) * step}px`);
  }
}

// ---- État ----

export function update(p) {
  if (!stage) return;
  for (const node of stage.querySelectorAll(`[data-id="${CSS.escape(p.id)}"]`)) node.dataset.state = p.state;
  const b = stage.querySelector(`[data-body="${CSS.escape(p.id)}"]`);
  if (b) {
    b.style.setProperty('--c', p.color);
    b.replaceChildren(glyph(p));
    b.classList.toggle('exposed', addressOf(p).exposed);
    b.setAttribute('aria-label', `${p.name}, ${statusText(p)}`);
    b.title = `${p.name} · ${statusText(p)}`;
    b.closest('.moon-wrap')?.style.setProperty('--c', p.color);
    if (b.classList.contains('planet')) b.parentElement.querySelector('.planet-label').replaceChildren(p.name);
  }
  if (p.id === (hovered ?? selected)) renderCard();
}

export function summary(up, total) {
  sunCount?.replaceChildren(t('orbit.online', { up, total }));
}

export function setMotion(on) {
  if (!stage) return;
  stage.classList.toggle('still', !on);
  motionButton.replaceChildren(icon(on ? 'pause' : 'play'));
  motionButton.title = on ? t('orbit.pause') : t('orbit.play');
  motionButton.setAttribute('aria-label', motionButton.title);
}

export function filter(keep) {
  for (const node of stage?.querySelectorAll('.lane') ?? []) node.classList.toggle('dim', !keep(handlers.get(node.dataset.id)));
}

// ---- Carte de détail (projet survolé ou choisi, sinon la légende) ----

function renderCard() {
  const p = handlers.get(hovered ?? selected);
  if (!p) {
    // Légende repliable ; l'état est gardé par le navigateur.
    const collapsed = store.get('orbitLegend', 'open') === 'closed';
    const item = (cls, text) => h('li', {}, h('i', { class: cls }), text);
    const toggle = h('button', { class: 'legend-title', type: 'button', 'aria-expanded': String(!collapsed) }, t('orbit.legend'), icon('chevron-right'));
    toggle.addEventListener('click', () => {
      store.set('orbitLegend', collapsed ? 'open' : 'closed');
      renderCard();
      card.querySelector('.legend-title').focus();
    });
    card.classList.toggle('collapsed', collapsed);
    card.replaceChildren(
      toggle,
      ...(collapsed
        ? []
        : [
            h('ul', { class: 'legend' },
              item('l-run', t('orbit.run')),
              item('l-off', t('orbit.off')),
              item('l-exp', t('orbit.exposed')),
              item('l-moon', t('orbit.moon'))),
            h('p', { class: 'hint legend-hint', text: t('orbit.hint') }),
          ]));
    return;
  }
  card.classList.remove('collapsed');
  const addr = addressOf(p);
  const main = primaryOf(p);
  const mainButton = h('button', { class: `btn small${main.primary ? ' primary' : ''}`, type: 'button', disabled: main.disabled, hidden: main.hidden }, main.icon ? icon(main.icon) : null, main.label);
  mainButton.addEventListener('click', () => handlers.primary(p, mainButton));
  const details = h('button', { class: 'btn small', type: 'button' }, icon('scroll-text'), t('orbit.details'));
  details.addEventListener('click', () => handlers.openPanel(p.id));
  card.replaceChildren(
    h('div', { class: 'card-top' },
      tile(p),
      h('div', {},
        h('h3', { text: p.name }),
        h('p', { class: 'status', dataset: { state: p.state } }, h('i', { class: 'dot' }), statusText(p)))),
    h('p', { class: `addr mono${addr.exposed ? ' exposed' : ''}`, text: addr.text }),
    h('div', { class: 'card-actions' }, mainButton, details));
}
