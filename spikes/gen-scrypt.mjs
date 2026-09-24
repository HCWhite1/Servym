// Produit des hachages avec le vrai hashSecret() de Node, pour l'essai scrypt-node.
import { writeFileSync } from 'node:fs';
import { hashSecret } from '../src/auth.js';

const secrets = [
  'motdepasse123',
  'Café crème', // é précomposé
  'Café crème', // é et è décomposés : NFKC les recompose
  'ﬁnal Ｆｕｌｌ①', // ligature, pleine chasse, chiffre cerclé : NFKC les remplace
  'ABCD-EFGH-JKLM-NPQR-STUV',
  '',
  '😀 emoji $ dollar',
];
const vectors = [];
for (const secret of secrets) vectors.push({ secret, hash: await hashSecret(secret) });
writeFileSync(new URL('./scrypt-node/vectors.json', import.meta.url), JSON.stringify(vectors, null, 2));
console.log(`${vectors.length} hachages écrits`);
