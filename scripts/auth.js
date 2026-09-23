// Secours en ligne de commande pour les comptes de Servynex, à lancer sur la machine :
//   npm run auth -- list                   comptes existants
//   npm run auth -- reset-password <nom>   code de réinitialisation valable 15 minutes
// Avoir accès à cette machine donne de toute façon la main sur Servynex : ces
// commandes n'ouvrent aucune porte nouvelle. Dernier recours : supprimer
// runtime/auth.json retire la protection (et tous les comptes).
import path from 'node:path';
import { AuthStore, createResetCode, roleOf } from '../src/auth.js';
import { ROOT_DIR } from '../src/config.js';

const ROLE_LABELS = { admin: 'administrateur', editor: 'éditeur', operator: 'opérateur', readonly: 'lecture seule', custom: 'personnalisé' };
const store = new AuthStore(path.join(ROOT_DIR, 'runtime', 'auth.json'));
const [command, name] = process.argv.slice(2);

if (store.error) {
  console.error(store.error);
  process.exit(1);
}
const data = store.load();

if (command === 'list') {
  if (!data.users.length) console.log('Aucun compte : la protection n’a jamais été activée.');
  else {
    console.log(`Protection ${data.enabled ? 'active' : 'désactivée'} · ${data.users.length} compte(s)`);
    for (const u of data.users) console.log(`  ${u.username.padEnd(20)} ${ROLE_LABELS[roleOf(u.permissions)]}${u.name && u.name !== u.username ? ` (${u.name})` : ''}`);
  }
} else if (command === 'reset-password' && name) {
  try {
    const code = createResetCode(store, name.toLowerCase());
    console.log(`Code de réinitialisation pour ${name} : ${code}`);
    console.log('Valable 15 minutes. Sur la page de connexion : « Mot de passe oublié ? » › « J’ai un code ».');
    console.log('Toutes les sessions de ce compte seront fermées.');
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
} else {
  console.log('Usage :\n  npm run auth -- list\n  npm run auth -- reset-password <nom>');
  process.exit(command ? 1 : 0);
}
