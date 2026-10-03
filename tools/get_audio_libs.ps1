# Fetches the native audio libraries the FMOD + Steam Audio sound path needs:
#  - FMOD Engine (fmod.dll, headers, import library) from the local FMOD install
#    (download "FMOD Engine" 2.03 for Windows from fmod.com; it needs an account)
#  - Steam Audio 4.8.1 (phonon.dll) and its FMOD plugin (phonon_fmod.dll) from
#    Valve's GitHub releases (Apache-2.0)
# Run from the project root:  powershell -ExecutionPolicy Bypass -File tools\get_audio_libs.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$fmod = "C:\Program Files (x86)\FMOD SoundSystem\FMOD Studio API Windows\api\core"
if ($env:FMOD_DIR) { $fmod = "$env:FMOD_DIR\api\core" }
New-Item -ItemType Directory -Force "$root\third_party\fmod\inc", "$root\third_party\fmod\lib" | Out-Null
Copy-Item "$fmod\inc\*.h" "$root\third_party\fmod\inc" -Force
Copy-Item "$fmod\lib\x64\fmod_vc.lib", "$fmod\lib\x64\fmod.dll" "$root\third_party\fmod\lib" -Force
Copy-Item "$fmod\lib\x64\fmod.dll" "$root\godot\bin" -Force
$ver = "4.8.1"
$tmp = Join-Path $env:TEMP "steamaudio_fmod_$ver.zip"
if (-not (Test-Path $tmp)) {
    Invoke-WebRequest "https://github.com/ValveSoftware/steam-audio/releases/download/v$ver/steamaudio_fmod_$ver.zip" -OutFile $tmp
}
$x = Join-Path $env:TEMP "steamaudio_fmod_$ver"
if (-not (Test-Path $x)) { Expand-Archive $tmp -DestinationPath $x -Force }
Copy-Item "$x\steamaudio_fmod\lib\windows-x64\phonon.dll", "$x\steamaudio_fmod\lib\windows-x64\phonon_fmod.dll" "$root\godot\bin" -Force
Get-ChildItem "$root\godot\bin" | Select-Object Name, Length
