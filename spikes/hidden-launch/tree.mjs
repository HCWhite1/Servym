// Arborescence de test pour l'essai 1 : des sous-processus en mode console, comme un vrai projet.
// node → cmd /c ping (console) · node → php -S (serveur) · sortie en UTF-8 et sur stderr.
import { spawn } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const port = process.argv[2] ?? '8765';
const php = process.argv[3] ?? 'php';
const root = mkdtempSync(path.join(tmpdir(), 'servym-essai-'));
writeFileSync(path.join(root, 'index.php'), '<?php echo "ok";');

console.log('démarrage : accents éèà ✓');
console.error('sur stderr');
spawn('cmd', ['/c', 'ping -n 600 127.0.0.1 >nul'], { stdio: 'inherit' });
spawn(php, ['-S', `127.0.0.1:${port}`, '-t', root], { stdio: 'inherit' });
setInterval(() => console.log(`toujours là ${new Date().toISOString()}`), 2000);
