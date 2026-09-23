// Configuration unique de Servynex : projects.json. Le serveur la valide au
// chargement et à chaque modification depuis l'interface : un projet n'est
// enregistré que si la configuration entière reste valide.
import { existsSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import path from 'node:path';

export const ROOT_DIR = path.resolve(import.meta.dirname, '..');

// Configuration de départ quand projects.json n'existe pas encore (nouvelle installation).
export const DEFAULT_CONFIG = { instance: { port: 9000 }, projects: [] };

const ID = /^[a-z0-9-]{1,32}$/;
const COLOR = /^#[0-9a-f]{6}$/i;
const ICON = /^[a-z][a-z0-9-]{0,31}$/;
// Les arguments passent par cmd.exe (voir launch.ps1) : guillemets et % y
// garderaient un sens même entre guillemets. & | < > ^ ( ) sont refusés aussi,
// pour que la sécurité ne repose pas sur le seul placement des guillemets.
const SAFE_ARG = /^[^"%\r\n&|<>^()]*$/;
const SAFE_CHARS = 'sans " % & | < > ^ ( )';
const ENV_NAME = /^[A-Za-z_][A-Za-z0-9_]{0,63}$/;
const HOST_CONTROLS = ['env', 'args', 'local'];
const TYPES = ['static', 'file', undefined];

const isPort = (v) => Number.isInteger(v) && v >= 1 && v <= 65535;
const isText = (v, max) => typeof v === 'string' && v.trim().length > 0 && v.length <= max;
const isArgList = (v) => Array.isArray(v) && v.length <= 64 && v.every((a) => typeof a === 'string' && a.length <= 1000 && SAFE_ARG.test(a));

export class ConfigError extends Error {
  status = 400;
}

function check(ok, message) {
  if (!ok) throw new ConfigError(message);
}

function checkProject(p, ids) {
  check(p && typeof p === 'object' && !Array.isArray(p), 'Projet invalide');
  check(ID.test(p.id ?? ''), `Identifiant invalide : ${p.id} (lettres minuscules, chiffres et tirets)`);
  const where = `Projet « ${p.name ?? p.id} »`;
  check(!ids.has(p.id), `${where} : identifiant en double`);
  ids.add(p.id);
  check(isText(p.name, 40), `${where} : nom manquant ou trop long (40 caractères au plus)`);
  check(p.desc === undefined || (typeof p.desc === 'string' && p.desc.length <= 160), `${where} : description trop longue (160 caractères au plus)`);
  check(p.icon === undefined || (typeof p.icon === 'string' && ICON.test(p.icon)) || isText(p.icon, 8), `${where} : icône (icon) invalide`);
  check(p.color === undefined || COLOR.test(p.color), `${where} : couleur invalide`);
  check(p.tags === undefined || (Array.isArray(p.tags) && p.tags.length <= 6 && p.tags.every((t) => isText(t, 20))),
    `${where} : 6 étiquettes au plus, de 20 caractères au plus`);
  check(TYPES.includes(p.type), `${where} : type inconnu`);
  if (p.type === 'file') {
    // Fichier ouvert directement dans le navigateur : pas de port, pas de processus.
    check(p.port === undefined, `${where} : un projet « fichier » n'a pas de port`);
    check(isText(p.open, 260) && !path.isAbsolute(p.open) && !p.open.split(/[\\/]/).includes('..'),
      `${where} : le fichier à ouvrir doit être un chemin relatif au dossier`);
  } else {
    check(isPort(p.port), `${where} : port invalide`);
  }
  check(typeof p.cwd === 'string' && p.cwd.length <= 260 && path.isAbsolute(p.cwd), `${where} : le dossier doit être un chemin absolu (C:/…)`);
  if (p.type === undefined) {
    check(isText(p.cmd, 260) && SAFE_ARG.test(p.cmd), `${where} : commande manquante ou invalide (${SAFE_CHARS})`);
    check(isArgList(p.args), `${where} : arguments (args) invalides (${SAFE_CHARS})`);
    check(HOST_CONTROLS.includes(p.hostControl), `${where} : adresse d'écoute (hostControl) : ${HOST_CONTROLS.join(', ')}`);
  }
  if (p.stop !== undefined) {
    // Exécutée sans shell, mais validée comme cmd/args : une erreur remonte au
    // chargement plutôt qu'au moment d'arrêter le projet.
    check(p.type === undefined, `${where} : seul un programme a une commande d'arrêt`);
    check(p.stop && isText(p.stop.cmd, 260) && SAFE_ARG.test(p.stop.cmd) && isArgList(p.stop.args ?? []),
      `${where} : commande d'arrêt (stop) invalide (${SAFE_CHARS})`);
  }
  if (p.env !== undefined) {
    check(p.env && typeof p.env === 'object' && !Array.isArray(p.env) && Object.keys(p.env).length <= 64
      && Object.entries(p.env).every(([k, v]) => ENV_NAME.test(k) && typeof v === 'string' && v.length <= 1000 && SAFE_ARG.test(v)),
      `${where} : variables d'environnement (env) invalides (NOM=valeur, valeurs ${SAFE_CHARS})`);
  }
  check(p.url === undefined || (typeof p.url === 'string' && p.url.length <= 300 && /^https?:\/\//.test(p.url)),
    `${where} : l'adresse doit commencer par http:// ou https://`);
  if (p.health !== undefined) {
    const hc = p.health;
    check(hc && typeof hc === 'object' && ['http', 'tcp', undefined].includes(hc.type), `${where} : vérification invalide`);
    check(hc.path === undefined || (typeof hc.path === 'string' && hc.path.startsWith('/') && hc.path.length <= 200), `${where} : le chemin vérifié doit commencer par /`);
    check(hc.expect === undefined || (typeof hc.expect === 'string' && hc.expect.length <= 200), `${where} : texte attendu trop long`);
  }
  check(p.startTimeout === undefined || (Number.isInteger(p.startTimeout) && p.startTimeout >= 1 && p.startTimeout <= 600),
    `${where} : délai de démarrage entre 1 et 600 secondes`);
  check(p.requires === undefined || (Array.isArray(p.requires) && p.requires.every((d) => typeof d === 'string')), `${where} : dépendances invalides`);
  check(p.service === undefined || typeof p.service === 'boolean', `${where} : service doit valoir true ou false`);
  check(p.expose === undefined || typeof p.expose === 'boolean', `${where} : expose doit valoir true ou false`);
  check(p.fixedPort === undefined || isText(p.fixedPort, 300), `${where} : fixedPort doit être un texte`);
  check(p.autostart === undefined || typeof p.autostart === 'boolean', `${where} : autostart doit valoir true ou false`);
  check(!(p.autostart && p.type === 'file'), `${where} : un fichier ne se lance pas au démarrage (autostart)`);
}

// Dépendances : projet existant, pas un fichier, pas de boucle (A attend B qui attend A).
function checkRequires(projects) {
  const byId = new Map(projects.map((p) => [p.id, p]));
  for (const p of projects) {
    for (const dep of p.requires ?? []) {
      check(dep !== p.id, `${p.name} ne peut pas dépendre de lui-même`);
      check(byId.has(dep), `${p.name} dépend d'un projet inconnu : ${dep}`);
      check(byId.get(dep).type !== 'file', `${p.name} dépend de ${byId.get(dep).name}, qui s'ouvre comme un fichier et ne se lance pas`);
    }
  }
  const done = new Set();
  const visit = (id, trail) => {
    if (trail.includes(id)) {
      const loop = [...trail.slice(trail.indexOf(id)), id].map((x) => byId.get(x).name);
      throw new ConfigError(`Dépendances en boucle : ${loop.join(' → ')}`);
    }
    if (done.has(id)) return;
    for (const dep of byId.get(id).requires ?? []) visit(dep, [...trail, id]);
    done.add(id);
  };
  for (const p of projects) visit(p.id, []);
}

export function validate(config) {
  check(config && Array.isArray(config.projects), 'projects.json : liste « projects » manquante');
  check(isPort(config.instance?.port), 'projects.json : instance.port invalide');
  check(config.instance.name === undefined || isText(config.instance.name, 40), 'projects.json : instance.name trop long (40 caractères au plus)');
  const ids = new Set();
  const ports = new Map([[config.instance.port, 'Servynex']]);
  for (const p of config.projects) {
    checkProject(p, ids);
    if (p.port === undefined) continue;
    check(!ports.has(p.port), `Port ${p.port} utilisé à la fois par ${ports.get(p.port)} et ${p.name}`);
    ports.set(p.port, p.name);
  }
  checkRequires(config.projects);
  return config;
}

// Fichier absent : configuration vide (première installation). Fichier illisible
// ou invalide : ConfigError, avec la position de l'erreur donnée par JSON.parse.
export function loadConfig(file) {
  if (!existsSync(file)) return structuredClone(DEFAULT_CONFIG);
  let raw;
  try {
    raw = JSON.parse(readFileSync(file, 'utf8').replace(/^\uFEFF/, ''));
  } catch (err) {
    throw new ConfigError(`${path.basename(file)} illisible : ${err.message}`);
  }
  return validate(raw);
}

// JSON lisible et compact, comme un fichier écrit à la main : un projet par bloc,
// mais les listes et petits objets (tags, health, env…) sur une ligne.
export function formatConfig(value, depth = 0) {
  const pad = '  '.repeat(depth);
  const inline = (v) => {
    if (Array.isArray(v)) return `[${v.map(inline).join(', ')}]`;
    if (v && typeof v === 'object') {
      const entries = Object.entries(v);
      return entries.length ? `{ ${entries.map(([k, x]) => `${JSON.stringify(k)}: ${inline(x)}`).join(', ')} }` : '{}';
    }
    return JSON.stringify(v);
  };
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  // Racine, liste des projets et chaque projet : toujours sur plusieurs lignes.
  const alwaysBlock = depth < 3 && (depth === 0 || Array.isArray(value) || depth === 2);
  const flat = inline(value);
  if (!alwaysBlock && pad.length + flat.length <= 160) return flat;
  if (Array.isArray(value)) {
    if (!value.length) return '[]';
    return `[\n${value.map((v) => `${pad}  ${formatConfig(v, depth + 1)}`).join(',\n')}\n${pad}]`;
  }
  const entries = Object.entries(value);
  if (!entries.length) return '{}';
  return `{\n${entries.map(([k, v]) => `${pad}  ${JSON.stringify(k)}: ${formatConfig(v, depth + 1)}`).join(',\n')}\n${pad}}`;
}

export function saveConfig(file, config) {
  const tmp = `${file}.tmp`;
  writeFileSync(tmp, `${formatConfig(config)}\n`);
  renameSync(tmp, file);
}

// Champs d'un projet modifiables depuis l'interface. L'identifiant ne l'est pas :
// il nomme le journal et l'historique. La valeur null retire un champ facultatif.
const FIELDS = ['name', 'desc', 'icon', 'color', 'tags', 'type', 'cwd', 'cmd', 'args', 'env', 'stop', 'port', 'hostControl',
  'expose', 'url', 'health', 'startTimeout', 'open', 'requires', 'service', 'fixedPort', 'autostart'];
const REQUIRED = ['name', 'cwd'];

function applyFields(project, patch) {
  check(patch && typeof patch === 'object' && !Array.isArray(patch), 'Modification invalide');
  for (const [key, value] of Object.entries(patch)) {
    check(FIELDS.includes(key), `Champ non modifiable : ${key}`);
    if (value === null || value === undefined) {
      check(!REQUIRED.includes(key), `Champ obligatoire : ${key}`);
      delete project[key];
    } else {
      project[key] = key === 'tags' && Array.isArray(value) ? value.map((t) => String(t).trim()) : value;
    }
  }
  // Champs sans objet pour le type choisi : retirés plutôt que refusés.
  if (project.type === 'file') for (const k of ['port', 'cmd', 'args', 'env', 'stop', 'hostControl', 'expose', 'url', 'health', 'startTimeout', 'autostart', 'fixedPort']) delete project[k];
  if (project.type === 'static') for (const k of ['cmd', 'args', 'env', 'stop', 'hostControl', 'open']) delete project[k];
  if (project.type === undefined) delete project.open;
}

// Renvoie une nouvelle configuration (l'ancienne reste intacte si le patch est refusé).
export function patchProject(config, id, patch) {
  const next = structuredClone(config);
  const project = next.projects.find((p) => p.id === id);
  if (!project) throw Object.assign(new ConfigError('Projet inconnu'), { status: 404 });
  const before = { port: project.port, fixedPort: project.fixedPort };
  applyFields(project, patch);
  if (project.port !== before.port && before.fixedPort && 'port' in patch && !('fixedPort' in patch)) check(false, before.fixedPort);
  if (patch.expose) check(project.hostControl !== 'local', `${project.name} écoute toujours en local : pas d'exposition possible`);
  return validate(next);
}

// Identifiant tiré du nom : « Mon Application » → mon-application (unique).
export function slugify(name, taken) {
  const base = String(name).normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase()
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 28) || 'projet';
  let id = base;
  for (let i = 2; taken.has(id); i++) id = `${base}-${i}`;
  return id;
}

// Ajoute un projet ; renvoie { config, id }.
export function createProject(config, input) {
  check(input && typeof input === 'object' && !Array.isArray(input), 'Projet invalide');
  const next = structuredClone(config);
  const taken = new Set(next.projects.map((p) => p.id));
  const id = input.id ?? slugify(input.name ?? '', taken);
  check(!taken.has(id), `Identifiant déjà utilisé : ${id}`);
  const project = { id };
  const { id: _, ...fields } = input;
  applyFields(project, fields);
  if (project.type === undefined) {
    project.args ??= [];
    project.hostControl ??= 'env';
  }
  if (project.expose) check(project.hostControl !== 'local', `${project.name} écoute toujours en local : pas d'exposition possible`);
  next.projects.push(project);
  return { config: validate(next), id };
}

// Adresse d'écoute demandée au projet.
export const hostOf = (p) => (p.expose && p.hostControl !== 'local' ? '0.0.0.0' : '127.0.0.1');

export function expand(text, p) {
  return text
    .replaceAll('{port}', String(p.port))
    .replaceAll('{host}', hostOf(p))
    .replaceAll('{servynex}', ROOT_DIR.replaceAll('\\', '/'));
}

export const urlOf = (p) => (p.url ? expand(p.url, p) : null);
