// Formulaire d'un projet : sert aux Réglages d'un projet existant et à l'ajout.
// Chaque champ porte la clé de projects.json qu'il modifie (data-key) ; à
// l'enregistrement d'un projet existant, seules les clés touchées sont envoyées,
// pour ne jamais renvoyer une valeur périmée (projet modifié ailleurs entre-temps).
import { PROJECT_ICONS, api, can, h, icon, toast } from './lib.js';

const TYPES = [
  ['process', 'Programme', 'Servynex lance une commande : node, npm, php, python…'],
  ['static', 'Site statique', 'Servynex sert lui-même les fichiers d’un dossier sur un port.'],
  ['file', 'Fichier', 'Ouvert dans le navigateur, sans serveur (page HTML locale).'],
];
const HOST_CONTROLS = [
  ['env', 'Le projet lit les variables PORT et HOST'],
  ['args', 'Servynex passe l’adresse dans les arguments ({host})'],
  ['local', 'Le projet écoute toujours sur ce PC seulement'],
];
const COLORS = ['#38bdf8', '#8b5cf6', '#14b8a6', '#f43f5e', '#f97316', '#ec4899', '#22c55e', '#eab308', '#60a5fa'];
// Champs de l'interface → clé de projects.json.
const KEY_OF = { healthKind: 'health', healthPath: 'health', healthExpect: 'health', stopCmd: 'stop', stopArgs: 'stop' };
const LAUNCH_KEYS = ['type', 'cwd', 'cmd', 'args', 'env', 'port', 'hostControl', 'expose'];
// Champs réservés à la permission « Commandes » (voir SENSITIVE dans server.js).
const SENSITIVE = ['cwd', 'cmd', 'args', 'env', 'stopCmd', 'stopArgs', 'expose'];

const lines = (text) => text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);

function field(label, input, hint, cls = '') {
  return h('label', { class: `field ${cls}`.trim() }, h('span', { text: label }), input, hint ? h('small', { class: 'field-hint', text: hint }) : null);
}

