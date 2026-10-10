# build_release.ps1 - builds the release trem_godot.dll without local paths in it
# and copies it to godot\bin\.
#
# Run it from anywhere:
#   powershell -ExecutionPolicy Bypass -File tools\build_release.ps1
#
# Panic locations and debug info carry the source path of every crate. Cargo
# passes workspace sources relative to the repo, but crates.io sources and
# generated files (target\...\out) by absolute path, which names the user
# folder. --remap-path-prefix rewrites those prefixes to /cargo and /src.
# Stable Cargo has no trim-paths yet, and .cargo/config.toml cannot read
# environment variables in rustflags, so the flags are set here.
#
# The remapped build uses its own target folder (target\dist) so it does not
# force a full rebuild of the normal target\release one, and vice versa.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$targetDir = Join-Path $root 'target\dist'

# CARGO_ENCODED_RUSTFLAGS: flags separated by 0x1f, so paths with spaces stay whole.
$flags = @(
    "--remap-path-prefix=$cargoHome=/cargo",
    "--remap-path-prefix=$root=/src"
)
$env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]0x1f

Push-Location $root
try {
    cargo build --release -p trem_godot --target-dir $targetDir
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
    Copy-Item (Join-Path $targetDir 'release\trem_godot.dll') 'godot\bin\trem_godot.dll' -Force
    $leak = Select-String -Path 'godot\bin\trem_godot.dll' -SimpleMatch -Pattern $env:USERNAME, $env:USERPROFILE -List
    if ($leak) { Write-Warning "godot\bin\trem_godot.dll still contains '$($leak.Pattern)'" }
    else { Write-Host "godot\bin\trem_godot.dll: built, no user paths inside" -ForegroundColor Green }
} finally {
    Pop-Location
    Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
}
