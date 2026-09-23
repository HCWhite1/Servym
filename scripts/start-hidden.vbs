' Lance Servynex sans fenêtre (tâche planifiée « Servynex », à l'ouverture de session).
' Relance après un plantage, mais abandonne s'il plante en boucle (port déjà pris
' par une autre instance, projects.json invalide...) : voir logs\servynex.log.
' Code 75 : redémarrage demandé depuis l'interface, relancé aussitôt et non compté
' comme un plantage. SERVYNEX_SUPERVISOR indique à Servynex qu'il peut le demander.
Set sh = CreateObject("WScript.Shell")
Set fso = CreateObject("Scripting.FileSystemObject")
sh.CurrentDirectory = fso.GetParentFolderName(fso.GetParentFolderName(WScript.ScriptFullName))
sh.Environment("Process")("SERVYNEX_SUPERVISOR") = "vbs"
failures = 0
Do
  started = Now
  code = sh.Run("node server.js", 0, True)
  If code = 0 Then Exit Do
  If code <> 75 Then
    If DateDiff("s", started, Now) > 60 Then failures = 0 Else failures = failures + 1
    If failures >= 5 Then Exit Do
    WScript.Sleep 5000
  End If
Loop
