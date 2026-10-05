# agent_runner.ps1 - runs the jobs Claude queues for this project.
#
# Start it from the project root:
#   powershell -ExecutionPolicy Bypass -File tools\agent_runner.ps1
# Stop it with Ctrl+C.
#
# Claude writes a job file .agent\queue\<id>.ps1 into the project folder.
# This script runs it with the project root as the working directory,
# writes the output to .agent\out\<id>.log, then writes .agent\out\<id>.done
# containing the exit code. Each finished job is moved to .agent\ran\.
# Every job's command text is echoed here before it runs.

$ErrorActionPreference = 'Continue'
$root  = Split-Path -Parent $PSScriptRoot
$agent = Join-Path $root '.agent'
$queue = Join-Path $agent 'queue'
$out   = Join-Path $agent 'out'
$ran   = Join-Path $agent 'ran'
foreach ($d in @($agent, $queue, $out, $ran)) { New-Item -ItemType Directory -Force -Path $d | Out-Null }

# Make the usual tools reachable (cargo, gamedb).
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:TREM11_DED  = "C:\Program Files (x86)\Tremulous\tremulous.exe"
$env:TREM11_BASE = "C:\Program Files (x86)\Tremulous"
$env:GODOT = "C:\Users\Owner\Downloads\Godot\Godot_v4.7.2-stable_win64.exe\Godot_v4.7.2-stable_win64_console.exe"

Write-Host "agent runner: watching $queue (Ctrl+C to stop)" -ForegroundColor Cyan

while ($true) {
    $jobs = Get-ChildItem -Path $queue -Filter '*.ps1' -ErrorAction SilentlyContinue | Sort-Object Name
    foreach ($j in $jobs) {
        $id   = [IO.Path]::GetFileNameWithoutExtension($j.Name)
        $log  = Join-Path $out "$id.log"
        $done = Join-Path $out "$id.done"
        Start-Sleep -Milliseconds 300   # let the writer finish the file
        $text = Get-Content -Raw -Path $j.FullName
        Write-Host ""
        Write-Host "=== job $id  $(Get-Date -Format HH:mm:ss)" -ForegroundColor Yellow
        Write-Host $text -ForegroundColor DarkGray
        Push-Location $root
        $code = 0
        try {
            $sw = [Diagnostics.Stopwatch]::StartNew()
            & powershell -NoProfile -ExecutionPolicy Bypass -File $j.FullName *>&1 |
                ForEach-Object { "$_" } | Tee-Object -FilePath $log | Out-Null
            $code = $LASTEXITCODE
            "`n[agent] exit $code in $([int]$sw.Elapsed.TotalSeconds)s" | Add-Content -Path $log
        } catch {
            "$_" | Add-Content -Path $log
            $code = 1
        } finally {
            Pop-Location
        }
        Set-Content -Path $done -Value $code
        Move-Item -Force -Path $j.FullName -Destination (Join-Path $ran $j.Name)
        Write-Host "=== job $id finished (exit $code)" -ForegroundColor Yellow
    }
    Start-Sleep -Seconds 1
}
