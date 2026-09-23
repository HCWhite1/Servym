// Paramètres : démarrage, apparence, fichier de configuration, Servynex, raccourcis.
import { renderAccount, renderMembers, renderSecurity } from './accounts.js';
import { api, applyTheme, can, confirmDialog, h, session, store, tile, toast } from './lib.js';

const $ = (sel) => document.querySelector(sel);
const dialog = $('#settings');
const TITLES = { account: 'Mon compte', general: 'Général', appearance: 'Apparence', config: 'Configuration', security: 'Sécurité', members: 'Membres', instance: 'Servynex', keys: 'Raccourcis clavier' };
const RENDER = { account: renderAccount, security: renderSecurity, members: renderMembers };

let handlers;

export function init(hs) {
  handlers = hs;
  dialog.addEventListener('click', (e) => {
    if (e.target === dialog || e.target.closest('[data-close]')) dialog.close();
  });
  for (const button of dialog.querySelectorAll('.settings-nav [data-section]')) {
    button.addEventListener('click', () => showSection(button.dataset.section));
  }
  $('#set-autostart').addEventListener('change', (e) => setInstanceAutostart(e.target.checked, e.target));
  $('#set-projects').addEventListener('change', (e) => setProjectAutostart(e.target.dataset.id, e.target.checked, e.target));
  radio('#set-theme', setTheme);
  radio('#set-layout', (value) => handlers.setLayout(value));
  $('#set-motion').addEventListener('change', (e) => handlers.setMotion(e.target.checked));
  $('#open-config').addEventListener('click', () => instanceAction('config'));
  $('#open-instance-log').addEventListener('click', () => instanceAction('log'));
  $('#open-instance-dir').addEventListener('click', () => instanceAction('folder'));
  $('#reload-config').addEventListener('click', reloadConfig);
  $('#restart-instance').addEventListener('click', restartInstance);
  $('#instance-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      const { name } = await api('instance', { method: 'PATCH', body: { name: $('#instance-name').value.trim() || null } });
      handlers.instance().name = name;
      document.dispatchEvent(new CustomEvent('servynex:user'));
      toast('Nom enregistré');
    } catch (err) {
      toast(err.message, 'error');
    }
  });
  $('#replay-onboarding').addEventListener('click', () => {
    dialog.close();
    handlers.onboarding();
  });
}

function radio(sel, onPick) {
  $(sel).addEventListener('click', (e) => {
    const button = e.target.closest('[data-value]');
    if (button) onPick(button.dataset.value);
  });
}

function check(sel, value) {
  for (const b of $(sel).querySelectorAll('[data-value]')) b.setAttribute('aria-checked', String(b.dataset.value === value));
}

export function open(section = 'general') {
  sync();
  fillInstance();
  applyPermissions();
  loadAutostart();
  showSection(section);
  if (!dialog.open) dialog.showModal();
}

function showSection(name) {
  for (const s of dialog.querySelectorAll('.modal-body > [data-section]')) s.hidden = s.dataset.section !== name;
  for (const b of dialog.querySelectorAll('.settings-nav [data-section]')) {
    if (b.dataset.section === name) b.setAttribute('aria-current', 'page');
    else b.removeAttribute('aria-current');
  }
  $('#settings-heading').textContent = TITLES[name];
  RENDER[name]?.().catch((err) => toast(err.message, 'error'));
}

// Ce que le compte connecté peut toucher (protection désactivée : tout).
function applyPermissions() {
  $('.settings-nav [data-section="account"]').hidden = !session.user;
  $('.settings-nav [data-section="members"]').hidden = !(session.user && can('admin'));
  const off = (sel, perm, why) => {
    const node = $(sel);
    node.disabled = !can(perm) || node.disabled;
    if (!can(perm)) node.title = why;
  };
  off('#open-config', 'commands', 'Permission « Commandes » nécessaire');
  off('#reload-config', 'edit', 'Permission « Modifier les projets » nécessaire');
  off('#open-instance-log', 'instance', 'Permission « Servynex » nécessaire');
  off('#open-instance-dir', 'instance', 'Permission « Servynex » nécessaire');
  $('#instance-name').disabled = !can('instance');
  $('#instance-form button').disabled = !can('instance');
  for (const input of document.querySelectorAll('#set-projects input')) input.disabled = !can('edit');
}

// Reflète les préférences actuelles (appelé aussi quand elles changent ailleurs).
export function sync() {
  const prefs = handlers.prefs();
  check('#set-theme', store.get('theme', 'system'));
  check('#set-layout', prefs.layout);
  $('#set-motion').checked = prefs.motion;
}

export function setTheme(value) {
  store.set('theme', value);
  applyTheme(value);
  check('#set-theme', value);
  savePref({ theme: value });
}

// Préférences enregistrées avec le compte quand la protection est active.
export function savePref(prefs) {
  if (!session.user) return;
  Object.assign(session.user.prefs, prefs);
  api('auth/me', { method: 'PATCH', body: { prefs } }).catch(() => {});
}

