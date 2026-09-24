import assert from 'node:assert/strict';
import fs from 'node:fs';
import { describe, test } from 'node:test';
import { LocalError, detectLang, langOf, msg, setServerLang, tr } from '../src/i18n.js';
import serverEn from '../src/locales/en.js';
import serverFr from '../src/locales/fr.js';
import uiEn from '../public/locales/en.js';
import uiFr from '../public/locales/fr.js';

const params = (text) => [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

// Chaque texte existe dans les deux langues, avec les mêmes paramètres {…}.
function sameShape(en, fr, where) {
  assert.deepEqual(Object.keys(fr).sort(), Object.keys(en).sort(), `${where} : mêmes clés`);
  for (const key of Object.keys(en)) {
    assert.equal(typeof en[key], 'string', `${where} ${key}`);
    assert.ok(en[key].trim() && fr[key].trim(), `${where} ${key} : texte vide`);
    assert.deepEqual(params(fr[key]), params(en[key]), `${where} ${key} : mêmes paramètres`);
  }
}

describe('langues', () => {
  test('serveur et interface : mêmes clés et mêmes paramètres en anglais et en français', () => {
    sameShape(serverEn, serverFr, 'serveur');
    sameShape(uiEn, uiFr, 'interface');
  });

  test('traduction : paramètres, message imbriqué, clé inconnue', () => {
    const key = Object.keys(serverEn).find((k) => params(serverEn[k]).length) ?? assert.fail('aucun message à paramètre');
    const values = Object.fromEntries(params(serverEn[key]).map((p) => [p, `<${p}>`]));
    assert.ok(!/\{\w+\}/.test(tr(key, values, 'en')), 'paramètres remplacés');
    assert.equal(tr('clé.inconnue', {}, 'fr'), 'clé.inconnue');
    assert.equal(tr('config.duplicatePort', { port: 3001, first: msg('config.instance'), second: 'B' }, 'en'), 'Port 3001 used by both Servynex and B');
    assert.equal(tr('config.duplicatePort', { port: 3001, first: msg('config.instance'), second: 'B' }, 'fr'), 'Port 3001 utilisé à la fois par Servynex et B');
  });

  test('langue du serveur : variable SERVYNEX_LANG, sinon celle du système, sinon l’anglais', () => {
    assert.equal(detectLang('fr-FR', undefined), 'fr');
    assert.equal(detectLang('de-DE', undefined), 'en');
    assert.equal(detectLang('fr-FR', 'en'), 'en');
    assert.equal(detectLang('en-US', 'fr-CA'), 'fr');
  });

  test('langue d’une requête : Accept-Language, sinon celle du serveur', () => {
    setServerLang('fr');
    assert.equal(langOf({ headers: { 'accept-language': 'en-US,en;q=0.9' } }), 'en');
    assert.equal(langOf({ headers: { 'accept-language': 'de-DE, fr;q=0.8' } }), 'fr');
    assert.equal(langOf({ headers: {} }), 'fr');
  });

  test('erreur traduisible : message dans la langue du serveur, retraduit à la demande', () => {
    setServerLang('fr');
    const err = new LocalError(404, 'project.unknown');
    assert.equal(err.message, serverFr['project.unknown']);
    assert.equal(err.in('en'), serverEn['project.unknown']);
    assert.equal(err.status, 404);
  });

  test('chaque clé utilisée par l’interface et le serveur existe', () => {
    const read = (dir, ext) => fs.readdirSync(dir).filter((f) => f.endsWith(ext)).map((f) => fs.readFileSync(`${dir}/${f}`, 'utf8')).join('\n');
    const ui = read('public/js', '.js');
    const pages = read('public', '.html');
    const used = new Set();
    for (const m of ui.matchAll(/\bt\('([\w.]+)'/g)) used.add(m[1]);
    for (const m of ui.matchAll(/\btn\('([\w.]+)'/g)) used.add(`${m[1]}.one`).add(`${m[1]}.other`);
    for (const m of pages.matchAll(/data-i18n(?:-rich)?="([\w.]+)"/g)) used.add(m[1]);
    for (const m of pages.matchAll(/data-i18n-attr="([^"]+)"/g)) for (const pair of m[1].split(';')) used.add(pair.split(':')[1]);
    assert.deepEqual([...used].filter((k) => !(k in uiEn)), [], 'interface');
    const server = [read('src', '.js'), fs.readFileSync('server.js', 'utf8'), fs.readFileSync('scripts/auth.js', 'utf8')].join('\n');
    const keys = [...server.matchAll(/'((?:config|project|process|http|auth|audit|history|log|system|cli|time|role)\.[A-Za-z.]+)'/g)].map((m) => m[1]).filter((k) => k !== 'auth.json');
    assert.deepEqual(keys.filter((k) => !(k in serverEn)), [], 'serveur');
  });
});
