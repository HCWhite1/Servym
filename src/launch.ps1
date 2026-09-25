# Lance $env:SERVYM_CMDLINE dans une console cachée, sortie ajoutée à $env:SERVYM_LOG.
# Start-Process sort le projet du « job » de Servym : il survit à un redémarrage
# de Servym, et ses propres sous-processus héritent de la console cachée (aucune
# fenêtre ne s'ouvre). Écrit « SERVYM_PID=<pid> », attend la fin et renvoie son code de sortie.
#
# Invariant : SERVYM_CMDLINE est interprété par cmd.exe. Chaque argument y est entre
# guillemets (processes.js) et config.js garantit qu'il ne contient ni guillemet,
# ni %, ni retour à la ligne, ni & | < > ^ ( ). Ne pas relâcher l'un sans l'autre.
$ErrorActionPreference = 'Stop'
# chcp 65001 : les messages de cmd.exe (« commande introuvable ») arrivent en UTF-8.
$inner = 'chcp 65001 >nul & {0} 1>>"{1}" 2>&1' -f $env:SERVYM_CMDLINE, $env:SERVYM_LOG
$p = Start-Process -FilePath $env:ComSpec -ArgumentList "/d /s /c `"$inner`"" `
  -WorkingDirectory $env:SERVYM_CWD -WindowStyle Hidden -PassThru
$null = $p.Handle  # sans cela, ExitCode reste vide
# Préfixe explicite : processes.js ne lit que cette ligne, même si PowerShell écrit autre chose avant.
[Console]::Out.WriteLine("SERVYM_PID=$($p.Id)")
$p.WaitForExit()
exit $p.ExitCode
