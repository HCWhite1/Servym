# Lance $env:SERVYNEX_CMDLINE dans une console cachée, sortie ajoutée à $env:SERVYNEX_LOG.
# Start-Process sort le projet du « job » de Servynex : il survit à un redémarrage
# de Servynex, et ses propres sous-processus héritent de la console cachée (aucune
# fenêtre ne s'ouvre). Écrit « SERVYNEX_PID=<pid> », attend la fin et renvoie son code de sortie.
#
# Invariant : SERVYNEX_CMDLINE est interprété par cmd.exe. Chaque argument y est entre
# guillemets (processes.js) et config.js garantit qu'il ne contient ni guillemet,
# ni %, ni retour à la ligne, ni & | < > ^ ( ). Ne pas relâcher l'un sans l'autre.
$ErrorActionPreference = 'Stop'
# chcp 65001 : les messages de cmd.exe (« commande introuvable ») arrivent en UTF-8.
$inner = 'chcp 65001 >nul & {0} 1>>"{1}" 2>&1' -f $env:SERVYNEX_CMDLINE, $env:SERVYNEX_LOG
$p = Start-Process -FilePath $env:ComSpec -ArgumentList "/d /s /c `"$inner`"" `
  -WorkingDirectory $env:SERVYNEX_CWD -WindowStyle Hidden -PassThru
$null = $p.Handle  # sans cela, ExitCode reste vide
# Préfixe explicite : processes.js ne lit que cette ligne, même si PowerShell écrit autre chose avant.
[Console]::Out.WriteLine("SERVYNEX_PID=$($p.Id)")
$p.WaitForExit()
exit $p.ExitCode
