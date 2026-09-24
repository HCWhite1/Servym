// Écrans d'authentification (connexion, session verrouillée, mot de passe oublié,
// invitation) et verrouillage après inactivité. Protection désactivée : rien de
// tout cela ne s'affiche.
import { t } from './i18n.js';
import { ROLE_LABELS, api, h, icon, initials, session, toast } from './lib.js';

const screen = document.getElementById('auth-screen');
let instanceName = 'Servynex';
// Retour d'une connexion ou d'une liaison GitHub (?auth_error=… ou ?auth_done=…).
let notice = null;
const OAUTH_ERRORS = ['state', 'denied', 'failed', 'unlinked', 'taken'];
const forever = () => new Promise(() => {});

// ---- Petits éléments de formulaire ----

function field(label, attrs) {
  const input = h('input', { class: 'auth-input', ...attrs });
  return [h('label', { class: 'field' }, h('span', { text: label }), input), input];
}

function card(title, lead, ...children) {
  const box = h('div', { class: 'auth-card' },
    h('div', { class: 'auth-brand' }, logo(), h('span', { text: instanceName })),
    h('h1', { text: title }),
    lead ? h('p', { class: 'auth-lead', text: lead }) : null,
    ...children);
  screen.replaceChildren(box);
  screen.hidden = false;
  document.body.classList.add('auth-open');
  box.querySelector('input')?.focus();
  return box;
}

function logo() {
  const span = h('span', { class: 'auth-logo' });
  span.innerHTML = '<svg viewBox="0 0 32 32"><use href="icons.svg#servynex-logo"/></svg>';
  return span;
}

const errorLine = () => h('p', { class: 'auth-error', role: 'alert', hidden: true });
function showError(node, message) {
  node.textContent = message;
  node.hidden = !message;
}
const link = (text, onClick) => {
  const b = h('button', { class: 'auth-link', type: 'button', text });
  b.addEventListener('click', onClick);
  return b;
};

// Envoi d'un formulaire : bouton occupé, message d'erreur sous le formulaire.
function onSubmit(form, error, action) {
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    const button = form.querySelector('button[type="submit"]');
    button.disabled = true;
    showError(error, '');
    try {
      await action();
    } catch (err) {
      showError(error, err.message);
    } finally {
      button.disabled = false;
    }
  });
}

function checkPasswords(a, b) {
  if (a.value.length < 8) throw new Error(t('account.passwordShort'));
  if (a.value !== b.value) throw new Error(t('account.passwordMismatch'));
}

// ---- Écrans ----

function showLogin() {
  const [userField, user] = field(t('account.username'), { name: 'username', autocomplete: 'username', autocapitalize: 'none', required: true });
  const [passField, pass] = field(t('account.password'), { name: 'password', type: 'password', autocomplete: 'current-password', required: true });
  const remember = h('input', { type: 'checkbox', name: 'remember' });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' },
    userField, passField,
    h('label', { class: 'check' }, remember, t('login.remember')),
    error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: t('login.submit') }));
  onSubmit(form, error, async () => {
    await api('auth/login', { method: 'POST', body: { username: user.value, password: pass.value, remember: remember.checked } });
    location.replace('/');
  });
  if (notice?.error) showError(error, t(`oauth.error.${notice.error}`));
  notice = null;
  const github = session.methods.github
    ? [h('p', { class: 'auth-or', text: t('login.or') }), h('a', { class: 'btn auth-provider', href: '/api/auth/github/start' }, icon('github'), t('login.withGithub'))]
    : [];
  card(t('login.title'), null, form, ...github, h('div', { class: 'auth-links' }, link(t('login.forgot'), showForgot)));
}

