// Paramètres › Mon compte, Sécurité, Membres (comptes et protection par mot de passe).
import { showKeyDialog, logout } from './auth.js';
import { t } from './i18n.js';
import { ROLE_LABELS, ROLE_PERMISSIONS, ago, api, ask, confirmDialog, h, icon, initials, permissionLabels, session, toast } from './lib.js';

const $ = (sel) => document.querySelector(sel);
const LOCKS = [[0, t('lock.never')], [120, '2 min'], [300, '5 min'], [900, '15 min'], [1800, '30 min'], [3600, '1 h']];
const isAdmin = () => session.user?.permissions.includes('admin');

// ---- Petits composants ----

function setting(label, desc, control) {
  return h('div', { class: 'setting' },
    h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: label }), desc ? h('p', { class: 'setting-desc', text: desc }) : null),
    control);
}

function input(attrs) {
  return h('input', { class: 'input', ...attrs });
}

function button(text, iconName, onClick, cls = 'btn') {
  const b = h('button', { class: cls, type: 'button' }, iconName ? icon(iconName) : null, text);
  b.addEventListener('click', async () => {
    b.disabled = true;
    try {
      await onClick(b);
    } catch (err) {
      toast(err.message, 'error');
    } finally {
      b.disabled = false;
    }
  });
  return b;
}

const passwordFields = (first = t('account.password')) => [
  { name: 'password', label: first, type: 'password', autocomplete: 'new-password' },
  { name: 'confirm', label: t('common.confirm'), type: 'password', autocomplete: 'new-password' },
];
function checkPair({ password, confirm }) {
  if (password.length < 8) throw new Error(t('account.passwordShort'));
  if (password !== confirm) throw new Error(t('account.passwordMismatch'));
}

// Cases à cocher des permissions, avec les rôles tout prêts au-dessus.
function permissionPicker(initial) {
  const boxes = new Map();
  const warn = h('p', { class: 'hint warn', text: t('members.commandsWarn'), hidden: true });
  const sync = (changed) => {
    const on = (p) => boxes.get(p).checked;
    // Permissions liées : commandes ⇒ modifier ⇒ piloter ; administration ⇒ tout.
    if (changed === 'admin' && on('admin')) for (const b of boxes.values()) b.checked = true;
    if (on('commands')) boxes.get('edit').checked = true;
    if (on('edit')) boxes.get('operate').checked = true;
    if (changed && changed !== 'admin' && !on(changed)) boxes.get('admin').checked = false;
    warn.hidden = !on('commands');
  };
  const presets = h('div', { class: 'segmented' }, ...Object.keys(ROLE_PERMISSIONS).map((role) => {
    const b = h('button', { type: 'button', text: ROLE_LABELS[role] });
    b.addEventListener('click', () => {
      for (const [p, box] of boxes) box.checked = ROLE_PERMISSIONS[role].includes(p);
      sync();
    });
    return b;
  }));
  const list = h('div', { class: 'sub-list' }, ...Object.entries(permissionLabels()).map(([p, [label, desc]]) => {
    const box = h('input', { type: 'checkbox', class: 'check-input', checked: initial.includes(p), disabled: p === 'view' });
    box.addEventListener('change', () => sync(p));
    boxes.set(p, box);
    return h('label', { class: 'sub-item perm-item' }, box, h('span', {}, h('strong', { text: label }), h('small', { text: desc })));
  }));
  sync();
  return {
    element: h('div', { class: 'perm-picker' }, presets, list, warn),
    value: () => [...boxes].filter(([, b]) => b.checked).map(([p]) => p),
  };
}

function permissionsDialog({ title, initial, confirm }) {
  const picker = permissionPicker(initial);
  const ok = h('button', { class: 'btn primary', type: 'button', text: confirm });
  const cancel = h('button', { class: 'btn', type: 'button', text: t('common.cancel') });
  const dialog = h('dialog', { class: 'modal perm-dialog' },
    h('header', { class: 'modal-head' }, h('h2', { text: title })),
    h('div', { class: 'modal-body' }, picker.element),
    h('footer', { class: 'modal-foot' }, cancel, ok));
  document.body.append(dialog);
  dialog.showModal();
  return new Promise((resolve) => {
    const close = (v) => {
      dialog.close();
      dialog.remove();
      resolve(v);
    };
    cancel.addEventListener('click', () => close(null));
    dialog.addEventListener('cancel', () => close(null));
    ok.addEventListener('click', () => close(picker.value()));
  });
}

const roleOf = (perms) => {
  if (perms.includes('admin')) return 'admin';
  const key = [...perms].sort().join();
  return Object.entries(ROLE_PERMISSIONS).find(([, p]) => [...p].sort().join() === key)?.[0] ?? 'custom';
};

