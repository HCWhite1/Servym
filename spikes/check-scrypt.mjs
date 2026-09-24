// Vérifie avec verifySecret() de Node les hachages produits par l'essai Rust.
import { readFileSync } from 'node:fs';
import { verifySecret } from '../src/auth.js';

const vectors = JSON.parse(readFileSync(new URL('./scrypt-node/rust-vectors.json', import.meta.url), 'utf8'));
let failures = 0;
for (const { secret, hash } of vectors) {
  const ok = await verifySecret(secret, hash);
  console.log(`${ok ? 'OK   ' : 'ÉCHEC'} ${JSON.stringify(secret)}`);
  if (!ok) failures++;
}
process.exit(failures ? 1 : 0);
