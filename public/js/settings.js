// Paramètres : démarrage, apparence, fichier de configuration, Servynex, raccourcis.
import { renderAccount, renderMembers, renderSecurity } from './accounts.js';
import { choice, setChoice, t, tn } from './i18n.js';
import { api, applyTheme, ask, can, confirmDialog, h, session, store, tile, toast, viewer } from './lib.js';
import { publicNetworks } from './network.js';

const $ = (sel) => document.querySelector(sel);
const dialog = $('#settings');
const TITLES = {
  account: t('user.account'), general: t('settings.general'), appearance: t('settings.appearance'), config: t('settings.config'),
  security: t('settings.security'), members: t('settings.members'), instance: 'Servynex', keys: t('settings.keysTitle'),
};
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
  radio('#set-access', setAccess);
  $('#set-server-lang').addEventListener('change', (e) => setServerLang(e.target.value));
  $('#set-notify').addEventListener('change', (e) => setNotify(e.target.checked));
  $('#notify-test').addEventListener('click', testNotify);
  $('#set-lang').value = choice();
  $('#set-lang').addEventListener('change', (e) => setLang(e.target.value));
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
      toast(t('settings.nameSaved'));
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
  const needs = (perm) => t('settings.needsPermission', { permission: t(`perm.${perm}`) });
  off('#open-config', 'commands', needs('commands'));
  off('#reload-config', 'edit', needs('edit'));
  off('#open-instance-log', 'instance', needs('instance'));
  off('#open-instance-dir', 'instance', needs('instance'));
  $('#instance-name').disabled = !can('instance');
  // Ouvrent une fenêtre sur la machine de Servynex : seulement pour qui est devant.
  for (const id of ['open-config', 'open-instance-log', 'open-instance-dir']) $(`#${id}`).hidden = !viewer.local;
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
// Nouvelle langue : gardée dans le navigateur et dans le compte, puis la page se
// recharge (les textes sont posés une fois, au chargement).
async function setLang(value) {
  setChoice(value);
  if (session.user) await api('auth/me', { method: 'PATCH', body: { prefs: { lang: value } } }).catch(() => {});
  location.reload();
}

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
    $('#autostart-desc').textContent = t('settings.autostartUnreadable', { error: err.message });
    return null;
  }
}

export async function setInstanceAutostart(on, input) {
  if (input) input.disabled = true;
  try {
    const { enabled } = await api('autostart', { method: 'PUT', body: { enabled: on } });
    toast(t(enabled ? 'settings.autostartOn' : 'settings.autostartOff'));
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
      h('input', { class: 'switch', type: 'checkbox', role: 'switch', 'aria-label': t('settings.bootProject', { name: p.name }), dataset: { id: p.id }, checked: p.autostart })));
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
    toast(t('settings.reloaded'));
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
    [t('col.address'), `http://localhost:${info.port}`],
    ['Node.js', info.node ?? '—'],
    [t('info.launch'), info.restartable ? t('settings.launchTask') : t('settings.launchHand')],
    [t('settings.tools'), [handlers.tools().code ? 'VS Code' : null, handlers.tools().terminal].filter(Boolean).join(', ')],
  ];
  $('#instance-info').replaceChildren(...rows.flatMap(([k, v]) => [h('dt', { text: k }), h('dd', { text: v })]));
  $('#restart-instance').disabled = !info.restartable || !can('instance');
  renderAccess();
  renderNotify();
  $('#restart-desc').textContent = info.restartable
    ? t('settings.restartDesc')
    : t('settings.restartUnavailable');
}

// ---- Accès à la page : ce PC seulement, ou aussi le réseau local ----

// Le réseau demande la protection par mot de passe (le serveur le refuse sinon) ; le
// changement s'applique au redémarrage, d'où l'avertissement tant qu'il n'est pas fait.
function renderAccess() {
  const info = handlers.instance();
  const access = info.access ?? 'local';
  for (const b of document.querySelectorAll('#set-access [data-value]')) {
    b.setAttribute('aria-checked', String(b.dataset.value === access));
    b.disabled = !can('admin') || (b.dataset.value === 'lan' && !session.enabled && access !== 'lan');
  }
  const notes = [];
  if (!session.enabled) notes.push(h('p', { class: 'hint', text: t('access.needsProtection') }));
  if (access !== (info.listening ?? 'local')) notes.push(h('p', { class: 'hint warn', text: t(access === 'lan' ? 'access.pendingLan' : 'access.pendingLocal') }));
  if (access === 'lan' || info.listening === 'lan') {
    const urls = (info.lan ?? []).map((a) => h('li', {}, h('code', { text: `http://${a.address}:${info.port}` }), ` · ${a.name}`));
    notes.push(urls.length ? h('div', {}, h('p', { class: 'hint', text: t('access.addresses') }), h('ul', { class: 'access-urls' }, ...urls)) : h('p', { class: 'hint warn', text: t('access.noNetwork') }));
    const pub = publicNetworks();
    if (pub.length) notes.push(h('p', { class: 'hint warn', text: tn('access.publicNetwork', pub.length, { networks: pub.join(', ') }) }));
    notes.push(h('p', { class: 'hint', text: t('access.plainHttp') }));
  }
  $('#access-note').replaceChildren(...notes);
}