// ---- Connexion avec GitHub ----

// Mon compte : lier (aller-retour par GitHub, retour sur Mon compte) ou délier.
function identitySetting(user) {
  const linked = user.identities?.find((i) => i.provider === 'github');
  if (!linked && !session.methods.github) return null;
  const control = linked
    ? button(t('oauth.unlink'), null, async () => {
      const res = await api('auth/me/identities/github', { method: 'DELETE' });
      Object.assign(session.user, res.user);
      toast(t('oauth.unlinked'));
      renderAccount();
    })
    : h('a', { class: 'btn', href: '/api/auth/github/start?link=1' }, icon('github'), t('oauth.link'));
  return setting(t('oauth.title'), linked ? t('oauth.linkedAs', { login: linked.login }) : t('oauth.linkDesc'), control);
}

// Sécurité : l'administrateur déclare l'application créée sur GitHub. Le secret
// n'est jamais renvoyé par le serveur : le champ reste vide, « inchangé » s'il existe.
async function providerSetting() {
  const gh = (await api('auth/providers').catch(() => ({}))).github;
  if (!gh) return null;
  const callback = `${location.origin}/auth/github/callback`;
  const clientId = h('input', { class: 'input', name: 'clientId', value: gh.clientId ?? '', autocomplete: 'off', spellcheck: 'false', placeholder: 'Client ID', 'aria-label': 'Client ID' });
  const secret = h('input', { class: 'input', name: 'clientSecret', type: 'password', autocomplete: 'new-password', placeholder: gh.configured ? t('oauth.secretKept') : 'Client secret', 'aria-label': 'Client secret' });
  const form = h('form', { class: 'inline-form provider-form' }, clientId, secret, h('button', { class: 'btn', type: 'submit', text: t('common.save') }));
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    try {
      await api('auth/providers/github', { method: 'PUT', body: { clientId: clientId.value, clientSecret: secret.value } });
      session.methods = { ...session.methods, github: true };
      toast(t('oauth.saved'));
      renderSecurity();
    } catch (err) {
      toast(err.message, 'error');
    }
  });
  const copy = button(t('common.copy'), 'copy', async () => {
    await navigator.clipboard.writeText(callback).catch(() => {});
    toast(t('oauth.callbackCopied'));
  }, 'btn small');
  const remove = gh.configured ? button(t('oauth.remove'), null, async () => {
    const ok = await confirmDialog({ title: t('oauth.removeTitle'), body: t('oauth.removeBody'), confirm: t('oauth.remove'), danger: true });
    if (!ok) return;
    await api('auth/providers/github', { method: 'DELETE' });
    session.methods = { ...session.methods, github: false };
    toast(t('oauth.removed'));
    renderSecurity();
  }, 'btn danger small') : null;
  return h('div', { class: 'setting stack provider-github' },
    h('div', { class: 'setting-head' },
      h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: t('oauth.title') }), h('p', { class: 'setting-desc', text: t('oauth.adminDesc') })),
      gh.configured ? h('span', { class: 'badge ok', text: t('security.on') }) : null,
      remove),
    h('ol', { class: 'provider-steps' },
      h('li', {}, t('oauth.step1'), ' ', h('a', { href: gh.registerUrl, target: '_blank', rel: 'noopener', text: gh.registerUrl.replace(/^https:\/\//, '') })),
      h('li', {}, t('oauth.step2'), h('span', { class: 'key-value' }, h('code', { text: callback }), copy)),
      h('li', {}, t('oauth.step3'))),
    form);
}

// ---- Mon compte ----

export async function renderAccount() {
  const body = $('#account-body');
  const user = session.user;
  if (!user) return body.replaceChildren();
  const name = input({ value: user.name, maxlength: '60', 'aria-label': t('account.displayName') });
  const sessionsList = h('div', { class: 'sub-list' });
  body.replaceChildren(...[
    h('div', { class: 'account-head' },
      h('span', { class: 'avatar lg', text: initials(user) }),
      h('div', {}, h('p', { class: 'setting-label', text: user.name }), h('p', { class: 'setting-desc', text: `${user.username} · ${ROLE_LABELS[user.role] ?? user.role}` }))),
    setting(t('account.displayName'), t('account.displayNameDesc'),
      h('div', { class: 'inline-form' }, name, button(t('common.save'), null, async () => {
        const res = await api('auth/me', { method: 'PATCH', body: { name: name.value } });
        Object.assign(session.user, res.user);
        document.dispatchEvent(new CustomEvent('servym:user'));
        toast(t('settings.nameSaved'));
      }))),
    setting(t('account.password'), t('account.passwordDesc'), button(t('account.change'), 'key-round', async () => {
      const res = await ask({
        title: t('account.changePassword'),
        fields: [{ name: 'current', label: t('account.currentPassword'), type: 'password', autocomplete: 'current-password' }, ...passwordFields(t('account.newPassword'))],
        confirm: t('account.change'),
        submit: async (v) => {
          checkPair(v);
          return api('auth/me/password', { method: 'POST', body: { current: v.current, password: v.password } });
        },
      });
      if (res) toast(res.closed ? t('account.passwordChangedClosed', { n: res.closed }) : t('account.passwordChanged'));
      renderAccount();
    })),
    identitySetting(user),
    setting(t('account.recoveryKey'), t('account.recoveryKeyDesc'), button(t('account.newKey'), 'key-round', async () => {
      const res = await ask({
        title: t('account.newRecoveryKey'),
        body: t('account.confirmWithPassword'),
        fields: [{ name: 'password', label: t('account.password'), type: 'password', autocomplete: 'current-password' }],
        confirm: t('account.generate'),
        submit: (v) => api('auth/me/recovery', { method: 'POST', body: v }),
      });
      if (res) await showKeyDialog(res.recovery, t('account.newRecoveryKey'));
    })),
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: t('account.sessions') }), h('p', { class: 'setting-desc', text: t('account.sessionsDesc') })),
      sessionsList,
      h('div', { class: 'setting-actions' }, button(t('account.signOutOthers'), null, async () => {
        const res = await api('auth/me/sessions/others', { method: 'POST' });
        toast(t('account.sessionsClosed', { n: res.closed }));
        renderAccount();
      }), button(t('user.logout'), 'log-out', () => logout(), 'btn danger'))),
  ].filter(Boolean));
  const { sessions } = await api('auth/me/sessions').catch(() => ({ sessions: [] }));
  sessionsList.replaceChildren(...sessions.map((s) => h('div', { class: 'sub-item' },
    icon('monitor'),
    h('span', {}, h('strong', { text: s.agent }), h('small', { class: 'block muted', text: [s.ip.replace('::ffff:', ''), t('account.sessionActive', { ago: ago(s.lastSeen) }), s.remember ? t('account.remember') : null].filter(Boolean).join(' · ') })),
    s.current ? h('span', { class: 'badge ok', text: t('account.thisSession') }) : button(t('account.signOut'), null, async () => {
      await api(`auth/me/sessions/${s.id}`, { method: 'DELETE' });
      renderAccount();
    }, 'btn small'))));
}