function showLocked() {
  const user = session.user;
  const [passField, pass] = field(t('account.password'), { name: 'password', type: 'password', autocomplete: 'current-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, passField, error, h('button', { class: 'btn primary auth-submit', type: 'submit', text: t('locked.submit') }));
  onSubmit(form, error, async () => {
    await api('auth/unlock', { method: 'POST', body: { password: pass.value } });
    location.reload();
  });
  const box = card(t('locked.title'), user ? `${user.name} · ${user.username}` : null, form,
    h('div', { class: 'auth-links' }, link(t('locked.switch'), () => logout())));
  box.querySelector('h1').before(h('span', { class: 'avatar lg', text: initials(user) }));
}

function showForgot() {
  const option = (title, text, onClick) => {
    const b = h('button', { class: 'choice auth-option', type: 'button' }, h('strong', { text: title }), h('small', { text }));
    b.addEventListener('click', onClick);
    return b;
  };
  card(t('forgot.title'), t('forgot.lead'),
    h('div', { class: 'auth-options' },
      option(t('forgot.withKey'), t('forgot.withKeyDesc'), () => showReset('recover')),
      option(t('forgot.withCode'), t('forgot.withCodeDesc'), () => showReset('reset'))),
    h('div', { class: 'auth-help' },
      h('p', { text: t('forgot.help') }),
      h('pre', {}, h('code', { text: t('forgot.command') }))),
    h('div', { class: 'auth-links' }, link(t('forgot.back'), showLogin)));
}

function showReset(kind) {
  const [userField, user] = field(t('account.username'), { autocomplete: 'username', autocapitalize: 'none', required: true });
  const [secretField, secret] = kind === 'recover'
    ? field(t('account.recoveryKey'), { placeholder: 'XXXX-XXXX-XXXX-XXXX-XXXX', autocomplete: 'off', spellcheck: 'false', required: true })
    : field(t('members.code'), { placeholder: 'XXXX-XXXX', autocomplete: 'one-time-code', spellcheck: 'false', required: true });
  const [p1Field, p1] = field(t('account.newPassword'), { type: 'password', autocomplete: 'new-password', required: true });
  const [p2Field, p2] = field(t('common.confirm'), { type: 'password', autocomplete: 'new-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, userField, secretField, p1Field, p2Field, error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: t('account.changePassword') }));
  onSubmit(form, error, async () => {
    checkPasswords(p1, p2);
    const body = { username: user.value, password: p1.value, [kind === 'recover' ? 'key' : 'code']: secret.value };
    const res = await api(`auth/${kind}`, { method: 'POST', body });
    if (res.recovery) await showKeyInScreen(res.recovery, t('reset.newKey'));
    location.replace('/');
  });
  card(kind === 'recover' ? t('account.recoveryKey') : t('reset.codeTitle'), t('reset.lead'), form,
    h('div', { class: 'auth-links' }, link(t('ob.back'), showForgot)));
}

async function showInvite(token) {
  const info = await api(`auth/invite/${encodeURIComponent(token)}`).catch(() => ({ valid: false }));
  if (!info.valid) {
    card(t('invite.expired'), t('invite.expiredDesc'),
      h('div', { class: 'auth-links' }, link(t('invite.goToLogin'), () => location.replace('/'))));
    return;
  }
  const [userField, user] = field(t('account.username'), { autocomplete: 'username', autocapitalize: 'none', placeholder: t('account.usernamePlaceholder'), required: true });
  const [nameField, name] = field(t('account.displayName'), { autocomplete: 'name', placeholder: t('invite.optional') });
  const [p1Field, p1] = field(t('account.password'), { type: 'password', autocomplete: 'new-password', required: true });
  const [p2Field, p2] = field(t('common.confirm'), { type: 'password', autocomplete: 'new-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, userField, nameField, p1Field, p2Field, error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: t('invite.submit') }));
  onSubmit(form, error, async () => {
    checkPasswords(p1, p2);
    const res = await api('auth/invite', { method: 'POST', body: { token, username: user.value, name: name.value, password: p1.value } });
    await showKeyInScreen(res.recovery);
    location.replace('/');
  });
  card(t('invite.title', { name: instanceName }), t('invite.lead', { role: ROLE_LABELS[info.role] ?? info.role }), form);
}

// Message laissé par un retour de GitHub, pour la page une fois chargée (main.js).
export function takeNotice() {
  const n = notice;
  notice = null;
  return n;
}

// ---- Clé de secours ----

// Bloc qui affiche une clé une seule fois ; « Continuer » n'est actif qu'une fois
// la case cochée.
export function keyBlock(key, onDone) {
  const copy = h('button', { class: 'btn small', type: 'button' }, icon('copy'), t('common.copy'));
  copy.addEventListener('click', async () => {
    await navigator.clipboard.writeText(key).catch(() => {});
    toast(t('key.copied'));
  });
  const ok = h('input', { type: 'checkbox' });
  const done = h('button', { class: 'btn primary', type: 'button', text: t('ob.next'), disabled: true });
  ok.addEventListener('change', () => (done.disabled = !ok.checked));
  done.addEventListener('click', onDone);
  return h('div', { class: 'key-block' },
    h('div', { class: 'key-value' }, h('code', { text: key }), copy),
    h('p', { class: 'hint', text: t('key.hint') }),
    h('label', { class: 'check' }, ok, t('key.noted')),
    done);
}

function showKeyInScreen(key, lead = t('key.lead')) {
  return new Promise((resolve) => card(t('key.title'), lead, keyBlock(key, resolve)));
}

// Même chose dans une fenêtre, depuis les Paramètres.
export function showKeyDialog(key, title = t('key.title')) {
  const dialog = h('dialog', { class: 'modal key-dialog' },
    h('header', { class: 'modal-head' }, h('h2', { text: title })));
  return new Promise((resolve) => {
    dialog.append(h('div', { class: 'modal-body' }, keyBlock(key, () => {
      dialog.close();
      dialog.remove();
      resolve();
    })));
    dialog.addEventListener('cancel', (e) => e.preventDefault()); // pas d'Échap : il faut confirmer
    document.body.append(dialog);
    dialog.showModal();
  });
}

// ---- Session ----

export async function logout() {
  await api('auth/logout', { method: 'POST' }).catch(() => {});
  location.replace('/');
}

export async function lockNow() {
  await api('auth/lock', { method: 'POST' }).catch(() => {});
  showLocked();
}

// Verrouillage après inactivité : souris ou clavier sur la page comptent comme
// activité (signalée à Servynex toutes les 30 s au plus) ; les mises à jour
// automatiques de la page ne comptent pas. Servynex applique la même règle.
function startIdleLock(seconds) {
  if (!seconds) return;
  let last = Date.now();
  let reported = Date.now();
  const mark = () => {
    last = Date.now();
    if (last - reported > 30_000) {
      reported = last;
      api('auth/activity', { method: 'POST' }).catch(() => {});
    }
  };
  for (const type of ['pointerdown', 'pointermove', 'keydown', 'wheel', 'touchstart']) addEventListener(type, mark, { passive: true });
  setInterval(() => {
    if (screen.hidden && Date.now() - last > seconds * 1000) lockNow();
  }, 5000);
}

// Avant de charger la page : protection désactivée ou session valide → on
// continue ; sinon l'écran voulu s'affiche et la page ne se charge pas.
export async function gate() {
  const params = new URLSearchParams(location.search);
  const status = await api('auth/status');
  Object.assign(session, { enabled: status.enabled, user: status.user, lockAfter: status.lockAfter, methods: status.methods ?? {} });
  if (params.has('auth_error') || params.has('auth_done')) {
    const error = params.get('auth_error');
    notice = { error: error && (OAUTH_ERRORS.includes(error) ? error : 'failed'), done: params.get('auth_done') === 'linked' ? 'linked' : null };
    history.replaceState(null, '', location.pathname + location.hash);
  }
  instanceName = status.name || 'Servynex';
  if (params.has('invite')) {
    await showInvite(params.get('invite'));
    return forever();
  }
  if (status.error) {
    card(t('auth.brokenTitle'), status.error, h('p', { class: 'hint', text: t('auth.brokenHint') }));
    return forever();
  }
  if (status.enabled && !status.user) {
    showLogin();
    return forever();
  }
  if (status.enabled && status.locked) {
    showLocked();
    return forever();
  }
  // Session expirée ou verrouillée pendant l'utilisation.
  addEventListener('servynex:auth', (e) => (e.detail.auth === 'locked' ? showLocked() : e.detail.auth === 'login' ? showLogin() : null));
  if (status.enabled) startIdleLock(status.lockAfter);
  return status;
}
