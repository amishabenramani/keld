# GH-814 experiment: per-process CPU/IO and running-task sampler (2 s period).
param([string]$Out)
$wv = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
$edge = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{56EB18F8-B008-4CBD-B6D2-8C97FE7E9062}'
$i = 0
& { while ($true) {
  $t = (Get-Date).ToUniversalTime().ToString('HH:mm:ss')
  try {
    $p = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -ErrorAction Stop | Where-Object { $_.Name -ne '_Total' -and $_.Name -ne 'Idle' }
    $p | Sort-Object PercentProcessorTime -Descending | Select-Object -First 6 | Where-Object PercentProcessorTime -ge 10 |
      ForEach-Object { "$t,cpu,$($_.Name),$($_.PercentProcessorTime),$($_.IODataBytesPersec)" }
    $p | Sort-Object IODataBytesPersec -Descending | Select-Object -First 6 | Where-Object IODataBytesPersec -ge 2000000 |
      ForEach-Object { "$t,io,$($_.Name),$($_.PercentProcessorTime),$($_.IODataBytesPersec)" }
  } catch { "$t,err,$($_.Exception.Message)" }
  if ($i % 5 -eq 0) {
    try { Get-ScheduledTask -ErrorAction Stop | Where-Object State -eq 'Running' | ForEach-Object { "$t,task,$($_.TaskPath)$($_.TaskName)" } } catch { "$t,err,task" }
    $a = (Get-ItemProperty $wv -ErrorAction SilentlyContinue).pv; $b = (Get-ItemProperty $edge -ErrorAction SilentlyContinue).pv
    "$t,ver,webview2=$a,edge=$b"
    Get-Process MicrosoftEdgeUpdate*, setup, mscorsvw, ngen*, TiWorker, TrustedInstaller, MsMpEng, MpCmdRun, CompatTelRunner, SearchIndexer -ErrorAction SilentlyContinue |
      ForEach-Object { "$t,proc,$($_.ProcessName),$([int]$_.CPU)" }
  }
  $i++
  Start-Sleep -Seconds 2
} } *>> $Out
