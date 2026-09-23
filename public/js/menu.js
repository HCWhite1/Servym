// Menu « … » d'une ligne : toutes les actions d'un projet (popover natif).
import { t } from './i18n.js';
import { can, canRestart, canStop, h, icon, primaryOf } from './lib.js';

const menu = document.getElementById('row-menu');
let handlers;
let currentId = null;
let anchor = null;
let closed = { id: null, at: 0 };

export function init(hs) {
  handlers = hs;
  menu.addEventListener('toggle', (e) => {
    if (e.newState !== 'closed') return;
    // Un clic sur « … » ferme d'abord le menu (clic extérieur) : ne pas le rouvrir aussitôt.
    closed = { id: currentId, at: performance.now() };
    anchor?.setAttribute('aria-expanded', 'false');
    currentId = null;
    anchor = null;
  });
  menu.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const items = [...menu.querySelectorAll('button:not(:disabled)')];
    const i = items.indexOf(document.activeElement);
    items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length]?.focus();
  });
  addEventListener('resize', () => menu.matches(':popover-open') && menu.hidePopover());
  addEventListener('scroll', () => menu.matches(':popover-open') && menu.hidePopover(), { passive: true });
}

export function open(button, p) {
  if (closed.id === p.id && performance.now() - closed.at < 250) return;
  currentId = p.id;
  anchor = button;
  render(p);
  menu.showPopover();
  place(button);
  button.setAttribute('aria-expanded', 'true');
  menu.querySelector('button:not(:disabled)')?.focus();
}

// Re-rendu seulement si les actions possibles changent, en gardant l'élément
// qui avait le focus (navigation au clavier).
let shape = '';
export function update(p) {
  if (p.id !== currentId || shapeOf(p) === shape) return;
  const focused = document.activeElement?.closest?.('#row-menu button')?.textContent;
  render(p);
  if (focused) [...menu.querySelectorAll('button')].find((b) => b.textContent === focused)?.focus();
}

const shapeOf = (p) => [p.state, p.managed, p.url].join('|');

function item(label, name, onClick, { danger = false, disabled = false } = {}) {
  const button = h('button', { type: 'button', role: 'menuitem', class: danger ? 'danger' : '', disabled }, icon(name), label);
  button.addEventListener('click', () => {
    menu.hidePopover();
    onClick(button);
  });
  return button;
}

function render(p) {
  shape = shapeOf(p);
  const file = p.type === 'file';
  const main = primaryOf(p);
  const tools = handlers.tools();
  const items = [];
  const operate = can('operate');
  const viewOnly = p.state === 'running' && !file && !p.service;
  if (!main.hidden && (operate || viewOnly)) items.push(item(main.label, main.icon ?? 'play', () => handlers.primary(p), { disabled: main.disabled }));
  if (operate && !file && canRestart(p)) items.push(item(t('action.restart'), 'rotate-cw', () => handlers.run(p, 'restart')));
  if (operate && !file && canStop(p)) items.push(item(t('action.stop'), 'square', () => handlers.run(p, 'stop'), { danger: true }));
  if (items.length) items.push(h('hr'));
  if (!file) {
    items.push(item(t('panel.logs'), 'scroll-text', () => handlers.openPanel(p.id, 'logs')));
    items.push(item(t('panel.history'), 'history', () => handlers.openPanel(p.id, 'history')));
  }
  items.push(item(t('panel.settings'), 'sliders-horizontal', () => handlers.openPanel(p.id, 'settings')));
  items.push(item(t('panel.info'), 'info', () => handlers.openPanel(p.id, 'info')));
  if (operate) {
    items.push(h('hr'));
    items.push(item(t('action.folder'), 'folder-open', () => handlers.run(p, 'folder')));
    if (tools.code) items.push(item(t('action.code'), 'code-xml', () => handlers.run(p, 'code')));
    items.push(item(tools.terminal ? t('menu.openTerminal', { terminal: tools.terminal }) : t('menu.openATerminal'), 'square-terminal', () => handlers.run(p, 'terminal')));
  }
  if (can('commands')) {
    items.push(h('hr'));
    items.push(item(t('menu.delete'), 'trash-2', () => handlers.remove(p), { danger: true }));
  }
  menu.replaceChildren(...items);
}

// Sous le bouton, aligné à droite ; au-dessus s'il n'y a pas la place.
function place(button) {
  const r = button.getBoundingClientRect();
  const { offsetWidth: w, offsetHeight: hgt } = menu;
  const left = Math.max(8, Math.min(r.right - w, innerWidth - w - 8));
  const below = r.bottom + 4 + hgt <= innerHeight - 8;
  menu.style.left = `${left}px`;
  menu.style.top = `${below ? r.bottom + 4 : Math.max(8, r.top - hgt - 4)}px`;
}
