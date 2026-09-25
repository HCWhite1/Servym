// Secours en ligne de commande pour les comptes de Servym, à lancer sur la machine :
//   npm run auth -- list                   comptes existants
//   npm run auth -- reset-password <nom>   code de réinitialisation valable 15 minutes
// Avoir accès à cette machine donne de toute façon la main sur Servym : ces
// commandes n'ouvrent aucune porte nouvelle. Dernier recours : supprimer
// runtime/auth.json retire la protection (et tous les comptes).
import path from 'node:path';
import { AuthStore, createResetCode, roleOf } from '../src/auth.js';
import { ROOT_DIR, loadConfig } from '../src/config.js';
import { setServerLang, tr } from '../src/i18n.js';

// Même langue que le serveur : instance.lang, sinon celle du système.
try {
  setServerLang(loadConfig(path.join(ROOT_DIR, 'projects.json')).instance.lang);
} catch {}

const store = new AuthStore(path.join(ROOT_DIR, 'runtime', 'auth.json'));
const [command, name] = process.argv.slice(2);

if (store.error) {
  console.error(tr('auth.brokenFile', { error: store.error }));
  process.exit(1);
}
const data = store.load();

if (command === 'list') {
  if (!data.users.length) console.log(tr('cli.noAccounts'));
  else {
    console.log(tr(data.enabled ? 'cli.summaryOn' : 'cli.summaryOff', { count: data.users.length }));
    for (const u of data.users) console.log(`  ${u.username.padEnd(20)} ${tr(`role.${roleOf(u.permissions)}`)}${u.name && u.name !== u.username ? ` (${u.name})` : ''}`);
  }
} else if (command === 'reset-password' && name) {
  try {
    const code = createResetCode(store, name.toLowerCase());
    console.log(tr('cli.resetCode', { name, code }));
    console.log(tr('cli.resetHelp'));
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
} else {
  console.log(tr('cli.usage'));
  process.exit(command ? 1 : 0);
}
