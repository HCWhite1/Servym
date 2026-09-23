// Écrans d'authentification (connexion, session verrouillée, mot de passe oublié,
// invitation) et verrouillage après inactivité. Protection désactivée : rien de
// tout cela ne s'affiche.
import { ROLE_LABELS, api, h, icon, initials, session, toast } from './lib.js';

const screen = document.getElementById('auth-screen');
let instanceName = 'Servynex';
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
  if (a.value.length < 8) throw new Error('Mot de passe trop court : 8 caractères au moins.');
  if (a.value !== b.value) throw new Error('Les deux mots de passe ne correspondent pas.');
}

// ---- Écrans ----

function showLogin() {
  const [userField, user] = field('Nom d’utilisateur', { name: 'username', autocomplete: 'username', autocapitalize: 'none', required: true });
  const [passField, pass] = field('Mot de passe', { name: 'password', type: 'password', autocomplete: 'current-password', required: true });
  const remember = h('input', { type: 'checkbox', name: 'remember' });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' },
    userField, passField,
    h('label', { class: 'check' }, remember, 'Rester connecté (30 jours)'),
    error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: 'Se connecter' }));
  onSubmit(form, error, async () => {
    await api('auth/login', { method: 'POST', body: { username: user.value, password: pass.value, remember: remember.checked } });
    location.replace('/');
  });
  card('Connexion', null, form, h('div', { class: 'auth-links' }, link('Mot de passe oublié ?', showForgot)));
}

