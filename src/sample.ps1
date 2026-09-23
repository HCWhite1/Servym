# Relevé des processus pour src/stats.js, lancé une fois et gardé ouvert : à chaque
# ligne reçue, une ligne par processus « pid ppid début(ms) cpu(ms) mémoire(octets) »,
# puis « END ». Garder PowerShell ouvert évite de le redémarrer toutes les 5 secondes.
$ErrorActionPreference = 'SilentlyContinue'
$epoch = [datetime]::new(1970, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)
$out = [Console]::Out
while ($null -ne [Console]::In.ReadLine()) {
  foreach ($p in Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate, KernelModeTime, UserModeTime, WorkingSetSize) {
    $start = if ($p.CreationDate) { [long]($p.CreationDate.ToUniversalTime() - $epoch).TotalMilliseconds } else { 0 }
    $cpu = [long](([double]$p.KernelModeTime + [double]$p.UserModeTime) / 10000)
    $out.WriteLine(('{0} {1} {2} {3} {4}' -f $p.ProcessId, $p.ParentProcessId, $start, $cpu, $p.WorkingSetSize))
  }
  $out.WriteLine('END')
  $out.Flush()
}
