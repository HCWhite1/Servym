// Paramètres › Mon compte, Sécurité, Membres (comptes et protection par mot de passe).
import { showKeyDialog, logout } from './auth.js';
import { PERMISSION_LABELS, ROLE_LABELS, ROLE_PERMISSIONS, ago, api, confirmDialog, h, icon, initials, session, toast } from './lib.js';

const $ = (sel) => document.querySelector(sel);
const LOCKS = [[0, 'Jamais'], [120, '2 min'], [300, '5 min'], [900, '15 min'], [1800, '30 min'], [3600, '1 h']];
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

// Fenêtre qui demande un ou plusieurs champs ; renvoie les valeurs, ou null si annulée.
function ask({ title, body, fields, confirm = 'Valider', danger = false, submit }) {
  const inputs = fields.map((f) => ({ ...f, el: input({ type: f.type ?? 'text', autocomplete: f.autocomplete ?? 'off', placeholder: f.placeholder ?? '' }) }));
  const error = h('p', { class: 'hint warn', hidden: true });
  const ok = h('button', { class: `btn ${danger ? 'danger-fill' : 'primary'}`, type: 'submit', text: confirm });
  const cancel = h('button', { class: 'btn', type: 'button', text: 'Annuler' });
  const form = h('form', { class: 'form' },
    body ? h('p', { class: 'muted', text: body }) : null,
    ...inputs.map((f) => h('label', { class: 'field' }, h('span', { text: f.label }), f.el)),
    error);
  const dialog = h('dialog', { class: 'modal' },
    h('header', { class: 'modal-head' }, h('h2', { text: title })),
    h('div', { class: 'modal-body' }, form),
    h('footer', { class: 'modal-foot' }, cancel, ok));
  document.body.append(dialog);
  dialog.showModal();
  inputs[0]?.el.focus();
  return new Promise((resolve) => {
    const close = (value) => {
      dialog.close();
      dialog.remove();
      resolve(value);
    };
    cancel.addEventListener('click', () => close(null));
    dialog.addEventListener('cancel', () => close(null));
    const run = async (e) => {
      e?.preventDefault();
      const values = Object.fromEntries(inputs.map((f) => [f.name, f.el.value]));
      ok.disabled = true;
      try {
        close(submit ? await submit(values) : values);
      } catch (err) {
        error.textContent = err.message;
        error.hidden = false;
        ok.disabled = false;
      }
    };
    form.addEventListener('submit', run);
    ok.addEventListener('click', run);
  });
}

const passwordFields = (first = 'Mot de passe') => [
  { name: 'password', label: first, type: 'password', autocomplete: 'new-password' },
  { name: 'confirm', label: 'Confirmer', type: 'password', autocomplete: 'new-password' },
];
function checkPair({ password, confirm }) {
  if (password.length < 8) throw new Error('Mot de passe trop court : 8 caractères au moins.');
  if (password !== confirm) throw new Error('Les deux mots de passe ne correspondent pas.');
}