function showLocked() {
  const user = session.user;
  const [passField, pass] = field('Mot de passe', { name: 'password', type: 'password', autocomplete: 'current-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, passField, error, h('button', { class: 'btn primary auth-submit', type: 'submit', text: 'Déverrouiller' }));
  onSubmit(form, error, async () => {
    await api('auth/unlock', { method: 'POST', body: { password: pass.value } });
    location.reload();
  });
  const box = card('Session verrouillée', user ? `${user.name} · ${user.username}` : null, form,
    h('div', { class: 'auth-links' }, link('Changer de compte', () => logout())));
  box.querySelector('h1').before(h('span', { class: 'avatar lg', text: initials(user) }));
}

function showForgot() {
  const option = (title, text, onClick) => {
    const b = h('button', { class: 'choice auth-option', type: 'button' }, h('strong', { text: title }), h('small', { text }));
    b.addEventListener('click', onClick);
    return b;
  };
  card('Mot de passe oublié', 'Deux façons de choisir un nouveau mot de passe :',
    h('div', { class: 'auth-options' },
      option('J’ai ma clé de secours', 'La clé de 20 caractères affichée à la création du compte.', () => showReset('recover')),
      option('J’ai un code', 'Un code de 15 minutes donné par un administrateur, ou obtenu sur la machine de Servynex.', () => showReset('reset'))),
    h('div', { class: 'auth-help' },
      h('p', { text: 'Sans clé : un administrateur peut créer un code depuis Paramètres › Membres. Sinon, sur la machine qui héberge Servynex, lancez :' }),
      h('pre', {}, h('code', { text: 'npm run auth -- reset-password <nom>' }))),
    h('div', { class: 'auth-links' }, link('Retour à la connexion', showLogin)));
}

function showReset(kind) {
  const [userField, user] = field('Nom d’utilisateur', { autocomplete: 'username', autocapitalize: 'none', required: true });
  const [secretField, secret] = kind === 'recover'
    ? field('Clé de secours', { placeholder: 'XXXX-XXXX-XXXX-XXXX-XXXX', autocomplete: 'off', spellcheck: 'false', required: true })
    : field('Code', { placeholder: 'XXXX-XXXX', autocomplete: 'one-time-code', spellcheck: 'false', required: true });
  const [p1Field, p1] = field('Nouveau mot de passe', { type: 'password', autocomplete: 'new-password', required: true });
  const [p2Field, p2] = field('Confirmer', { type: 'password', autocomplete: 'new-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, userField, secretField, p1Field, p2Field, error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: 'Changer le mot de passe' }));
  onSubmit(form, error, async () => {
    checkPasswords(p1, p2);
    const body = { username: user.value, password: p1.value, [kind === 'recover' ? 'key' : 'code']: secret.value };
    const res = await api(`auth/${kind}`, { method: 'POST', body });
    if (res.recovery) await showKeyInScreen(res.recovery, 'Votre clé de secours a servi : voici la nouvelle.');
    location.replace('/');
  });
  card(kind === 'recover' ? 'Clé de secours' : 'Code de réinitialisation', 'Toutes les sessions ouvertes de ce compte seront fermées.', form,
    h('div', { class: 'auth-links' }, link('Retour', showForgot)));
}

async function showInvite(token) {
  const info = await api(`auth/invite/${encodeURIComponent(token)}`).catch(() => ({ valid: false }));
  if (!info.valid) {
    card('Invitation expirée', 'Ce lien n’est plus valable (48 heures, un seul usage). Demandez-en un nouveau à un administrateur.',
      h('div', { class: 'auth-links' }, link('Aller à la connexion', () => location.replace('/'))));
    return;
  }
  const [userField, user] = field('Nom d’utilisateur', { autocomplete: 'username', autocapitalize: 'none', placeholder: 'minuscules, chiffres, - . _', required: true });
  const [nameField, name] = field('Nom affiché', { autocomplete: 'name', placeholder: 'Facultatif' });
  const [p1Field, p1] = field('Mot de passe', { type: 'password', autocomplete: 'new-password', required: true });
  const [p2Field, p2] = field('Confirmer', { type: 'password', autocomplete: 'new-password', required: true });
  const error = errorLine();
  const form = h('form', { class: 'auth-form' }, userField, nameField, p1Field, p2Field, error,
    h('button', { class: 'btn primary auth-submit', type: 'submit', text: 'Créer mon compte' }));
  onSubmit(form, error, async () => {
    checkPasswords(p1, p2);
    const res = await api('auth/invite', { method: 'POST', body: { token, username: user.value, name: name.value, password: p1.value } });
    await showKeyInScreen(res.recovery);
    location.replace('/');
  });
  card(`Rejoindre ${instanceName}`, `Vous êtes invité avec le rôle « ${ROLE_LABELS[info.role] ?? info.role} ».`, form);
}

// ---- Clé de secours ----

// Bloc qui affiche une clé une seule fois ; « Continuer » n'est actif qu'une fois
// la case cochée.
export function keyBlock(key, onDone) {
  const copy = h('button', { class: 'btn small', type: 'button' }, icon('copy'), 'Copier');
  copy.addEventListener('click', async () => {
    await navigator.clipboard.writeText(key).catch(() => {});
    toast('Clé copiée');
  });
  const ok = h('input', { type: 'checkbox' });
  const done = h('button', { class: 'btn primary', type: 'button', text: 'Continuer', disabled: true });
  ok.addEventListener('change', () => (done.disabled = !ok.checked));
  done.addEventListener('click', onDone);
  return h('div', { class: 'key-block' },
    h('div', { class: 'key-value' }, h('code', { text: key }), copy),
    h('p', { class: 'hint', text: 'Elle permet de choisir un nouveau mot de passe si vous l’oubliez. Elle ne sera plus jamais affichée : notez-la dans un gestionnaire de mots de passe ou sur papier.' }),
    h('label', { class: 'check' }, ok, 'Je l’ai notée en lieu sûr'),
    done);
}

function showKeyInScreen(key, lead = 'Notez-la maintenant : elle ne sera plus affichée.') {
  return new Promise((resolve) => card('Votre clé de secours', lead, keyBlock(key, resolve)));
}

// Même chose dans une fenêtre, depuis les Paramètres.
export function showKeyDialog(key, title = 'Votre clé de secours') {
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
  Object.assign(session, { enabled: status.enabled, user: status.user, lockAfter: status.lockAfter });
  instanceName = status.name || 'Servynex';
  if (params.has('invite')) {
    await showInvite(params.get('invite'));
    return forever();
  }
  if (status.error) {
    card('Comptes illisibles', status.error, h('p', { class: 'hint', text: 'Corrigez le fichier, ou supprimez-le sur la machine pour retirer la protection et tous les comptes.' }));
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
