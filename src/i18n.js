// Langues des messages du serveur : erreurs, états, journaux. Les textes sont
// dans src/locales/<langue>.js, sous des clés (« config.duplicatePort ») ;
// {nom} y est remplacé par le paramètre du même nom.
//
// Langue du serveur (journaux, états en direct) : instance.lang dans
// projects.json, sinon SERVYM_LANG, sinon celle du système, sinon l'anglais.
// Une erreur renvoyée à la page est retraduite dans la langue de la page
// (en-tête Accept-Language, posé par l'interface).
import en from './locales/en.js';
import fr from './locales/fr.js';

export const LANGS = ['en', 'fr'];
const MESSAGES = { en, fr };

const pick = (value) => LANGS.find((l) => String(value ?? '').trim().toLowerCase().startsWith(l)) ?? null;

export function detectLang(locale = Intl.DateTimeFormat().resolvedOptions().locale, env = process.env.SERVYM_LANG) {
  return pick(env) ?? pick(locale) ?? 'en';
}

let current = detectLang();
export const serverLang = () => current;

// lang absente : détection (variable d'environnement, système).
export function setServerLang(lang) {
  current = LANGS.includes(lang) ? lang : detectLang();
}

export function langOf(req) {
  for (const part of String(req.headers?.['accept-language'] ?? '').split(',')) {
    const lang = pick(part);
    if (lang) return lang;
  }
  return current;
}

// Message à traduire plus tard (paramètre d'un autre message, état d'un projet).
export const msg = (key, params = {}) => ({ key, params });

export function tr(key, params = {}, lang = current) {
  const text = MESSAGES[lang]?.[key] ?? MESSAGES.en[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (whole, name) => {
    if (!(name in params)) return whole;
    const value = params[name];
    return value && typeof value === 'object' && 'key' in value ? tr(value.key, value.params, lang) : String(value);
  });
}

// Erreur au message traduisible : message dans la langue du serveur (journaux),
// in(lang) pour la réponse à une page.
export class LocalError extends Error {
  constructor(status, key, params = {}, extra = null) {
    super(tr(key, params));
    this.status = status;
    this.key = key;
    this.params = params;
    if (extra) this.extra = extra;
  }

  in(lang) {
    return tr(this.key, this.params, lang);
  }
}