// Cases à cocher des permissions, avec les rôles tout prêts au-dessus.
function permissionPicker(initial) {
  const boxes = new Map();
  const warn = h('p', { class: 'hint warn', text: 'Commandes : ce compte pourra faire exécuter n’importe quoi à la machine de Servynex.', hidden: true });
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
  const list = h('div', { class: 'sub-list' }, ...Object.entries(PERMISSION_LABELS).map(([p, [label, desc]]) => {
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
  const cancel = h('button', { class: 'btn', type: 'button', text: 'Annuler' });
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

// ---- Mon compte ----

export async function renderAccount() {
  const body = $('#account-body');
  const user = session.user;
  if (!user) return body.replaceChildren();
  const name = input({ value: user.name, maxlength: '60', 'aria-label': 'Nom affiché' });
  const sessionsList = h('div', { class: 'sub-list' });
  body.replaceChildren(
    h('div', { class: 'account-head' },
      h('span', { class: 'avatar lg', text: initials(user) }),
      h('div', {}, h('p', { class: 'setting-label', text: user.name }), h('p', { class: 'setting-desc', text: `${user.username} · ${ROLE_LABELS[user.role] ?? user.role}` }))),
    setting('Nom affiché', 'Visible dans la barre latérale et le journal d’audit.',
      h('div', { class: 'inline-form' }, name, button('Enregistrer', null, async () => {
        const res = await api('auth/me', { method: 'PATCH', body: { name: name.value } });
        Object.assign(session.user, res.user);
        document.dispatchEvent(new CustomEvent('servynex:user'));
        toast('Nom enregistré');
      }))),
    setting('Mot de passe', 'Changer le mot de passe ferme vos autres sessions.', button('Changer', 'key-round', async () => {
      const res = await ask({
        title: 'Changer le mot de passe',
        fields: [{ name: 'current', label: 'Mot de passe actuel', type: 'password', autocomplete: 'current-password' }, ...passwordFields('Nouveau mot de passe')],
        confirm: 'Changer',
        submit: async (v) => {
          checkPair(v);
          return api('auth/me/password', { method: 'POST', body: { current: v.current, password: v.password } });
        },
      });
      if (res) toast(res.closed ? `Mot de passe changé, ${res.closed} autre(s) session(s) fermée(s)` : 'Mot de passe changé');
      renderAccount();
    })),
    setting('Clé de secours', 'Sert à choisir un nouveau mot de passe si vous l’oubliez. En générer une nouvelle rend l’ancienne inutilisable.', button('Nouvelle clé', 'key-round', async () => {
      const res = await ask({
        title: 'Nouvelle clé de secours',
        body: 'Confirmez avec votre mot de passe.',
        fields: [{ name: 'password', label: 'Mot de passe', type: 'password', autocomplete: 'current-password' }],
        confirm: 'Générer',
        submit: (v) => api('auth/me/recovery', { method: 'POST', body: v }),
      });
      if (res) await showKeyDialog(res.recovery, 'Nouvelle clé de secours');
    })),
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: 'Sessions ouvertes' }), h('p', { class: 'setting-desc', text: 'Navigateurs connectés à votre compte.' })),
      sessionsList,
      h('div', { class: 'setting-actions' }, button('Déconnecter les autres sessions', null, async () => {
        const res = await api('auth/me/sessions/others', { method: 'POST' });
        toast(`${res.closed} session(s) fermée(s)`);
        renderAccount();
      }), button('Se déconnecter', 'log-out', () => logout(), 'btn danger'))),
  );
  const { sessions } = await api('auth/me/sessions').catch(() => ({ sessions: [] }));
  sessionsList.replaceChildren(...sessions.map((s) => h('div', { class: 'sub-item' },
    icon('monitor'),
    h('span', {}, h('strong', { text: s.agent }), h('small', { class: 'block muted', text: `${s.ip.replace('::ffff:', '')} · active ${ago(s.lastSeen)}${s.remember ? ' · rester connecté' : ''}` })),
    s.current ? h('span', { class: 'badge ok', text: 'Cette session' }) : button('Déconnecter', null, async () => {
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
        h('h3', { text: 'Accès libre' }),
        h('p', { text: 'Toute personne qui ouvre Servynex a tous les droits. C’est normal en local, sur votre PC. Pour une instance partagée ou accessible à distance, activez la protection par mot de passe.' }),
        reactivate ? h('p', { class: 'hint', text: 'Des comptes existent déjà : la réactivation demande les identifiants d’un administrateur.' }) : null,
        button(reactivate ? 'Réactiver la protection' : 'Activer la protection', 'lock', async () => {
          const res = reactivate
            ? await ask({
              title: 'Réactiver la protection',
              fields: [{ name: 'username', label: 'Administrateur', autocomplete: 'username' }, { name: 'password', label: 'Mot de passe', type: 'password', autocomplete: 'current-password' }],
              confirm: 'Réactiver',
              submit: (v) => api('auth/enable', { method: 'POST', body: v }),
            })
            : await ask({
              title: 'Créer le compte administrateur',
              body: 'Ce compte aura tous les droits. D’autres comptes pourront être invités ensuite dans Paramètres › Membres.',
              fields: [
                { name: 'username', label: 'Nom d’utilisateur', autocomplete: 'username', placeholder: 'minuscules, chiffres, - . _' },
                { name: 'name', label: 'Nom affiché (facultatif)', autocomplete: 'name' },
                ...passwordFields(),
              ],
              confirm: 'Activer la protection',
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
    body.replaceChildren(setting('Protection par mot de passe', 'Active. Réglée par un administrateur.', h('span', { class: 'badge ok', text: 'Active' })));
    return;
  }
  const lock = h('div', { class: 'segmented', role: 'radiogroup', 'aria-label': 'Verrouillage' }, ...LOCKS.map(([value, label]) => {
    const b = h('button', { type: 'button', role: 'radio', 'aria-checked': String(value === status.lockAfter), text: label });
    b.addEventListener('click', async () => {
      try {
        await api('auth/settings', { method: 'PATCH', body: { lockAfter: value } });
        toast(value ? `Verrouillage après ${label} d’inactivité (au prochain chargement de chaque page)` : 'Verrouillage désactivé');
        renderSecurity();
      } catch (err) {
        toast(err.message, 'error');
      }
    });
    return b;
  }));
  body.replaceChildren(
    setting('Protection par mot de passe', 'Seule la page de connexion répond tant qu’on n’est pas connecté.', h('span', { class: 'badge ok', text: 'Active' })),
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' },
        h('p', { class: 'setting-label', text: 'Verrouillage après inactivité' }),
        h('p', { class: 'setting-desc', text: 'Pour tous les comptes. Sans souris ni clavier sur la page pendant ce temps, la session se verrouille et le mot de passe est redemandé.' })),
      lock),
    setting('Désactiver la protection', 'Servynex redevient accessible sans connexion. Les comptes sont conservés pour une réactivation.', button('Désactiver', null, async () => {
      const res = await ask({
        title: 'Désactiver la protection ?',
        body: 'Toute personne qui ouvre Servynex aura de nouveau tous les droits. Confirmez avec votre mot de passe.',
        fields: [{ name: 'password', label: 'Mot de passe', type: 'password', autocomplete: 'current-password' }],
        confirm: 'Désactiver',
        danger: true,
        submit: (v) => api('auth/disable', { method: 'POST', body: v }),
      });
      if (res) location.reload();
    }, 'btn danger')),
  );
}

// ---- Membres ----

export async function renderMembers() {
  const body = $('#members-body');
  if (!isAdmin()) return body.replaceChildren();
  const [{ users }, { invites }] = await Promise.all([api('auth/users'), api('auth/invites')]);
  const row = (u) => {
    const me = u.id === session.user.id;
    const actions = h('div', { class: 'btn-row' },
      button('Permissions', null, async () => {
        const perms = await permissionsDialog({ title: `Permissions de ${u.name}`, initial: u.permissions, confirm: 'Enregistrer' });
        if (!perms) return;
        await api(`auth/users/${u.id}`, { method: 'PATCH', body: { permissions: perms } });
        toast('Permissions enregistrées');
        renderMembers();
      }, 'btn small'),
      me ? null : button('Code', 'key-round', async () => {
        const { code } = await api(`auth/users/${u.id}/reset`, { method: 'POST' });
        await confirmDialog({
          title: `Code de réinitialisation pour ${u.username}`,
          body: `${code}\n\nValable 15 minutes. À saisir sur la page de connexion : « Mot de passe oublié ? » › « J’ai un code ». Ses sessions seront fermées.`,
          confirm: 'Fermer',
        });
      }, 'btn small'),
      me ? null : button('', 'x', async () => {
        const ok = await confirmDialog({ title: `Supprimer le compte ${u.username} ?`, body: 'Ses sessions sont fermées. Ses actions restent dans le journal d’audit.', confirm: 'Supprimer', danger: true });
        if (!ok) return;
        await api(`auth/users/${u.id}`, { method: 'DELETE' });
        renderMembers();
      }, 'icon-btn'));
    return h('div', { class: 'sub-item member' },
      h('span', { class: 'avatar', text: initials(u) }),
      h('span', {}, h('strong', { text: u.name }), h('small', { class: 'block muted', text: `${u.username}${me ? ' · vous' : ''}` })),
      h('span', { class: `badge${u.role === 'admin' ? ' ok' : ''}`, text: ROLE_LABELS[u.role] ?? u.role }),
      actions);
  };
  const inviteRow = (i) => h('div', { class: 'sub-item' },
    icon('link'),
    h('span', {}, h('strong', { text: ROLE_LABELS[i.role] ?? i.role }), h('small', { class: 'block muted', text: `par ${i.createdBy} · expire ${ago(i.expiresAt)}` })),
    button('Révoquer', null, async () => {
      await api(`auth/invites/${i.id}`, { method: 'DELETE' });
      renderMembers();
    }, 'btn small'));
  body.replaceChildren(...[
    h('div', { class: 'setting stack' },
      h('div', { class: 'setting-head' },
        h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: `Comptes (${users.length})` }), h('p', { class: 'setting-desc', text: 'Le dernier administrateur ne peut être ni supprimé ni rétrogradé.' })),
        button('Inviter', 'user-plus', invite, 'btn primary')),
      h('div', { class: 'sub-list' }, ...users.map(row))),
    invites.length ? h('div', { class: 'setting stack' },
      h('div', { class: 'setting-text' }, h('p', { class: 'setting-label', text: 'Invitations en attente' })),
      h('div', { class: 'sub-list' }, ...invites.map(inviteRow))) : null,
  ].filter(Boolean));
}

async function invite() {
  const perms = await permissionsDialog({ title: 'Inviter un membre', initial: ROLE_PERMISSIONS.operator, confirm: 'Créer le lien' });
  if (!perms) return;
  const { token } = await api('auth/invites', { method: 'POST', body: { permissions: perms } });
  const url = `${location.origin}/?invite=${token}`;
  await navigator.clipboard.writeText(url).catch(() => {});
  await confirmDialog({
    title: `Lien d’invitation (${ROLE_LABELS[roleOf(perms)]})`,
    body: `${url}\n\nCopié dans le presse-papiers. Valable 48 heures, pour une seule personne : elle choisira elle-même son nom d’utilisateur et son mot de passe.`,
    confirm: 'Fermer',
  });
  renderMembers();
}