export function createForm({ handlers, submitLabel, onSubmit }) {
  const form = h('form', { class: 'form', novalidate: true });
  let project = null; // projet affiché (vue du serveur, avec .source), null pour un ajout
  const dirty = new Set();

  // ---- Construction ----

  const input = (name, attrs = {}) => h('input', { name, ...attrs });
  const area = (name, attrs = {}) => h('textarea', { name, rows: '3', spellcheck: 'false', ...attrs });
  const picker = h('div', { class: 'icon-picker', role: 'radiogroup', 'aria-label': 'Icône' });
  const typeGroup = h('div', { class: 'type-choices', role: 'radiogroup', 'aria-label': 'Type de projet' },
    ...TYPES.map(([value, label, desc]) => h('button', { type: 'button', role: 'radio', class: 'choice', dataset: { value } }, h('span', { text: label }), h('small', { text: desc }))));
  const requiresList = h('div', { class: 'sub-list' });
  const portHint = h('small', { class: 'field-hint' });
  const exposeHint = h('span', { class: 'hint' });
  const message = h('p', { class: 'hint', role: 'status' });
  const findPort = h('button', { class: 'btn', type: 'button', text: 'Trouver un port libre' });
  const submit = h('button', { class: 'btn primary', type: 'submit', text: submitLabel });
  const notice = h('p', { class: 'alert', hidden: true });

  const section = (title, cls, ...children) => h('section', { class: `form-section ${cls}`.trim() }, h('h3', { text: title }), ...children);

  form.append(
    notice,
    section('Général', 'first',
      h('div', { class: 'form-row' },
        field('Nom', input('name', { maxlength: '40', required: true, placeholder: 'Mon projet' }), null, 'grow'),
        field('Couleur', input('color', { type: 'color' }))),
      field('Description', input('desc', { maxlength: '160', placeholder: 'Ce que fait le projet, en une phrase' })),
      field('Étiquettes', input('tags', { placeholder: 'Express, SQLite' }), 'Séparées par des virgules, 6 au plus.'),
      h('div', { class: 'field' }, h('span', { text: 'Icône' }), picker)),
    section('Type', '', typeGroup),
    section('Lancement', '',
      field('Dossier', input('cwd', { placeholder: 'C:/MesProjets/mon-projet', spellcheck: 'false' }), 'Chemin complet. La commande est lancée depuis ce dossier.'),
      h('div', { class: 'only-process' },
        field('Commande', input('cmd', { placeholder: 'node', spellcheck: 'false' }), 'Programme lancé : node, npm, php, python, symfony, ou le chemin complet d’un .exe.'),
        field('Arguments', area('args', { placeholder: 'server.js' }), 'Un argument par ligne. {port}, {host} et {servynex} sont remplacés au lancement.')),
      h('div', { class: 'only-file' },
        field('Fichier à ouvrir', input('open', { placeholder: 'index.html', spellcheck: 'false' }), 'Chemin relatif au dossier.')),
      h('div', { class: 'not-file form-row' },
        field('Port', input('port', { type: 'number', min: '1', max: '65535' }), null, 'narrow'),
        findPort),
      h('div', { class: 'not-file' }, portHint),
      h('div', { class: 'only-process' },
        field('Adresse d’écoute', h('select', { name: 'hostControl', class: 'select' }, ...HOST_CONTROLS.map(([v, l]) => h('option', { value: v, text: l }))))),
      h('label', { class: 'setting-line not-file' },
        h('span', {}, h('strong', { text: 'Exposer sur le réseau local' }), exposeHint),
        input('expose', { type: 'checkbox', role: 'switch', class: 'switch' })),
      h('div', { class: 'only-process' },
        field('Variables d’environnement', area('env', { placeholder: 'NODE_ENV=production' }), 'Une par ligne, NOM=valeur. PORT et HOST sont fixés par Servynex.'))),
    section('Ouverture et vérification', 'not-file',
      field('Adresse ouverte', input('url', { placeholder: 'http://localhost:{port}', spellcheck: 'false' }), 'Ouverte par le bouton « Ouvrir ». Laisser vide s’il n’y a rien à ouvrir (base de données…).'),
      h('div', { class: 'form-row' },
        field('Vérifier que le projet répond', h('select', { name: 'healthKind', class: 'select' },
          h('option', { value: 'http', text: 'Une page web répond' }),
          h('option', { value: 'tcp', text: 'Le port est ouvert' })), null, 'grow'),
        field('Délai max. au démarrage (s)', input('startTimeout', { type: 'number', min: '1', max: '600', placeholder: '60' }), null, 'narrow')),
      h('div', { class: 'form-row health-http' },
        field('Page vérifiée', input('healthPath', { placeholder: '/', spellcheck: 'false' }), null, 'narrow'),
        field('Texte attendu dans la page', input('healthExpect', { placeholder: '<title>Mon projet', spellcheck: 'false' }), 'Facultatif : distingue le projet d’un autre site qui occuperait le port.', 'grow'))),
    section('Dépendances', 'not-file',
      h('div', { class: 'field' }, h('span', { text: 'Démarrer d’abord' }), requiresList,
        h('small', { class: 'field-hint', text: 'Lancés automatiquement avant ce projet, et arrêtés avec lui s’ils ne servent plus à rien d’autre.' })),
      h('label', { class: 'setting-line' },
        h('span', {}, h('strong', { text: 'C’est un service' }), h('span', { class: 'hint', text: 'Base de données, outil : rangé dans « Services », et en lune dans l’orbite.' })),
        input('service', { type: 'checkbox', role: 'switch', class: 'switch' })),
      h('label', { class: 'setting-line' },
        h('span', {}, h('strong', { text: 'Lancer au démarrage du PC' }), h('span', { class: 'hint', text: 'Une fois par démarrage, si Servynex démarre avec Windows.' })),
        input('autostart', { type: 'checkbox', role: 'switch', class: 'switch' }))),
    h('details', { class: 'form-section only-process' },
      h('summary', { text: 'Arrêt propre (facultatif)' }),
      h('p', { class: 'hint', text: 'Commande lancée pour arrêter le projet proprement (mysqladmin shutdown…). Sans elle, ou si elle ne suffit pas, Servynex arrête le processus de force.' }),
      field('Commande d’arrêt', input('stopCmd', { spellcheck: 'false' })),
      field('Arguments', area('stopArgs'), 'Un argument par ligne.')),
    h('div', { class: 'form-foot' }, message, submit),
  );

  const el = form.elements;
  const val = (name) => el[name].value.trim();

  // ---- Remplissage ----

  function renderRequires(selected) {
    const others = handlers.all().filter((x) => x.type !== 'file' && x.id !== project?.id);
    requiresList.replaceChildren(...(others.length
      ? others.map((x) => h('label', { class: 'sub-item' },
        h('input', { type: 'checkbox', class: 'check-input', name: 'requires', value: x.id, checked: selected.includes(x.id) }),
        h('span', { text: x.name })))
      : [h('p', { class: 'sub-item hint', text: 'Aucun autre projet à démarrer.' })]));
  }

  function renderPicker(color, current, name) {
    picker.style.setProperty('--c', color);
    const known = PROJECT_ICONS.includes(current);
    const initial = ((name ?? '').trim()[0] ?? '?').toUpperCase();
    const options = [[known ? 'initial' : current ?? 'initial', h('span', { text: initial }), 'Initiale du nom'], ...PROJECT_ICONS.map((n) => [n, icon(n), n])];
    picker.replaceChildren(...options.map(([value, content, title]) =>
      h('button', { type: 'button', role: 'radio', title, 'aria-label': title, dataset: { value } }, content)));
    pickIcon(current ?? 'initial');
  }

  function pickIcon(value) {
    picker.dataset.value = value;
    for (const b of picker.children) b.setAttribute('aria-checked', String(b.dataset.value === value));
  }

  function setType(value) {
    form.dataset.type = value;
    for (const b of typeGroup.children) b.setAttribute('aria-checked', String(b.dataset.value === value));
    hints();
  }

  async function fill(p) {
    project = p;
    dirty.clear();
    const s = p?.source ?? {};
    el.name.value = s.name ?? '';
    el.color.value = s.color ?? COLORS[handlers.all().length % COLORS.length];
    el.desc.value = s.desc ?? '';
    el.tags.value = (s.tags ?? []).join(', ');
    renderPicker(el.color.value, p ? s.icon ?? 'box' : 'box', s.name);
    setType(s.type ?? 'process');
    el.cwd.value = s.cwd ?? '';
    el.cmd.value = s.cmd ?? (p ? '' : 'node');
    el.args.value = (s.args ?? []).join('\n');
    el.open.value = s.open ?? (p ? '' : 'index.html');
    el.port.value = s.port ?? '';
    el.port.disabled = Boolean(s.fixedPort);
    findPort.disabled = Boolean(s.fixedPort);
    el.hostControl.value = s.hostControl ?? 'env';
    el.expose.checked = Boolean(s.expose);
    el.env.value = Object.entries(s.env ?? {}).map(([k, v]) => `${k}=${v}`).join('\n');
    el.url.value = s.url ?? (p ? '' : 'http://localhost:{port}');
    el.healthKind.value = s.health?.type === 'tcp' ? 'tcp' : 'http';
    el.healthPath.value = s.health?.path ?? '';
    el.healthExpect.value = s.health?.expect ?? '';
    el.startTimeout.value = s.startTimeout ?? '';
    renderRequires(s.requires ?? []);
    el.service.checked = Boolean(s.service);
    el.autostart.checked = Boolean(s.autostart);
    el.stopCmd.value = s.stop?.cmd ?? '';
    el.stopArgs.value = (s.stop?.args ?? []).join('\n');
    form.querySelector('details').open = Boolean(s.stop);
    message.textContent = '';
    message.className = 'hint';
    applyPermissions();
    hints();
    if (!p) {
      try {
        el.port.value = (await api('free-port?from=3000')).port;
        hints();
      } catch {}
    }
  }

  // Lecture seule sans « Modifier les projets » ; sans « Commandes », les champs
  // qui décident de ce qui s'exécute restent grisés (Servynex le refuse de toute façon).
  function applyPermissions() {
    const edit = can('edit');
    const commands = can('commands');
    for (const el of form.querySelectorAll('input, select, textarea, button:not([type="submit"])')) {
      el.disabled = !edit || (!commands && (SENSITIVE.includes(el.name) || typeGroup.contains(el)));
    }
    if (project?.source?.fixedPort) el.port.disabled = findPort.disabled = true;
    submit.hidden = !edit;
    notice.hidden = edit && commands;
    notice.textContent = !edit
      ? 'Lecture seule : votre compte ne peut pas modifier les projets.'
      : 'Dossier, commande, arguments, environnement, arrêt et exposition demandent la permission « Commandes ».';
  }

  // Le projet a changé ailleurs (fichier relu, autre onglet) : on réaffiche s'il
  // n'y a pas de saisie en cours ; sinon, seuls les champs touchés seront envoyés.
  function refresh(p) {
    if (!project || p.id !== project.id) return;
    const changed = JSON.stringify(p.source) !== JSON.stringify(project.source);
    if (!changed) {
      project = p;
      return hints();
    }
    if (!dirty.size) return fill(p);
    project = p;
    message.textContent = 'Ce projet vient d’être modifié ailleurs : seuls les champs que vous avez changés seront enregistrés.';
    message.className = 'hint warn';
  }

  // ---- Lecture ----

  function values() {
    const type = form.dataset.type;
    const tags = el.tags.value.split(',').map((t) => t.trim()).filter(Boolean);
    const v = {
      name: val('name'),
      desc: val('desc') || null,
      tags: tags.length ? tags : null,
      icon: picker.dataset.value,
      color: el.color.value,
      type: type === 'process' ? null : type,
      cwd: val('cwd'),
    };
    if (type === 'file') return { ...v, open: val('open') };
    const kind = el.healthKind.value;
    const health = kind === 'tcp'
      ? { type: 'tcp' }
      : Object.fromEntries([['path', val('healthPath')], ['expect', el.healthExpect.value]].filter(([, x]) => x));
    Object.assign(v, {
      port: el.port.value === '' ? null : Number(el.port.value),
      expose: el.expose.checked,
      url: val('url') || null,
      health: Object.keys(health).length ? health : null,
      startTimeout: el.startTimeout.value === '' ? null : Number(el.startTimeout.value),
      requires: [...form.querySelectorAll('input[name="requires"]:checked')].map((c) => c.value),
      service: el.service.checked || null,
      autostart: el.autostart.checked || null,
    });
    if (!v.requires.length) v.requires = null;
    if (type === 'static') return v;
    const env = {};
    for (const line of lines(el.env.value)) {
      const i = line.indexOf('=');
      if (i < 1) throw new Error(`Variable d’environnement sans « = » : ${line}`);
      env[line.slice(0, i).trim()] = line.slice(i + 1).trim();
    }
    return {
      ...v,
      cmd: val('cmd'),
      args: lines(el.args.value),
      hostControl: el.hostControl.value,
      env: Object.keys(env).length ? env : null,
      stop: val('stopCmd') ? { cmd: val('stopCmd'), args: lines(el.stopArgs.value) } : null,
    };
  }

  // Modification envoyée pour un projet existant : les clés touchées seulement ;
  // tout, si le type a changé (les champs requis diffèrent).
  function patch() {
    const all = values();
    if (dirty.has('type')) return all;
    return Object.fromEntries(Object.entries(all).filter(([k]) => dirty.has(k)));
  }

  // ---- Aides contextuelles ----

  function hint(node, text, kind = '') {
    node.textContent = text;
    node.className = `${node.tagName === 'SMALL' ? 'field-hint' : 'hint'} ${kind}`.trim();
  }

  function reachableAt(port) {
    const urls = (handlers.instance().lan ?? []).map((a) => `http://${a.address}:${port}`);
    return urls.length ? `, par exemple ${urls.join(', ')}` : '';
  }

  function hints() {
    picker.style.setProperty('--c', el.color.value);
    form.classList.toggle('tcp', el.healthKind.value === 'tcp');
    const p = project;
    const s = p?.source ?? {};
    const port = Number(el.port.value);
    const expose = el.expose.checked;
    const hostControl = form.dataset.type === 'static' ? 'internal' : el.hostControl.value;
    if (s.fixedPort) hint(portHint, s.fixedPort);
    else if (p?.activePort && p.activePort !== port) hint(portHint, `Tourne encore sur le port ${p.activePort} : redémarrer le projet pour appliquer.`, 'warn');
    else if (hostControl === 'env') hint(portHint, 'Transmis par la variable PORT : le projet doit la lire.');
    else hint(portHint, 'Utilisé quand Servynex lance le projet.');
    el.expose.disabled = hostControl === 'local' || !can('commands');
    if (hostControl === 'local') hint(exposeHint, 'Le projet n’écoute que sur ce PC, quel que soit ce réglage.');
    else if (p?.activeHost && (p.activeHost === '0.0.0.0') !== expose) hint(exposeHint, 'Redémarrer le projet pour appliquer.', 'warn');
    else if (expose) hint(exposeHint, `Écoute sur 0.0.0.0:${port || '…'} : joignable par toute machine du réseau${reachableAt(port)}, si le pare-feu Windows le permet. À réserver aux réseaux de confiance.`, 'warn');
    else hint(exposeHint, 'Écoute sur 127.0.0.1 : seul ce PC peut l’ouvrir.');
  }

  // ---- Évènements ----

  const touch = (name) => {
    if (name) dirty.add(KEY_OF[name] ?? name);
  };
  form.addEventListener('input', (e) => {
    touch(e.target.name);
    if (e.target.name === 'name' && !PROJECT_ICONS.includes(picker.dataset.value)) {
      picker.firstElementChild.replaceChildren(h('span', { text: (val('name')[0] ?? '?').toUpperCase() }));
    }
    hints();
  });
  form.addEventListener('change', (e) => touch(e.target.name));
  picker.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b) return;
    pickIcon(b.dataset.value);
    touch('icon');
  });
  typeGroup.addEventListener('click', (e) => {
    const b = e.target.closest('button');
    if (!b || b.dataset.value === form.dataset.type) return;
    setType(b.dataset.value);
    touch('type');
  });
  findPort.addEventListener('click', async () => {
    try {
      el.port.value = (await api(`free-port?from=${(Number(el.port.value) || 2999) + 1}`)).port;
      touch('port');
      hints();
    } catch (err) {
      toast(err.message, 'error');
    }
  });
  form.addEventListener('submit', async (e) => {
    e.preventDefault();
    if (!val('name')) return hint(message, 'Le nom est obligatoire.', 'warn');
    let body;
    try {
      body = project ? patch() : values();
    } catch (err) {
      return hint(message, err.message, 'warn');
    }
    if (project && !Object.keys(body).length) return hint(message, 'Rien à enregistrer.');
    submit.disabled = true;
    try {
      const result = await onSubmit(body, project);
      const relaunch = project && project.state === 'running' && project.managed && Object.keys(body).some((k) => LAUNCH_KEYS.includes(k));
      dirty.clear();
      hint(message, relaunch ? 'Enregistré. Redémarrer le projet pour appliquer.' : 'Enregistré.', 'ok');
      return result;
    } catch (err) {
      hint(message, err.message, 'warn');
    } finally {
      submit.disabled = false;
    }
  });

  return {
    element: form,
    fill,
    refresh,
    get project() {
      return project;
    },
    get changedKeys() {
      return [...dirty];
    },
    // Les autres projets ont changé : la liste « Démarrer d'abord » suit.
    projectsChanged() {
      if (!dirty.has('requires')) renderRequires(project?.source?.requires ?? []);
    },
  };
}
