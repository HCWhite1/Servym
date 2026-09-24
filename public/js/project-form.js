// Formulaire d'un projet : sert aux Réglages d'un projet existant et à l'ajout.
// Chaque champ porte la clé de projects.json qu'il modifie (data-key) ; à
// l'enregistrement d'un projet existant, seules les clés touchées sont envoyées,
// pour ne jamais renvoyer une valeur périmée (projet modifié ailleurs entre-temps).
import { t } from './i18n.js';
import { PROJECT_ICONS, api, can, h, icon, toast } from './lib.js';

const TYPES = ['process', 'static', 'file'].map((v) => [v, t(`form.type.${v}`), t(`form.type.${v}.desc`)]);
const HOST_CONTROLS = ['env', 'args', 'local'].map((v) => [v, t(`form.host.${v}`)]);
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
  const picker = h('div', { class: 'icon-picker', role: 'radiogroup', 'aria-label': t('form.icon') });
  const typeGroup = h('div', { class: 'type-choices', role: 'radiogroup', 'aria-label': t('form.typeLabel') },
    ...TYPES.map(([value, label, desc]) => h('button', { type: 'button', role: 'radio', class: 'choice', dataset: { value } }, h('span', { text: label }), h('small', { text: desc }))));
  const requiresList = h('div', { class: 'sub-list' });
  const portHint = h('small', { class: 'field-hint' });
  const exposeHint = h('span', { class: 'hint' });
  const message = h('p', { class: 'hint', role: 'status' });
  const findPort = h('button', { class: 'btn', type: 'button', text: t('form.findPort') });
  const submit = h('button', { class: 'btn primary', type: 'submit', text: submitLabel });
  const notice = h('p', { class: 'alert', hidden: true });
  const folderNotice = h('div', { class: 'alert warn', role: 'status', hidden: true });

  const section = (title, cls, ...children) => h('section', { class: `form-section ${cls}`.trim() }, h('h3', { text: title }), ...children);

  form.append(
    notice,
    folderNotice,
    section(t('settings.general'), 'first',
      h('div', { class: 'form-row' },
        field(t('col.name'), input('name', { maxlength: '40', required: true, placeholder: t('form.namePlaceholder') }), null, 'grow'),
        field(t('form.color'), input('color', { type: 'color' }))),
      field(t('form.desc'), input('desc', { maxlength: '160', placeholder: t('form.descPlaceholder') })),
      field(t('info.tags'), input('tags', { placeholder: 'Express, SQLite' }), t('form.tagsHint')),
      h('div', { class: 'field' }, h('span', { text: t('form.icon') }), picker)),
    section(t('form.typeSection'), '', typeGroup),
    section(t('form.launch'), '',
      field(t('info.folder'), input('cwd', { placeholder: t('form.cwdPlaceholder'), spellcheck: 'false' }), t('form.cwdHint')),
      h('div', { class: 'only-process' },
        field(t('info.command'), input('cmd', { placeholder: 'node', spellcheck: 'false' }), t('form.cmdHint')),
        field(t('form.args'), area('args', { placeholder: 'server.js' }), t('form.argsHint'))),
      h('div', { class: 'only-file' },
        field(t('form.open'), input('open', { placeholder: 'index.html', spellcheck: 'false' }), t('form.openHint'))),
      h('div', { class: 'not-file form-row' },
        field(t('network.col.port'), input('port', { type: 'number', min: '1', max: '65535' }), null, 'narrow'),
        findPort),
      h('div', { class: 'not-file' }, portHint),
      h('div', { class: 'only-process' },
        field(t('form.hostControl'), h('select', { name: 'hostControl', class: 'select' }, ...HOST_CONTROLS.map(([v, l]) => h('option', { value: v, text: l }))))),
      h('label', { class: 'setting-line not-file' },
        h('span', {}, h('strong', { text: t('form.expose') }), exposeHint),
        input('expose', { type: 'checkbox', role: 'switch', class: 'switch' })),
      h('div', { class: 'only-process' },
        field(t('form.env'), area('env', { placeholder: 'NODE_ENV=production' }), t('form.envHint')))),
    section(t('form.openCheck'), 'not-file',
      field(t('form.url'), input('url', { placeholder: 'http://localhost:{port}', spellcheck: 'false' }), t('form.urlHint')),
      h('div', { class: 'form-row' },
        field(t('form.health'), h('select', { name: 'healthKind', class: 'select' },
          h('option', { value: 'http', text: t('form.healthHttp') }),
          h('option', { value: 'tcp', text: t('form.healthTcp') })), null, 'grow'),
        field(t('form.timeout'), input('startTimeout', { type: 'number', min: '1', max: '600', placeholder: '60' }), null, 'narrow')),
      h('div', { class: 'form-row health-http' },
        field(t('form.healthPath'), input('healthPath', { placeholder: '/', spellcheck: 'false' }), null, 'narrow'),
        field(t('form.healthExpect'), input('healthExpect', { placeholder: t('form.expectPlaceholder'), spellcheck: 'false' }), t('form.healthExpectHint'), 'grow'))),
    section(t('form.requires'), 'not-file',
      h('div', { class: 'field' }, h('span', { text: t('form.requiresFirst') }), requiresList,
        h('small', { class: 'field-hint', text: t('form.requiresHint') })),
      h('label', { class: 'setting-line' },
        h('span', {}, h('strong', { text: t('form.service') }), h('span', { class: 'hint', text: t('form.serviceHint') })),
        input('service', { type: 'checkbox', role: 'switch', class: 'switch' })),
      h('label', { class: 'setting-line' },
        h('span', {}, h('strong', { text: t('form.autostart') }), h('span', { class: 'hint', 'data-i18n': 'form.autostartHint', text: t('form.autostartHint') })),
        input('autostart', { type: 'checkbox', role: 'switch', class: 'switch' })),
      h('label', { class: 'setting-line only-process' },
        h('span', {}, h('strong', { text: t('form.restartOnCrash') }), h('span', { class: 'hint', text: t('form.restartOnCrashHint') })),
        input('restartOnCrash', { type: 'checkbox', role: 'switch', class: 'switch' }))),
    h('details', { class: 'form-section only-process' },
      h('summary', { text: t('form.stop') }),
      h('p', { class: 'hint', text: t('form.stopHint') }),
      field(t('form.stopCmd'), input('stopCmd', { spellcheck: 'false' })),
      field(t('form.args'), area('stopArgs'), t('form.stopArgsHint'))),
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
      : [h('p', { class: 'sub-item hint', text: t('form.noOther') })]));
  }

  function renderPicker(color, current, name) {
    picker.style.setProperty('--c', color);
    const known = PROJECT_ICONS.includes(current);
    const initial = ((name ?? '').trim()[0] ?? '?').toUpperCase();
    const options = [[known ? 'initial' : current ?? 'initial', h('span', { text: initial }), t('form.initial')], ...PROJECT_ICONS.map((n) => [n, icon(n), n])];
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
    el.restartOnCrash.checked = Boolean(s.restartOnCrash);
    el.stopCmd.value = s.stop?.cmd ?? '';
    el.stopArgs.value = (s.stop?.args ?? []).join('\n');
    form.querySelector('details').open = Boolean(s.stop);
    message.textContent = '';
    message.className = 'hint';
    folderNotice.hidden = true;
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
      ? t('form.readOnly')
      : t('form.needsCommands');
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
    message.textContent = t('form.changedElsewhere');
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
      restartOnCrash: type === 'static' ? null : el.restartOnCrash.checked || null,
    });
    if (!v.requires.length) v.requires = null;
    if (type === 'static') return v;
    const env = {};
    for (const line of lines(el.env.value)) {
      const i = line.indexOf('=');
      if (i < 1) throw new Error(t('form.envNoEquals', { line }));
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
    return urls.length ? t('expose.forExample', { urls: urls.join(', ') }) : '';
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
    else if (p?.activePort && p.activePort !== port) hint(portHint, t('form.portStillRunning', { port: p.activePort }), 'warn');
    else if (hostControl === 'env') hint(portHint, t('form.portEnv'));
    else hint(portHint, t('form.portUsed'));
    el.expose.disabled = hostControl === 'local' || !can('commands');
    if (hostControl === 'local') hint(exposeHint, t('form.exposeLocal'));
    else if (p?.activeHost && (p.activeHost === '0.0.0.0') !== expose) hint(exposeHint, t('form.restartToApply'), 'warn');
    else if (expose) hint(exposeHint, t('form.exposeOn', { port: port || '…', reachable: reachableAt(port) }), 'warn');
    else hint(exposeHint, t('form.exposeOff'));
  }

  // ---- Évènements ----

  const touch = (name) => {
    if (name) dirty.add(KEY_OF[name] ?? name);
  };
  form.addEventListener('input', (e) => {
    touch(e.target.name);
    if (e.target.name === 'cwd') folderNotice.hidden = true;
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
    if (!val('name')) return hint(message, t('form.nameRequired'), 'warn');
    let body;
    try {
      body = project ? patch() : values();
    } catch (err) {
      return hint(message, err.message, 'warn');
    }
    if (project && !Object.keys(body).length) return hint(message, t('form.nothing'));
    submit.disabled = true;
    try {
      const result = await onSubmit(body, project);
      const relaunch = project && project.state === 'running' && project.managed && Object.keys(body).some((k) => LAUNCH_KEYS.includes(k));
      dirty.clear();
      hint(message, relaunch ? t('form.savedRestart') : t('form.saved'), 'ok');
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
    // Copie qui vient d'être créée dans le même dossier que l'original : certains
    // outils (serveur Symfony) n'y acceptent qu'un serveur. Masqué dès que le dossier change.
    warnSameFolder(original) {
      folderNotice.replaceChildren(icon('triangle-alert'), h('span', { text: t('duplicate.sameFolder', { name: original.name, path: original.cwd }) }));
      folderNotice.hidden = false;
    },
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