async function setAccess(value) {
  const info = handlers.instance();
  if (value === (info.access ?? 'local')) return;
  try {
    const res = value === 'lan'
      ? await ask({
        title: t('access.askTitle'),
        body: t('access.askBody'),
        fields: [{ name: 'password', label: t('account.password'), type: 'password', autocomplete: 'current-password' }],
        confirm: t('access.lanOk'),
        valid: (v) => v.password.length > 0,
        submit: (v) => api('instance/access', { method: 'PUT', body: { access: 'lan', password: v.password } }),
      })
      : await api('instance/access', { method: 'PUT', body: { access: 'local' } });
    if (!res) return;
    info.access = res.access;
    toast(t(res.access === 'lan' ? 'access.savedLan' : 'access.savedLocal'));
  } catch (err) {
    toast(err.message, 'error');
  }
  renderAccess();
  renderServerLang();
}

// Langue des journaux, de l'historique et des états en direct (côté serveur),
// distincte de celle de cette page. Appliquée tout de suite.
function renderServerLang() {
  const info = handlers.instance();
  const select = $('#set-server-lang');
  select.value = info.lang ?? 'auto';
  select.disabled = !can('instance');
  $('#server-lang-desc').textContent = t('serverLang.desc', { current: LANG_NAMES[info.serverLang] ?? info.serverLang ?? '—' });
}

const LANG_NAMES = { fr: 'Français', en: 'English' };

async function setServerLang(value) {
  const info = handlers.instance();
  try {
    const res = await api('instance/lang', { method: 'PUT', body: { lang: value === 'auto' ? null : value } });
    Object.assign(info, { lang: res.lang, serverLang: res.serverLang });
    toast(t('serverLang.saved', { current: LANG_NAMES[res.serverLang] ?? res.serverLang }));
  } catch (err) {
    toast(err.message, 'error');
  }
  renderServerLang();
}

// ---- Notifications du système : un projet qui plante, même la page fermée ----

function renderNotify() {
  const info = handlers.instance();
  $('#set-notify').checked = info.notify === true;
  $('#set-notify').disabled = !can('instance');
  $('#notify-test').disabled = !can('instance');
}

async function setNotify(on) {
  const info = handlers.instance();
  try {
    const { notify } = await api('instance/notify', { method: 'PUT', body: { notify: on } });
    info.notify = notify;
    toast(t(notify ? 'notify.on' : 'notify.off'));
  } catch (err) {
    toast(err.message, 'error');
  }
  renderNotify();
}

async function testNotify() {
  const button = $('#notify-test');
  button.disabled = true;
  try {
    const { sent } = await api('instance/notify-test', { method: 'POST' });
    toast(t(sent ? 'notify.sent' : 'notify.unsupported'), sent ? undefined : 'error');
  } catch (err) {
    toast(err.message, 'error');
  } finally {
    button.disabled = !can('instance');
  }
}

// Les projets continuent de tourner et sont repris. La page attend que l'ancien
// Servynex soit tombé puis que le nouveau réponde, et se recharge (l'interface a pu changer).
async function restartInstance() {
  const ok = await confirmDialog({
    title: t('settings.restartTitle'),
    body: t('settings.restartBody'),
    confirm: t('action.restart'),
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
  toast(t('settings.restarting'));
  const up = () => fetch('/api/projects', { cache: 'no-store' }).then((r) => r.ok, () => false);
  const until = async (want, ms) => {
    for (const end = Date.now() + ms; Date.now() < end; await new Promise((r) => setTimeout(r, 400))) {
      if ((await up()) === want) return true;
    }
    return false;
  };
  await until(false, 5_000);
  if (await until(true, 30_000)) location.reload();
  else toast(t('settings.restartTimeout'), 'error');
}