// ---- Sécurité ----

export async function renderSecurity() {
  const body = $('#security-body');
  const status = await api('auth/status');
  if (!status.enabled) {
    const reactivate = status.hasAccounts;
    body.replaceChildren(
      h('div', { class: 'empty-state security-empty' },
        icon('shield-check'),
        h('h3', { text: t('security.open') }),
        h('p', { text: t('security.openDesc') }),
        reactivate ? h('p', { class: 'hint', text: t('security.accountsExist') }) : null,
        button(reactivate ? t('security.reactivate') : t('security.activate'), 'lock', async () => {
          const res = reactivate
            ? await ask({
              title: t('security.reactivate'),
              fields: [{ name: 'username', label: t('roleLabel.admin'), autocomplete: 'username' }, { name: 'password', label: t('account.password'), type: 'password', autocomplete: 'current-password' }],
              confirm: t('security.reactivateOk'),
              submit: (v) => api('auth/enable', { method: 'POST', body: v }),
            })
            : await ask({
              title: t('security.createAdmin'),
              body: t('security.createAdminDesc'),
              fields: [
                { name: 'username', label: t('account.username'), autocomplete: 'username', placeholder: t('account.usernamePlaceholder') },
                { name: 'name', label: t('account.displayNameOptional'), autocomplete: 'name' },
                ...passwordFields(),
              ],
              confirm: t('security.activate'),
              submit: async (v) => {
                checkPair(v);
                return api('auth/setup', { method: 'POST', body: { username: v.username, name: v.name, password: v.password } });
              },
            });
          if (!res) return;
          if (res.recovery) await showKeyDialog(res.recovery);
          location.reload();
        }, 'btn primary')));
    return;
  }
  if (!isAdmin()) {
    body.replaceChildren(setting(t('security.protection'), t('security.activeByAdmin'), h('span', { class: 'badge ok', text: t('security.on') })));
    return;
  }
  const lock = h('div', { class: 'segmented', role: 'radiogroup', 'aria-label': t('security.lock') }, ...LOCKS.map(([value, label]) => {
    const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(value === status.lockAfter), text: label });
    b.addEventListener('click', async () => {
      try {
        await api('auth/settings', { method: 'PATCH', body: { lockAfter: value } });
        toast(value ? t('security.lockSet', { delay: label }) : t('security.lockOff'));
        renderSecurity();
      } catch (err) {
        toast(err.message, 'error');
      }
    });
    return b;
  }));
  const github = await providerSetting();
  body.replaceChildren(...[
    setting(t('security.protection'), t('security.onDesc'), h('span', { class: 'badge ok', text: t('security.on') })),
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' },
        h('p', { class: 'setting-label', text: t('security.lockAfter') }),
        h('p', { class: 'setting-desc', text: t('security.lockAfterDesc') })),
      lock),
    github,
    setting(t('security.disable'), t('security.disableDesc'), button(t('security.disableOk'), null, async () => {
      const res = await ask({
        title: t('security.disableTitle'),
        body: t('security.disableBody'),
        fields: [{ name: 'password', label: t('account.password'), type: 'password', autocomplete: 'current-password' }],
        confirm: t('security.disableOk'),
        danger: true,
        submit: (v) => api('auth/disable', { method: 'POST', body: v }),
      });
      if (res) location.reload();
    }, 'btn danger')),
  ].filter(Boolean));
}