// ---- Démarrage avec Windows ----

export async function loadAutostart() {
  const input = $('#set-autostart');
  try {
    const { enabled } = await api('autostart');
    input.checked = enabled;
    input.disabled = !can('instance');
    $('#set-projects-warn').hidden = enabled;
    return enabled;
  } catch (err) {
    $('#autostart-desc').textContent = `État de la tâche planifiée illisible : ${err.message}`;
    return null;
  }
}

export async function setInstanceAutostart(on, input) {
  if (input) input.disabled = true;
  try {
    const { enabled } = await api('autostart', { method: 'PUT', body: { enabled: on } });
    toast(enabled ? 'Servynex démarrera avec Windows.' : 'Servynex ne démarrera plus avec Windows.');
    for (const node of document.querySelectorAll('#set-autostart, #ob-autostart')) node.checked = enabled;
    $('#set-projects-warn').hidden = enabled;
    return enabled;
  } catch (err) {
    toast(err.message, 'error');
    if (input) input.checked = !on;
    return null;
  } finally {
    if (input) input.disabled = !can('instance');
  }
}

// ---- Projets lancés au démarrage ----

export async function setProjectAutostart(id, on, input) {
  if (!id) return;
  if (input) input.disabled = true;
  try {
    await api(`projects/${encodeURIComponent(id)}`, { method: 'PATCH', body: { autostart: on } });
  } catch (err) {
    toast(err.message, 'error');
    if (input) input.checked = !on;
  } finally {
    if (input) input.disabled = !can('edit');
  }
}

// Liste d'interrupteurs, un par projet lançable (ici et dans la présentation).
export function projectSwitches() {
  return handlers.all().filter((p) => p.type !== 'file').map((p) =>
    h('label', { class: 'sub-item' },
      tile(p, 'sm'),
      h('span', { text: p.name }),
      h('input', { class: 'switch', type: 'checkbox', role: 'switch', 'aria-label': `Lancer ${p.name} au démarrage`, dataset: { id: p.id }, checked: p.autostart })));
}

export function build() {
  $('#set-projects').replaceChildren(...projectSwitches());
}

export function update(p) {
  for (const input of document.querySelectorAll(`.sub-item input[data-id="${CSS.escape(p.id)}"]`)) input.checked = p.autostart;
}

// ---- Configuration et Servynex ----

async function instanceAction(action) {
  try {
    await api(`instance/${action}`, { method: 'POST' });
  } catch (err) {
    toast(err.message, 'error');
  }
}

async function reloadConfig() {
  try {
    await api('reload', { method: 'POST' });
    toast('projects.json relu');
    handlers.refreshGit();
  } catch (err) {
    toast(err.message, 'error');
  }
}

function fillInstance() {
  const info = handlers.instance();
  $('#config-path').textContent = info.config ?? '';
  $('#instance-name').value = info.name ?? '';
  $('#instance-dir').textContent = info.dir ?? '';
  const rows = [
    ['Version', info.version ?? '—'],
    ['Adresse', `http://localhost:${info.port}`],
    ['Node.js', info.node ?? '—'],
    ['Lancement', info.restartable ? 'Tâche planifiée « Servynex » (sans fenêtre)' : 'À la main (npm start)'],
    ['Outils', [handlers.tools().code ? 'VS Code' : null, handlers.tools().terminal].filter(Boolean).join(', ')],
  ];
  $('#instance-info').replaceChildren(...rows.flatMap(([k, v]) => [h('dt', { text: k }), h('dd', { text: v })]));
  $('#restart-instance').disabled = !info.restartable || !can('instance');
  $('#restart-desc').textContent = info.restartable
    ? 'Les projets lancés continuent de tourner et sont repris au redémarrage.'
    : 'Indisponible : Servynex a été lancé à la main (npm start). Le relancer depuis son terminal.';
}

// Les projets continuent de tourner et sont repris. La page attend que l'ancien
// Servynex soit tombé puis que le nouveau réponde, et se recharge (l'interface a pu changer).
async function restartInstance() {
  const ok = await confirmDialog({
    title: 'Redémarrer Servynex ?',
    body: 'Les projets lancés continuent de tourner et seront repris. La page se rechargera toute seule.',
    confirm: 'Redémarrer',
  });
  if (!ok) return;
  const button = $('#restart-instance');
  button.disabled = true;
  try {
    await api('restart', { method: 'POST' });
  } catch (err) {
    button.disabled = false;
    return toast(err.message, 'error');
  }
  toast('Redémarrage de Servynex…');
  const up = () => fetch('/api/projects', { cache: 'no-store' }).then((r) => r.ok, () => false);
  const until = async (want, ms) => {
    for (const end = Date.now() + ms; Date.now() < end; await new Promise((r) => setTimeout(r, 400))) {
      if ((await up()) === want) return true;
    }
    return false;
  };
  await until(false, 5_000);
  if (await until(true, 30_000)) location.reload();
  else toast('Servynex ne répond pas après 30 s : voir logs/servynex.log.', 'error');
}
