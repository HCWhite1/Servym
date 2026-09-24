// Parité des tests : chaque test JavaScript (tests/*.test.js) a son test Rust, qui le nomme
// dans un commentaire « /// JS : <fichier> › <describe> › <test> ». urls.test.js reste en
// JavaScript (code de l'interface). Usage : node tools/test-parity.mjs
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const STAYS_JS = new Set(['urls.test.js']);

// Noms complets des tests JS : describe imbriqués (indentation de 2 espaces) puis le test.
function jsTests(file) {
  const names = [];
  const stack = []; // [indentation, nom]
  for (const line of readFileSync(path.join(root, 'tests', file), 'utf8').split(/\r?\n/)) {
    const m = line.match(/^(\s*)(describe|test)\((?:'((?:[^'\\]|\\.)*)'|"((?:[^"\\]|\\.)*)"|`([^`]*)`)/);
    if (!m) continue;
    const indent = m[1].length;
    const name = (m[3] ?? m[4] ?? m[5]).replace(/\\(.)/g, '$1');
    while (stack.length && stack.at(-1)[0] >= indent) stack.pop();
    if (m[2] === 'describe') stack.push([indent, name]);
    else names.push([file, ...stack.map(([, n]) => n), name].join(' › '));
  }
  return names;
}

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    const full = path.join(dir, e.name);
    if (e.isDirectory()) return e.name === 'target' ? [] : rustFiles(full);
    return e.name.endsWith('.rs') ? [full] : [];
  });
}

const js = readdirSync(path.join(root, 'tests')).filter((f) => f.endsWith('.test.js') && !STAYS_JS.has(f)).flatMap(jsTests);
const rust = new Map(); // nom → fichiers Rust
for (const file of rustFiles(path.join(root, 'crates'))) {
  for (const m of readFileSync(file, 'utf8').matchAll(/^\s*\/\/\/ JS : (.+)$/gm)) {
    const name = m[1].trim();
    rust.set(name, [...(rust.get(name) ?? []), path.relative(root, file)]);
  }
}

const missing = js.filter((n) => !rust.has(n));
const unknown = [...rust.keys()].filter((n) => !js.includes(n));
const doubles = [...rust].filter(([, files]) => files.length > 1);
for (const n of missing) console.log(`sans test Rust : ${n}`);
for (const n of unknown) console.log(`test JS introuvable : ${n} (${rust.get(n).join(', ')})`);
for (const [n, files] of doubles) console.log(`en double : ${n} (${files.join(', ')})`);
const skipped = readdirSync(path.join(root, 'tests')).filter((f) => STAYS_JS.has(f)).flatMap(jsTests).length;
console.log(`${js.length} tests JS portés, ${rust.size} tests Rust nommés, ${skipped} restés en JavaScript.`);
process.exit(missing.length || unknown.length || doubles.length ? 1 : 0);
