// Langue de l'interface : français ou anglais. Choisie dans Paramètres ›
// Apparence (gardée par le navigateur, et par le compte si la protection est
// active), sinon celle du navigateur. Textes : public/locales/<langue>.js, sous
// des clés ; {nom} y est remplacé par le paramètre du même nom.
import en from '../locales/en.js';
import fr from '../locales/fr.js';

export const LANGS = { en: 'English', fr: 'Français' };
const MESSAGES = { en, fr };
const STORE_KEY = 'servynex.lang';

const pick = (value) => Object.keys(LANGS).find((l) => String(value ?? '').toLowerCase().startsWith(l)) ?? null;

// 'auto' (langue du navigateur) ou une langue de LANGS.
export function choice() {
  try {
    return pick(JSON.parse(localStorage.getItem(STORE_KEY))) ?? 'auto';
  } catch {
    return 'auto';
  }
}

export function setChoice(value) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(value));
  } catch {}
}

export const lang = pick(choice()) ?? navigator.languages.map(pick).find(Boolean) ?? 'en';
document.documentElement.lang = lang;

// Système de la machine de Servynex : une clé peut avoir sa variante « clé@linux »
// (« Démarrer Servynex avec la session » plutôt qu'« avec Windows »).
let platform = null;
export function setPlatform(value) {
  const changed = value !== platform;
  platform = value;
  return changed;
}

const message = (key) => MESSAGES[lang][key] ?? MESSAGES.en[key];

export function t(key, params = {}) {
  const text = (platform && message(`${key}@${platform}`)) || message(key) || key;
  return text.replace(/\{(\w+)\}/g, (whole, name) => (name in params ? String(params[name]) : whole));
}

// Pluriel : clé.one ou clé.other selon n, avec les règles de la langue
// (en français, 0 et 1 sont au singulier).
const plurals = new Intl.PluralRules(lang);
export const tn = (key, n, params = {}) => t(`${key}.${plurals.select(n) === 'one' ? 'one' : 'other'}`, { n, ...params });

// Texte avec <b>, <kbd> ou <code> : chaque élément est créé à part, sans innerHTML.
const TAGS = { b: 'strong', kbd: 'kbd', code: 'code' };
export function rich(key, params) {
  const nodes = [];
  let open = null;
  for (const part of t(key, params).split(/(<\/?(?:b|kbd|code)>)/)) {
    const tag = part.match(/^<(\/?)(b|kbd|code)>$/);
    if (tag) {
      open = tag[1] ? null : document.createElement(TAGS[tag[2]]);
      if (open) nodes.push(open);
    } else if (part) {
      if (open) open.append(part);
      else nodes.push(document.createTextNode(part));
    }
  }
  return nodes;
}

// data-i18n : texte propre de l'élément (ses icônes et autres enfants restent) ·
// data-i18n-rich : contenu mis en forme · data-i18n-attr="title:clé;placeholder:clé".
export function translate(root = document) {
  for (const el of root.querySelectorAll('[data-i18n]')) setOwnText(el, t(el.dataset.i18n));
  for (const el of root.querySelectorAll('[data-i18n-rich]')) el.replaceChildren(...rich(el.dataset.i18nRich));
  for (const el of root.querySelectorAll('[data-i18n-attr]')) {
    for (const pair of el.dataset.i18nAttr.split(';')) {
      const [attr, key] = pair.split(':');
      el.setAttribute(attr, t(key));
    }
  }
  for (const tpl of root.querySelectorAll('template')) translate(tpl.content);
}

function setOwnText(el, text) {
  if (!el.children.length) {
    el.textContent = text;
    return;
  }
  const node = [...el.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && n.nodeValue.trim());
  if (!node) return el.append(text);
  // Espaces autour du texte gardés (« <input> Texte »).
  const [, before, , after] = node.nodeValue.match(/^(\s*)([\s\S]*?)(\s*)$/);
  node.nodeValue = `${before}${text}${after}`;
}