// ---- Membres ----

export async function renderMembers() {
  const body = $('#members-body');
  if (!isAdmin()) return body.replaceChildren();
  const [{ users }, { invites }] = await Promise.all([api('auth/users'), api('auth/invites')]);
  const row = (u) => {
    const me = u.id === session.user.id;
    const actions = h('div', { class: 'btn-row' },
      button(t('members.permissions'), null, async () => {
        const perms = await permissionsDialog({ title: t('members.permissionsOf', { name: u.name }), initial: u.permissions, confirm: t('common.save') });
        if (!perms) return;
        await api(`auth/users/${u.id}`, { method: 'PATCH', body: { permissions: perms } });
        toast(t('members.permissionsSaved'));
        renderMembers();
      }, 'btn small'),
      me ? null : button(t('members.code'), 'key-round', async () => {
        const { code } = await api(`auth/users/${u.id}/reset`, { method: 'POST' });
        await confirmDialog({
          title: t('members.resetTitle', { username: u.username }),
          body: t('members.resetBody', { code }),
          confirm: t('common.closeOk'),
        });
      }, 'btn small'),
      me ? null : button('', 'x', async () => {
        const ok = await confirmDialog({ title: t('members.deleteTitle', { username: u.username }), body: t('members.deleteBody'), confirm: t('action.delete'), danger: true });
        if (!ok) return;
        await api(`auth/users/${u.id}`, { method: 'DELETE' });
        renderMembers();
      }, 'icon-btn'));
    return h('div', { class: 'sub-item member' },
      h('span', { class: 'avatar', text: initials(u) }),
      h('span', {}, h('strong', { text: u.name }), h('small', { class: 'block muted', text: [me ? t('members.you', { username: u.username }) : u.username, ...(u.identities ?? []).map((i) => t('members.identity', { provider: 'GitHub', login: i.login }))].join(' · ') })),
      h('span', { class: `badge${u.role === 'admin' ? ' ok' : ''}`, text: ROLE_LABELS[u.role] ?? u.role }),
      actions);
  };
  const inviteRow = (i) => h('div', { class: 'sub-item' },
    icon('link'),
    h('span', {}, h('strong', { text: ROLE_LABELS[i.role] ?? i.role }), h('small', { class: 'block muted', text: t('members.inviteBy', { user: i.createdBy, expires: ago(i.expiresAt) }) })),
    button(t('members.revoke'), null, async () => {
      await api(`auth/invites/${i.id}`, { method: 'DELETE' });
      renderMembers();
    }, 'btn small'));
  body.replaceChildren(...[
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-head' },
        h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: t('members.accounts', { n: users.length }) }), h('p', { class: 'setting-desc', text: t('members.lastAdmin') })),
        button(t('members.invite'), 'user-plus', invite, 'btn primary')),
      h('div', { class: 'sub-list' }, ...users.map(row))),
    invites.length ? h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: t('members.pending') })),
      h('div', { class: 'sub-list' }, ...invites.map(inviteRow))) : null,
  ].filter(Boolean));
}

async function invite() {
  const perms = await permissionsDialog({ title: t('members.inviteTitle'), initial: ROLE_PERMISSIONS.operator, confirm: t('members.createLink') });
  if (!perms) return;
  const { token } = await api('auth/invites', { method: 'POST', body: { permissions: perms } });
  const url = `${location.origin}/?invite=${token}`;
  await navigator.clipboard.writeText(url).catch(() => {});
  await confirmDialog({
    title: t('members.linkTitle', { role: ROLE_LABELS[roleOf(perms)] }),
    body: t('members.linkBody', { url }),
    confirm: t('common.closeOk'),
  });
  renderMembers();
}
