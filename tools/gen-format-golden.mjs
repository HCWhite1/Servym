// Fichiers de référence du formateur : entrée JSON et sortie de formatConfig() de la version Node.
// Le test Rust (crates/servynex-core/tests/format.rs) doit produire la même sortie, octet par octet.
// Usage : node tools/gen-format-golden.mjs
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { formatConfig } from '../src/config.js';

const root = path.resolve(import.meta.dirname, '..');
const out = path.join(root, 'crates', 'servynex-core', 'tests', 'golden');
mkdirSync(out, { recursive: true });

const long = 'x'.repeat(170);
const cases = {
  example: readFileSync(path.join(root, 'projects.example.json'), 'utf8'),
  // Nombres écrits avec décimale, textes à échapper, Unicode (hors BMP compris).
  numbers: '{ "instance": { "port": 9000.0 }, "projects": [{ "id": "a", "port": 3000.0, "startTimeout": 1.5, "big": 1e21, "neg": -0.25 }] }',
  strings: JSON.stringify({ instance: { port: 1 }, projects: [{ id: 'é', name: 'Tab\tquote" back\\ \u0001 🎯  ', desc: '</script>' }] }),
  // Petits objets sur une ligne, grands sur plusieurs ; listes vides ; objets vides.
  nesting: JSON.stringify({
    instance: { port: 9000, name: 'Maison', lang: 'fr' },
    projects: [
      { id: 'a', tags: [], env: {}, health: { path: '/', expect: 'ok' }, extra: { deep: { deeper: { deepest: [1, 2, { k: 'v' }] } } } },
      { id: 'b', env: { A: long }, args: [long, 'b'], nested: { list: [{ a: long }, { b: 1 }] } },
    ],
    unknown: { kept: [true, false, null], empty: [] },
  }),
  // Limite de 160 caractères pour une ligne.
  limit: JSON.stringify({ instance: { port: 1 }, projects: [{ id: 'l', args: ['a'.repeat(140)], args2: ['a'.repeat(160)] }] }),
  empty: '{ "instance": { "port": 9000 }, "projects": [] }',
};

for (const [name, text] of Object.entries(cases)) {
  writeFileSync(path.join(out, `${name}.json`), text);
  writeFileSync(path.join(out, `${name}.out`), `${formatConfig(JSON.parse(text))}\n`);
}
console.log(`${Object.keys(cases).length} cas écrits dans ${path.relative(root, out)}`);
